use serde::{Deserialize, Serialize};
mod classes;
mod fingerprint;
mod fixtures;
mod replay;
mod setup;
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
    /// Globs for changes that select nothing. Relative to the workspace; a leading `/`
    /// anchors the pattern at the repository root. Absent means `DEFAULT_IGNORE`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ignore: Option<Vec<String>>,
    /// `run` compiles the selected modules first and narrows the selection to the test
    /// classes whose bytecode reaches a changed class.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    class_level: bool,
}

const DEFAULT_IGNORE: &[&str] = &["README.md", "docs/**", "/README.md", "/docs/**"];

const REPOSITORY: &str = "@repository/";

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
    reason: String,
}

impl Config {
    fn read(workspace: &Path) -> Result<Self> {
        let config: Self = serde_json::from_slice(&fs::read(workspace.join("impact.json"))?)?;
        config.validate(workspace)?;
        Ok(config)
    }

    fn validate(&self, workspace: &Path) -> Result<()> {
        if !matches!(self.tool.as_str(), "maven" | "gradle") || self.modules.is_empty() {
            return Err("impact.json requires tool maven/gradle and a nonempty module map".into());
        }
        for (module, dependencies) in &self.modules {
            if module.is_empty()
                || module == ".."
                || !module
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
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
        Ok(())
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
            None => DEFAULT_IGNORE.iter().any(|p| matches(p)),
        }
    }

    fn all(&self, reason: impl Into<String>) -> Selection {
        Selection {
            mode: "ALL",
            modules: self.modules.keys().cloned().collect(),
            tests: BTreeSet::new(),
            changed: BTreeSet::new(),
            sources: BTreeSet::new(),
            reason: reason.into(),
        }
    }

    fn select(&self, changed: BTreeSet<String>, prefix: &str) -> Selection {
        let mut selected = BTreeSet::new();
        let mut sources = BTreeSet::new();
        for path in &changed {
            if self.ignored(path, prefix) {
                continue;
            }
            if self.modules.contains_key(".") && path.starts_with("src/") {
                selected.insert(".".into());
                sources.insert(path.clone());
                continue;
            }
            // Build/configuration changes may alter the module graph or test discovery.
            if let Some((module, rest)) = path.split_once('/') {
                if self.modules.contains_key(module) && rest.starts_with("src/") {
                    selected.insert(module.to_owned());
                    sources.insert(path.clone());
                    continue;
                }
            }
            let mut result = self.all(format!("Unclassified or build input changed: {path}"));
            result.changed = changed;
            return result;
        }
        loop {
            let before = selected.len();
            for (module, dependencies) in &self.modules {
                if dependencies
                    .iter()
                    .any(|dependency| selected.contains(dependency))
                {
                    selected.insert(module.clone());
                }
            }
            if selected.len() == before {
                break;
            }
        }
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

/// Returns changed paths and the workspace prefix below the repository root.
fn changed_paths(workspace: &Path, base: &str) -> Result<(BTreeSet<String>, String)> {
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
    Ok((changed, prefix))
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
        modules
            .iter()
            .map(|module| match module.as_str() {
                "." => format!(":{goal}"),
                module => format!(":{module}:{goal}"),
            })
            .collect()
    } else {
        vec![goal.to_owned()]
    }
}

/// `filter` is set after the class-level compile step: the build skips `clean` and reads
/// the unselected (Maven) or selected (Gradle) test classes from that file.
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
                "-Dimpact.skip.{}={}",
                if module == "." { "root" } else { module },
                compile_only || !selection.modules.contains(module)
            ));
        }
        // Excludes keep the POM's includes and suite assignment.
        if let Some(excludes) = filter.filter(|_| subset) {
            for plugin in ["surefire", "failsafe"] {
                args.push(format!("-D{plugin}.excludesFile={}", excludes.display()));
            }
        }
    } else {
        let modules: Vec<_> = if compile_only {
            Vec::new()
        } else {
            selection.modules.iter().map(String::as_str).collect()
        };
        args.push(format!("-Pimpact.modules={}", modules.join(",")));
        // The Gradle filter intersects with each Test task's own patterns.
        if let Some(tests) = filter.filter(|_| subset) {
            args.push(format!("-Pimpact.testsFile={}", tests.display()));
        }
    }
    args
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

