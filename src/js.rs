//! JavaScript and TypeScript projects on Jest or Vitest (ADR 0006): Sieve classifies the changed
//! paths and asks the runner which test files relate to the changed script files.
use crate::{changed_paths, Config, Result, Selection, REPOSITORY};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

pub const TOOLS: [&str; 2] = ["jest", "vitest"];
const SCRIPTS: [&str; 8] = ["js", "jsx", "ts", "tsx", "mjs", "cjs", "mts", "cts"];
const HELPER: &[u8] = include_bytes!("vitest.helper.mjs");
/// Where `run` writes Jest-format JSON results when `SIEVE_JS_RESULTS` is set (replay):
/// ignored by Git and kept by `git clean -e node_modules`.
pub const RESULTS: &str = "node_modules/.cache/sieve/results.json";

pub fn is_js(tool: &str) -> bool {
    TOOLS.contains(&tool)
}

pub fn default_executable(workspace: &Path, tool: &str) -> String {
    let local = workspace.join("node_modules/.bin").join(tool);
    if local.is_file() {
        local.to_string_lossy().into_owned()
    } else {
        tool.into()
    }
}

fn script(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| SCRIPTS.contains(&e))
}

/// The test file a snapshot belongs to: `dir/__snapshots__/name.snap` → `dir/name`.
fn snapshot_test(path: &str) -> Option<String> {
    let name = path.strip_suffix(".snap")?;
    let (dir, file) = name.rsplit_once('/').unwrap_or(("", name));
    let parent = dir.strip_suffix("__snapshots__")?;
    Some(format!("{parent}{file}"))
}

struct Runner<'a> {
    tool: &'a str,
    executable: &'a str,
    workspace: &'a Path,
    extra: &'a [String],
}

/// Absolute paths printed by the runner, as workspace-relative paths.
fn relative(workspace: &Path, paths: &[String]) -> BTreeSet<String> {
    paths
        .iter()
        .map(|p| {
            Path::new(p)
                .strip_prefix(workspace)
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| p.clone())
        })
        .collect()
}

