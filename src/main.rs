use serde::{Deserialize, Serialize};
mod bytecode;
mod catalog;
mod classes;
mod fingerprint;
mod fixtures;
mod generated;
mod js;
mod observation;
mod receipt;
mod records;
mod replay;
mod reports;
mod settings;
mod setup;
mod summary;
mod timing;
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Config {
    tool: String,
    modules: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    build_fingerprint: Option<String>,
    /// Modules whose `src/test` code other modules use, through a test-jar, a test output, or a
    /// shared directory. A test-only change in any other module selects that module alone.
    /// Absent means every module may share its tests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shared_tests: Option<Vec<String>>,
    /// Globs for changes that select nothing. Relative to the workspace; a leading `/`
    /// anchors the pattern at the repository root. Absent means `DEFAULT_IGNORE`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ignore: Option<Vec<String>>,
    /// `run` compiles the selected modules first and narrows the selection to the test
    /// classes whose bytecode reaches a changed class.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    class_level: bool,
    /// Globs for build inputs, such as OpenAPI specifications, that reach tests only
    /// through the sources generated from them. Class-level selection compares those
    /// sources with the ones generated at the comparison base (Maven only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    generated: Vec<String>,
    /// Local mode for single-module Maven projects: `run` loads the agent into the test JVM,
    /// which keeps test records and drops the test classes whose records are unchanged.
    /// Enabled only by `"records": true` or an explicit `run --records`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    records: Option<bool>,
    /// Environment variables that local test records depend on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    record_env: Vec<String>,
    /// Test-JVM system properties that local test records ignore: names, or prefixes ending
    /// in `*`. `jgitver.*` is always ignored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    record_ignore_properties: Vec<String>,
    /// Speed-ups that local `run` enables by default, as with `--with`; `--without` overrides.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    with: Vec<String>,
    /// Jest/Vitest: the directories whose script files Sieve maps to related test files
    /// (ADR 0006). A change anywhere else selects every test.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    sources: Vec<String>,
    /// Test class patterns added to every JVM selection, including unchanged runs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    must_run: Vec<String>,
    /// Workspace-local included Gradle builds supplying convention plugins.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    build_logic: Vec<String>,
}

/// Documentation outside module sources. Module sources stay test inputs whatever their name.
const DEFAULT_IGNORE: &[&str] = &[
    "**/*.md",
    "**/*.adoc",
    "docs/**",
    "/*.md",
    "/*.adoc",
    "/docs/**",
];

const REPOSITORY: &str = "@repository/";

/// A module key: `.` for the workspace itself, or the module's directory below it, such as
/// `services/orders`, in plain path segments.
pub(crate) fn valid_module(module: &str) -> bool {
    module == "."
        || (!module.is_empty()
            && module.split('/').all(|part| {
                !matches!(part, "" | "." | "..")
                    && part
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
            }))
}

/// `*` and `?` stay within one path segment; `**` crosses segments, and `**/` also
/// matches zero segments.
fn glob(pattern: &[u8], path: &[u8]) -> bool {
    match pattern {
        [] => path.is_empty(),
        [b'*', b'*', b'/', rest @ ..] => {
            glob(rest, path)
                || (0..path.len()).any(|i| path[i] == b'/' && glob(rest, &path[i + 1..]))
        }
        [b'*', b'*', rest @ ..] => (0..=path.len()).any(|i| glob(rest, &path[i..])),
        [b'*', rest @ ..] => (0..=path.len())
            .take_while(|&i| i == 0 || path[i - 1] != b'/')
            .any(|i| glob(rest, &path[i..])),
        [b'?', rest @ ..] => matches!(path, [c, tail @ ..] if *c != b'/' && glob(rest, tail)),
        [c, rest @ ..] => matches!(path, [d, tail @ ..] if d == c && glob(rest, tail)),
    }
}

#[derive(Debug, Serialize)]
struct Selection {
    mode: &'static str,
    modules: BTreeSet<String>,
    /// `SUBSET` only: binary names of the selected test classes.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    tests: BTreeSet<String>,
    changed: BTreeSet<String>,
    /// Changed paths inside module source directories, which class-level selection maps.
    #[serde(skip)]
    sources: BTreeSet<String>,
    /// Generated sources that differ from the comparison base's.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    generated: BTreeSet<String>,
    /// Local mode: test classes dropped at discovery.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    skipped: BTreeSet<String>,
    /// Why each test class ran: how it reaches a change (class level), or why it ran or was
    /// dropped (local mode).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    reasons: BTreeMap<String, String>,
    /// Local mode: the state of each speed-up.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    speedups: BTreeMap<String, String>,
    reason: String,
    /// `run` only: tests run against the suite, and the estimated time saved.
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<summary::Summary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observation: Option<observation::Observation>,
}

impl Config {
    fn read(workspace: &Path) -> Result<Self> {
        let config: Self = serde_json::from_slice(&fs::read(workspace.join("impact.json"))?)?;
        config.validate(workspace)?;
        Ok(config)
    }

    fn validate(&self, workspace: &Path) -> Result<()> {
        if js::is_js(&self.tool) {
            if self.modules.len() != 1 || self.modules.get(".").is_none_or(|d| !d.is_empty()) {
                return Err("jest/vitest projects support only the single module \".\"".into());
            }
            if self.sources.is_empty() || !self.sources.iter().all(|s| s != "." && valid_module(s))
            {
                return Err(
                    "jest/vitest projects require relative source directories in sources".into(),
                );
            }
            if self.class_level
                || self.records.is_some()
                || !self.generated.is_empty()
                || !self.with.is_empty()
                || self.build_fingerprint.is_some()
                || self.shared_tests.is_some()
                || !self.must_run.is_empty()
                || !self.build_logic.is_empty()
            {
                return Err(
                    "jest/vitest projects support only tool, modules, sources, and ignore".into(),
                );
            }
        } else if !self.sources.is_empty() {
            return Err("sources applies only to jest/vitest projects".into());
        }
        if !matches!(self.tool.as_str(), "maven" | "gradle" | "jest" | "vitest")
            || self.modules.is_empty()
        {
            return Err(
                "impact.json requires tool maven/gradle/jest/vitest and a nonempty module map"
                    .into(),
            );
        }
        for (module, dependencies) in &self.modules {
            if !valid_module(module)
                || !workspace.join(module).is_dir()
                || !workspace
                    .join(module)
                    .canonicalize()?
                    .starts_with(workspace.canonicalize()?)
                || dependencies.iter().any(|d| !self.modules.contains_key(d))
            {
                return Err(format!("Invalid module or dependency: {module}").into());
            }
        }
        if let Some(pattern) = self
            .ignore
            .iter()
            .flatten()
            .find(|p| p.trim_start_matches('/').is_empty())
        {
            return Err(format!("Invalid ignore pattern: {pattern:?}").into());
        }
        if let Some(pattern) = self
            .generated
            .iter()
            .find(|p| p.is_empty() || p.starts_with('/'))
        {
            return Err(format!("Invalid generated pattern: {pattern:?}").into());
        }
        if let Some(module) = self
            .shared_tests
            .iter()
            .flatten()
            .find(|m| !self.modules.contains_key(*m))
        {
            return Err(format!("Invalid shared_tests module: {module:?}").into());
        }
        if let Some(name) = self.record_env.iter().find(|name| {
            name.is_empty()
                || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
                || name.starts_with(|c: char| c.is_ascii_digit())
        }) {
            return Err(format!("Invalid record_env variable: {name:?}").into());
        }
        if let Some(pattern) = self.record_ignore_properties.iter().find(|p| {
            let name = p.strip_suffix('*').unwrap_or(p);
            name.is_empty()
                || name.contains('*')
                || !name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
        }) {
            return Err(format!("Invalid record_ignore_properties entry: {pattern:?}").into());
        }
        for pattern in &self.must_run {
            if pattern.is_empty()
                || !pattern
                    .chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '$' | '*' | '?'))
            {
                return Err(format!("Invalid must_run class pattern: {pattern:?}").into());
            }
        }
        for root in &self.build_logic {
            if self.tool != "gradle"
                || root == "."
                || !valid_module(root)
                || !workspace.join(root).is_dir()
                || !workspace
                    .join(root)
                    .canonicalize()?
                    .starts_with(workspace.canonicalize()?)
            {
                return Err(format!("Invalid Gradle build_logic directory: {root:?}").into());
            }
        }
        records::Levers {
            on: self.with.iter().cloned().collect(),
            off: BTreeSet::new(),
        }
        .validate()?;
        Ok(())
    }

    /// The configuration `run --records` uses without `impact.json`:
    /// local mode, with OpenAPI specifications as generated inputs.
    fn local_default(workspace: &Path) -> Option<Self> {
        let pom = fs::read_to_string(workspace.join("pom.xml")).ok()?;
        if pom.contains("<modules>") {
            return None;
        }
        let mut generated = BTreeSet::new();
        for spec in pom.split("<inputSpec>").skip(1) {
            let spec = spec.split("</inputSpec>").next()?.trim();
            let spec = spec
                .trim_start_matches("${project.basedir}/")
                .trim_start_matches("${basedir}/");
            if let Some((dir, _)) = spec.rsplit_once('/').filter(|_| !spec.starts_with('$')) {
                generated.insert(format!("{dir}/**"));
            }
        }
        serde_json::from_value(serde_json::json!({
            "tool": "maven", "modules": {".": []}, "records": true, "generated": generated,
        }))
        .ok()
    }

    fn must_run(&self, class: &str) -> bool {
        self.must_run.iter().any(|pattern| {
            glob(pattern.as_bytes(), class.as_bytes())
                || (!class.contains('$')
                    && pattern
                        .split_once('$')
                        .is_some_and(|(outer, _)| glob(outer.as_bytes(), class.as_bytes())))
        })
    }

    fn generated_input(&self, path: &str) -> bool {
        self.generated
            .iter()
            .any(|pattern| glob(pattern.as_bytes(), path.as_bytes()))
    }

    /// The module whose sources (`<module>/src/**`) hold a workspace path: the innermost one,
    /// when a module's directory holds another module.
    fn module_of<'p>(&self, path: &'p str) -> Option<&'p str> {
        let named = self
            .modules
            .keys()
            .filter(|module| {
                path.strip_prefix(module.as_str())
                    .is_some_and(|rest| rest.starts_with("/src/"))
            })
            .map(String::len)
            .max();
        match named {
            Some(length) => Some(&path[..length]),
            None => (self.modules.contains_key(".") && path.starts_with("src/")).then_some("."),
        }
    }

    /// `prefix` is the workspace path below the repository root, used by `/` patterns.
    fn ignored(&self, path: &str, prefix: &str) -> bool {
        let repository = match path.strip_prefix(REPOSITORY) {
            Some(outside) => outside.to_owned(),
            None if prefix.is_empty() => path.to_owned(),
            None => format!("{prefix}/{path}"),
        };
        let workspace = (!path.starts_with(REPOSITORY)).then_some(path);
        let matches = |pattern: &str| match pattern.strip_prefix('/') {
            Some(anchored) => glob(anchored.as_bytes(), repository.as_bytes()),
            None => workspace.is_some_and(|p| glob(pattern.as_bytes(), p.as_bytes())),
        };
        match &self.ignore {
            Some(patterns) => patterns.iter().any(|p| matches(p)),
            // The default patterns never hide module sources, such as a `docs` module's.
            None => self.module_of(path).is_none() && DEFAULT_IGNORE.iter().any(|p| matches(p)),
        }
    }

    /// Whether other modules may use the module's `src/test` code.
    fn shares_tests(&self, module: &str) -> bool {
        self.shared_tests
            .as_ref()
            .is_none_or(|shared| shared.iter().any(|m| m == module))
    }

    fn all(&self, reason: impl Into<String>) -> Selection {
        Selection {
            mode: "ALL",
            modules: self.modules.keys().cloned().collect(),
            tests: BTreeSet::new(),
            changed: BTreeSet::new(),
            sources: BTreeSet::new(),
            generated: BTreeSet::new(),
            skipped: BTreeSet::new(),
            reasons: BTreeMap::new(),
            speedups: BTreeMap::new(),
            summary: None,
            observation: None,
            reason: reason.into(),
        }
    }

    fn select(&self, changed: BTreeSet<String>, prefix: &str) -> Selection {
        let mut selected = BTreeSet::new();
        // Changed modules whose change can reach the modules depending on them.
        let mut reaching = BTreeSet::new();
        let mut sources = BTreeSet::new();
        for path in &changed {
            if self
                .build_logic
                .iter()
                .any(|root| path == root || path.starts_with(&format!("{root}/")))
            {
                let mut result =
                    self.all(format!("Gradle convention-plugin input changed: {path}"));
                result.changed = changed;
                return result;
            }
            if self.ignored(path, prefix) {
                continue;
            }
            if let Some(module) = self.module_of(path) {
                selected.insert(module.to_owned());
                sources.insert(path.clone());
                // Other modules see test code only through a shared test artifact.
                let inside = path.strip_prefix(&format!("{module}/")).unwrap_or(path);
                if !inside.starts_with("src/test/") || self.shares_tests(module) {
                    reaching.insert(module.to_owned());
                }
                continue;
            }
            // Build/configuration changes may alter the module graph or test discovery.
            let mut result = self.all(format!("Unclassified or build input changed: {path}"));
            result.changed = changed;
            return result;
        }
        loop {
            let before = reaching.len();
            for (module, dependencies) in &self.modules {
                if dependencies
                    .iter()
                    .any(|dependency| reaching.contains(dependency))
                {
                    reaching.insert(module.clone());
                }
            }
            if reaching.len() == before {
                break;
            }
        }
        selected.extend(reaching);
        let reason = if changed.is_empty() {
            "No changes"
        } else if selected.is_empty() {
            "Only ignored paths changed"
        } else {
            "Changed source/resource modules and their transitive dependents"
        };
        Selection {
            mode: if selected.is_empty() {
                "NONE"
            } else {
                "MODULES"
            },
            modules: selected,
            tests: BTreeSet::new(),
            reason: reason.into(),
            changed,
            sources,
            generated: BTreeSet::new(),
            skipped: BTreeSet::new(),
            reasons: BTreeMap::new(),
            speedups: BTreeMap::new(),
            summary: None,
            observation: None,
        }
    }
}

