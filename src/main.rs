use serde::{Deserialize, Serialize};
mod fingerprint;
mod fixtures;
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
    /// Class-level dependency map for single-module projects (`"modules": {".":[...]}`).
    /// Maps workspace-relative source paths to test identifiers in `module:suite:FQCN`
    /// format. When present and all changed source paths are covered, `select` emits
    /// `SUBSET` instead of `MODULES`, enabling class-level test filtering.
    /// A path that maps to an empty list contributes no tests (unused-code case).
    /// Any changed source path absent from the map falls back to `MODULES`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    class_tests: Option<BTreeMap<String, Vec<String>>>,
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
    /// Non-empty only for `SUBSET` mode: the exact test identifiers (`module:suite:FQCN`)
    /// that the class-level map resolved from the changed source paths.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    tests: BTreeSet<String>,
    changed: BTreeSet<String>,
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
        if let Some(class_tests) = &self.class_tests {
            for (path, tests) in class_tests {
                if path.is_empty() || path.starts_with('/') || path.contains("..") {
                    return Err(format!("Invalid class_tests key: {path:?}").into());
                }
                for test_id in tests {
                    let mut parts = test_id.splitn(3, ':');
                    let module = parts.next().unwrap_or("");
                    let suite = parts.next().unwrap_or("");
                    let fqcn = parts.next().unwrap_or("");
                    if module.is_empty() || suite.is_empty() || fqcn.is_empty() {
                        return Err(format!(
                            "class_tests entry must be module:suite:class, got {test_id:?}"
                        )
                        .into());
                    }
                    if !self.modules.contains_key(module) {
                        return Err(format!(
                            "class_tests references unknown module {module:?} in {test_id:?}"
                        )
                        .into());
                    }
                }
            }
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
            reason: reason.into(),
        }
    }

    fn select(&self, changed: BTreeSet<String>, prefix: &str) -> Selection {
        // For single-module projects that declare a class_tests map, attempt class-level
        // selection. Every changed src/ path must be present in the map; any absent path
        // falls back to module-level selection so the result is always conservative.
        let can_subset = self.class_tests.is_some() && self.modules.contains_key(".");
        let mut selected = BTreeSet::new();
        let mut selected_tests: BTreeSet<String> = BTreeSet::new();
        let mut subset_possible = can_subset;
        for path in &changed {
            if self.modules.contains_key(".") && path.starts_with("src/") {
                selected.insert(".".into());
                if can_subset {
                    if let Some(map) = &self.class_tests {
                        match map.get(path) {
                            Some(tests) => selected_tests.extend(tests.iter().cloned()),
                            None => subset_possible = false,
                        }
                    }
                }
                continue;
            }
            // Build/configuration changes may alter the module graph or test discovery.
            if let Some((module, rest)) = path.split_once('/') {
                if self.modules.contains_key(module) && rest.starts_with("src/") {
                    selected.insert(module.to_owned());
                    subset_possible = false;
                    continue;
                }
            }
            if self.ignored(path, prefix) {
                continue;
            }
            let mut result = self.all(format!("Unclassified or build input changed: {path}"));
            result.changed = changed;
            return result;
        }
        // ponytail: module granularity includes extra tests; use semantic class dependencies
        // only when this conservative policy no longer saves enough CI time.
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
        // Promote to SUBSET only when the class_tests map covered every changed src path
        // and we are in single-module mode (no dependency propagation can widen the set).
        // An empty union still keeps the module so `run` compiles it without executing tests.
        if subset_possible && !selected.is_empty() {
            return Selection {
                mode: if selected_tests.is_empty() {
                    "NONE"
                } else {
                    "SUBSET"
                },
                reason: if selected_tests.is_empty() {
                    "Class-level selection: changed sources map to no tests; compile only"
                } else {
                    "Class-level selection from source dependency map"
                }
                .into(),
                modules: selected,
                tests: selected_tests,
                changed,
            };
        }
        Selection {
            mode: if selected.is_empty() {
                "NONE"
            } else {
                "MODULES"
            },
            modules: selected,
            tests: BTreeSet::new(),
            changed,
            reason: "Changed source/resource modules and their transitive dependents".into(),
        }
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

/// Matches no test class, so a Maven suite without selected tests executes nothing.
const NO_TESTS: &str = "sieve.NoSelectedTests";

fn build_args(config: &Config, selection: &Selection) -> Vec<String> {
    let mut args: Vec<String> = match config.tool.as_str() {
        "maven" => vec!["-B", "-ntp", "clean"],
        _ => vec!["--no-daemon", "--console=plain", "clean"],
    }
    .into_iter()
    .map(str::to_owned)
    .collect();
    // NONE with modules comes from class-level selection: compile them but run no tests.
    let compile_only = selection.mode == "NONE" && !selection.modules.is_empty();
    if selection.mode == "NONE" && !compile_only {
        return args;
    }
    args.push(
        if config.tool == "maven" {
            "verify"
        } else {
            "check"
        }
        .into(),
    );
    if selection.mode == "MODULES" || compile_only {
        if config.tool == "maven" {
            for module in config.modules.keys() {
                args.push(format!(
                    "-Dimpact.skip.{}={}",
                    if module == "." { "root" } else { module },
                    compile_only || !selection.modules.contains(module)
                ));
            }
        } else {
            let modules = if compile_only {
                Vec::new()
            } else {
                selection.modules.iter().cloned().collect()
            };
            args.push(format!("-Pimpact.modules={}", modules.join(",")));
        }
    } else if selection.mode == "SUBSET" {
        // Maven: Surefire runs non-integration IDs and Failsafe runs integration IDs; a
        // suite without selected IDs receives a pattern that matches nothing.
        // Gradle: the init script filters every Test task to the selected classes.
        let parsed = selection.tests.iter().filter_map(|t| {
            let mut parts = t.splitn(3, ':');
            let _module = parts.next()?;
            Some((parts.next()?, parts.next()?))
        });
        if config.tool == "maven" {
            let (failsafe, surefire): (Vec<_>, Vec<_>) =
                parsed.partition(|(suite, _)| *suite == "integration");
            let classes = |tests: Vec<(&str, &str)>| {
                if tests.is_empty() {
                    NO_TESTS.to_owned()
                } else {
                    tests
                        .iter()
                        .map(|(_, fqcn)| *fqcn)
                        .collect::<Vec<_>>()
                        .join(",")
                }
            };
            args.push("-Dsurefire.failIfNoSpecifiedTests=false".into());
            args.push("-Dit.failIfNoSpecifiedTests=false".into());
            args.push(format!("-Dtest={}", classes(surefire)));
            args.push(format!("-Dit.test={}", classes(failsafe)));
        } else {
            args.push(format!(
                "-Pimpact.tests={}",
                parsed.map(|(_, fqcn)| fqcn).collect::<Vec<_>>().join(",")
            ));
        }
    }
    args
}

fn main_result() -> Result<u8> {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_default();
    if command == "fixtures" {
        return fixtures::main(args.collect());
    }
    if command == "init" || command == "refresh" {
        return setup::init(args.collect(), command == "refresh");
    }
    if matches!(command.as_str(), "" | "--help" | "-h") {
        println!(
            "sieve <select|run> --workspace PATH [--base REV | --full]\n\
                  [--output FILE] [--executable PATH] [-- BUILD_ARGS...]\n\n\
                  sieve <init|refresh> [--workspace PATH] [--tool maven|gradle] [--executable PATH]\n\
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
    let selection = match base {
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
    let json = serde_json::to_string_pretty(&selection)? + "\n";
    if let Some(path) = output {
        fs::write(path, &json)?;
    }
    if command == "select" {
        print!("{json}");
        return Ok(0);
    }
    eprintln!("{json}");
    let executable =
        executable.unwrap_or_else(|| setup::default_executable(&workspace, &config.tool));
    let init_script = if config.tool == "gradle" {
        Some(setup::gradle_script()?)
    } else {
        None
    };
    let mut build_args = build_args(&config, &selection);
    if let Some(script) = &init_script {
        build_args.extend([
            "--init-script".into(),
            script.path().to_string_lossy().into_owned(),
        ]);
    }
    let status = Command::new(executable)
        .current_dir(workspace)
        .args(build_args)
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
        let args = build_args(&config, &tax);
        assert!(args.contains(&"-Dimpact.skip.runtime=true".into()));
        assert!(args.contains(&"-Dimpact.skip.checkout=false".into()));
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
        assert_eq!(build_args(&config, &docs), ["-B", "-ntp", "clean"]);
        assert_eq!(config.select(BTreeSet::new(), "").mode, "NONE");
        config.tool = "gradle".into();
        assert!(build_args(&config, &tax).contains(&"-Pimpact.modules=checkout,pricing".into()));
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
    fn class_level_selection_subset_none_and_fallback() {
        // CT-01: covered src file → SUBSET with mapped tests.
        // CT-02: covered src file with empty test list → NONE (no tests to run).
        // CT-03: uncovered src file → MODULES fallback (conservative).
        // CT-04: class_tests ignored for multi-module projects.
        let config: Config = serde_json::from_value(serde_json::json!({
            "tool": "maven",
            "modules": { ".": [] },
            "class_tests": {
                "src/main/java/example/Calculator.java": [".:unit:example.CalculatorTest"],
                "src/main/java/example/StringUtils.java": [".:unit:example.StringUtilsTest"],
                "src/main/java/example/Unused.java": [],
                "src/test/java/example/CalculatorTest.java": [".:unit:example.CalculatorTest"]
            }
        }))
        .unwrap();
        let select = |path: &str| config.select(BTreeSet::from([path.into()]), "");

        // CT-01: single covered source file selects its test.
        let s = select("src/main/java/example/Calculator.java");
        assert_eq!(s.mode, "SUBSET");
        assert_eq!(
            s.tests,
            BTreeSet::from([".:unit:example.CalculatorTest".into()])
        );
        assert_eq!(s.modules, BTreeSet::from([".".into()]));

        // CT-01b: changed test file selects itself.
        let s = select("src/test/java/example/CalculatorTest.java");
        assert_eq!(s.mode, "SUBSET");
        assert_eq!(
            s.tests,
            BTreeSet::from([".:unit:example.CalculatorTest".into()])
        );

        // CT-02: file with no test coverage → NONE.
        // The module is kept, so run compiles it without executing tests.
        let s = select("src/main/java/example/Unused.java");
        assert_eq!(s.mode, "NONE");
        assert!(s.tests.is_empty());
        assert_eq!(s.modules, BTreeSet::from([".".into()]));
        assert_eq!(
            build_args(&config, &s),
            ["-B", "-ntp", "clean", "verify", "-Dimpact.skip.root=true"]
        );
        let mut gradle: Config =
            serde_json::from_value(serde_json::to_value(&config).unwrap()).unwrap();
        gradle.tool = "gradle".into();
        assert_eq!(
            build_args(&gradle, &s),
            [
                "--no-daemon",
                "--console=plain",
                "clean",
                "check",
                "-Pimpact.modules="
            ]
        );

        // CT-03: file absent from map → MODULES fallback.
        let s = select("src/main/java/example/Unknown.java");
        assert_eq!(s.mode, "MODULES");
        assert!(s.tests.is_empty());

        // Two covered files → union of their tests.
        let s = config.select(
            BTreeSet::from([
                "src/main/java/example/Calculator.java".into(),
                "src/main/java/example/StringUtils.java".into(),
            ]),
            "",
        );
        assert_eq!(s.mode, "SUBSET");
        assert_eq!(
            s.tests,
            BTreeSet::from([
                ".:unit:example.CalculatorTest".into(),
                ".:unit:example.StringUtilsTest".into(),
            ])
        );

        // One covered, one not → falls back to MODULES.
        let s = config.select(
            BTreeSet::from([
                "src/main/java/example/Calculator.java".into(),
                "src/main/java/example/Unknown.java".into(),
            ]),
            "",
        );
        assert_eq!(s.mode, "MODULES");

        // CT-04: class_tests must be ignored for multi-module projects.
        let multi: Config = serde_json::from_value(serde_json::json!({
            "tool": "maven",
            "modules": { "app": [], "lib": [] },
            "class_tests": {
                "app/src/main/java/Foo.java": ["app:unit:FooTest"]
            }
        }))
        .unwrap();
        let s = multi.select(BTreeSet::from(["app/src/main/java/Foo.java".into()]), "");
        assert_eq!(s.mode, "MODULES");
    }

    #[test]
    fn subset_build_args_maven_and_gradle() {
        // BA-01: Maven SUBSET produces -Dtest= and -Dit.test= with the right class lists.
        // BA-02: Gradle SUBSET produces -Pimpact.tests=.
        let mut config: Config = serde_json::from_value(serde_json::json!({
            "tool": "maven",
            "modules": { ".": [] },
            "class_tests": {}
        }))
        .unwrap();
        let subset = |tests: &[&str]| Selection {
            mode: "SUBSET",
            modules: BTreeSet::from([".".into()]),
            tests: tests.iter().map(|s| s.to_string()).collect(),
            changed: BTreeSet::new(),
            reason: String::new(),
        };

        // BA-01a: unit-only SUBSET.
        let s = subset(&[".:unit:example.FooTest", ".:unit:example.BarTest"]);
        let args = build_args(&config, &s);
        assert!(args.contains(&"-Dsurefire.failIfNoSpecifiedTests=false".into()));
        assert!(args.iter().any(|a| a.starts_with("-Dtest=")));
        let test_arg = args.iter().find(|a| a.starts_with("-Dtest=")).unwrap();
        assert!(
            test_arg.contains("example.FooTest") && test_arg.contains("example.BarTest"),
            "{test_arg}"
        );
        assert!(args.contains(&"-Dit.test=sieve.NoSelectedTests".into()));
        assert!(args.contains(&"-Dit.failIfNoSpecifiedTests=false".into()));

        // BA-01b: integration-only SUBSET.
        let s = subset(&[".:integration:example.FooIT"]);
        let args = build_args(&config, &s);
        assert!(args.contains(&"-Dsurefire.failIfNoSpecifiedTests=false".into()));
        assert!(args.contains(&"-Dtest=sieve.NoSelectedTests".into()));
        let it_arg = args.iter().find(|a| a.starts_with("-Dit.test=")).unwrap();
        assert!(it_arg.contains("example.FooIT"), "{it_arg}");

        // BA-01c: mixed SUBSET.
        let s = subset(&[".:unit:example.FooTest", ".:integration:example.FooIT"]);
        let args = build_args(&config, &s);
        assert!(args.iter().any(|a| a.starts_with("-Dtest=")));
        assert!(args.iter().any(|a| a.starts_with("-Dit.test=")));

        // BA-02: Gradle SUBSET.
        config.tool = "gradle".into();
        let s = subset(&[".:unit:example.FooTest", ".:integration:example.FooIT"]);
        let args = build_args(&config, &s);
        assert!(
            args.iter().any(|a| a.starts_with("-Pimpact.tests=")),
            "{args:?}"
        );
        let p_arg = args
            .iter()
            .find(|a| a.starts_with("-Pimpact.tests="))
            .unwrap();
        assert!(
            p_arg.contains("example.FooTest") && p_arg.contains("example.FooIT"),
            "{p_arg}"
        );
        // Gradle SUBSET must not add -Pimpact.modules= (that is for MODULES mode).
        assert!(!args.iter().any(|a| a.starts_with("-Pimpact.modules=")));
    }

    #[test]
    fn class_tests_validation_rejects_bad_entries() {
        // VAL-CT: validate() catches bad keys and bad test-id formats.
        let temp = tempfile::tempdir().unwrap();
        let check = |json: serde_json::Value| -> bool {
            let c: Config = serde_json::from_value(json).unwrap();
            c.validate(temp.path()).is_err()
        };
        // Valid single-module config with class_tests passes.
        let ok: Config = serde_json::from_value(serde_json::json!({
            "tool": "maven",
            "modules": { ".": [] },
            "class_tests": {
                "src/main/java/A.java": [".:unit:example.ATest"]
            }
        }))
        .unwrap();
        assert!(ok.validate(temp.path()).is_ok());
        // Bad key: contains "..".
        assert!(check(serde_json::json!({
            "tool": "maven", "modules": { ".": [] },
            "class_tests": { "../escape.java": [] }
        })));
        // Bad key: starts with '/'.
        assert!(check(serde_json::json!({
            "tool": "maven", "modules": { ".": [] },
            "class_tests": { "/absolute.java": [] }
        })));
        // Bad test id: missing suite segment.
        assert!(check(serde_json::json!({
            "tool": "maven", "modules": { ".": [] },
            "class_tests": { "src/A.java": ["."] }
        })));
        // Bad test id: unknown module.
        assert!(check(serde_json::json!({
            "tool": "maven", "modules": { ".": [] },
            "class_tests": { "src/A.java": ["other:unit:Foo"] }
        })));
    }
}
