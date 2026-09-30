//! The edit catalog (feedback time per kind of edit, weighted by the project's history) and
//! planted bugs (a fault in changed code must fail the selected run whenever it fails the full
//! run).
use crate::{timing, timing::Timed, Config, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

const USAGE: &str = "sieve catalog --workspace PATH --catalog FILE [--samples N] [--commits N]\n\
    [--output DIR] [--plant] [--levers] [--in-place] [--executable PATH]\n\
    [--config IMPACT_JSON] [--history REPOSITORY] [-- BUILD_ARGS]\n\
    sieve classify --workspace PATH [--commits N]\n\n\
    Applies each catalog edit to HEAD, times `sieve run` against a native full build and\n\
    `sieve run --full`, and weights edit kinds by the last N first-parent commits.\n\
    --plant also plants a bug in each edit and reports missed failures (exit 1).\n\
    --levers times each edit again with each speed-up switched off.\n\
    Runs in a temporary copy unless --in-place is given; --config replaces the copy's\n\
    impact.json, and --history names the repository whose commits are classified.";

/// Edit kinds, from the most to the least expensive change a commit can contain.
pub const KINDS: &[&str] = &[
    "pom",
    "rename",
    "new-class",
    "structural",
    "config",
    "listener-body",
    "body",
    "resource",
    "new-test",
    "test-config",
    "fixture",
    "test",
    "other",
    "docs",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    #[serde(default)]
    samples: Option<usize>,
    /// Appended to every build, for example to skip report plugins.
    #[serde(default)]
    build_args: Vec<String>,
    edits: Vec<Edit>,
}

#[derive(Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
struct Edit {
    name: String,
    kind: String,
    /// Workspace-relative file.
    file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    search: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    replace: Option<String>,
    /// Creates `file` with this content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    /// Moves `file` here, then applies `search`/`replace` to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rename_to: Option<String>,
}

/// Original contents of edited files, kept in `.sieve/restore.json` until they are restored,
/// so that an interrupted run is undone by the next one.
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Journal {
    /// Path to original bytes; `None` for a file that did not exist.
    files: BTreeMap<String, Option<Vec<u8>>>,
}

impl Journal {
    fn path(workspace: &Path) -> PathBuf {
        workspace.join(".sieve").join("restore.json")
    }

    fn save(&self, workspace: &Path) -> Result<()> {
        let dir = workspace.join(".sieve");
        fs::create_dir_all(&dir)?;
        // Untracked files would count as changes.
        if !dir.join(".gitignore").is_file() {
            fs::write(dir.join(".gitignore"), "*\n")?;
        }
        fs::write(Self::path(workspace), serde_json::to_vec(self)?)?;
        Ok(())
    }

    /// Records a file before its first change.
    fn keep(&mut self, workspace: &Path, file: &str) -> Result<()> {
        if !self.files.contains_key(file) {
            self.files
                .insert(file.to_owned(), fs::read(workspace.join(file)).ok());
            self.save(workspace)?;
        }
        Ok(())
    }

    fn restore(&mut self, workspace: &Path) -> Result<()> {
        for (file, bytes) in std::mem::take(&mut self.files) {
            let path = workspace.join(&file);
            match bytes {
                Some(bytes) => {
                    fs::create_dir_all(path.parent().ok_or("No parent")?)?;
                    fs::write(path, bytes)?;
                }
                None => {
                    let _ = fs::remove_file(path);
                }
            }
        }
        let _ = fs::remove_file(Self::path(workspace));
        Ok(())
    }

    /// Undoes what an interrupted run left behind.
    pub fn recover(workspace: &Path) -> Result<bool> {
        let path = Self::path(workspace);
        let Ok(bytes) = fs::read(&path) else {
            return Ok(false);
        };
        let mut journal: Journal = serde_json::from_slice(&bytes)?;
        journal.restore(workspace)?;
        Ok(true)
    }
}

/// Line numbers (0-based, inclusive) of the text an edit wrote.
pub(crate) type Region = (String, usize, usize);

/// Local-mode records and run state, saved before an edit and restored after it, so that
/// what one edit recorded, such as a new test's record, cannot decide a later measurement.
struct State(Option<tempfile::TempDir>);

impl State {
    /// `last-run.json` stays as the edit left it, so that paths the edit created or moved still
    /// clean stale build output afterwards.
    const SAVED: &'static [&'static str] = &["records", "snapshots"];

    fn save(workspace: &Path) -> Result<Self> {
        let dir = workspace.join(".sieve");
        if !dir.join("records").is_dir() {
            return Ok(Self(None));
        }
        let temp = tempfile::tempdir()?;
        for name in Self::SAVED {
            copy_any(&dir.join(name), &temp.path().join(name))?;
        }
        Ok(Self(Some(temp)))
    }

    fn restore(&self, workspace: &Path) -> Result<()> {
        let Some(temp) = &self.0 else {
            return Ok(());
        };
        let dir = workspace.join(".sieve");
        for name in Self::SAVED {
            let target = dir.join(name);
            if target.is_dir() {
                fs::remove_dir_all(&target)?;
            } else if target.exists() {
                fs::remove_file(&target)?;
            }
            copy_any(&temp.path().join(name), &target)?;
        }
        Ok(())
    }
}