impl Selection {
    /// Narrows a module selection to the test classes that reach the changed classes,
    /// returning the unselected test classes.
    fn refine(
        &mut self,
        classes: &[classes::Class],
        workspace: &Path,
        maven: bool,
    ) -> Result<BTreeSet<String>> {
        Ok(
            match classes::affected(classes, &self.sources, workspace)? {
                classes::Impact::Fallback(reason) => {
                    self.reason = format!("Class-level selection unavailable: {reason}");
                    BTreeSet::new()
                }
                classes::Impact::Tests {
                    mut selected,
                    unselected,
                    mut reasons,
                } => {
                    if maven {
                        // `-am` also compiles the tests of upstream modules, which stay skipped.
                        selected.retain(|test| {
                            let class = format!("{}.class", test.replace('.', "/"));
                            self.modules.iter().any(|module| {
                                workspace
                                    .join(module)
                                    .join("target/test-classes")
                                    .join(&class)
                                    .is_file()
                            })
                        });
                    }
                    self.reason = if selected.is_empty() {
                        self.mode = "NONE";
                        "No test class reaches the changed classes; compile only"
                    } else {
                        self.mode = "SUBSET";
                        "Test classes whose bytecode reaches the changed classes"
                    }
                    .into();
                    reasons.retain(|test, _| selected.contains(test));
                    self.reasons = reasons;
                    self.tests = selected;
                    unselected
                }
            },
        )
    }
}

fn git(workspace: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .current_dir(workspace)
        .args(args)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "git {}: {}",
            args[0],
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(output.stdout)
}

/// Returns changed paths, the workspace prefix below the repository root, and the merge base.
fn changed_paths(workspace: &Path, base: &str) -> Result<(BTreeSet<String>, String, String)> {
    // Resolve revisions separately, so user input cannot become a Git option/pathspec.
    let revision = format!("{base}^{{commit}}");
    let base = String::from_utf8(git(
        workspace,
        &["rev-parse", "--verify", "--end-of-options", &revision],
    )?)?;
    let merge_base = String::from_utf8(git(workspace, &["merge-base", base.trim(), "HEAD"])?)?;
    let root = String::from_utf8(git(workspace, &["rev-parse", "--show-toplevel"])?)?;
    let root = Path::new(root.trim()).canonicalize()?;
    let prefix = workspace.strip_prefix(&root)?;
    // Run from the repository root: sibling files and shared build inputs must not disappear.
    let mut paths = git(
        &root,
        &[
            "diff",
            "--name-only",
            "-z",
            "--no-renames",
            // `submodule.<name>.ignore` and `diff.ignoreSubmodules` would hide gitlink changes.
            "--ignore-submodules=none",
            merge_base.trim(),
            "--",
        ],
    )?;
    paths.extend(git(
        &root,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )?);
    let changed = paths
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| {
            let path = std::str::from_utf8(p)?;
            Ok(match Path::new(path).strip_prefix(prefix) {
                Ok(relative) => relative.to_str().ok_or("Non-UTF-8 path")?.to_owned(),
                // External changes cause a full run unless ignored, including changes to the
                // selector and shared CI scripts.
                Err(_) => format!("{REPOSITORY}{path}"),
            })
        })
        .collect::<Result<_>>()?;
    let prefix = prefix.to_str().ok_or("Non-UTF-8 path")?.replace('\\', "/");
    Ok((changed, prefix, merge_base.trim().to_owned()))
}

/// The goal or task limited to `modules`. Maven also builds the modules they depend on;
/// Gradle resolves task dependencies itself. Returns the plain goal for a whole build.
fn scope(config: &Config, modules: &BTreeSet<String>, goal: &str) -> Vec<String> {
    let partial = modules.len() < config.modules.len();
    if config.tool == "maven" {
        let mut args = vec![goal.to_owned()];
        // The root of a Maven reactor aggregates every module, so it builds everything.
        if partial && !modules.contains(".") {
            let list: Vec<_> = modules.iter().map(String::as_str).collect();
            args.extend(["-pl".into(), list.join(","), "-am".into()]);
        }
        args
    } else if partial {
        // A module's directory is its Gradle project path: `services/orders` is `:services:orders`.
        modules
            .iter()
            .map(|module| match module.as_str() {
                "." => format!(":{goal}"),
                module => format!(":{}:{goal}", module.replace('/', ":")),
            })
            .collect()
    } else {
        vec![goal.to_owned()]
    }
}

/// `filter` is set after Maven's class-level compile step: the build skips `clean` and reads
/// the unselected test classes from that file. Gradle selects classes inside its one build.
fn build_args(config: &Config, selection: &Selection, filter: Option<&Path>) -> Vec<String> {
    let maven = config.tool == "maven";
    let mut args: Vec<String> = if maven {
        vec!["-B", "-ntp"]
    } else {
        vec!["--no-daemon", "--console=plain"]
    }
    .into_iter()
    .map(str::to_owned)
    .collect();
    if filter.is_none() {
        args.push("clean".into());
    }
    // NONE with modules comes from class-level selection: compile them but run no tests.
    let compile_only = selection.mode == "NONE" && !selection.modules.is_empty();
    if selection.mode == "NONE" && !compile_only {
        return args;
    }
    let goal = if maven { "verify" } else { "check" };
    if selection.mode == "ALL" {
        args.push(goal.into());
        return args;
    }
    args.extend(scope(config, &selection.modules, goal));
    let subset = selection.mode == "SUBSET";
    if maven {
        // `-am` also builds unselected dependencies; their tests stay skipped.
        for module in config.modules.keys() {
            args.push(format!(
                "-D{}={}",
                setup::skip_property(module),
                compile_only || !selection.modules.contains(module)
            ));
        }
        // The compile step just built these main classes from the same sources; generated
        // sources may be rewritten, which would otherwise recompile everything.
        if filter.is_some() {
            args.push("-Dmaven.main.skip=true".into());
        }
        // Excludes keep the POM's includes and suite assignment.
        if let Some(excludes) = filter.filter(|_| subset) {
            for plugin in ["surefire", "failsafe"] {
                args.push(format!("-D{plugin}.excludesFile={}", excludes.display()));
            }
        }
    } else {
        let modules: Vec<_> = selection.modules.iter().map(String::as_str).collect();
        args.push(format!("-Pimpact.modules={}", modules.join(",")));
    }
    args
}

/// Explicit tasks retain the consumer's complete build graph; only tests are filtered.
fn task_args(
    config: &Config,
    selection: &Selection,
    filter: Option<&Path>,
    tasks: &[String],
) -> Vec<String> {
    if tasks.is_empty() {
        return build_args(config, selection, filter);
    }
    let mut args: Vec<String> = if config.tool == "maven" {
        vec!["-B".into(), "-ntp".into()]
    } else {
        vec!["--console=plain".into()]
    };
    args.extend_from_slice(tasks);
    if selection.mode == "ALL" {
        return args;
    }
    let none = selection.mode == "NONE";
    if config.tool == "maven" {
        for module in config.modules.keys() {
            args.push(format!(
                "-D{}={}",
                setup::skip_property(module),
                none || !selection.modules.contains(module)
            ));
        }
        if let Some(excludes) = filter.filter(|_| selection.mode == "SUBSET") {
            for plugin in ["surefire", "failsafe"] {
                args.push(format!("-D{plugin}.excludesFile={}", excludes.display()));
            }
        }
    } else {
        let modules: Vec<_> = selection
            .modules
            .iter()
            .filter(|_| !none)
            .map(String::as_str)
            .collect();
        args.push(format!("-Pimpact.modules={}", modules.join(",")));
    }
    args
}

