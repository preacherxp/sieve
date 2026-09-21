//! Independent fixture oracle: expectations come from scenarios.json, never the selector.
use crate::Result;
use quick_xml::{events::Event, Reader};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    io::Write,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};

const MARKER: &str = ".fixture-workspace.json";

#[derive(Deserialize)]
struct Catalog {
    baseline_tests: BTreeSet<String>,
    scenarios: Vec<Scenario>,
}

#[derive(Deserialize)]
struct Scenario {
    id: String,
    description: String,
    changes: Vec<Change>,
    required: BTreeSet<String>,
    expected_failures: BTreeSet<String>,
}

#[derive(Deserialize)]
struct Change {
    path: String,
    tool: Option<String>,
    create: Option<String>,
    #[serde(default)]
    delete: bool,
    before: Option<String>,
    after: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct Marker {
    tool: String,
    scenario: String,
}

impl Catalog {
    fn read(root: &Path) -> Result<Self> {
        Ok(serde_json::from_slice(&fs::read(
            root.join("scenarios.json"),
        )?)?)
    }

    fn scenario(&self, name: &str) -> Result<&Scenario> {
        self.scenarios
            .iter()
            .find(|s| s.id == name)
            .ok_or_else(|| format!("Unknown scenario: {name}").into())
    }

    fn inventory(&self, name: &str) -> Result<BTreeSet<String>> {
        let mut tests = self.baseline_tests.clone();
        if name != "baseline" {
            tests.extend(self.scenario(name)?.required.iter().cloned());
        }
        Ok(tests)
    }
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    fs::write(path, serde_json::to_string_pretty(value)? + "\n")?;
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        if ["target", "build", ".gradle", ".git"]
            .iter()
            .any(|s| entry.file_name() == *s)
        {
            continue;
        }
        let target = destination.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), target)?;
        } else {
            return Err(format!("Unsupported fixture file: {}", entry.path().display()).into());
        }
    }
    Ok(())
}

fn tool_name(tool: &str) -> Result<&str> {
    if matches!(tool, "maven" | "gradle") {
        Ok(tool)
    } else {
        Err(format!("Unknown build tool: {tool}").into())
    }
}

fn prepare(root: &Path, tool: &str, destination: &Path, git: bool) -> Result<PathBuf> {
    tool_name(tool)?;
    if destination.try_exists()? || fs::symlink_metadata(destination).is_ok() {
        return Err(format!("Refusing to overwrite {}", destination.display()).into());
    }
    if let Some(parent) = destination.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    copy_tree(&root.join("projects").join(tool), destination)?;
    let destination = destination.canonicalize()?;
    write_json(
        &destination.join(MARKER),
        &Marker {
            tool: tool.into(),
            scenario: "baseline".into(),
        },
    )?;
    writeln!(
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(destination.join(".gitignore"))?,
        "\n{MARKER}"
    )?;
    if git {
        for args in [
            vec!["init", "-q"],
            vec!["add", "."],
            vec![
                "-c",
                "user.name=Fixture Runner",
                "-c",
                "user.email=fixtures@example.invalid",
                "commit",
                "--no-gpg-sign",
                "-qm",
                "Fixture baseline",
            ],
        ] {
            crate::git(&destination, &args)?;
        }
    }
    Ok(destination)
}

fn mutation_path(workspace: &Path, relative: &str) -> Result<PathBuf> {
    let mut path = workspace.to_owned();
    if relative.is_empty() {
        return Err("Empty mutation path".into());
    }
    for component in Path::new(relative).components() {
        if !matches!(component, Component::Normal(_)) {
            return Err("Mutation path must stay inside the workspace".into());
        }
        path.push(component);
        if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("Mutation paths cannot contain symlinks".into());
        }
    }
    Ok(path)
}

fn apply(workspace: &Path, scenario: &Scenario) -> Result<()> {
    let marker_path = workspace.join(MARKER);
    let mut marker: Marker = serde_json::from_slice(&fs::read(&marker_path)?)?;
    if marker.scenario != "baseline" {
        return Err("Scenarios must start from a fresh baseline workspace".into());
    }
    let mut edits = Vec::new();
    for change in &scenario.changes {
        if change
            .tool
            .as_ref()
            .is_some_and(|tool| tool != &marker.tool)
        {
            continue;
        }
        let path = mutation_path(workspace, &change.path)?;
        let text = if let Some(text) = &change.create {
            if path.exists() {
                return Err(format!("Creation target exists: {}", path.display()).into());
            }
            Some(text.clone())
        } else if change.delete {
            if !path.is_file() {
                return Err(format!("Deletion target absent: {}", path.display()).into());
            }
            None
        } else {
            let before = change.before.as_ref().ok_or("Mutation missing before")?;
            let after = change.after.as_ref().ok_or("Mutation missing after")?;
            let text = fs::read_to_string(&path)?;
            if before.is_empty() || text.matches(before).count() != 1 {
                return Err(format!("Mutation needs one exact match: {}", path.display()).into());
            }
            Some(text.replace(before, after))
        };
        edits.push((path, text));
    }
    for (path, text) in edits {
        if let Some(text) = text {
            fs::create_dir_all(path.parent().ok_or("Missing mutation parent")?)?;
            fs::write(path, text)?;
        } else {
            fs::remove_file(path)?;
        }
    }
    marker.scenario = scenario.id.clone();
    write_json(&marker_path, &marker)
}