/// Copies a file or a directory tree; a missing source copies nothing.
fn copy_any(from: &Path, to: &Path) -> Result<()> {
    if from.is_dir() {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_any(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else if from.is_file() {
        fs::copy(from, to)?;
    }
    Ok(())
}

fn apply(workspace: &Path, edit: &Edit, journal: &mut Journal) -> Result<Region> {
    let mut file = edit.file.clone();
    if let Some(content) = &edit.content {
        if workspace.join(&file).exists() {
            return Err(format!("{}: {file} exists already", edit.name).into());
        }
        journal.keep(workspace, &file)?;
        let path = workspace.join(&file);
        fs::create_dir_all(path.parent().ok_or("No parent")?)?;
        fs::write(path, content)?;
        return Ok((file, 0, content.lines().count().saturating_sub(1)));
    }
    if let Some(to) = &edit.rename_to {
        journal.keep(workspace, &file)?;
        journal.keep(workspace, to)?;
        let text =
            fs::read(workspace.join(&file)).map_err(|e| format!("{}: {file}: {e}", edit.name))?;
        let target = workspace.join(to);
        fs::create_dir_all(target.parent().ok_or("No parent")?)?;
        fs::write(target, text)?;
        fs::remove_file(workspace.join(&file))?;
        file = to.clone();
    }
    let path = workspace.join(&file);
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {file}: {e}", edit.name))?;
    match (&edit.search, &edit.replace) {
        (Some(search), Some(replace)) => {
            let at = text
                .find(search.as_str())
                .ok_or_else(|| format!("{}: {file} does not contain {search:?}", edit.name))?;
            journal.keep(workspace, &file)?;
            fs::write(&path, text.replacen(search.as_str(), replace, 1))?;
            let start = text[..at].matches('\n').count();
            Ok((file, start, start + replace.matches('\n').count()))
        }
        (None, None) if edit.rename_to.is_some() => {
            Ok((file, 0, text.lines().count().saturating_sub(1)))
        }
        _ => Err(format!(
            "{}: give search and replace, content, or rename_to",
            edit.name
        )
        .into()),
    }
}

/// Mutants of the lines in `start..=end`, most promising first: a negated condition, a
/// flipped comparison, swapped arithmetic, a flipped boolean, a default return value, and a
/// dropped call.
pub fn mutants(text: &str, start: usize, end: usize) -> Vec<(String, usize, String)> {
    let lines: Vec<&str> = text.split('\n').collect();
    let code = |line: &str| {
        let t = line.trim();
        !(t.is_empty()
            || t.starts_with("//")
            || t.starts_with('*')
            || t.starts_with("/*")
            || t.starts_with("import ")
            || t.starts_with("package ")
            || t.starts_with('@'))
    };
    type Operator = fn(&str) -> Option<(String, String)>;
    let operators: &[Operator] = &[
        |l| {
            let at = l.find("if (")? + 3;
            let mut depth = 0;
            for (i, c) in l[at..].char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            let close = at + i;
                            let text =
                                format!("{}(!{}){}", &l[..at], &l[at..=close], &l[close + 1..]);
                            return Some(("negated condition".into(), text));
                        }
                    }
                    _ => {}
                }
            }
            None
        },
        |l| swap(l, &[(" == ", " != "), (" != ", " == ")], "flipped equality"),
        |l| {
            swap(
                l,
                &[
                    (" < ", " >= "),
                    (" > ", " <= "),
                    (" <= ", " > "),
                    (" >= ", " < "),
                ],
                "flipped comparison",
            )
        },
        |l| {
            if !(l.contains("return ") || l.contains(" = ")) {
                return None;
            }
            swap(
                l,
                &[(" + ", " - "), (" - ", " + "), (" * ", " / ")],
                "swapped arithmetic",
            )
        },
        |l| {
            swap(
                l,
                &[("true", "false"), ("false", "true")],
                "flipped boolean",
            )
        },
        |l| default_return(l, "null"),
        |l| default_return(l, "0"),
        |l| default_return(l, "false"),
        |l| {
            let t = l.trim();
            let call = (t.ends_with(");") || (t.ends_with(')') && !t.contains('{')))
                && t.contains('(')
                && t.chars().next().is_some_and(|c| c.is_alphabetic())
                && !t.split('(').next().unwrap_or("").contains([' ', '='])
                && ![
                    "return", "throw", "super(", "this(", "if", "while", "for", "fun ",
                ]
                .iter()
                .any(|k| t.starts_with(k));
            call.then(|| {
                let indent = &l[..l.len() - l.trim_start().len()];
                (
                    "dropped call".into(),
                    format!("{indent}// sieve planted bug: dropped {t}"),
                )
            })
        },
    ];
    let mut out = Vec::new();
    let end = end.min(lines.len().saturating_sub(1));
    for operator in operators {
        for (i, line) in lines.iter().enumerate().take(end + 1).skip(start) {
            if !code(line) {
                continue;
            }
            if let Some((label, mutated)) = operator(line) {
                let mut copy = lines.clone();
                copy[i] = &mutated;
                out.push((label, i, copy.join("\n")));
            }
        }
    }
    out
}

/// `return value;` becomes `return default;`, unless it returns a literal already.
fn default_return(line: &str, default: &str) -> Option<(String, String)> {
    let t = line.trim();
    let value = t.strip_prefix("return ")?.strip_suffix(';')?.trim();
    let literal = ["null", "0", "false", "true", "\"\""].contains(&value);
    (!literal).then(|| {
        let indent = &line[..line.len() - line.trim_start().len()];
        (
            "returned a default".into(),
            format!("{indent}return {default};"),
        )
    })
}

/// Replaces the first of `pairs` found in `line`, matching whole words for alphabetic ones.
fn swap(line: &str, pairs: &[(&str, &str)], label: &str) -> Option<(String, String)> {
    for (from, to) in pairs {
        let mut at = 0;
        while let Some(i) = line[at..].find(from).map(|i| at + i) {
            let word = from.chars().all(char::is_alphabetic);
            let before = line[..i].chars().next_back();
            let after = line[i + from.len()..].chars().next();
            let ident = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
            let quoted = line[..i].matches('"').count() % 2 == 1;
            if !quoted && (!word || !ident(before) && !ident(after)) {
                return Some((
                    label.to_owned(),
                    format!("{}{to}{}", &line[..i], &line[i + from.len()..]),
                ));
            }
            at = i + from.len();
        }
    }
    None
}