/// Explicit tasks may reuse build outputs, but this run must not report old test XML.
fn clear_reports(config: &Config, workspace: &Path) -> Result<()> {
    reports::clear_native_reports(workspace, &config.tool)
}

/// Forced tests are additive. Without class-level opt-in, retain every test of the originally
/// selected modules, even when the build must compile other modules to discover forced classes.
fn refine_policy(
    selection: &mut Selection,
    config: &Config,
    classes: &[classes::Class],
    workspace: &Path,
    maven: bool,
    preserve: Option<&BTreeSet<String>>,
    dirs: &[(PathBuf, bool)],
) -> Result<BTreeSet<String>> {
    let mut unselected = if let Some(modules) = preserve {
        let mut unselected = BTreeSet::new();
        for class in classes
            .iter()
            .filter(|c| c.test && !c.annotation && !c.name.contains('$'))
        {
            let name = class.name.replace('/', ".");
            let kept = dirs.iter().filter(|(_, test)| *test).any(|(dir, _)| {
                if !dir.join(format!("{}.class", class.name)).is_file() {
                    return false;
                }
                let owner = config
                    .modules
                    .keys()
                    .filter(|m| dir.starts_with(workspace.join(m)))
                    .max_by_key(|m| m.len());
                owner.is_none_or(|module| modules.contains(module))
            });
            if kept {
                selection.tests.insert(name.clone());
                selection.reasons.insert(name, "Selected module".into());
            } else {
                unselected.insert(name);
            }
        }
        selection.mode = if selection.tests.is_empty() {
            "NONE"
        } else {
            "SUBSET"
        };
        unselected
    } else {
        selection.refine(classes, workspace, maven)?
    };
    if selection.mode != "MODULES" {
        for class in classes.iter().filter(|c| c.test && !c.annotation) {
            let name = class.name.replace('/', ".");
            let top = name.split('$').next().unwrap_or(&name);
            if config.must_run(&name) || config.must_run(top) {
                selection.tests.insert(top.to_owned());
                selection
                    .reasons
                    .insert(top.to_owned(), "Required by must_run policy".into());
                unselected.remove(top);
            }
        }
        selection.mode = if selection.tests.is_empty() {
            "NONE"
        } else {
            "SUBSET"
        };
        if !config.must_run.is_empty() {
            selection.reason = "Changed tests and additive must_run policy".into();
        }
    }
    unselected.retain(|test| !selection.tests.contains(test));
    Ok(unselected)
}

/// A scoped Maven build cleans only its reactor; test reports of the other modules would
/// otherwise survive and misreport this run.
fn remove_stale_reports(config: &Config, selection: &Selection, workspace: &Path) -> Result<()> {
    if config.tool != "maven" || selection.mode == "ALL" {
        return Ok(());
    }
    for module in config.modules.keys() {
        for folder in ["surefire-reports", "failsafe-reports"] {
            let dir = workspace.join(module).join("target").join(folder);
            if dir.is_dir() {
                fs::remove_dir_all(dir)?;
            }
        }
    }
    Ok(())
}

/// Maven's class-level compile step, before the build that runs the selected tests.
fn compile_args(config: &Config, selection: &Selection) -> Vec<String> {
    let mut args: Vec<String> = vec!["-B".into(), "-ntp".into(), "clean".into()];
    args.extend(scope(config, &selection.modules, "test-compile"));
    args
}

/// Compiled class directories, each marked `true` when it holds test classes. Includes
/// unselected modules: a path from a test to a change may pass through a dependency.
fn class_dirs(config: &Config, workspace: &Path, listing: &Path) -> Result<Vec<(PathBuf, bool)>> {
    if config.tool == "maven" {
        return Ok(config
            .modules
            .keys()
            .flat_map(|module| {
                let target = workspace.join(module).join("target");
                [
                    (target.join("classes"), false),
                    (target.join("test-classes"), true),
                ]
            })
            .collect());
    }
    #[derive(Deserialize)]
    struct Listing {
        classes: Vec<PathBuf>,
        tests: Vec<PathBuf>,
    }
    let listing: Listing = serde_json::from_slice(&fs::read(listing)?)?;
    Ok(listing
        .classes
        .into_iter()
        .map(|dir| {
            let test = listing.tests.contains(&dir);
            (dir, test)
        })
        .collect())
}

/// What a build needs to select test classes once the selected modules compiled: `run` writes
/// it, and Gradle's `impactSelect` task or the Maven extension hands it to `sieve classes`.
#[derive(Deserialize, Serialize)]
struct ClassRequest {
    workspace: PathBuf,
    modules: BTreeSet<String>,
    sources: BTreeSet<String>,
    changed: BTreeSet<String>,
    reason: String,
    /// Scratch files. Gradle: the `classes.json` listing, `selected.txt` for the `Test` tasks
    /// (`all`, or `filter` and the test classes to run), and `refined.json` for `run`. Maven:
    /// each module's excludes file and part of the refined selection, below `modules/`.
    dir: PathBuf,
    output: Option<PathBuf>,
    #[serde(default)]
    observe: bool,
    #[serde(default)]
    preserve_modules: Option<BTreeSet<String>>,
    #[serde(default = "module_mode")]
    mode: String,
    #[serde(default)]
    capture_receipt: bool,
    #[serde(default)]
    receipt: Option<receipt::Receipt>,
}

fn module_mode() -> String {
    "MODULES".into()
}

impl ClassRequest {
    fn selection(&self, config: &Config) -> Selection {
        let mut selection = config.all(self.reason.clone());
        selection.mode = match self.mode.as_str() {
            "ALL" => "ALL",
            "NONE" => "NONE",
            _ => "MODULES",
        };
        selection.modules = self.modules.clone();
        selection.sources = self.sources.clone();
        selection.changed = self.changed.clone();
        selection
    }
}

/// The refined selection that `sieve classes` reports back to `run`.
#[derive(Deserialize, Serialize)]
struct Refined {
    #[serde(default)]
    module: Option<String>,
    mode: String,
    #[serde(default)]
    tests: BTreeSet<String>,
    #[serde(default)]
    reasons: BTreeMap<String, String>,
    reason: String,
    unselected: BTreeSet<String>,
}

impl Refined {
    fn of(selection: Selection, unselected: BTreeSet<String>) -> Self {
        Refined {
            module: selection
                .modules
                .iter()
                .next()
                .filter(|_| selection.modules.len() == 1)
                .cloned(),
            mode: selection.mode.into(),
            tests: selection.tests,
            reasons: selection.reasons,
            reason: selection.reason,
            unselected,
        }
    }
}

/// Folds what the build selected into `selection`: one part from Gradle, one per Maven module
/// whose tests ran. A module that fell back ran every test, and the run says so.
fn merge_refined(
    parts: Vec<Refined>,
    selection: &mut Selection,
    unselected: &mut BTreeSet<String>,
) {
    if parts.is_empty() {
        return;
    }
    for part in &parts {
        selection.tests.extend(part.tests.iter().cloned());
        selection.reasons.extend(part.reasons.clone());
        unselected.extend(part.unselected.iter().cloned());
    }
    selection.mode = if parts.iter().any(|p| p.mode == "ALL") {
        "ALL"
    } else if parts.iter().any(|p| p.mode == "MODULES") {
        "MODULES"
    } else if selection.tests.is_empty() {
        "NONE"
    } else {
        "SUBSET"
    };
    if let Some(part) = parts.iter().find(|p| p.mode == selection.mode) {
        selection.reason = part.reason.clone();
    }
}

/// `sieve classes --request FILE [--module DIR]`: class-level selection inside the build, after
/// the selected modules compiled and before their tests run. Gradle asks once for every module.
/// The Maven extension asks for each module right before its tests, and reads the path of the
/// module's excludes file from standard output, which stays empty when every test runs.
/// Analysis problems keep every test.
fn classes_command(args: Vec<String>) -> Result<u8> {
    let (mut file, mut module, mut suite, mut classpath) = (None, None, None, None);
    let mut repeated = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--repeat" {
            repeated = true;
            continue;
        }
        let value = args
            .next()
            .ok_or_else(|| format!("Missing value for {arg}"))?;
        match arg.as_str() {
            "--request" => file = Some(value),
            "--module" => module = Some(value),
            "--suite" => suite = Some(value),
            "--classpath" => classpath = Some(value),
            _ => return Err(format!("Unknown classes option: {arg}").into()),
        }
    }
    let file = file.ok_or("Usage: sieve classes --request FILE [--module DIR --suite NAME]")?;
    let request: ClassRequest = serde_json::from_slice(&fs::read(file)?)?;
    let config = Config::read(&request.workspace)?;
    match module {
        Some(dir) => maven_classes(
            &config,
            &request,
            Path::new(&dir),
            suite.as_deref().unwrap_or("unit"),
            classpath.as_deref().map(Path::new),
            repeated,
        ),
        None => gradle_classes(&config, &request),
    }
}

fn receipt_inventory(
    request: &ClassRequest,
    suites: &BTreeMap<String, Vec<PathBuf>>,
) -> Result<receipt::Exclusions> {
    if !request.capture_receipt && request.receipt.is_none() {
        return Ok(BTreeMap::new());
    }
    let inventory = match receipt::inventory(suites) {
        Ok(inventory) => inventory,
        Err(error) => {
            eprintln!("sieve: cannot verify suite inputs; running tests without receipt exclusions: {error}");
            return Ok(BTreeMap::new());
        }
    };
    let dir = request.dir.join("inventory");
    fs::create_dir_all(&dir)?;
    let inputs = request.dir.join("inventory-inputs");
    fs::create_dir_all(&inputs)?;
    for (suite, digest) in &inventory {
        let path = dir.join(format!("{}.json", bytecode::hash(suite.as_bytes())));
        fs::write(
            path,
            serde_json::to_vec(&BTreeMap::from([(suite, digest)]))?,
        )?;
        fs::write(
            inputs.join(format!("{}.json", bytecode::hash(suite.as_bytes()))),
            serde_json::to_vec(&BTreeMap::from([(suite, &suites[suite])]))?,
        )?;
    }
    Ok(request
        .receipt
        .as_ref()
        .map(|r| receipt::excluded(r, &inventory))
        .unwrap_or_default())
}