fn string_set(payload: &Value, key: &str) -> Result<BTreeSet<String>> {
    let mut result = BTreeSet::new();
    if let Some(value) = payload.get(key) {
        for value in value
            .as_array()
            .ok_or_else(|| format!("{key} must be an array"))?
        {
            let value = value
                .as_str()
                .ok_or("Test IDs and modules must be strings")?;
            if !result.insert(value.to_owned()) {
                return Err(format!("Duplicate {key} entry: {value}").into());
            }
        }
    }
    Ok(result)
}

fn expand_selection(available: &BTreeSet<String>, payload: &Value) -> Result<BTreeSet<String>> {
    let tests = string_set(payload, "tests")?;
    match payload.get("mode").and_then(Value::as_str) {
        Some("ALL") if tests.is_empty() => Ok(available.clone()),
        Some("NONE") if tests.is_empty() => Ok(BTreeSet::new()),
        Some("SUBSET") if !tests.is_empty() => Ok(tests),
        Some("MODULES") if tests.is_empty() => {
            let modules = string_set(payload, "modules")?;
            let known: BTreeSet<_> = available
                .iter()
                .filter_map(|t| t.split(':').next())
                .collect();
            if modules.is_empty() || modules.iter().any(|m| !known.contains(m.as_str())) {
                return Err("MODULES requires nonempty, unique, known module names".into());
            }
            Ok(available
                .iter()
                .filter(|t| modules.contains(t.split(':').next().unwrap_or_default()))
                .cloned()
                .collect())
        }
        _ => Err("Expected ALL/NONE with no tests, nonempty SUBSET, or MODULES".into()),
    }
}

#[derive(Serialize)]
struct SelectionCheck {
    ok: bool,
    missing: BTreeSet<String>,
    unknown: BTreeSet<String>,
    extra: BTreeSet<String>,
    selected: BTreeSet<String>,
    required: BTreeSet<String>,
    exact: bool,
}

fn check_selection(
    catalog: &Catalog,
    name: &str,
    payload: &Value,
    exact: bool,
) -> Result<SelectionCheck> {
    let available = catalog.inventory(name)?;
    let required = catalog.scenario(name)?.required.clone();
    let selected = expand_selection(&available, payload)?;
    let missing: BTreeSet<_> = required.difference(&selected).cloned().collect();
    let unknown: BTreeSet<_> = selected.difference(&available).cloned().collect();
    let extra: BTreeSet<_> = selected.difference(&required).cloned().collect();
    Ok(SelectionCheck {
        ok: missing.is_empty() && unknown.is_empty() && (!exact || extra.is_empty()),
        missing,
        unknown,
        extra,
        selected,
        required,
        exact,
    })
}

#[derive(Default, Serialize)]
struct Reports {
    executed: BTreeSet<String>,
    failed: BTreeSet<String>,
    skipped: BTreeSet<String>,
    cases: usize,
}