/// Plants a bug in `region` and compares the selected run with the full run. `selected` and
/// `full` run the build; the file is restored before returning.
pub fn plant(
    workspace: &Path,
    region: &Region,
    journal: &mut Journal,
    selected: &mut dyn FnMut(&str) -> Result<Timed>,
    full: &mut dyn FnMut(&str) -> Result<Timed>,
) -> Result<Value> {
    let (file, start, end) = region;
    let path = workspace.join(file);
    let original = fs::read_to_string(&path)?;
    let mut candidates = mutants(&original, *start, *end);
    if candidates.is_empty() {
        candidates = mutants(&original, 0, usize::MAX);
    }
    // The shared journal, so that an interruption restores the edit and the planted bug alike.
    let added = !journal.files.contains_key(file.as_str());
    journal.keep(workspace, file)?;
    let done = |journal: &mut Journal| -> Result<()> {
        fs::write(&path, &original)?;
        if added {
            journal.files.remove(file.as_str());
            journal.save(workspace)?;
        }
        Ok(())
    };
    let mut tried = Vec::new();
    for (label, line, text) in candidates.into_iter().take(8) {
        fs::write(&path, &text)?;
        let tag = format!("{label} at {file}:{}", line + 1);
        let chosen = selected(&tag);
        let chosen = match chosen {
            Ok(timed) if timed.compile_error => {
                tried.push(tag);
                continue;
            }
            other => other,
        };
        let result = chosen.and_then(|chosen| Ok((chosen, full(&tag)?)));
        done(journal)?;
        let (chosen, reference) = result?;
        let missed = missed(&reference, &chosen);
        return Ok(json!({
            "mutant": tag,
            "not_compiling": tried,
            "full_failed": reference.failed,
            "selected_failed": chosen.failed,
            "detected": !reference.failed.is_empty() || reference.exit != Some(0),
            "missed": missed,
        }));
    }
    done(journal)?;
    Ok(json!({"mutant": null, "not_compiling": tried}))
}

/// Tests failing in the reference run but not in the selected one. A reference build that
/// failed while the selected one passed, or unreadable reports, count as a missed failure too.
pub(crate) fn missed(reference: &Timed, selected: &Timed) -> BTreeSet<String> {
    let mut missed: BTreeSet<String> = reference
        .failed
        .difference(&selected.failed)
        .cloned()
        .collect();
    if reference.exit != Some(0) && selected.exit == Some(0) {
        missed.insert("<the reference build failed; the selected build passed>".into());
    }
    for (run, timed) in [("reference", reference), ("selected", selected)] {
        if let Some(error) = &timed.report_error {
            missed.insert(format!("<unreadable {run} reports: {error}>"));
        }
    }
    missed
}

/// Kinds of the commits' changed paths, one kind per commit: the most expensive one, and
/// whether the commit changed POMs only, as automated dependency updates do.
fn classify_commit(
    root: &Path,
    prefix: &Path,
    commit: &str,
    config: &Config,
) -> Result<(&'static str, bool)> {
    let git = |args: &[&str]| crate::replay::git(root, args);
    let listing = git(&[
        "diff-tree",
        "-r",
        "-M",
        "--no-commit-id",
        "--name-status",
        &format!("{commit}^"),
        commit,
    ])?;
    let mut best = KINDS.len() - 1;
    let mut pom_only = !listing.trim().is_empty();
    for line in listing.lines() {
        let mut fields = line.split('\t');
        let status = fields.next().unwrap_or("");
        let Some(path) = fields.next_back() else {
            continue;
        };
        pom_only &= path.rsplit('/').next() == Some("pom.xml");
        let Ok(path) = Path::new(path).strip_prefix(prefix) else {
            // Shared inputs outside the project, such as a parent POM or CI scripts.
            best = best.min(0);
            continue;
        };
        let path = path.to_string_lossy().replace('\\', "/");
        let full = prefix.join(&path).to_string_lossy().replace('\\', "/");
        let kind = kind_of(&path, status, config, || {
            let parent = format!("{commit}^");
            let diff = git(&["diff", "-U0", &parent, commit, "--", &full]).unwrap_or_default();
            let text = git(&["show", &format!("{commit}:{full}")]).unwrap_or_default();
            (diff, text)
        });
        best = best.min(
            KINDS
                .iter()
                .position(|k| *k == kind)
                .unwrap_or(KINDS.len() - 1),
        );
    }
    Ok((KINDS[best], pom_only))
}