/// A receipt excludes an outer class as a unit, including any nested policy matches.
fn receipt_must_run(config: &Config, test: &str, test_dirs: &[PathBuf]) -> bool {
    if config.must_run(test) {
        return true;
    }
    if config.must_run.is_empty() {
        return false;
    }
    let path = PathBuf::from(test.replace('.', "/"));
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(test);
    let nested = format!("{name}$");
    let package = test
        .rsplit_once('.')
        .map(|(package, _)| format!("{package}."))
        .unwrap_or_default();
    for dir in test_dirs {
        let folder = dir.join(path.parent().unwrap_or(Path::new("")));
        let entries = match fs::read_dir(folder) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return true,
        };
        for entry in entries {
            let Ok(entry) = entry else {
                return true;
            };
            let Ok(name) = entry.file_name().into_string() else {
                return true;
            };
            if name.starts_with(&nested)
                && name.ends_with(".class")
                && config.must_run(&format!("{package}{}", name.trim_end_matches(".class")))
            {
                return true;
            }
        }
    }
    false
}

fn gradle_classes(config: &Config, request: &ClassRequest) -> Result<u8> {
    let mut selection = request.selection(config);
    let listing = request.dir.join("classes.json");
    let unselected = if selection.mode == "MODULES" {
        class_dirs(config, &request.workspace, &listing)
            .and_then(|dirs| {
                let classes = classes::load(&dirs)?;
                refine_policy(
                    &mut selection,
                    config,
                    &classes,
                    &request.workspace,
                    false,
                    request.preserve_modules.as_ref(),
                    &dirs,
                )
            })
            .unwrap_or_else(|error| {
                selection.reason = format!("Class-level selection unavailable: {error}");
                BTreeSet::new()
            })
    } else {
        BTreeSet::new()
    };
    #[derive(Deserialize)]
    struct Suites {
        #[serde(default)]
        suites: BTreeMap<String, Vec<PathBuf>>,
        #[serde(default)]
        tests: Vec<PathBuf>,
    }
    let suites: Suites = serde_json::from_slice(&fs::read(&listing)?)?;
    let mut excluded = receipt_inventory(request, &suites.suites)?;
    for tests in excluded.values_mut() {
        tests.retain(|test| !receipt_must_run(config, test, &suites.tests));
    }
    fs::write(
        request.dir.join("remaining.json"),
        serde_json::to_vec(&excluded)?,
    )?;
    let filter = if request.observe || matches!(selection.mode, "MODULES" | "ALL") {
        "all\n".to_owned()
    } else if selection.mode == "NONE" {
        "none\n".to_owned()
    } else {
        unselected
            .iter()
            .fold("exclude\n".to_owned(), |text, test| text + test + "\n")
    };
    fs::write(request.dir.join("selected.txt"), filter)?;
    let json = serde_json::to_string_pretty(&selection)? + "\n";
    if let Some(output) = &request.output {
        fs::write(output, &json)?;
    }
    eprintln!("{json}");
    let mut refined = Refined::of(selection, unselected);
    refined.module = None;
    fs::write(
        request.dir.join("refined.json"),
        serde_json::to_vec(&refined)?,
    )?;
    Ok(0)
}

fn maven_classes(
    config: &Config,
    request: &ClassRequest,
    dir: &Path,
    suite: &str,
    classpath: Option<&Path>,
    repeated: bool,
) -> Result<u8> {
    if !matches!(suite, "unit" | "integration") {
        return Err("Unknown Maven test suite".into());
    }
    let canonical = dir.canonicalize()?;
    let Ok(relative) = canonical.strip_prefix(&request.workspace) else {
        return Ok(0);
    };
    let module = match relative.to_str() {
        Some("") => ".".to_owned(),
        Some(path) => path.replace('\\', "/"),
        None => return Ok(0),
    };
    let mut visible = BTreeSet::from([module.clone()]);
    loop {
        let before = visible.len();
        for known in visible.clone() {
            visible.extend(config.modules.get(&known).into_iter().flatten().cloned());
        }
        if visible.len() == before {
            break;
        }
    }
    let mut selection = request.selection(config);
    if !request.modules.contains(&module) {
        selection.mode = "NONE";
        selection.modules.clear();
        selection.reason = "Module outside the proposed selection".into();
    } else {
        selection.modules = BTreeSet::from([module.clone()]);
    }
    selection
        .sources
        .retain(|path| config.module_of(path).is_some_and(|m| visible.contains(m)));
    let dirs: Vec<(PathBuf, bool)> = visible
        .iter()
        .flat_map(|m| {
            let target = request.workspace.join(m).join("target");
            [
                (target.join("classes"), false),
                (target.join("test-classes"), true),
            ]
        })
        .collect();
    let unselected = if selection.mode == "MODULES" {
        classes::load(&dirs)
            .and_then(|classes| {
                refine_policy(
                    &mut selection,
                    config,
                    &classes,
                    &request.workspace,
                    true,
                    request.preserve_modules.as_ref(),
                    &dirs,
                )
            })
            .unwrap_or_else(|error| {
                selection.reason = format!("Class-level selection unavailable: {error}");
                BTreeSet::new()
            })
    } else {
        BTreeSet::new()
    };
    let tests = request.workspace.join(&module).join("target/test-classes");
    let mut unselected: BTreeSet<String> = unselected
        .into_iter()
        .filter(|name| {
            tests
                .join(format!("{}.class", name.replace('.', "/")))
                .is_file()
        })
        .collect();
    let suite_key = format!("{module}:{suite}");
    let invalid = request.dir.join("invalid-inventory");
    if repeated && (request.capture_receipt || request.receipt.is_some()) {
        fs::create_dir_all(&invalid)?;
        fs::write(
            invalid.join(bytecode::hash(suite_key.as_bytes())),
            &suite_key,
        )?;
    }
    if let Some(classpath) = classpath.filter(|_| !repeated) {
        if request.capture_receipt || request.receipt.is_some() {
            let paths = fs::read_to_string(classpath)?
                .lines()
                .map(PathBuf::from)
                .collect();
            let mut exclusions =
                receipt_inventory(request, &BTreeMap::from([(suite_key.clone(), paths)]))?;
            if !invalid.join(bytecode::hash(suite_key.as_bytes())).exists() {
                unselected.extend(
                    exclusions
                        .remove(&suite_key)
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|test| {
                            !receipt_must_run(config, test, std::slice::from_ref(&tests))
                        }),
                );
            }
        }
    }
    let parts = request.dir.join("modules");
    fs::create_dir_all(&parts)?;
    let name = format!("{}-{suite}", bytecode::hash(module.as_bytes()));
    let mut refined = Refined::of(selection, unselected.clone());
    refined.module = Some(module);
    fs::write(
        parts.join(format!("{name}.json")),
        serde_json::to_vec(&refined)?,
    )?;
    if !request.observe && !unselected.is_empty() {
        let mut excludes = String::new();
        for name in &unselected {
            excludes += &format!("{}.*\n", name.replace('.', "/"));
            excludes += &format!("{}$*.*\n", name.replace('.', "/"));
        }
        let path = parts.join(format!("{name}.excludes"));
        fs::write(&path, excludes)?;
        println!("{}", path.display());
    }
    Ok(0)
}