/// The last stdout line that parses as JSON: runners may print warnings first.
fn json_output(command: &mut Command) -> Result<Value> {
    let output = command.output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        return Err(format!(
            "runner query failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    serde_json::from_str(stdout.trim())
        .ok()
        .or_else(|| {
            stdout
                .lines()
                .rev()
                .filter(|line| line.starts_with(['[', '{']))
                .find_map(|line| serde_json::from_str(line.trim()).ok())
        })
        .ok_or_else(|| format!("runner query printed no JSON: {}", stdout.trim()).into())
}

fn strings(value: &Value) -> Vec<String> {
    serde_json::from_value(value.clone()).unwrap_or_default()
}

impl Runner<'_> {
    fn command(&self) -> Command {
        let mut command = Command::new(self.executable);
        command.current_dir(self.workspace);
        command
    }

    /// Every test file, for `init`.
    fn list(&self) -> Result<BTreeSet<String>> {
        let tests = if self.tool == "jest" {
            strings(&json_output(
                self.command()
                    .args(self.extra)
                    .args(["--listTests", "--json"]),
            )?)
        } else {
            strings(&self.helper(&[])?["tests"])
        };
        Ok(relative(self.workspace, &tests))
    }

    /// Related test files of `changed`, and the changed files that reach a runner-loaded file.
    fn query(&self, changed: &[PathBuf]) -> Result<(BTreeSet<String>, BTreeSet<String>)> {
        let (tests, reached) = if self.tool == "jest" {
            self.jest_query(changed)?
        } else {
            let output = self.helper(changed)?;
            (strings(&output["tests"]), strings(&output["reached"]))
        };
        Ok((
            relative(self.workspace, &tests),
            relative(self.workspace, &reached),
        ))
    }

    fn jest_query(&self, changed: &[PathBuf]) -> Result<(Vec<String>, Vec<String>)> {
        let related = |extra: &[String]| -> Result<Vec<String>> {
            Ok(strings(&json_output(
                self.command()
                    .args(self.extra)
                    .args(["--listTests", "--json"])
                    .args(extra)
                    .arg("--findRelatedTests")
                    .args(changed),
            )?))
        };
        let tests = related(&[])?;
        let config = json_output(self.command().args(self.extra).arg("--showConfig"))?;
        // Runner-loaded files: every project script the resolved configuration names (setup
        // files, environments, transforms, mappers, resolvers, reporters).
        let mut loaded = BTreeSet::new();
        collect_scripts(&config, self.workspace, &mut loaded);
        if loaded.is_empty() {
            return Ok((tests, Vec::new()));
        }
        // Ask which loaded files relate to the change, with them as the only test files and the
        // whole workspace searched. Jest rejects testMatch and testRegex together.
        let regex = config["configs"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|c| c["testRegex"].as_array().is_some_and(|r| !r.is_empty()));
        let mut extra = vec![format!("--roots={}", self.workspace.display())];
        if regex {
            let alternatives: Vec<_> = loaded.iter().map(|p| regex_escape(p)).collect();
            extra.push(format!("--testRegex=^({})$", alternatives.join("|")));
        } else {
            extra.extend(
                loaded
                    .iter()
                    .map(|p| format!("--testMatch={}", glob_escape(p))),
            );
        }
        Ok((tests, related(&extra)?))
    }

    fn helper(&self, changed: &[PathBuf]) -> Result<Value> {
        let mut script = tempfile::Builder::new()
            .prefix("sieve-vitest-")
            .suffix(".mjs")
            .tempfile()?;
        script.write_all(HELPER)?;
        // The helper loads the project's Vitest itself; `--executable` names the runner only.
        json_output(
            Command::new(env::var("SIEVE_NODE").unwrap_or_else(|_| "node".into()))
                .current_dir(self.workspace)
                .arg(script.path())
                .arg(self.workspace)
                .args(changed)
                .arg("--")
                .args(self.extra),
        )
    }

    fn run(&self, tests: Option<&BTreeSet<String>>) -> Result<bool> {
        let mut command = self.command();
        if self.tool == "vitest" {
            command.arg("run");
        }
        command.args(self.extra);
        if env::var_os("SIEVE_JS_RESULTS").is_some() {
            let results = self.workspace.join(RESULTS);
            fs::create_dir_all(results.parent().unwrap())?;
            if self.tool == "jest" {
                command.arg("--json");
                command.arg(format!("--outputFile={}", results.display()));
            } else {
                command.args(["--reporter=default", "--reporter=json"]);
                command.arg(format!("--outputFile.json={}", results.display()));
            }
        }
        if let Some(tests) = tests {
            if self.tool == "jest" {
                command.arg("--runTestsByPath");
            }
            command.args(tests);
        }
        Ok(command.status()?.success())
    }
}

fn collect_scripts(value: &Value, workspace: &Path, out: &mut BTreeSet<String>) {
    match value {
        Value::String(s) => {
            let path = Path::new(s);
            if path.is_absolute()
                && path.starts_with(workspace)
                && !s.contains("/node_modules/")
                && script(s)
                && path.is_file()
            {
                out.insert(s.clone());
            }
        }
        Value::Array(items) => items
            .iter()
            .for_each(|v| collect_scripts(v, workspace, out)),
        Value::Object(map) => map
            .values()
            .for_each(|v| collect_scripts(v, workspace, out)),
        _ => {}
    }
}

fn regex_escape(path: &str) -> String {
    path.chars()
        .flat_map(|c| {
            let special = "\\^$.|?*+()[]{}".contains(c);
            special.then_some('\\').into_iter().chain([c])
        })
        .collect()
}

fn glob_escape(path: &str) -> String {
    path.chars()
        .flat_map(|c| {
            let special = "\\*?[](){}!+@".contains(c);
            special.then_some('\\').into_iter().chain([c])
        })
        .collect()
}

fn in_sources(config: &Config, path: &str) -> bool {
    config.sources.iter().any(|root| {
        path.strip_prefix(root.as_str())
            .is_some_and(|r| r.starts_with('/'))
    })
}

fn subset(config: &Config, changed: BTreeSet<String>, tests: BTreeSet<String>) -> Selection {
    let mut selection = config.all("");
    selection.changed = changed;
    if tests.is_empty() {
        selection.mode = "NONE";
        selection.modules.clear();
        selection.reason = "No test file relates to the changed files".into();
    } else {
        selection.mode = "SUBSET";
        selection.tests = tests;
        selection.reason = "Test files related to the changed files".into();
    }
    selection
}

fn select(
    config: &Config,
    workspace: &Path,
    changed: BTreeSet<String>,
    prefix: &str,
    runner: &Runner,
) -> Selection {
    let all = |reason: String, changed: BTreeSet<String>| {
        let mut selection = config.all(reason);
        selection.changed = changed;
        selection
    };
    let mut tests = BTreeSet::new();
    let mut scripts = Vec::new();
    for path in &changed {
        let inside = !path.starts_with(REPOSITORY) && in_sources(config, path);
        // Default patterns never hide files in source roots, as for module sources.
        if config.ignored(path, prefix) && (config.ignore.is_some() || !inside) {
            continue;
        }
        let reason = if !inside {
            "Changed outside the source roots"
        } else if !workspace.join(path).exists() {
            "Deleted"
        } else if path.split('/').any(|part| part == "__mocks__") {
            "Runner-loaded mock changed"
        } else if let Some(test) = snapshot_test(path) {
            if workspace.join(&test).is_file() {
                tests.insert(test);
                continue;
            }
            "Snapshot without its test file"
        } else if script(path) {
            scripts.push(workspace.join(path));
            continue;
        } else {
            "Non-script file changed"
        };
        return all(format!("{reason}: {path}"), changed);
    }
    if !scripts.is_empty() {
        match runner.query(&scripts) {
            Ok((_, reached)) if !reached.is_empty() => {
                let first = reached.iter().next().unwrap();
                return all(format!("Runner-loaded file reached: {first}"), changed);
            }
            Ok((related, _)) => tests.extend(related),
            Err(error) => {
                return all(
                    format!("Cannot query {}; full fallback: {error}", config.tool),
                    changed,
                )
            }
        }
    }
    subset(config, changed, tests)
}

/// `select` and `run` for a JavaScript or TypeScript project.
pub fn main(
    command: &str,
    config: &Config,
    workspace: &Path,
    base: Option<String>,
    output: Option<PathBuf>,
    executable: Option<String>,
    extra: &[String],
) -> Result<u8> {
    let executable = executable.unwrap_or_else(|| default_executable(workspace, &config.tool));
    let runner = Runner {
        tool: &config.tool,
        executable: &executable,
        workspace,
        extra,
    };
    let mut selection = match base {
        Some(base) => match changed_paths(workspace, &base) {
            Ok((changed, prefix, _)) => select(config, workspace, changed, &prefix, &runner),
            Err(error) => config.all(format!(
                "Cannot establish changed inputs; full fallback: {error}"
            )),
        },
        None => config.all("Full run requested or no comparison base supplied"),
    };
    // Test hook: a selector that drops everything, which planted bugs must catch.
    if command == "run" && env::var_os("SIEVE_BROKEN_SELECTOR").is_some() {
        selection.mode = "NONE";
        selection.modules.clear();
        selection.tests.clear();
        selection.reason = "SIEVE_BROKEN_SELECTOR drops every test".into();
    }
    let json = serde_json::to_string_pretty(&selection)? + "\n";
    if let Some(path) = &output {
        fs::write(path, &json)?;
    }
    if command == "select" {
        print!("{json}");
        return Ok(0);
    }
    eprint!("{json}");
    let passed = match selection.mode {
        "NONE" => true,
        "SUBSET" => runner.run(Some(&selection.tests))?,
        _ => runner.run(None)?,
    };
    Ok(if passed { 0 } else { 1 })
}

/// Test cases of the last replayed run, from Jest-format JSON (Vitest's `json` reporter
/// writes the same shape). No file means no runner started: nothing executed.
pub(crate) fn read_results(workspace: &Path) -> Result<crate::reports::Reports> {
    let mut reports = crate::reports::Reports::default();
    let Ok(bytes) = fs::read(workspace.join(RESULTS)) else {
        return Ok(reports);
    };
    let value: Value = serde_json::from_slice(&bytes)?;
    let workspace = &workspace.canonicalize()?;
    for file in value["testResults"]
        .as_array()
        .ok_or("results without testResults")?
    {
        let name = relative(
            workspace,
            &[file["name"].as_str().unwrap_or_default().to_owned()],
        )
        .pop_first()
        .unwrap_or_default();
        let cases = file["assertionResults"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        // A test file that fails to load reports no cases.
        if cases.is_empty() && file["status"] == "failed" {
            reports.executed.insert(name.clone());
            reports.failed.insert(name.clone());
        }
        for case in cases {
            let id = format!("{name}::{}", case["fullName"].as_str().unwrap_or_default());
            match case["status"].as_str() {
                Some("passed") => {}
                Some("failed") => {
                    reports.failed.insert(id.clone());
                }
                _ => {
                    reports.skipped.insert(id);
                    continue;
                }
            }
            reports.cases += 1;
            reports.executed.insert(id);
        }
    }
    Ok(reports)
}

/// Installs the dependencies a lockfile pins, for replayed commits.
pub fn install(workspace: &Path) -> Result<()> {
    let (program, args): (&str, &[&str]) = if workspace.join("pnpm-lock.yaml").is_file() {
        ("pnpm", &["install", "--frozen-lockfile"])
    } else if workspace.join("yarn.lock").is_file() {
        ("yarn", &["install", "--frozen-lockfile"])
    } else if ["bun.lock", "bun.lockb"]
        .iter()
        .any(|f| workspace.join(f).is_file())
    {
        ("bun", &["install", "--frozen-lockfile"])
    } else if workspace.join("package-lock.json").is_file() {
        ("npm", &["ci", "--no-audit", "--no-fund"])
    } else {
        ("npm", &["install", "--no-audit", "--no-fund"])
    };
    let status = Command::new(program)
        .args(args)
        .current_dir(workspace)
        .stdout(std::io::stderr())
        .status()?;
    if !status.success() {
        return Err(format!("{program} {} failed", args.join(" ")).into());
    }
    Ok(())
}

/// The lockfile contents that decide whether replay must reinstall.
pub fn lockfiles(workspace: &Path) -> Vec<u8> {
    [
        "package.json",
        "pnpm-lock.yaml",
        "yarn.lock",
        "bun.lock",
        "bun.lockb",
        "package-lock.json",
    ]
    .iter()
    .flat_map(|f| fs::read(workspace.join(f)).unwrap_or_default())
    .collect()
}

/// `init`/`refresh`: the source roots are `src` and every top-level directory holding a test
/// file. Refresh keeps the roots declared before.
pub fn configure(
    workspace: &Path,
    tool: &str,
    executable: Option<String>,
    extra: &[String],
    previous: Option<Config>,
) -> Result<Config> {
    let executable = executable.unwrap_or_else(|| default_executable(workspace, tool));
    let runner = Runner {
        tool,
        executable: &executable,
        workspace,
        extra,
    };
    let mut sources: BTreeSet<String> = runner
        .list()?
        .iter()
        .filter_map(|test| test.split_once('/').map(|(dir, _)| dir.to_owned()))
        .filter(|dir| dir != "node_modules" && !dir.starts_with(REPOSITORY))
        .collect();
    if workspace.join("src").is_dir() {
        sources.insert("src".into());
    }
    let mut config: Config = serde_json::from_value(serde_json::json!({
        "tool": tool, "modules": {".": []}, "sources": sources,
    }))?;
    if let Some(previous) = previous {
        config.sources.extend(previous.sources);
        config.sources.sort();
        config.sources.dedup();
        config.ignore = previous.ignore;
    }
    Ok(config)
}

/// The JavaScript runner a `package.json` declares, if exactly one.
pub fn detect(workspace: &Path) -> Option<Result<String>> {
    let package: Value =
        serde_json::from_slice(&fs::read(workspace.join("package.json")).ok()?).ok()?;
    let found: Vec<_> = TOOLS
        .into_iter()
        .filter(|tool| {
            ["dependencies", "devDependencies"]
                .iter()
                .any(|deps| package[deps].get(tool).is_some())
        })
        .collect();
    Some(match found.as_slice() {
        [tool] => Ok((*tool).to_owned()),
        [] => Err("package.json declares neither jest nor vitest".into()),
        _ => Err("package.json declares both jest and vitest; specify --tool".into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_map_to_their_test_file() {
        assert_eq!(
            snapshot_test("src/__snapshots__/a.test.ts.snap").as_deref(),
            Some("src/a.test.ts")
        );
        assert_eq!(
            snapshot_test("__snapshots__/a.test.ts.snap").as_deref(),
            Some("a.test.ts")
        );
        assert_eq!(snapshot_test("src/a.snap"), None);
        assert_eq!(snapshot_test("src/a.test.ts"), None);
    }

    #[test]
    fn script_extensions_and_escaping() {
        assert!(script("src/a.tsx") && script("a.mjs") && !script("a.json") && !script("a.vue"));
        assert_eq!(regex_escape("/a.b/(c)"), "/a\\.b/\\(c\\)");
        assert_eq!(glob_escape("/a/[b]*"), "/a/\\[b\\]\\*");
    }
}
