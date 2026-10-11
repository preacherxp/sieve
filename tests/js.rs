//! Jest and Vitest selection (ADR 0006) against the independent oracle in
//! `projects/js/scenarios.json`. One fixture carries both runner configurations, so both
//! runners face identical sources and expectations.
mod support;
use serde_json::Value;
use std::{fs, path::Path, process::Command};
use support::*;

const FIXTURE: &str = "projects/js";

fn tools() -> Vec<&'static str> {
    let requested = std::env::var("IMPACT_TOOL").unwrap_or_else(|_| "both".into());
    ["jest", "vitest"]
        .into_iter()
        .filter(|tool| requested == "both" || requested == *tool)
        .collect()
}

/// A Git copy of the fixture whose `node_modules` links to an `npm ci` of the original.
fn prepare(tool: &str) -> tempfile::TempDir {
    let fixture = Path::new(ROOT).join(FIXTURE);
    if !fixture.join("node_modules/.bin/vitest").exists() {
        let status = Command::new("npm")
            .args(["ci", "--no-audit", "--no-fund"])
            .current_dir(&fixture)
            .status()
            .unwrap();
        assert!(status.success(), "npm ci failed");
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    copy(&fixture, &root);
    #[cfg(unix)]
    std::os::unix::fs::symlink(fixture.join("node_modules"), root.join("node_modules")).unwrap();
    let path = root.to_str().unwrap();
    git(path, &["init", "-q"]);
    commit(&root, "fixture");
    success(&["init", "--workspace", path, "--tool", tool]);
    commit(&root, "configure");
    temp
}

fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == "node_modules" || name == "scenarios.json" {
            continue;
        }
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &to.join(&name));
        } else {
            fs::copy(entry.path(), to.join(&name)).unwrap();
        }
    }
}

fn apply(root: &Path, change: &Value) {
    let path = root.join(change["path"].as_str().unwrap());
    if change["delete"] == true {
        fs::remove_file(path).unwrap();
    } else if let Some(text) = change["write"].as_str() {
        write(root, change["path"].as_str().unwrap(), text);
    } else {
        let mut text = fs::read_to_string(&path).unwrap();
        text += change["append"].as_str().unwrap();
        fs::write(path, text).unwrap();
    }
}

fn reset(root: &Path) {
    let path = root.to_str().unwrap();
    git(path, &["checkout", "-q", "--", "."]);
    git(path, &["clean", "-fdq", "-e", "node_modules"]);
}

fn run(root: &Path) -> std::process::Output {
    cli(&[
        "run",
        "--workspace",
        root.to_str().unwrap(),
        "--base",
        "HEAD",
    ])
}

#[test]
#[ignore = "requires Node.js and npm; run by the JavaScript CI job"]
fn javascript_selection_matches_the_oracle() {
    let oracle: Value = serde_json::from_slice(
        &fs::read(Path::new(ROOT).join(FIXTURE).join("scenarios.json")).unwrap(),
    )
    .unwrap();
    for tool in tools() {
        let temp = prepare(tool);
        let root = temp.path().join("project");
        let config: Value =
            serde_json::from_slice(&fs::read(root.join("impact.json")).unwrap()).unwrap();
        assert_eq!(
            config["sources"], oracle["sources"],
            "{tool}: init source roots"
        );
        for scenario in oracle["scenarios"].as_array().unwrap() {
            for change in scenario["changes"].as_array().unwrap() {
                apply(&root, change);
            }
            let selection = select(root.to_str().unwrap(), "HEAD");
            let id = &scenario["id"];
            assert_eq!(
                selection["mode"], scenario["mode"],
                "{tool} {id}: {selection}"
            );
            let tests = selection
                .get("tests")
                .cloned()
                .unwrap_or(Value::Array(vec![]));
            assert_eq!(tests, scenario["tests"], "{tool} {id}: {selection}");
            reset(&root);
        }
    }
}

#[test]
#[ignore = "requires Node.js and npm; run by the JavaScript CI job"]
fn javascript_runs_propagate_failures() {
    for tool in tools() {
        let temp = prepare(tool);
        let root = temp.path().join("project");
        // A selected run fails with the broken source.
        write(
            &root,
            "src/math.ts",
            "export const add = (a: number, b: number): number => a - b;\n",
        );
        assert_eq!(run(&root).status.code(), Some(1), "{tool}: broken source");
        // A dropped-everything selector would hide it, so it must be detectable.
        let output = Command::new(BIN)
            .args([
                "run",
                "--workspace",
                root.to_str().unwrap(),
                "--base",
                "HEAD",
            ])
            .env("SIEVE_BROKEN_SELECTOR", "1")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{tool}: broken selector");
        reset(&root);
        // NONE starts no runner; a full run passes.
        write(&root, "src/main.ts", "export {};\n");
        let output = run(&root);
        assert!(
            output.status.success(),
            "{tool}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let full = cli(&["run", "--workspace", root.to_str().unwrap(), "--full"]);
        assert!(full.status.success(), "{tool}: full run");
    }
}

#[test]
#[ignore = "requires Node.js and npm; run by the JavaScript CI job"]
fn javascript_replay_compares_selected_and_full_runs() {
    for tool in tools() {
        let temp = prepare(tool);
        let root = temp.path().join("project");
        write(
            &root,
            "src/math.ts",
            "export const add = (a: number, b: number): number => a + b;\nexport const one = 1;\n",
        );
        commit(&root, "source");
        write(&root, "src/main.ts", "export {};\n");
        commit(&root, "unreached");
        write(
            &root,
            "src/math.ts",
            "export const add = (a: number, b: number): number => a - b;\n",
        );
        commit(&root, "break math");
        let output = cli(&[
            "replay",
            "--workspace",
            root.to_str().unwrap(),
            "--commits",
            "3",
            "--run",
        ]);
        assert!(
            output.status.success(),
            "{tool}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        let modes: Vec<_> = report["records"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["selection"]["mode"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(modes, ["SUBSET", "NONE", "SUBSET"], "{tool}: {report}");
        let broken = &report["records"][0];
        assert_eq!(
            broken["full"]["failed"],
            serde_json::json!(["src/math.test.ts::add"])
        );
        assert_eq!(
            broken["selected"]["failed"], broken["full"]["failed"],
            "{tool}"
        );
        assert_eq!(report["summary"]["missed_failures"], 0, "{tool}");
    }
}