/// The kind of one changed path. `contents` yields the path's diff and new text on demand.
fn kind_of(
    path: &str,
    status: &str,
    config: &Config,
    contents: impl FnOnce() -> (String, String),
) -> &'static str {
    let name = path.rsplit('/').next().unwrap_or(path);
    let extension = name.rsplit_once('.').map_or("", |(_, e)| e);
    if status.starts_with('R') {
        return "rename";
    }
    if config.ignored(path, "") || matches!(extension, "md" | "txt" | "adoc") {
        return "docs";
    }
    let build = [
        "pom.xml",
        "impact.json",
        "build.gradle",
        "build.gradle.kts",
        "settings.gradle",
        "gradle.properties",
    ];
    if build.contains(&name) || path.starts_with(".mvn/") || path.starts_with("gradle/") {
        return "pom";
    }
    let config_file = matches!(extension, "properties" | "yml" | "yaml");
    let source = matches!(extension, "java" | "kt");
    if path.contains("src/test/") {
        return match () {
            _ if source && status == "A" => "new-test",
            _ if source => "test",
            _ if config_file => "test-config",
            _ => "fixture",
        };
    }
    if path.contains("src/main/") {
        if !source {
            let startup = name.starts_with("application") || name.starts_with("bootstrap");
            return if config_file && startup {
                "config"
            } else {
                "resource"
            };
        }
        if status == "A" {
            return "new-class";
        }
        if status == "D" {
            return "structural";
        }
        let (diff, text) = contents();
        if diff.lines().any(|l| {
            (l.starts_with('+') || l.starts_with('-'))
                && !l.starts_with("+++")
                && !l.starts_with("---")
                && declaration(&l[1..])
        }) {
            return "structural";
        }
        let listener = [
            "@KafkaListener",
            "@EventListener",
            "@RabbitListener",
            "@JmsListener",
            "@SqsListener",
            "@StreamListener",
        ];
        return if listener.iter().any(|a| text.contains(a)) {
            "listener-body"
        } else {
            "body"
        };
    }
    "other"
}

/// Whether a changed source line declares something: an annotation, a type, a method
/// signature, or a field.
fn declaration(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty()
        || t.starts_with("//")
        || t.starts_with('*')
        || t.starts_with("/*")
        || t.starts_with("import ")
    {
        return false;
    }
    if t.starts_with('@') {
        return true;
    }
    let modifiers = [
        "public ",
        "protected ",
        "private ",
        "static ",
        "final ",
        "abstract ",
        "default ",
        "override ",
        "fun ",
        "val ",
        "var ",
        "open ",
        "internal ",
    ];
    let types = ["class ", "interface ", "enum ", "record ", "object "];
    let modified = modifiers.iter().any(|m| t.starts_with(m));
    modified
        || types
            .iter()
            .any(|k| t.starts_with(k) || t.contains(&format!(" {k}")) && modified)
}

/// Edit-kind weights of the last `commits` first-parent commits.
pub fn classify(workspace: &Path, commits: usize, config: &Config) -> Result<Value> {
    let root = PathBuf::from(crate::replay::git(
        workspace,
        &["rev-parse", "--show-toplevel"],
    )?)
    .canonicalize()?;
    let prefix = workspace.strip_prefix(&root)?.to_owned();
    let history = crate::replay::git(
        &root,
        &[
            "rev-list",
            "--first-parent",
            "-n",
            &commits.to_string(),
            "HEAD",
        ],
    )?;
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut without: BTreeMap<&str, usize> = BTreeMap::new();
    let mut per_commit = Vec::new();
    for commit in history.lines() {
        let Ok((kind, pom_only)) = classify_commit(&root, &prefix, commit, config) else {
            continue;
        };
        *counts.entry(kind).or_default() += 1;
        if !pom_only {
            *without.entry(kind).or_default() += 1;
        }
        per_commit.push(json!({"commit": commit, "kind": kind, "pom_only": pom_only}));
    }
    let weights = |counts: &BTreeMap<&'static str, usize>| -> BTreeMap<&'static str, f64> {
        let total: usize = counts.values().sum();
        counts
            .iter()
            .map(|(k, c)| (*k, *c as f64 / total.max(1) as f64))
            .collect()
    };
    Ok(json!({
        "commits": counts.values().sum::<usize>(),
        "pom_only": counts.values().sum::<usize>() - without.values().sum::<usize>(),
        "counts": counts,
        "weights": weights(&counts),
        "weights_without_pom_only": weights(&without),
        "per_commit": per_commit,
    }))
}

/// Writes `config` as the copy's `impact.json`, with the Maven adapter that class- and
/// module-level selection need and a current build fingerprint.
fn configure(copy: &Path, config: &Config) -> Result<()> {
    let mut config: Config = serde_json::from_value(serde_json::to_value(config)?)?;
    config.validate(copy)?;
    if config.tool == "maven" && config.records != Some(true) {
        for module in config.modules.keys() {
            let pom = copy.join(module).join("pom.xml");
            let xml = fs::read_to_string(&pom)?;
            fs::write(
                &pom,
                crate::setup::install_maven_adapter(&xml, module, true)?,
            )?;
        }
    }
    config.build_fingerprint = Some(crate::fingerprint::build_inputs(copy)?);
    fs::write(
        copy.join("impact.json"),
        serde_json::to_string_pretty(&config)? + "\n",
    )?;
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if matches!(
            name.to_str(),
            Some("target" | ".sieve" | ".git" | "build" | ".gradle" | ".idea" | "node_modules")
        ) {
            continue;
        }
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_tree(&entry.path(), &to.join(name))?;
        } else if kind.is_file() {
            fs::copy(entry.path(), to.join(name))?;
        }
    }
    Ok(())
}

pub(crate) struct Runner<'a> {
    pub(crate) workspace: &'a Path,
    pub(crate) tool: String,
    pub(crate) executable: Option<String>,
    pub(crate) extra: Vec<String>,
    pub(crate) logs: PathBuf,
    pub(crate) count: usize,
    /// Planted-bug runs ignore test failures, so that every suite runs to completion.
    pub(crate) planting: bool,
}