fn main_result() -> Result<u8> {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_default();
    if command == "fixtures" {
        return fixtures::main(args.collect());
    }
    if command == "replay" {
        return replay::main(args.collect());
    }
    if command == "init" || command == "refresh" {
        return setup::init(args.collect(), command == "refresh");
    }
    match command.as_str() {
        "record" => return records::record(args.collect()),
        "decide" => return records::decide(args.collect()),
        "classes" => return classes_command(args.collect()),
        "env" => return records::env_command(args.collect()),
        "catalog" => return catalog::main(args.collect()),
        "classify" => return catalog::classify_main(args.collect()),
        _ => {}
    }
    if matches!(command.as_str(), "--version" | "-V") {
        println!("sieve {}", env!("CARGO_PKG_VERSION"));
        return Ok(0);
    }
    if matches!(command.as_str(), "" | "--help" | "-h") {
        println!(
            "sieve <select|run> [--workspace PATH] [--base REV | --full]\n\
                  [--records | --ci] [--output FILE] [--executable PATH] [--with|--without LEVERS]\n\
                  [--observe] [--receipt FILE] [--remaining FILE] [--task GOAL_OR_TASK]... [-- BUILD_ARGS...]\n\n\
                  sieve <init|refresh> [--workspace PATH] [--tool maven|gradle|jest|vitest] [--executable PATH]\n\
                  sieve env [--workspace PATH] [--base REV]\n\
                  sieve catalog --workspace PATH --catalog FILE [--plant] [--levers] [...]\n\
                  sieve classify --workspace PATH [--commits N]\n\
                  sieve replay --workspace PATH [--commits N] [--run] [-- BUILD_ARGS...]\n\
                  sieve fixtures <list|prepare|apply|check-selection|reports|verify|benchmark>\n\
                  sieve --version\n\n\
                  Local mode requires `run --records` or impact.json with records: true\n\
                  (single-module Maven or Gradle, test JVM on Java 17+; the first run records every test).\n\
                  --observe runs the native full task graph once and records the frozen proposal in --output.\n\
                  --task preserves explicit native goals/tasks, including non-test work on NONE.\n\
                  --receipt records successful executions; --remaining reuses them only at identical inputs.\n\
                  --ci keeps records that hold on other machines, for sharing through a CI cache.\n\
                  Static selection requires impact.json and the build adapters in README.md; no base or\n\
                  unavailable Git history selects ALL. run propagates build failures."
        );
        return Ok(0);
    }
    if command != "select" && command != "run" {
        return Err(format!("Unknown command: {command}").into());
    }
    let mut workspace = None;
    let mut base = None;
    let mut full = false;
    let mut records = false;
    let mut output = None;
    let mut executable = None;
    let mut levers = records::Levers::default();
    let mut ci = false;
    let mut observe = false;
    let mut receipt_output: Option<PathBuf> = None;
    let mut remaining: Option<PathBuf> = None;
    let mut tasks = Vec::new();
    let mut extra = Vec::new();
    while let Some(arg) = args.next() {
        if arg == "--" {
            extra.extend(args);
            break;
        }
        if arg == "--observe" {
            observe = true;
            continue;
        }
        if arg == "--full" {
            full = true;
            continue;
        }
        if arg == "--records" {
            records = true;
            continue;
        }
        // CI: local mode with records portable between machines and checkouts.
        if arg == "--ci" {
            records = true;
            ci = true;
            continue;
        }
        let value = args
            .next()
            .ok_or_else(|| format!("Missing value for {arg}"))?;
        match arg.as_str() {
            "--workspace" => workspace = Some(PathBuf::from(value)),
            "--base" => base = Some(value),
            "--output" => output = Some(PathBuf::from(value)),
            "--receipt" => receipt_output = Some(PathBuf::from(value)),
            "--remaining" => remaining = Some(PathBuf::from(value)),
            "--task" => {
                if value.is_empty()
                    || value.starts_with('-')
                    || value.chars().any(char::is_whitespace)
                {
                    return Err("--task expects one native goal or task, not build flags".into());
                }
                tasks.push(value);
            }
            "--executable" => executable = Some(value),
            "--with" => levers.on.extend(value.split(',').map(str::to_owned)),
            "--without" => levers.off.extend(value.split(',').map(str::to_owned)),
            _ => return Err(format!("Unknown option: {arg}").into()),
        }
    }
    if full && base.is_some() {
        return Err("Use either --base or --full".into());
    }
    let receipt_requested = receipt_output.is_some() || remaining.is_some();
    if command != "run" && (observe || receipt_requested || !tasks.is_empty()) {
        return Err("--observe, --receipt, --remaining and --task require run".into());
    }
    if observe && (output.is_none() || remaining.is_some()) {
        return Err("--observe requires --output and cannot be combined with --remaining".into());
    }
    if remaining.is_some() && (base.is_some() || full) {
        return Err("--remaining executes the whole task graph; omit --base and --full".into());
    }
    if remaining.is_some()
        && extra.iter().any(|arg| {
            arg == "--tests"
                || arg.starts_with("--tests=")
                || arg.starts_with("-Dtest=")
                || arg.starts_with("-Dit.test=")
        })
    {
        return Err(
            "Explicitly requested tests must run; omit --remaining when using native test filters"
                .into(),
        );
    }
    output = output.map(std::path::absolute).transpose()?;
    receipt_output = receipt_output.map(std::path::absolute).transpose()?;
    remaining = remaining.map(std::path::absolute).transpose()?;
    if output.is_some() && (output == receipt_output || output == remaining) {
        return Err("Selection output and receipt paths must be different".into());
    }
    let started = std::time::Instant::now();
    if records && command != "run" {
        return Err("--records and --ci are supported only by run".into());
    }
    let workspace = workspace
        .unwrap_or_else(|| PathBuf::from("."))
        .canonicalize()?;
    let local_default = records && !workspace.join("impact.json").exists();
    let config = match Config::read(&workspace) {
        Err(error) if local_default => Config::local_default(&workspace).ok_or(error)?,
        config => config?,
    };
    levers.on.extend(config.with.iter().cloned());
    levers.validate()?;
    if js::is_js(&config.tool) {
        if observe || receipt_requested || !tasks.is_empty() {
            return Err("Observation, receipts and native tasks support Maven and Gradle".into());
        }
        if records {
            return Err("--records and --ci support only Maven and Gradle".into());
        }
        return js::main(
            &command, &config, &workspace, base, output, executable, &extra,
        );
    }
    let local = records || config.records == Some(true);
    if local && (observe || receipt_requested || !tasks.is_empty()) {
        return Err("Observation, receipts and --task currently require static selection; local mode accepts native tasks after --".into());
    }
    if local && command == "run" {
        if local_default {
            eprintln!("No impact.json: local mode with defaults, records in .sieve/");
        }
        let run = records::Run {
            base,
            full,
            output,
            executable,
            levers,
            extra,
            ci,
        };
        return records::run(&config, &workspace, run);
    }
    let executable =
        executable.unwrap_or_else(|| setup::default_executable(&workspace, &config.tool));
    let mut comparison = None;
    let mut selection = match base {
        Some(base) => match changed_paths(&workspace, &base) {
            Ok((changed, prefix, merge_base)) => {
                match fingerprint::build_inputs_with_logic(&workspace, &config.build_logic) {
                    Ok(current) if config.build_fingerprint.as_ref() == Some(&current) => {
                        let selection = config.select(changed, &prefix);
                        comparison = Some((prefix, merge_base));
                        selection
                    }
                    // Build files changed since the last refresh. Only a module selection follows
                    // the graph, so it is checked against the graph the build declares now.
                    Ok(_) if config.build_fingerprint.is_some() => {
                        let mut selection = config.select(changed, &prefix);
                        if selection.mode == "MODULES" {
                            let fallback = match setup::stale(&config, &workspace, &executable, &extra) {
                            Ok(None) => {
                                eprintln!("sieve: impact.json is out of date, but the build's module graph still matches it; run sieve refresh and commit impact.json");
                                selection.reason += "; impact.json is out of date but matches the build's module graph (run sieve refresh)";
                                None
                            }
                            Ok(Some(change)) => Some(format!("The module graph changed since impact.json was written: {change}; run sieve refresh and review impact.json")),
                            Err(error) => Some(format!("Build inputs changed and the module graph cannot be checked ({error}); run sieve refresh and review impact.json")),
                        };
                            if let Some(reason) = fallback {
                                let changed = std::mem::take(&mut selection.changed);
                                selection = config.all(reason);
                                selection.changed = changed;
                            }
                        }
                        comparison = Some((prefix, merge_base));
                        selection
                    }
                    Ok(_) => {
                        let mut selection = config.all(
                            "No build fingerprint exists; run sieve refresh and review impact.json",
                        );
                        selection.changed = changed;
                        selection
                    }
                    Err(error) => config.all(format!(
                        "Cannot verify build inputs; full fallback: {error}"
                    )),
                }
            }
            Err(error) => config.all(format!(
                "Cannot establish changed inputs; full fallback: {error}"
            )),
        },
        None => config.all("Full run requested or no comparison base supplied"),
    };
    if records::explicit_tests(&extra) {
        let changed = std::mem::take(&mut selection.changed);
        selection = config.all("Explicit native test filter; executing requested tests");
        selection.changed = changed;
    }
    let preserve_modules = (!config.class_level).then(|| selection.modules.clone());
    if !config.must_run.is_empty() && selection.mode != "ALL" {
        selection.modules = config.modules.keys().cloned().collect();
        selection.mode = "MODULES";
        selection.reason += "; compiling test inventories to apply must_run policy";
    }
    let artifacts: Vec<PathBuf> = output
        .iter()
        .chain(receipt_output.iter())
        .chain(remaining.iter())
        .cloned()
        .collect();
    let mut invocation = if tasks.is_empty() {
        vec![
            "clean".into(),
            if config.tool == "maven" {
                "verify".into()
            } else {
                "check".into()
            },
        ]
    } else {
        tasks.clone()
    };
    invocation.extend(extra.iter().cloned());
    let receipt_before = if receipt_requested {
        match receipt::capture(&workspace, &executable, &invocation, &artifacts) {
            Ok(inputs) => Some(inputs),
            Err(error) => {
                eprintln!("sieve: receipts unavailable; all remaining tests will run: {error}");
                None
            }
        }
    } else {
        None
    };
    let accepted_receipt = remaining.as_ref().and_then(|path| {
        receipt_before
            .as_ref()
            .and_then(|inputs| match receipt::read(path, inputs) {
                Ok(receipt) => Some(receipt),
                Err(error) => {
                    eprintln!("sieve: receipt unavailable; full fallback: {error}");
                    None
                }
            })
    });
    if let Some(path) = &receipt_output {
        fs::write(path, "null\n")?;
    }
    if remaining.is_some() {
        selection = config.all(if accepted_receipt.is_some() {
            "Running the remaining tests using verified suite receipts"
        } else {
            "Receipt unavailable or inputs changed; full fallback"
        });
    }
    // Test hook: a selector that drops everything, which planted bugs must catch.
    if command == "run"
        && env::var_os("SIEVE_BROKEN_SELECTOR").is_some()
        && selection.mode != "NONE"
    {
        selection.mode = "NONE";
        selection.modules.clear();
        selection.reason = "SIEVE_BROKEN_SELECTOR drops every test".into();
    }
    let write = |selection: &Selection| -> Result<String> {
        let json = serde_json::to_string_pretty(selection)? + "\n";
        if let Some(path) = &output {
            fs::write(path, &json)?;
        }
        Ok(json)
    };
    let mut json = write(&selection)?;
    if command == "select" {
        print!("{json}");
        return Ok(0);
    }
    let init_script = if config.tool == "gradle" {
        Some(setup::gradle_script()?)
    } else {
        None
    };
    let mut script_args: Vec<String> = match &init_script {
        Some(script) => vec![
            "--init-script".into(),
            script.path().to_string_lossy().into_owned(),
        ],
        None => Vec::new(),
    };
    if config.tool == "gradle" && observe {
        script_args.push("-Pimpact.observe=true".into());
    }
    if observe || receipt_requested || !tasks.is_empty() {
        clear_reports(&config, &workspace)?;
    } else {
        remove_stale_reports(&config, &selection, &workspace)?;
    }
    let temp = tempfile::tempdir()?;
    let mut filter = None;
    let mut unselected = BTreeSet::new();
    let class_level =
        (config.class_level || !config.must_run.is_empty()) && selection.mode == "MODULES";
    let hooking = class_level || receipt_requested;
    let proposed_modules = selection.modules.clone();
    // The comparison base generates the sources of changed generated inputs in a worktree.
    let inputs: BTreeSet<String> = selection
        .sources
        .iter()
        .filter(|path| config.generated_input(path))
        .cloned()
        .collect();
    // Test classes are selected inside the one build that runs them, between compiling and
    // testing: by Gradle's `impactSelect`, or by Sieve's Maven extension. A Maven build that
    // compares generated sources, or loads extensions of its own the same way, compiles first.
    let own_extensions = extra.iter().any(|a| a.contains("maven.ext.class.path"))
        || fs::read_to_string(workspace.join(".mvn/maven.config"))
            .is_ok_and(|c| c.contains("maven.ext.class.path"));
    let extension = match config.tool.as_str() {
        "maven" if hooking && inputs.is_empty() && !own_extensions => {
            records::maven_extension().ok()
        }
        _ => None,
    };
    let mut request = None;
    let mut selecting = Vec::new();
    if receipt_requested && config.tool == "maven" && extension.is_none() {
        return Err("Execution receipts require the embedded Maven extension and no conflicting maven.ext.class.path/generated-input comparison".into());
    }
    if hooking && (config.tool == "gradle" || extension.is_some()) {
        let dir = temp.path().to_path_buf();
        let path = dir.join("request.json");
        let asked = ClassRequest {
            workspace: workspace.clone(),
            modules: selection.modules.clone(),
            sources: selection.sources.clone(),
            changed: selection.changed.clone(),
            reason: selection.reason.clone(),
            dir: dir.clone(),
            output: output.clone(),
            observe,
            preserve_modules: preserve_modules.clone(),
            mode: selection.mode.into(),
            capture_receipt: receipt_requested,
            receipt: accepted_receipt,
        };
        fs::write(&path, serde_json::to_vec(&asked)?)?;
        let exe = env::current_exe()?.canonicalize()?;
        selecting = match &extension {
            Some(jar) => vec![
                format!("-Dmaven.ext.class.path={}", jar.display()),
                format!("-Dsieve.request={}", path.display()),
                format!("-Dsieve.exe={}", exe.display()),
            ],
            None => vec![
                format!("-Pimpact.classes={}", dir.join("classes.json").display()),
                format!("-Pimpact.selected={}", dir.join("selected.txt").display()),
                format!("-Pimpact.request={}", path.display()),
                format!("-Pimpact.sieve={}", exe.display()),
            ],
        };
        if config.tool == "maven" && receipt_requested {
            selecting.push("-Dsieve.receipt=true".into());
        }
        if config.tool == "gradle" {
            if receipt_requested {
                selecting.push(format!(
                    "-Pimpact.remaining={}",
                    dir.join("remaining.json").display()
                ));
            }
            selecting.push(format!(
                "-Pimpact.modules={}",
                proposed_modules
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        request = Some(asked);
    } else if class_level {
        eprintln!("{json}Compiling for class-level selection");
        let base = match &comparison {
            Some((prefix, merge_base)) if !inputs.is_empty() && config.tool == "maven" => {
                Some(generated::Base::start(
                    &config,
                    &selection.modules,
                    &workspace,
                    prefix,
                    merge_base,
                    &executable,
                    &extra,
                ))
            }
            _ => None,
        };
        let status = Command::new(&executable)
            .current_dir(&workspace)
            .args(compile_args(&config, &selection))
            .args(&extra)
            .status()?;
        if !status.success() {
            return Ok(1);
        }
        // Analysis problems keep the module selection instead of failing the build.
        unselected = base
            .map(|base| base.and_then(|base| base.changes(&selection.modules, &workspace)))
            .transpose()
            .and_then(|generated| {
                // Generated inputs are replaced by the generated sources they changed.
                if let Some(generated) = generated {
                    selection.sources.retain(|path| !inputs.contains(path));
                    selection.sources.extend(generated.iter().cloned());
                    selection.generated = generated;
                }
                // Maven's class directories follow from the module layout; no listing.
                class_dirs(&config, &workspace, Path::new(""))
            })
            .and_then(|dirs| {
                let classes = classes::load(&dirs)?;
                refine_policy(
                    &mut selection,
                    &config,
                    &classes,
                    &workspace,
                    true,
                    preserve_modules.as_ref(),
                    &dirs,
                )
            })
            .unwrap_or_else(|error| {
                selection.reason = format!("Class-level selection unavailable: {error}");
                BTreeSet::new()
            });
        // Surefire drops its default nested-class exclude once an excludes file is given.
        let mut excludes = String::from("**/*$*\n");
        for name in &unselected {
            excludes += &format!("{}.*\n", name.replace('.', "/"));
        }
        let path = temp.path().join("tests.txt");
        fs::write(&path, excludes)?;
        filter = Some(path);
        json = write(&selection)?;
    }
    eprintln!("{json}");
    let execution = config.all("Full execution for shadow observation");
    let execute = if observe { &execution } else { &selection };
    let build_filter = if observe { None } else { filter.as_deref() };
    let status = Command::new(&executable)
        .current_dir(&workspace)
        .args(task_args(&config, execute, build_filter, &tasks))
        .args(selecting)
        .args(script_args)
        .args(&extra)
        .status()?;
    // A build that got as far as selecting reports what it selected.
    let mut module_proposals: BTreeMap<String, observation::Proposal> = BTreeMap::new();
    if let Some(request) = &request {
        let files: Vec<PathBuf> = if config.tool == "gradle" {
            vec![request.dir.join("refined.json")]
        } else {
            fs::read_dir(request.dir.join("modules"))
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|e| e == "json"))
                .collect()
        };
        let parts: Vec<Refined> = files
            .iter()
            .filter_map(|file| serde_json::from_slice::<Refined>(&fs::read(file).ok()?).ok())
            .collect();
        for part in &parts {
            if let Some(module) = &part.module {
                let proposal = observation::Proposal {
                    mode: part.mode.clone(),
                    modules: BTreeSet::from([module.clone()]),
                    tests: part.tests.clone(),
                    reason: part.reason.clone(),
                };
                module_proposals
                    .entry(module.clone())
                    .and_modify(|existing| {
                        existing.tests.extend(proposal.tests.iter().cloned());
                        if matches!(proposal.mode.as_str(), "ALL" | "MODULES") {
                            existing.mode = proposal.mode.clone();
                        } else if existing.mode == "NONE" && proposal.mode == "SUBSET" {
                            existing.mode = "SUBSET".into();
                        }
                    })
                    .or_insert(proposal);
            }
        }
        merge_refined(parts, &mut selection, &mut unselected);
    }
    if observe {
        let observed = observation::finish(
            observation::Proposal::of(&selection),
            module_proposals,
            &workspace,
            &config.tool,
            status.success(),
            &config.build_logic,
        );
        eprintln!(
            "sieve: {} ({} missed failing classes)",
            observed.evidence,
            observed.missed_failed_classes.len()
        );
        selection.observation = Some(observed);
    }
    if let Some(path) = &receipt_output {
        if let Some(before) = &receipt_before {
            let result = (|| -> Result<()> {
                let after = receipt::capture(&workspace, &executable, &invocation, &artifacts)?;
                let mut inventory = receipt::Inventory::new();
                if let Some(request) = &request {
                    for entry in fs::read_dir(request.dir.join("inventory"))
                        .into_iter()
                        .flatten()
                        .flatten()
                    {
                        let part: receipt::Inventory =
                            serde_json::from_slice(&fs::read(entry.path())?)?;
                        inventory.extend(part);
                    }
                    for entry in fs::read_dir(request.dir.join("invalid-inventory"))
                        .into_iter()
                        .flatten()
                        .flatten()
                    {
                        inventory.remove(&fs::read_to_string(entry.path())?);
                    }
                    // A passing result cannot cover inputs rewritten while its tests ran.
                    for entry in fs::read_dir(request.dir.join("inventory-inputs"))
                        .into_iter()
                        .flatten()
                        .flatten()
                    {
                        let suites: BTreeMap<String, Vec<PathBuf>> =
                            serde_json::from_slice(&fs::read(entry.path())?)?;
                        let current = receipt::inventory(&suites).unwrap_or_default();
                        for suite in suites.keys() {
                            if current.get(suite) != inventory.get(suite) {
                                inventory.remove(suite);
                            }
                        }
                    }
                }
                receipt::write(
                    path,
                    before,
                    &after,
                    &reports::read_reports(&workspace, &config.tool)?,
                    status.success(),
                    inventory,
                )
            })();
            if let Err(error) = result {
                eprintln!("sieve: no reusable receipt: {error}");
            }
        }
    }
    let modules: BTreeSet<String> = config.modules.keys().cloned().collect();
    // The summary is informational: a problem with it never fails the run. Durations persist
    // only where local mode keeps `.sieve/`: static selection leaves the workspace alone.
    let sieve_dir = workspace.join(".sieve");
    match summary::finish(
        sieve_dir.is_dir(),
        &sieve_dir,
        &workspace,
        &config.tool,
        true,
        &unselected,
        None,
        &summary::test_dirs(&workspace, &modules, &config.tool),
        started.elapsed().as_secs_f64(),
    ) {
        Ok(summary) => {
            for line in summary.lines() {
                eprintln!("{line}");
            }
            summary.publish(&selection, config.modules.len());
            selection.summary = Some(summary);
            write(&selection)?;
        }
        Err(error) => eprintln!("sieve: no summary: {error}"),
    }
    write(&selection)?;
    let observed_failure = selection.observation.as_ref().is_some_and(|observation| {
        !observation.failed_classes.is_empty() || observation.report_error.is_some()
    });
    Ok(if status.success() && !observed_failure {
        0
    } else {
        1
    })
}

fn main() -> ExitCode {
    match main_result() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("Error: {error}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_nested_must_run_policy_always_keeps_its_outer_class() {
        let config: Config = serde_json::from_value(serde_json::json!({
            "tool": "maven", "modules": {".": []},
            "must_run": ["example.ContractTest$Nested*", "example.Smoke?Test"]
        }))
        .unwrap();
        assert!(config.must_run("example.ContractTest"));
        assert!(config.must_run("example.ContractTest$NestedCase"));
        assert!(config.must_run("example.Smoke1Test"));
        assert!(!config.must_run("example.OtherTest"));
    }

    #[test]
    fn receipt_policy_keeps_outer_classes_with_wildcard_nested_matches() {
        let config: Config = serde_json::from_value(serde_json::json!({
            "tool": "maven", "modules": {".": []}, "must_run": ["*Inner*"]
        }))
        .unwrap();
        let temp = tempfile::tempdir().unwrap();
        let package = temp.path().join("example");
        fs::create_dir(&package).unwrap();
        fs::write(package.join("OuterTest$InnerCase.class"), []).unwrap();
        let dirs = vec![temp.path().to_owned()];
        assert!(receipt_must_run(&config, "example.OuterTest", &dirs));
        assert!(!receipt_must_run(&config, "example.OtherTest", &dirs));
        fs::remove_dir_all(&package).unwrap();
        fs::write(package, []).unwrap();
        assert!(receipt_must_run(&config, "example.OuterTest", &dirs));
    }

    #[test]
    fn must_run_keeps_selected_modules_without_class_level_opt_in() {
        let temp = tempfile::tempdir().unwrap();
        let config: Config = serde_json::from_value(serde_json::json!({
            "tool": "maven", "modules": {"app": [], "checks": []},
            "must_run": ["example.Contract*Test"]
        }))
        .unwrap();
        let mut graph = Vec::new();
        let mut dirs = Vec::new();
        for (module, names) in [
            ("app", vec!["example/AppTest", "example/OtherTest"]),
            (
                "checks",
                vec!["example/ContractApiTest", "example/UnrelatedTest"],
            ),
        ] {
            let dir = temp.path().join(module).join("target/test-classes");
            fs::create_dir_all(dir.join("example")).unwrap();
            for name in names {
                fs::write(dir.join(format!("{name}.class")), "").unwrap();
                graph.push(classes::Class {
                    name: name.into(),
                    test: true,
                    ..Default::default()
                });
            }
            dirs.push((dir, true));
        }
        let mut selection = config.all("discover policy classes");
        selection.mode = "MODULES";
        let unselected = refine_policy(
            &mut selection,
            &config,
            &graph,
            temp.path(),
            true,
            Some(&BTreeSet::from(["app".into()])),
            &dirs,
        )
        .unwrap();
        assert_eq!(
            selection.tests,
            BTreeSet::from([
                "example.AppTest".into(),
                "example.OtherTest".into(),
                "example.ContractApiTest".into()
            ])
        );
        assert_eq!(unselected, BTreeSet::from(["example.UnrelatedTest".into()]));
        assert_eq!(
            selection.reasons["example.ContractApiTest"],
            "Required by must_run policy"
        );
    }

    #[test]
    fn graph_closure_handles_chains_diamonds_roots_and_duplicates() {
        // DEP-02/08/09: reverse alphabetical names force more than one traversal.
        let config: Config = serde_json::from_value(serde_json::json!({
            "tool":"maven", "modules": {
                "zcore":[], "yleft":["zcore", "zcore"], "xright":["zcore"],
                "wjoin":["yleft","xright"], "vapp":["wjoin"], ".":["vapp"], "isolated":[]
            }
        }))
        .unwrap();
        for (path, expected) in [
            (
                "zcore/src/main/java/Provider.java",
                vec![".", "vapp", "wjoin", "xright", "yleft", "zcore"],
            ),
            ("vapp/src/test/java/AppTest.java", vec![".", "vapp"]),
            ("src/test/java/RootTest.java", vec!["."]),
            ("isolated/src/main/resources/template", vec!["isolated"]),
        ] {
            assert_eq!(
                config.select(BTreeSet::from([path.into()]), "").modules,
                expected.into_iter().map(str::to_owned).collect()
            );
        }
    }

    #[test]
    fn test_only_changes_reach_dependents_through_shared_tests_only() {
        let mut config: Config = serde_json::from_value(serde_json::json!({
            "tool": "maven", "modules": {"core": [], "app": ["core"], "web": ["app"]},
            "shared_tests": []
        }))
        .unwrap();
        let select = |config: &Config, paths: &[&str]| {
            let changed = paths.iter().map(|p| p.to_string()).collect();
            let modules = config.select(changed, "").modules;
            modules.into_iter().collect::<Vec<_>>()
        };
        assert_eq!(
            select(&config, &["core/src/test/java/a/CoreTest.java"]),
            ["core"]
        );
        assert_eq!(
            select(&config, &["core/src/test/resources/in.json"]),
            ["core"]
        );
        // Main code and test fixtures reach the modules that depend on them.
        let everything = ["app", "core", "web"];
        assert_eq!(
            select(&config, &["core/src/main/java/a/Core.java"]),
            everything
        );
        assert_eq!(
            select(&config, &["core/src/testFixtures/java/a/F.java"]),
            everything
        );
        // A module that a change reaches still passes it on, whatever else changed in it.
        assert_eq!(
            select(
                &config,
                &["core/src/main/java/a/Core.java", "app/src/test/java/T.java"]
            ),
            everything
        );
        config.shared_tests = Some(vec!["core".into()]);
        assert_eq!(
            select(&config, &["core/src/test/java/a/CoreTest.java"]),
            everything
        );
        // Without the list, as in configurations written before it existed, nothing narrows.
        config.shared_tests = None;
        assert_eq!(
            select(&config, &["core/src/test/java/a/CoreTest.java"]),
            everything
        );
        let root: Config = serde_json::from_value(serde_json::json!({
            "tool": "gradle", "modules": {".": ["core"], "core": []}, "shared_tests": []
        }))
        .unwrap();
        assert_eq!(select(&root, &["src/test/java/RootTest.java"]), ["."]);
        assert_eq!(
            select(&root, &["core/src/test/java/CoreTest.java"]),
            ["core"]
        );
        assert_eq!(
            select(&root, &["core/src/main/java/Core.java"]),
            [".", "core"]
        );
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("core")).unwrap();
        let unknown: Config = serde_json::from_value(serde_json::json!({
            "tool": "maven", "modules": {"core": []}, "shared_tests": ["gone"]
        }))
        .unwrap();
        assert!(unknown.validate(temp.path()).is_err());
    }

    #[test]
    fn nested_modules_select_and_build_by_directory() {
        let temp = tempfile::tempdir().unwrap();
        for dir in ["libs/core", "services/orders/plugin"] {
            fs::create_dir_all(temp.path().join(dir)).unwrap();
        }
        let mut config: Config = serde_json::from_value(serde_json::json!({
            "tool": "maven", "shared_tests": [], "modules": {
                "libs/core": [], "services/orders": ["libs/core"], "services/orders/plugin": []
            }
        }))
        .unwrap();
        assert!(config.validate(temp.path()).is_ok());
        let select = |config: &Config, path: &str| config.select(BTreeSet::from([path.into()]), "");
        // The innermost module owns its sources; an aggregator's POM is a build input.
        let plugin = select(&config, "services/orders/plugin/src/main/java/P.java");
        assert_eq!(
            plugin.modules,
            BTreeSet::from(["services/orders/plugin".into()])
        );
        assert_eq!(select(&config, "services/pom.xml").mode, "ALL");
        let core = select(&config, "libs/core/src/main/java/C.java");
        assert_eq!(
            build_args(&config, &core, None)[2..],
            [
                "clean",
                "verify",
                "-pl",
                "libs/core,services/orders",
                "-am",
                "-Dimpact.skip.libs.core=false",
                "-Dimpact.skip.services.orders=false",
                "-Dimpact.skip.services.orders.plugin=true"
            ]
        );
        config.tool = "gradle".into();
        assert_eq!(
            build_args(&config, &core, None)[2..],
            [
                "clean",
                ":libs:core:check",
                ":services:orders:check",
                "-Pimpact.modules=libs/core,services/orders"
            ]
        );
        for bad in ["a//b", "../x", "a/./b", "/abs", "a/", "a/../b"] {
            assert!(!valid_module(bad), "{bad}");
        }
    }

    #[test]
    fn refined_parts_merge_into_one_selection() {
        let config: Config = serde_json::from_value(serde_json::json!({
            "tool": "maven", "modules": {"a": [], "b": ["a"]}
        }))
        .unwrap();
        let names = |list: &[&str]| list.iter().map(|t| t.to_string()).collect::<BTreeSet<_>>();
        let part = |mode: &str, tests: &[&str], unselected: &[&str]| Refined {
            module: None,
            mode: mode.into(),
            tests: names(tests),
            reasons: tests
                .iter()
                .map(|t| (t.to_string(), "reaches".into()))
                .collect(),
            reason: format!("{mode} reason"),
            unselected: names(unselected),
        };
        let merged = |parts: Vec<Refined>| {
            let mut selection = config.all("module selection");
            selection.mode = "MODULES";
            let mut unselected = BTreeSet::new();
            merge_refined(parts, &mut selection, &mut unselected);
            (selection, unselected)
        };
        let (selection, unselected) = merged(vec![
            part("SUBSET", &["a.T"], &["a.U"]),
            part("NONE", &[], &["b.V"]),
        ]);
        assert_eq!(
            (selection.mode, selection.reason.as_str()),
            ("SUBSET", "SUBSET reason")
        );
        assert_eq!(selection.tests, names(&["a.T"]));
        assert_eq!(unselected, names(&["a.U", "b.V"]));
        let (selection, _) = merged(vec![part("NONE", &[], &[]), part("NONE", &[], &["b.V"])]);
        assert_eq!(selection.mode, "NONE");
        // A module that fell back ran every test.
        let (selection, _) = merged(vec![
            part("SUBSET", &["a.T"], &[]),
            part("MODULES", &[], &[]),
        ]);
        assert_eq!(
            (selection.mode, selection.reason.as_str()),
            ("MODULES", "MODULES reason")
        );
        // A build that reported nothing keeps the module selection.
        let (selection, _) = merged(Vec::new());
        assert_eq!(
            (selection.mode, selection.reason.as_str()),
            ("MODULES", "module selection")
        );
    }

    #[test]
    fn glob_segments() {
        for (pattern, path, expected) in [
            ("README.md", "README.md", true),
            ("README.md", "api/README.md", false),
            ("docs/**", "docs/a/b.md", true),
            ("docs/**", "docsx/a.md", false),
            ("**/*.md", "README.md", true),
            ("**/*.md", "a/b/c.md", true),
            ("*.md", "a/c.md", false),
            ("?.txt", "a.txt", true),
            ("?.txt", "/.txt", false),
            ("a/**/z", "a/z", true),
            ("a/**/z", "a/b/c/z", true),
        ] {
            assert_eq!(
                glob(pattern.as_bytes(), path.as_bytes()),
                expected,
                "{pattern} {path}"
            );
        }
    }

    #[test]
    fn local_mode_settings_are_validated() {
        let temp = tempfile::tempdir().unwrap();
        let config = |extra: serde_json::Value| -> Config {
            let mut value = serde_json::json!({"tool": "maven", "modules": {".": []}});
            value
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            serde_json::from_value(value).unwrap()
        };
        let valid = config(serde_json::json!({
            "with": ["jgitver"], "record_ignore_properties": ["git.*", "build.number"]
        }));
        assert!(valid.validate(temp.path()).is_ok());
        for extra in [
            serde_json::json!({"with": ["turbo"]}),
            serde_json::json!({"record_ignore_properties": ["*"]}),
            serde_json::json!({"record_ignore_properties": ["a*b"]}),
            serde_json::json!({"record_ignore_properties": ["a b"]}),
        ] {
            assert!(
                config(extra.clone()).validate(temp.path()).is_err(),
                "{extra}"
            );
        }
    }

    #[test]
    fn javascript_settings_are_validated() {
        let temp = tempfile::tempdir().unwrap();
        let config =
            |value: serde_json::Value| -> Config { serde_json::from_value(value).unwrap() };
        let js = |extra: serde_json::Value| {
            let mut value =
                serde_json::json!({"tool": "vitest", "modules": {".": []}, "sources": ["src"]});
            value
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            config(value)
        };
        assert!(js(serde_json::json!({})).validate(temp.path()).is_ok());
        for extra in [
            serde_json::json!({"sources": []}),
            serde_json::json!({"sources": ["."]}),
            serde_json::json!({"sources": ["../x"]}),
            serde_json::json!({"class_level": true}),
            serde_json::json!({"records": true}),
            serde_json::json!({"build_fingerprint": "x"}),
            serde_json::json!({"modules": {".": [], "a": []}}),
        ] {
            assert!(js(extra.clone()).validate(temp.path()).is_err(), "{extra}");
        }
        let maven = serde_json::json!({"tool": "maven", "modules": {".": []}, "sources": ["src"]});
        assert!(config(maven).validate(temp.path()).is_err());
    }

    #[test]
    fn ignore_patterns_are_workspace_or_repository_relative() {
        let mut config: Config = serde_json::from_value(serde_json::json!({
            "tool": "maven", "modules": {"core": []}
        }))
        .unwrap();
        let mode = |config: &Config, path: &str, prefix: &str| {
            config.select(BTreeSet::from([path.into()]), prefix).mode
        };
        // Defaults cover documentation outside module sources, and the repository's top-level
        // and `docs/` documentation, nothing else.
        assert_eq!(mode(&config, "README.md", "app"), "NONE");
        assert_eq!(mode(&config, "@repository/docs/x.md", "app"), "NONE");
        assert_eq!(mode(&config, "@repository/CHANGELOG.md", "app"), "NONE");
        assert_eq!(mode(&config, "VALIDATION.md", ""), "NONE");
        assert_eq!(mode(&config, "core/README.md", ""), "NONE");
        assert_eq!(mode(&config, "core/docs/guide.adoc", ""), "NONE");
        assert_eq!(
            mode(&config, "core/src/main/resources/help.md", ""),
            "MODULES"
        );
        assert_eq!(mode(&config, "@repository/other/README.md", "app"), "ALL");
        assert_eq!(mode(&config, "core/notes.txt", ""), "ALL");
        // Local mode hashes the same paths: module sources stay inputs.
        assert!(!config.ignored("core/src/test/resources/expected.md", ""));
        config.ignore = Some(vec!["**/*.md".into(), "/other/**".into()]);
        assert_eq!(mode(&config, "core/README.md", ""), "NONE");
        assert_eq!(mode(&config, "@repository/other/src/A.java", "app"), "NONE");
        // Anchored patterns see workspace paths under their repository location.
        assert_eq!(mode(&config, "other/A.java", "app"), "ALL");
        assert_eq!(mode(&config, "other/A.java", ""), "NONE");
        // Unanchored patterns never match outside the workspace.
        assert_eq!(mode(&config, "@repository/x.md", "app"), "ALL");
        assert_eq!(mode(&config, "core/src/A.java", "app"), "MODULES");
        // Ignore globs apply inside module source directories too.
        config.ignore = Some(vec!["core/src/site/**".into()]);
        assert_eq!(mode(&config, "core/src/site/index.md", ""), "NONE");
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("core")).unwrap();
        assert!(config.validate(temp.path()).is_ok());
        config.ignore = Some(vec!["/".into()]);
        assert!(config.validate(temp.path()).is_err());
    }

    #[test]
    fn conservative_selection_and_build_filters() {
        let mut config: Config =
            serde_json::from_str(include_str!("../projects/maven/impact.json")).unwrap();
        let select = |path: &str| config.select(BTreeSet::from([path.into()]), "");
        let tax = select("pricing/src/main/java/example/TaxRules.java");
        assert_eq!(
            tax.modules,
            BTreeSet::from(["checkout".into(), "pricing".into()])
        );
        // Only the selected modules and their dependencies build.
        assert_eq!(
            build_args(&config, &tax, None),
            [
                "-B",
                "-ntp",
                "clean",
                "verify",
                "-pl",
                "checkout,pricing",
                "-am",
                "-Dimpact.skip.checkout=false",
                "-Dimpact.skip.pricing=false",
                "-Dimpact.skip.runtime=true"
            ]
        );
        assert_eq!(
            select("runtime/src/main/resources/application.properties").modules,
            BTreeSet::from(["runtime".into()])
        );
        assert_eq!(
            select("runtime/src/main/resources/docs/readme.md").mode,
            "MODULES"
        );
        assert_eq!(select("pom.xml").mode, "ALL");
        assert_eq!(select("impact.json").mode, "ALL");
        // Default ignore patterns yield to module sources; explicit ones do not.
        let mut docs: Config = serde_json::from_value(serde_json::json!({
            "tool": "maven", "modules": {"docs": [], "app": ["docs"]}
        }))
        .unwrap();
        let path = || BTreeSet::from(["docs/src/test/java/SnippetTest.java".into()]);
        assert_eq!(docs.select(path(), "").modules.len(), 2);
        assert_eq!(
            docs.select(BTreeSet::from(["docs/guide.md".into()]), "")
                .mode,
            "NONE"
        );
        docs.ignore = Some(vec!["docs/**".into()]);
        assert_eq!(docs.select(path(), "").mode, "NONE");
        assert_eq!(select("removed/src/main/java/Old.java").mode, "ALL");
        let docs = select("README.md");
        assert_eq!(docs.mode, "NONE");
        assert_eq!(build_args(&config, &docs, None), ["-B", "-ntp", "clean"]);
        let all = select("pom.xml");
        assert_eq!(
            build_args(&config, &all, None),
            ["-B", "-ntp", "clean", "verify"]
        );
        assert_eq!(config.select(BTreeSet::new(), "").mode, "NONE");
        config.tool = "gradle".into();
        assert_eq!(
            build_args(&config, &tax, None),
            [
                "--no-daemon",
                "--console=plain",
                "clean",
                ":checkout:check",
                ":pricing:check",
                "-Pimpact.modules=checkout,pricing"
            ]
        );
        let mut root = config.select(BTreeSet::from(["src/main/java/Root.java".into()]), "");
        root.mode = "MODULES";
        root.modules = BTreeSet::from([".".into(), "pricing".into()]);
        assert_eq!(
            build_args(&config, &root, None)[3..5],
            [":check", ":pricing:check"]
        );
        // A cycle terminates with both of its modules.
        config
            .modules
            .get_mut("pricing")
            .unwrap()
            .push("checkout".into());
        assert_eq!(
            config
                .select(
                    BTreeSet::from(["checkout/src/main/java/New.java".into()]),
                    ""
                )
                .modules
                .len(),
            2
        );
    }

    #[test]
    fn sample_configurations_are_current() {
        for sample in ["bookstore", "webshop"] {
            let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("samples")
                .join(sample);
            let config = Config::read(&workspace).unwrap();
            assert!(config.class_level, "{sample}");
            assert_eq!(
                config.build_fingerprint,
                Some(fingerprint::build_inputs(&workspace).unwrap()),
                "{sample}"
            );
        }
    }

    #[test]
    fn class_level_refinement_and_build_args() {
        let temp = tempfile::tempdir().unwrap();
        for module in ["app", "core", "other"] {
            fs::create_dir(temp.path().join(module)).unwrap();
        }
        let mut config: Config = serde_json::from_value(serde_json::json!({
            "tool": "maven", "modules": { "app": ["core"], "core": [], "other": [] },
            "class_level": true
        }))
        .unwrap();
        assert!(config.validate(temp.path()).is_ok());
        let source = "core/src/main/java/a/Calc.java";
        fs::create_dir_all(temp.path().join("core/src/main/java/a")).unwrap();
        fs::write(temp.path().join(source), "").unwrap();
        let class = |name: &str, source: &str, test: bool, refs: &[&str]| classes::Class {
            name: name.into(),
            source: Some(source.into()),
            refs: refs.iter().map(|r| r.to_string()).collect(),
            test,
            ..Default::default()
        };
        let mut graph = vec![
            class("a/Calc", "a/Calc.java", false, &[]),
            class("b/CalcTest", "b/CalcTest.java", true, &["a/Calc"]),
            class("a/OtherTest", "a/OtherTest.java", true, &[]),
            class("c/UpstreamTest", "c/UpstreamTest.java", true, &["a/Calc"]),
        ];
        for (module, test) in [("app", "b/CalcTest"), ("other", "c/UpstreamTest")] {
            let file = temp
                .path()
                .join(module)
                .join("target/test-classes")
                .join(test);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file.with_extension("class"), "").unwrap();
        }
        let changed = || BTreeSet::from([source.into(), "README.md".into()]);
        let mut selection = config.select(changed(), "");
        assert_eq!(selection.mode, "MODULES");
        let listing = Path::new("/tmp/classes.json");
        assert_eq!(
            compile_args(&config, &selection),
            [
                "-B",
                "-ntp",
                "clean",
                "test-compile",
                "-pl",
                "app,core",
                "-am"
            ]
        );
        // Unselected modules may lie on a path from a test to the change.
        assert_eq!(class_dirs(&config, temp.path(), listing).unwrap().len(), 6);
        // Tests compiled in modules that stay skipped are not reported.
        let unselected = selection.refine(&graph, temp.path(), true).unwrap();
        assert_eq!(selection.mode, "SUBSET");
        assert_eq!(selection.tests, BTreeSet::from(["b.CalcTest".into()]));
        assert_eq!(unselected, BTreeSet::from(["a.OtherTest".into()]));
        let filter = Path::new("/tmp/tests.txt");
        assert_eq!(
            build_args(&config, &selection, Some(filter)),
            [
                "-B",
                "-ntp",
                "verify",
                "-pl",
                "app,core",
                "-am",
                "-Dimpact.skip.app=false",
                "-Dimpact.skip.core=false",
                "-Dimpact.skip.other=true",
                "-Dmaven.main.skip=true",
                "-Dsurefire.excludesFile=/tmp/tests.txt",
                "-Dfailsafe.excludesFile=/tmp/tests.txt"
            ]
        );
        // Gradle selects inside its one build, which cleans like a module selection.
        config.tool = "gradle".into();
        assert_eq!(
            build_args(&config, &config.select(changed(), ""), None),
            [
                "--no-daemon",
                "--console=plain",
                "clean",
                ":app:check",
                ":core:check",
                "-Pimpact.modules=app,core"
            ]
        );
        config.tool = "maven".into();
        // No test in a selected module reaches the class: compile only, still without clean.
        graph[1].refs.clear();
        let mut selection = config.select(changed(), "");
        selection.refine(&graph, temp.path(), true).unwrap();
        assert_eq!(selection.mode, "NONE");
        assert_eq!(
            selection.modules,
            BTreeSet::from(["app".into(), "core".into()])
        );
        assert!(build_args(&config, &selection, Some(filter)).ends_with(&[
            "-Dimpact.skip.app=true".into(),
            "-Dimpact.skip.core=true".into(),
            "-Dimpact.skip.other=true".into(),
            "-Dmaven.main.skip=true".into()
        ]));
        // Fallbacks keep the module selection.
        let mut selection = config.select(
            BTreeSet::from(["core/src/main/java/a/Gone.java".into()]),
            "",
        );
        selection.refine(&graph, temp.path(), true).unwrap();
        assert_eq!(selection.mode, "MODULES");
        assert!(selection.reason.contains("deleted"), "{}", selection.reason);
        // A single module builds without a reactor restriction.
        let single: Config = serde_json::from_value(serde_json::json!({
            "tool": "maven", "modules": { ".": [] }, "class_level": true
        }))
        .unwrap();
        let mut selection = single.select(BTreeSet::from(["src/main/java/a/Calc.java".into()]), "");
        selection.mode = "SUBSET";
        assert_eq!(
            build_args(&single, &selection, Some(filter))[..4],
            ["-B", "-ntp", "verify", "-Dimpact.skip.root=false"]
        );
    }
}