fn parse_report(path: &Path, prefix: &str, reports: &mut Reports) -> Result<()> {
    let mut reader = Reader::from_file(path)?;
    reader.config_mut().expand_empty_elements = true;
    let mut buffer = Vec::new();
    let mut case = None;
    let mut depth = 0usize;
    let (mut failed, mut skipped) = (false, false);
    loop {
        let event = reader.read_event_into(&mut buffer)?;
        match &event {
            Event::Start(_) => depth += 1,
            Event::End(_) => depth = depth.checked_sub(1).ok_or("Unexpected XML closing tag")?,
            Event::Eof if depth != 0 => return Err("Truncated XML report".into()),
            _ => {}
        }
        match event {
            Event::Start(tag) if tag.name().as_ref() == b"testcase" => {
                if case.is_some() {
                    return Err("Nested testcase in XML report".into());
                }
                let class = tag
                    .try_get_attribute(b"classname")?
                    .ok_or("Testcase missing classname")?
                    .decode_and_unescape_value(reader.decoder())?
                    .into_owned();
                case = Some(format!("{prefix}:{class}"));
                (failed, skipped) = (false, false);
            }
            Event::Start(tag) if case.is_some() => match tag.name().as_ref() {
                b"failure" | b"error" => failed = true,
                b"skipped" => skipped = true,
                _ => {}
            },
            Event::End(tag) if tag.name().as_ref() == b"testcase" => {
                let id = case.take().ok_or("Unexpected testcase end")?;
                if skipped {
                    reports.skipped.insert(id);
                } else {
                    reports.cases += 1;
                    reports.executed.insert(id.clone());
                    if failed {
                        reports.failed.insert(id);
                    }
                }
            }
            Event::Eof => {
                if case.is_some() {
                    return Err("Truncated XML testcase".into());
                }
                break;
            }
            _ => {}
        }
        buffer.clear();
    }
    Ok(())
}

fn read_reports(workspace: &Path, tool: &str) -> Result<Reports> {
    tool_name(tool)?;
    let mut reports = Reports::default();
    for module in ["pricing", "checkout", "runtime"] {
        let (base, unit, integration) = if tool == "maven" {
            ("target", "surefire-reports", "failsafe-reports")
        } else {
            ("build/test-results", "test", "integrationTest")
        };
        for (suite, folder) in [("unit", unit), ("integration", integration)] {
            let folder = workspace.join(module).join(base).join(folder);
            if !folder.exists() {
                continue;
            }
            for entry in fs::read_dir(folder)? {
                let entry = entry?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with("TEST-") && name.ends_with(".xml") {
                    parse_report(&entry.path(), &format!("{module}:{suite}"), &mut reports)?;
                }
            }
        }
    }
    Ok(reports)
}

fn verify(
    root: &Path,
    catalog: &Catalog,
    tool: &str,
    scenario: &str,
    executable: &str,
    output: &Path,
    selected_run: bool,
) -> Result<bool> {
    fs::create_dir_all(output)?;
    let output = output.canonicalize()?;
    let temp = tempfile::Builder::new().prefix("impact-").tempdir()?;
    let workspace = prepare(root, tool, &temp.path().join("project"), selected_run)?;
    if scenario != "baseline" {
        apply(&workspace, catalog.scenario(scenario)?)?;
    }
    let selection_path = output.join(format!("{tool}-{scenario}-selection.json"));
    if selection_path.exists() {
        fs::remove_file(&selection_path)?;
    }
    let ignore = if tool == "maven" {
        "-Dmaven.test.failure.ignore=true"
    } else {
        "-PfixtureIgnoreFailures=true"
    };
    let mut command = if selected_run {
        let mut command = Command::new(env::current_exe()?);
        command
            .arg("run")
            .arg("--workspace")
            .arg(&workspace)
            .arg("--executable")
            .arg(executable)
            .arg("--output")
            .arg(&selection_path);
        if scenario == "baseline" {
            command.arg("--full");
        } else {
            command.args(["--base", "HEAD"]);
        }
        command.args(["--", ignore]);
        command
    } else {
        let mut command = Command::new(executable);
        command.args(if tool == "maven" {
            vec!["-B", "-ntp", "clean", "verify", ignore]
        } else {
            vec!["--no-daemon", "--console=plain", "clean", "check", ignore]
        });
        command
    };
    let log_path = output.join(format!("{tool}-{scenario}.log"));
    let log = fs::File::create(&log_path)?;
    let status = command
        .current_dir(&workspace)
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .status()?;
    let reports = read_reports(&workspace, tool)?;
    let expected = if scenario == "baseline" {
        BTreeSet::new()
    } else {
        catalog.scenario(scenario)?.expected_failures.clone()
    };
    let mut selected = catalog.inventory(scenario)?;
    let mut selection_ok = !selected_run;
    if selected_run && selection_path.exists() {
        let payload: Value = serde_json::from_slice(&fs::read(selection_path)?)?;
        if scenario == "baseline" {
            selection_ok = payload["mode"] == "ALL";
        } else {
            let checked = check_selection(catalog, scenario, &payload, false)?;
            selection_ok = checked.ok;
            selected = checked.selected;
        }
    }
    let missing: BTreeSet<_> = selected.difference(&reports.executed).cloned().collect();
    let unexpected: BTreeSet<_> = reports.executed.difference(&selected).cloned().collect();
    let expected_cases = selected.len()
        + usize::from(selected.contains("checkout:unit:example.ParameterizedCheckoutTest"));
    let ok = status.success()
        && selection_ok
        && reports.failed == expected
        && missing.is_empty()
        && unexpected.is_empty()
        && reports.cases == expected_cases
        && reports.skipped.is_empty();
    let mut result = serde_json::to_value(&reports)?;
    result.as_object_mut().ok_or("Invalid report object")?.extend(json!({
        "tool": tool, "scenario": scenario, "ok": ok, "build_exit": status.code(),
        "selection_ok": selection_ok, "expected_failures": expected, "expected_cases": expected_cases,
        "missing_tests": missing, "unexpected_tests": unexpected,
    }).as_object().ok_or("Invalid result object")?.clone());
    write_json(&output.join(format!("{tool}-{scenario}.json")), &result)?;
    println!(
        "{tool:6} {scenario:24} {} ({} cases, {} failing classes)",
        if ok { "PASS" } else { "FAIL" },
        reports.cases,
        reports.failed.len()
    );
    if !ok {
        eprintln!("Inspect {}", log_path.display());
    }
    Ok(ok)
}