impl Runner<'_> {
    fn log(&mut self, label: &str) -> PathBuf {
        self.count += 1;
        let safe: String = label
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect();
        self.logs.join(format!("{:04}-{safe}.log", self.count))
    }

    /// `sieve run` with `options`, timed.
    pub(crate) fn sieve(&mut self, label: &str, options: &[&str]) -> Result<Timed> {
        let log = self.log(label);
        let mut command = Command::new(env::current_exe()?);
        command
            .arg("run")
            .arg("--workspace")
            .arg(self.workspace)
            .args(options);
        if let Some(executable) = &self.executable {
            command.args(["--executable", executable]);
        }
        command.arg("--").args(&self.extra);
        if self.planting {
            // Every suite runs, so that failures in later ones are compared too.
            command.arg("-Dmaven.test.failure.ignore=true");
        }
        timing::run(&mut command, &log, self.workspace, &self.tool)
    }

    /// The build tool's own full build on the same tree, without `clean`.
    pub(crate) fn native(&mut self, label: &str) -> Result<Timed> {
        let log = self.log(label);
        let executable = self
            .executable
            .clone()
            .unwrap_or_else(|| crate::setup::default_executable(self.workspace, &self.tool));
        let mut command = Command::new(executable);
        command.current_dir(self.workspace);
        if self.tool == "maven" {
            command.args(["-B", "-ntp", "verify"]);
        } else {
            command.args(["--console=plain", "check"]);
        }
        command.args(&self.extra);
        if self.planting {
            command.arg("-Dmaven.test.failure.ignore=true");
        }
        timing::run(&mut command, &log, self.workspace, &self.tool)
    }
}

fn median(runs: &[Timed]) -> f64 {
    timing::stats(&runs.iter().map(|r| r.seconds).collect::<Vec<_>>())["median"]
        .as_f64()
        .unwrap_or(f64::NAN)
}

fn summary(runs: &[Timed]) -> Value {
    let mut phases: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for run in runs {
        for (name, seconds) in &run.phases {
            phases.entry(name.clone()).or_default().push(*seconds);
        }
    }
    let phase_medians: BTreeMap<String, Value> = phases
        .into_iter()
        .map(|(name, samples)| (name, timing::stats(&samples)["median"].clone()))
        .collect();
    json!({
        "seconds": timing::stats(&runs.iter().map(|r| r.seconds).collect::<Vec<_>>()),
        "phases": phase_medians,
        "context_seconds": timing::stats(&runs.iter().map(|r| r.context_seconds).collect::<Vec<_>>())["median"],
        "container_seconds": timing::stats(&runs.iter().map(|r| r.container_seconds).collect::<Vec<_>>())["median"],
        "cases": runs.last().map(|r| r.cases),
        "exits": runs.iter().map(|r| r.exit).collect::<Vec<_>>(),
        "runs": runs,
    })
}

pub fn classify_main(args: Vec<String>) -> Result<u8> {
    let mut workspace = None;
    let mut commits = 50;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("Missing value for {arg}"))?;
        match arg.as_str() {
            "--workspace" => workspace = Some(PathBuf::from(value)),
            "--commits" => commits = value.parse()?,
            _ => return Err(format!("Unknown option: {arg}\n{USAGE}").into()),
        }
    }
    let workspace = workspace.ok_or("--workspace is required")?.canonicalize()?;
    // Classification only needs ignore patterns, so projects without Sieve use the defaults.
    let config = match workspace.join("impact.json").is_file() {
        true => Config::read(&workspace)?,
        false => serde_json::from_value(json!({"tool": "maven", "modules": {".": []}}))?,
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&classify(&workspace, commits, &config)?)?
    );
    Ok(0)
}