fn compile_args(config: &Config, selection: &Selection, listing: &Path) -> Vec<String> {
    if config.tool == "maven" {
        let mut args: Vec<String> = vec!["-B".into(), "-ntp".into(), "clean".into()];
        args.extend(scope(config, &selection.modules, "test-compile"));
        args
    } else {
        let mut args: Vec<String> = vec![
            "--no-daemon".into(),
            "--console=plain".into(),
            "clean".into(),
        ];
        args.extend(scope(config, &selection.modules, "impactCompile"));
        args.push(":impactClasses".into());
        args.push(format!("-Pimpact.classes={}", listing.display()));
        args
    }
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
    if matches!(command.as_str(), "" | "--help" | "-h") {
        println!(
            "sieve <select|run> --workspace PATH [--base REV | --full]\n\
                  [--output FILE] [--executable PATH] [-- BUILD_ARGS...]\n\n\
                  sieve <init|refresh> [--workspace PATH] [--tool maven|gradle] [--executable PATH]\n\
                  sieve replay --workspace PATH [--commits N] [--run] [-- BUILD_ARGS...]\n\
                  sieve fixtures <list|prepare|apply|check-selection|reports|verify|benchmark>\n\n\
                  Requires impact.json and the build adapters documented in README.md.\n\
                  No base or unavailable Git history selects ALL. run propagates build failures."
        );
        return Ok(0);
    }
    if command != "select" && command != "run" {
        return Err(format!("Unknown command: {command}").into());
    }
    let mut workspace = None;
    let mut base = None;
    let mut full = false;
    let mut output = None;
    let mut executable = None;
    let mut extra = Vec::new();
    while let Some(arg) = args.next() {
        if arg == "--" {
            extra.extend(args);
            break;
        }
        if arg == "--full" {
            full = true;
            continue;
        }
        let value = args
            .next()
            .ok_or_else(|| format!("Missing value for {arg}"))?;
        match arg.as_str() {
            "--workspace" => workspace = Some(PathBuf::from(value)),
            "--base" => base = Some(value),
            "--output" => output = Some(PathBuf::from(value)),
            "--executable" => executable = Some(value),
            _ => return Err(format!("Unknown option: {arg}").into()),
        }
    }
    if full && base.is_some() {
        return Err("Use either --base or --full".into());
    }
    let workspace = workspace.ok_or("--workspace is required")?.canonicalize()?;
    let config = Config::read(&workspace)?;
    let mut selection = match base {
        Some(base) => match changed_paths(&workspace, &base) {
            Ok((changed, prefix)) => match fingerprint::build_inputs(&workspace) {
                Ok(current) if config.build_fingerprint.as_ref() == Some(&current) => {
                    config.select(changed, &prefix)
                }
                Ok(_) => {
                    let mut selection = config.all("Build inputs changed or no fingerprint exists; run sieve refresh and review impact.json");
                    selection.changed = changed;
                    selection
                }
                Err(error) => config.all(format!(
                    "Cannot verify build inputs; full fallback: {error}"
                )),
            },
            Err(error) => config.all(format!(
                "Cannot establish changed inputs; full fallback: {error}"
            )),
        },
        None => config.all("Full run requested or no comparison base supplied"),
    };
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
    let executable =
        executable.unwrap_or_else(|| setup::default_executable(&workspace, &config.tool));
    let init_script = if config.tool == "gradle" {
        Some(setup::gradle_script()?)
    } else {
        None
    };
    let script_args: Vec<String> = match &init_script {
        Some(script) => vec![
            "--init-script".into(),
            script.path().to_string_lossy().into_owned(),
        ],
        None => Vec::new(),
    };
    remove_stale_reports(&config, &selection, &workspace)?;
    let temp = tempfile::tempdir()?;
    let mut filter = None;
    if config.class_level && selection.mode == "MODULES" {
        eprintln!("{json}Compiling for class-level selection");
        let listing = temp.path().join("classes.json");
        let status = Command::new(&executable)
            .current_dir(&workspace)
            .args(compile_args(&config, &selection, &listing))
            .args(&script_args)
            .args(&extra)
            .status()?;
        if !status.success() {
            return Ok(1);
        }
        // Analysis problems keep the module selection instead of failing the build.
        let unselected = class_dirs(&config, &workspace, &listing)
            .and_then(|dirs| classes::load(&dirs))
            .and_then(|classes| selection.refine(&classes, &workspace, config.tool == "maven"))
            .unwrap_or_else(|error| {
                selection.reason = format!("Class-level selection unavailable: {error}");
                BTreeSet::new()
            });
        let text = if config.tool == "maven" {
            // Surefire drops its default nested-class exclude once an excludes file is given.
            let mut excludes = String::from("**/*$*\n");
            for name in unselected {
                excludes += &format!("{}.*\n", name.replace('.', "/"));
            }
            excludes
        } else {
            selection
                .tests
                .iter()
                .map(|test| format!("{test}\n"))
                .collect()
        };
        let path = temp.path().join("tests.txt");
        fs::write(&path, text)?;
        filter = Some(path);
        json = write(&selection)?;
    }
    eprintln!("{json}");
    let status = Command::new(executable)
        .current_dir(workspace)
        .args(build_args(&config, &selection, filter.as_deref()))
        .args(script_args)
        .args(extra)
        .status()?;
    Ok(if status.success() { 0 } else { 1 })
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
    fn ignore_patterns_are_workspace_or_repository_relative() {
        let mut config: Config = serde_json::from_value(serde_json::json!({
            "tool": "maven", "modules": {"core": []}
        }))
        .unwrap();
        let mode = |config: &Config, path: &str, prefix: &str| {
            config.select(BTreeSet::from([path.into()]), prefix).mode
        };
        // Defaults cover workspace and repository READMEs and docs, nothing else.
        assert_eq!(mode(&config, "README.md", "app"), "NONE");
        assert_eq!(mode(&config, "@repository/docs/x.md", "app"), "NONE");
        assert_eq!(mode(&config, "VALIDATION.md", ""), "ALL");
        assert_eq!(mode(&config, "core/README.md", ""), "ALL");
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
        config
            .modules
            .get_mut("pricing")
            .unwrap()
            .push("checkout".into());
        assert_eq!(
            config
                .select(
                    BTreeSet::from(["checkout/src/test/java/NewTest.java".into()]),
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
            compile_args(&config, &selection, listing),
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
                "-Dsurefire.excludesFile=/tmp/tests.txt",
                "-Dfailsafe.excludesFile=/tmp/tests.txt"
            ]
        );
        config.tool = "gradle".into();
        assert_eq!(
            compile_args(&config, &selection, listing),
            [
                "--no-daemon",
                "--console=plain",
                "clean",
                ":app:impactCompile",
                ":core:impactCompile",
                ":impactClasses",
                "-Pimpact.classes=/tmp/classes.json"
            ]
        );
        assert_eq!(
            build_args(&config, &selection, Some(filter)),
            [
                "--no-daemon",
                "--console=plain",
                ":app:check",
                ":core:check",
                "-Pimpact.modules=app,core",
                "-Pimpact.testsFile=/tmp/tests.txt"
            ]
        );
        // No test in a selected module reaches the class: compile only, still without clean.
        graph[1].refs.clear();
        let mut selection = config.select(changed(), "");
        selection.refine(&graph, temp.path(), true).unwrap();
        assert_eq!(selection.mode, "NONE");
        assert_eq!(
            selection.modules,
            BTreeSet::from(["app".into(), "core".into()])
        );
        assert_eq!(
            build_args(&config, &selection, Some(filter)),
            [
                "--no-daemon",
                "--console=plain",
                ":app:check",
                ":core:check",
                "-Pimpact.modules="
            ]
        );
        config.tool = "maven".into();
        assert!(build_args(&config, &selection, Some(filter)).ends_with(&[
            "-Dimpact.skip.app=true".into(),
            "-Dimpact.skip.core=true".into(),
            "-Dimpact.skip.other=true".into()
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