pub fn main(args: Vec<String>) -> Result<u8> {
    let mut args = args.into_iter();
    let command = args.next().unwrap_or_default();
    if matches!(command.as_str(), "" | "--help" | "-h") {
        println!(
            "sieve fixtures <command> [--root FIXTURE_REPO]\n\
          list\n\
          prepare --tool maven|gradle --dest PATH [--git]\n\
          apply SCENARIO --workspace PATH\n\
          check-selection SCENARIO --actual FILE [--exact]\n\
          reports --tool maven|gradle --workspace PATH\n\
          verify [--tool maven|gradle|both] [--scenario baseline|all|SCENARIO]\n\
                 [--selected] [--maven PATH] [--gradle PATH] [--output PATH]"
        );
        return Ok(0);
    }
    let allowed: &[&str] = match command.as_str() {
        "list" => &[],
        "prepare" => &["--tool", "--dest", "--git"],
        "apply" => &["--workspace"],
        "check-selection" => &["--actual", "--exact"],
        "reports" => &["--tool", "--workspace"],
        "verify" => &[
            "--tool",
            "--scenario",
            "--selected",
            "--maven",
            "--gradle",
            "--output",
        ],
        _ => return Err(format!("Unknown fixture command: {command}").into()),
    };
    let mut options = BTreeMap::new();
    let mut positional = None;
    while let Some(arg) = args.next() {
        if arg.starts_with("--") {
            if arg != "--root" && !allowed.contains(&arg.as_str()) {
                return Err(format!("Unknown option: {arg}").into());
            }
            let value = if matches!(arg.as_str(), "--git" | "--exact" | "--selected") {
                String::new()
            } else {
                args.next()
                    .ok_or_else(|| format!("Missing value for {arg}"))?
            };
            if options.insert(arg, value).is_some() {
                return Err("Duplicate option".into());
            }
        } else if matches!(command.as_str(), "apply" | "check-selection") && positional.is_none() {
            positional = Some(arg);
        } else {
            return Err(format!("Unexpected argument: {arg}").into());
        }
    }
    let required = |name: &str| -> Result<&str> {
        options
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| format!("{name} is required").into())
    };
    let option =
        |name: &str, default: &str| options.get(name).cloned().unwrap_or_else(|| default.into());
    let root = PathBuf::from(option("--root", ".")).canonicalize()?;
    let catalog = Catalog::read(&root)?;
    match command.as_str() {
        "list" => {
            for scenario in &catalog.scenarios {
                println!("{:24} {}", scenario.id, scenario.description);
            }
        }
        "prepare" => println!(
            "{}",
            prepare(
                &root,
                required("--tool")?,
                Path::new(required("--dest")?),
                options.contains_key("--git")
            )?
            .display()
        ),
        "apply" => {
            let name = positional.as_deref().ok_or("Scenario is required")?;
            apply(Path::new(required("--workspace")?), catalog.scenario(name)?)?;
            println!("Applied {name}");
        }
        "check-selection" => {
            let name = positional.as_deref().ok_or("Scenario is required")?;
            let payload = serde_json::from_slice(&fs::read(required("--actual")?)?)?;
            let result =
                check_selection(&catalog, name, &payload, options.contains_key("--exact"))?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            return Ok(u8::from(!result.ok));
        }
        "reports" => println!(
            "{}",
            serde_json::to_string_pretty(&read_reports(
                Path::new(required("--workspace")?),
                required("--tool")?
            )?)?
        ),
        "verify" => {
            let tool = option("--tool", "both");
            let tools = if tool == "both" {
                vec!["maven", "gradle"]
            } else {
                vec![tool_name(&tool)?]
            };
            let scenario = option("--scenario", "baseline");
            let scenarios = if scenario == "all" {
                std::iter::once("baseline")
                    .chain(catalog.scenarios.iter().map(|s| s.id.as_str()))
                    .collect()
            } else {
                if scenario != "baseline" {
                    catalog.scenario(&scenario)?;
                }
                vec![scenario.as_str()]
            };
            let output = PathBuf::from(option("--output", "validation-results"));
            let mut ok = true;
            for tool in tools {
                let executable = option(
                    &format!("--{tool}"),
                    if tool == "maven" { "mvn" } else { "gradle" },
                );
                // Resolve explicit relative executables before entering a temporary workspace.
                let executable = if Path::new(&executable).components().count() > 1 {
                    Path::new(&executable)
                        .canonicalize()?
                        .to_string_lossy()
                        .into_owned()
                } else {
                    executable
                };
                for scenario in &scenarios {
                    ok &= verify(
                        &root,
                        &catalog,
                        tool,
                        scenario,
                        &executable,
                        &output,
                        options.contains_key("--selected"),
                    )?;
                }
            }
            return Ok(u8::from(!ok));
        }
        _ => unreachable!(),
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_oracle_and_report_parser() -> Result<()> {
        let catalog = Catalog::read(Path::new(env!("CARGO_MANIFEST_DIR")))?;
        let required = &catalog.scenario("tax-transitive")?.required;
        assert!(
            check_selection(
                &catalog,
                "tax-transitive",
                &json!({"mode":"SUBSET","tests":required}),
                true
            )?
            .ok
        );
        let missing: Vec<_> = required.iter().skip(1).collect();
        assert!(
            !check_selection(
                &catalog,
                "tax-transitive",
                &json!({"mode":"SUBSET","tests":missing}),
                false
            )?
            .ok
        );
        assert!(check_selection(&catalog, "tax-transitive", &json!({"mode":"ALL"}), false)?.ok);
        assert!(!check_selection(&catalog, "tax-transitive", &json!({"mode":"ALL"}), true)?.ok);
        assert!(check_selection(&catalog, "docs-only", &json!({"mode":"NONE"}), true)?.ok);
        assert!(
            check_selection(
                &catalog,
                "tax-transitive",
                &json!({"mode":"MODULES","modules":["pricing","checkout"]}),
                false
            )?
            .ok
        );
        assert!(
            !check_selection(
                &catalog,
                "unused",
                &json!({"mode":"SUBSET","tests":["bogus"]}),
                false
            )?
            .ok
        );
        for payload in [
            json!({"mode":"bad"}),
            json!({"mode":"NONE","tests":["x"]}),
            json!({"mode":"SUBSET","tests":[]}),
            json!({"mode":"SUBSET","tests":["x","x"]}),
            json!({"mode":"SUBSET","tests":[null]}),
            json!({"mode":"SUBSET","tests":"x"}),
            json!({"mode":"MODULES","modules":[]}),
            json!({"mode":"MODULES","modules":["bogus"]}),
            json!({"mode":"MODULES","modules":["pricing","pricing"]}),
        ] {
            assert!(
                check_selection(&catalog, "unused", &payload, false).is_err(),
                "{payload}"
            );
        }
        let temp = tempfile::tempdir()?;
        for (tool, folder) in [
            ("maven", "target/surefire-reports"),
            ("gradle", "build/test-results/test"),
        ] {
            let folder = temp.path().join(tool).join("checkout").join(folder);
            fs::create_dir_all(&folder)?;
            let path = folder.join("TEST-example.xml");
            fs::write(
                &path,
                r#"<testsuite>
              <testcase classname="example.ParameterizedCheckoutTest" name="one"/>
              <testcase classname="example.ParameterizedCheckoutTest" name="two"><failure/></testcase>
              <testcase classname="example.SkippedTest" name="skip"><skipped/></testcase>
              <testcase classname="example.ErrorTest" name="error"><error>bad</error></testcase>
            </testsuite>"#,
            )?;
            let reports = read_reports(&temp.path().join(tool), tool)?;
            assert_eq!(reports.cases, 3);
            assert_eq!(reports.executed.len(), 2);
            assert_eq!(reports.failed.len(), 2);
            assert_eq!(reports.skipped.len(), 1);
            fs::write(&path, r#"<testsuite><testcase classname="broken">"#)?;
            assert!(read_reports(&temp.path().join(tool), tool).is_err());
        }
        assert!(mutation_path(temp.path(), "../escape").is_err());
        assert!(mutation_path(temp.path(), "/absolute").is_err());
        Ok(())
    }
}