pub fn main(args: Vec<String>) -> Result<u8> {
    let (mut workspace, mut catalog_path, mut output) = (None, None, None);
    let (mut samples, mut commits, mut executable) = (None, 50, None);
    let (mut plant_bugs, mut compare_levers, mut in_place) = (false, false, false);
    let (mut config_path, mut history_path) = (None, None);
    let mut extra = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--" => {
                extra.extend(args.by_ref());
                break;
            }
            "--plant" => plant_bugs = true,
            "--levers" => compare_levers = true,
            "--in-place" => in_place = true,
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(0);
            }
            _ => {
                let value = args
                    .next()
                    .ok_or_else(|| format!("Missing value for {arg}"))?;
                match arg.as_str() {
                    "--workspace" => workspace = Some(PathBuf::from(value)),
                    "--catalog" => catalog_path = Some(PathBuf::from(value)),
                    "--output" => output = Some(PathBuf::from(value)),
                    "--samples" => samples = Some(value.parse()?),
                    "--commits" => commits = value.parse()?,
                    "--executable" => executable = Some(value),
                    "--config" => config_path = Some(PathBuf::from(value)),
                    "--history" => history_path = Some(PathBuf::from(value)),
                    _ => return Err(format!("Unknown option: {arg}\n{USAGE}").into()),
                }
            }
        }
    }
    let original = workspace.ok_or("--workspace is required")?.canonicalize()?;
    let catalog_path = catalog_path.ok_or("--catalog is required")?;
    let catalog: Catalog = serde_json::from_slice(&fs::read(&catalog_path)?)
        .map_err(|e| format!("{}: {e}", catalog_path.display()))?;
    if let Some(edit) = catalog
        .edits
        .iter()
        .find(|e| !KINDS.contains(&e.kind.as_str()))
    {
        return Err(format!(
            "{}: unknown kind {:?}; known: {}",
            edit.name,
            edit.kind,
            KINDS.join(", ")
        )
        .into());
    }
    if extra
        .iter()
        .chain(&catalog.build_args)
        .any(|a| a == "-q" || a == "--quiet")
    {
        return Err("Quiet builds hide the phase markers; drop -q".into());
    }
    let samples = samples.or(catalog.samples).unwrap_or(3).max(1);
    // A configuration from outside lets a project be measured before it adopts Sieve.
    let config: Config = match &config_path {
        Some(path) if in_place => {
            return Err(format!("--config {} needs a copy; drop --in-place", path.display()).into())
        }
        Some(path) => serde_json::from_slice(&fs::read(path)?)
            .map_err(|e| format!("{}: {e}", path.display()))?,
        None => Config::read(&original)?,
    };
    let output = output.unwrap_or_else(|| PathBuf::from("validation-results/catalog"));
    let logs = output.join("logs");
    fs::create_dir_all(&logs)?;
    let history_root = match &history_path {
        Some(path) => path.canonicalize()?,
        None => original.clone(),
    };
    let history = classify(&history_root, commits, &config).unwrap_or_else(|error| {
        eprintln!("History classification unavailable, weighting kinds equally: {error}");
        json!({"commits": 0, "weights": {}, "error": error.to_string()})
    });
    let temp = tempfile::Builder::new()
        .prefix("sieve-catalog-")
        .tempdir()?;
    let workspace = if in_place {
        if Journal::recover(&original)? {
            eprintln!("Restored files that an interrupted catalog run had changed");
        }
        original.clone()
    } else {
        let copy = temp.path().join("workspace");
        copy_tree(&original, &copy)?;
        if config_path.is_some() {
            configure(&copy, &config)?;
        }
        crate::replay::git(&copy, &["init", "-q"])?;
        crate::replay::git(&copy, &["add", "-A"])?;
        crate::replay::git(
            &copy,
            &["commit", "-q", "--no-verify", "-m", "catalog base"],
        )?;
        copy.canonicalize()?
    };
    extra.extend(catalog.build_args.iter().cloned());
    let mut runner = Runner {
        workspace: &workspace,
        tool: config.tool.clone(),
        executable,
        extra,
        logs,
        count: 0,
        planting: false,
    };
    let base: &[&str] = &["--base", "HEAD"];
    eprintln!("catalog: warming up {}", workspace.display());
    let warm = runner.sieve("warm-full", &["--full"])?;
    runner.sieve("warm-settle", base)?;
    let mut native = Vec::new();
    let mut full = Vec::new();
    let mut edits: Vec<Vec<Timed>> = vec![Vec::new(); catalog.edits.len()];
    let mut selections: Vec<Value> = vec![Value::Null; catalog.edits.len()];
    let mut journal = Journal::default();
    let selection_file = temp.path().join("selection.json");
    let selection_arg = selection_file.to_str().ok_or("Non-UTF-8 path")?.to_owned();
    for sample in 1..=samples {
        eprintln!("catalog: sample {sample}/{samples}");
        native.push(runner.native("native")?);
        full.push(runner.sieve("full", &["--full"])?);
        runner.sieve("settle", base)?;
        for (i, edit) in catalog.edits.iter().enumerate() {
            eprintln!("catalog: {} ({})", edit.name, edit.kind);
            let saved = State::save(&workspace)?;
            let applied = apply(&workspace, edit, &mut journal);
            let timed = applied.and_then(|_| {
                runner.sieve(&edit.name, &["--base", "HEAD", "--output", &selection_arg])
            });
            journal.restore(&workspace)?;
            saved.restore(&workspace)?;
            edits[i].push(timed?);
            selections[i] = serde_json::from_slice(&fs::read(&selection_file).unwrap_or_default())
                .unwrap_or(Value::Null);
            runner.sieve("settle", base)?;
        }
    }
    let mut levers = BTreeMap::new();
    if compare_levers {
        for lever in crate::records::LEVERS {
            let default_on = *lever != "mvnd";
            let switch = if default_on { "--without" } else { "--with" };
            let mut per_edit = Vec::new();
            for (i, edit) in catalog.edits.iter().enumerate() {
                let mut runs = Vec::new();
                for _ in 0..samples {
                    let saved = State::save(&workspace)?;
                    apply(&workspace, edit, &mut journal)?;
                    let timed = runner.sieve(
                        &format!("{switch}-{lever}-{}", edit.name),
                        &["--base", "HEAD", switch, lever],
                    );
                    journal.restore(&workspace)?;
                    saved.restore(&workspace)?;
                    runs.push(timed?);
                    runner.sieve("settle", &["--base", "HEAD", switch, lever])?;
                }
                let toggled = median(&runs);
                let default = median(&edits[i]);
                let state = &selections[i]["speedups"][lever];
                per_edit.push(json!({"edit": edit.name, "default_state": state, "default_seconds": default, "toggled_seconds": toggled}));
            }
            levers.insert(
                lever.to_string(),
                json!({"toggle": switch, "edits": per_edit}),
            );
        }
    }
    let mut planted = Vec::new();
    let mut missed = 0usize;
    if plant_bugs {
        runner.planting = true;
        for edit in &catalog.edits {
            eprintln!("catalog: planting a bug in {}", edit.name);
            let saved = State::save(&workspace)?;
            let region = apply(&workspace, edit, &mut journal)?;
            let source = region.0.ends_with(".java") || region.0.ends_with(".kt");
            let result = if source {
                let (workspace_ref, runner_ref) =
                    (&workspace, std::cell::RefCell::new(&mut runner));
                let mut selected = |tag: &str| {
                    runner_ref
                        .borrow_mut()
                        .sieve(&format!("planted-selected {tag}"), &["--base", "HEAD"])
                };
                let mut reference = |tag: &str| {
                    runner_ref
                        .borrow_mut()
                        .sieve(&format!("planted-full {tag}"), &["--full"])
                };
                plant(
                    workspace_ref,
                    &region,
                    &mut journal,
                    &mut selected,
                    &mut reference,
                )
            } else {
                Ok(json!({"mutant": null, "reason": "not a Java or Kotlin source"}))
            };
            journal.restore(&workspace)?;
            saved.restore(&workspace)?;
            runner.sieve("settle", base)?;
            let mut result = result?;
            missed += result["missed"].as_array().map_or(0, Vec::len);
            result["edit"] = json!(edit.name);
            planted.push(result);
        }
        runner.planting = false;
    }
    // Weighted total: each kind's mean of edit medians, by the kind's share of history.
    let mut kinds: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for (edit, runs) in catalog.edits.iter().zip(&edits) {
        kinds.entry(&edit.kind).or_default().push(median(runs));
    }
    let (covered, weighted) = weighted_total(&history["weights"], &kinds);
    let (covered_without, weighted_without) =
        weighted_total(&history["weights_without_pom_only"], &kinds);
    let weights = history["weights"].as_object().cloned().unwrap_or_default();
    let report = json!({
        "workspace": original,
        "catalog": catalog_path,
        "samples": samples,
        "history": history,
        "warm_up": warm,
        "native": summary(&native),
        "full": summary(&full),
        "edits": catalog.edits.iter().zip(&edits).zip(&selections).map(|((edit, runs), selection)| json!({
            "name": edit.name, "kind": edit.kind, "selection": {
                "mode": selection["mode"], "tests": selection["tests"], "speedups": selection["speedups"]
            }, "result": summary(runs),
        })).collect::<Vec<_>>(),
        "weighted": {
            "covered_weight": if weights.is_empty() { Value::Null } else { json!(covered) },
            "sieve_seconds": weighted,
            "without_pom_only": {
                "covered_weight": covered_without,
                "sieve_seconds": weighted_without,
            },
            "native_seconds": median(&native),
            "full_seconds": median(&full),
        },
        "levers": levers,
        "planted": planted,
        "missed_failures": missed,
    });
    fs::write(
        output.join("catalog.json"),
        serde_json::to_string_pretty(&report)? + "\n",
    )?;
    let text = markdown(&report);
    fs::write(output.join("summary.md"), &text)?;
    eprint!("{text}");
    Ok(u8::from(missed > 0))
}

/// The weighted mean of each kind's mean edit median, over the kinds the catalog covers, and
/// the history weight those kinds cover. Without weights every kind counts equally.
fn weighted_total(weights: &Value, kinds: &BTreeMap<&str, Vec<f64>>) -> (f64, f64) {
    let weights = weights.as_object().cloned().unwrap_or_default();
    let weight = |kind: &str| -> f64 {
        if weights.is_empty() {
            1.0
        } else {
            weights.get(kind).and_then(Value::as_f64).unwrap_or(0.0)
        }
    };
    let covered: f64 = kinds.keys().map(|k| weight(k)).sum();
    let total = kinds
        .iter()
        .map(|(k, v)| weight(k) * v.iter().sum::<f64>() / v.len() as f64)
        .sum::<f64>();
    (
        covered,
        if covered > 0.0 {
            total / covered
        } else {
            f64::NAN
        },
    )
}

fn markdown(report: &Value) -> String {
    let seconds = |v: &Value| v["seconds"]["median"].as_f64().unwrap_or(f64::NAN);
    let range = |v: &Value| {
        format!(
            "{:.1}–{:.1}",
            v["seconds"]["min"].as_f64().unwrap_or(f64::NAN),
            v["seconds"]["max"].as_f64().unwrap_or(f64::NAN)
        )
    };
    let phases = |v: &Value| {
        timing::PHASES
            .iter()
            .map(|p| format!("{:.1}", v["phases"][p].as_f64().unwrap_or(0.0)))
            .collect::<Vec<_>>()
            .join(" / ")
    };
    let mut out = format!(
        "# Edit catalog\n\nPhases: {}. Seconds are medians of {} samples.\n\n\
         | Edit | Kind | Selection | Median s | Range s | Phases s |\n| --- | --- | --- | ---: | ---: | --- |\n",
        timing::PHASES.join(" / "),
        report["samples"]
    );
    for (label, key) in [
        ("native full build", "native"),
        ("sieve run --full", "full"),
    ] {
        let v = &report[key];
        out += &format!(
            "| {label} | | {} test cases | {:.1} | {} | {} |\n",
            v["cases"],
            seconds(v),
            range(v),
            phases(v)
        );
    }
    for edit in report["edits"].as_array().into_iter().flatten() {
        let v = &edit["result"];
        let tests = edit["selection"]["tests"].as_array().map_or(0, Vec::len);
        let selection = match edit["selection"]["mode"].as_str().unwrap_or("?") {
            "SUBSET" => format!("{tests} classes"),
            mode => mode.to_owned(),
        };
        out += &format!(
            "| {} | {} | {selection} | {:.1} | {} | {} |\n",
            edit["name"].as_str().unwrap_or(""),
            edit["kind"].as_str().unwrap_or(""),
            seconds(v),
            range(v),
            phases(v)
        );
    }
    let w = &report["weighted"];
    out += &format!(
        "\nWeighted feedback time per commit: sieve {:.1} s, native {:.1} s, sieve --full {:.1} s (history weight covered: {}).\n",
        w["sieve_seconds"].as_f64().unwrap_or(f64::NAN),
        w["native_seconds"].as_f64().unwrap_or(f64::NAN),
        w["full_seconds"].as_f64().unwrap_or(f64::NAN),
        w["covered_weight"].as_f64().map_or("equal weights".into(), |c| format!("{:.0}%", c * 100.0)),
    );
    if let Some(seconds) = w["without_pom_only"]["sieve_seconds"]
        .as_f64()
        .filter(|s| s.is_finite())
    {
        out += &format!(
            "Without POM-only commits, such as automated dependency updates: sieve {seconds:.1} s.\n"
        );
    }
    if let Some(levers) = report["levers"].as_object().filter(|l| !l.is_empty()) {
        out += "\n| Speed-up | Default state | Toggle | Edit | Default s | Toggled s |\n| --- | --- | --- | --- | ---: | ---: |\n";
        for (lever, v) in levers {
            for edit in v["edits"].as_array().into_iter().flatten() {
                out += &format!(
                    "| {lever} | {} | {} | {} | {:.1} | {:.1} |\n",
                    edit["default_state"].as_str().unwrap_or("?"),
                    v["toggle"].as_str().unwrap_or(""),
                    edit["edit"].as_str().unwrap_or(""),
                    edit["default_seconds"].as_f64().unwrap_or(f64::NAN),
                    edit["toggled_seconds"].as_f64().unwrap_or(f64::NAN)
                );
            }
        }
    }
    if let Some(planted) = report["planted"].as_array().filter(|p| !p.is_empty()) {
        out += "\n| Edit | Planted bug | Full run failed | Missed |\n| --- | --- | ---: | ---: |\n";
        for p in planted {
            out += &format!(
                "| {} | {} | {} | {} |\n",
                p["edit"].as_str().unwrap_or(""),
                p["mutant"]
                    .as_str()
                    .or(p["reason"].as_str())
                    .unwrap_or("no compiling mutant"),
                p["full_failed"].as_array().map_or(0, Vec::len),
                p["missed"].as_array().map_or(0, Vec::len)
            );
        }
        out += &format!("\nMissed failures: {}\n", report["missed_failures"]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutants_negate_flip_swap_and_drop() {
        let text = "class A {\n  int f(int a) {\n    if (a > 0 && ok(a)) {\n      log(a);\n      return a + 1;\n    }\n    return a == 2 ? 1 : 0;\n  }\n}";
        let labels: Vec<(String, usize)> = mutants(text, 2, 6)
            .into_iter()
            .map(|(label, line, _)| (label, line))
            .collect();
        assert_eq!(labels[0], ("negated condition".into(), 2));
        assert!(labels.contains(&("flipped equality".into(), 6)));
        assert!(labels.contains(&("flipped comparison".into(), 2)));
        assert!(labels.contains(&("swapped arithmetic".into(), 4)));
        assert!(labels.contains(&("dropped call".into(), 3)));
        let negated = &mutants(text, 2, 2)[0].2;
        assert!(negated.contains("if (!(a > 0 && ok(a))) {"), "{negated}");
        // Strings, comments, and identifiers are left alone.
        assert!(mutants("  String s = \"a == b\";\n  // x == y\n  truex = 1;", 0, 2).is_empty());
    }

    #[test]
    fn commits_classify_by_path_status_and_declarations() {
        let config: Config =
            serde_json::from_value(json!({"tool": "maven", "modules": {".": []}})).unwrap();
        let none = || (String::new(), String::new());
        let kind = |path, status| kind_of(path, status, &config, none);
        assert_eq!(kind("pom.xml", "M"), "pom");
        assert_eq!(kind("README.md", "M"), "docs");
        assert_eq!(kind("src/main/java/a/B.java", "R100"), "rename");
        assert_eq!(kind("src/main/java/a/B.java", "A"), "new-class");
        assert_eq!(kind("src/test/java/a/BTest.java", "A"), "new-test");
        assert_eq!(kind("src/test/java/a/BTest.java", "M"), "test");
        assert_eq!(kind("src/test/resources/a.json", "M"), "fixture");
        assert_eq!(
            kind("src/test/resources/application-test.yml", "M"),
            "test-config"
        );
        assert_eq!(kind("src/main/resources/application.yml", "M"), "config");
        assert_eq!(kind("src/main/resources/db/v1.sql", "A"), "resource");
        assert_eq!(kind("Dockerfile", "M"), "other");
        let body = kind_of("src/main/java/a/B.java", "M", &config, || {
            (
                "-    return a;\n+    return a + 1;\n".into(),
                "class B {}".into(),
            )
        });
        assert_eq!(body, "body");
        let listener = kind_of("src/main/java/a/L.java", "M", &config, || {
            ("+    log(x);\n".into(), "@KafkaListener class L {}".into())
        });
        assert_eq!(listener, "listener-body");
        let structural = kind_of("src/main/java/a/B.java", "M", &config, || {
            ("+    private final Clock clock;\n".into(), String::new())
        });
        assert_eq!(structural, "structural");
    }

    #[test]
    fn edits_apply_and_the_journal_restores_them() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/A.java"), "a\nb\nc\n").unwrap();
        let edit =
            |search: Option<&str>, content: Option<&str>, rename: Option<&str>, file: &str| Edit {
                name: "e".into(),
                kind: "body".into(),
                file: file.into(),
                search: search.map(Into::into),
                replace: search.map(|_| "B\nB2".into()),
                content: content.map(Into::into),
                rename_to: rename.map(Into::into),
            };
        let mut journal = Journal::default();
        let region = apply(
            root,
            &edit(Some("b"), None, None, "src/A.java"),
            &mut journal,
        )
        .unwrap();
        assert_eq!(region, ("src/A.java".into(), 1, 2));
        apply(
            root,
            &edit(None, Some("x\ny\n"), None, "src/New.java"),
            &mut journal,
        )
        .unwrap();
        // An interrupted run leaves the journal behind; the next one restores from it.
        assert!(Journal::recover(root).unwrap());
        assert_eq!(
            fs::read_to_string(root.join("src/A.java")).unwrap(),
            "a\nb\nc\n"
        );
        assert!(!root.join("src/New.java").exists());
        assert!(!Journal::recover(root).unwrap());
        let mut journal = Journal::default();
        apply(
            root,
            &edit(None, None, Some("src/C.java"), "src/A.java"),
            &mut journal,
        )
        .unwrap();
        assert!(root.join("src/C.java").is_file() && !root.join("src/A.java").exists());
        journal.restore(root).unwrap();
        assert!(root.join("src/A.java").is_file() && !root.join("src/C.java").exists());
    }
}
