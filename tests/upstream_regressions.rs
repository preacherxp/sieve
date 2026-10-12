//! Independent, immutable upstream failures: the harness never authors its own bug or tests.
mod support;
use serde_json::{json, Value};
use std::{fs, path::Path, process::Command};
use support::*;

const CASE: &str = "fixtures/upstream/commons-text-text158";
const PRODUCTION: &str = "src/main/java/org/apache/commons/text/similarity/JaccardSimilarity.java";
const FAILURES: [&str; 2] = [
    ".:unit:org.apache.commons.text.similarity.JaccardDistanceTest",
    ".:unit:org.apache.commons.text.similarity.JaccardSimilarityTest",
];

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &to.join(entry.file_name()));
        } else {
            fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
}

#[test]
fn vendored_text158_sources_match_immutable_upstream_blobs() {
    let root = Path::new(ROOT).join(CASE);
    let manifest: Value =
        serde_json::from_slice(&fs::read(root.join("provenance.json")).unwrap()).unwrap();
    for file in manifest["files"].as_array().unwrap() {
        let output = Command::new("git")
            .args(["hash-object"])
            .arg(root.join(file["path"].as_str().unwrap()))
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            file["git_blob"].as_str().unwrap(),
            "upstream file changed: {}",
            file["path"]
        );
    }
}

#[test]
#[ignore = "requires Maven, Java 17+, and cached/prefetched pinned test dependencies"]
fn upstream_text158_shadow_retains_real_failures_and_detects_a_broken_selector() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    copy_tree(&Path::new(ROOT).join(CASE), &root);
    let results = std::env::var_os("SIEVE_UPSTREAM_RESULTS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| Path::new(ROOT).join("target/upstream-regressions/text158"));
    fs::create_dir_all(&results).unwrap();
    let results = results.canonicalize().unwrap();
    if results.join("evidence.json").exists() {
        fs::remove_file(results.join("evidence.json")).unwrap();
    }
    let executable = std::env::var("IMPACT_MAVEN").unwrap_or_else(|_| "mvn".into());
    let version = Command::new(&executable).arg("-version").output().unwrap();
    assert!(version.status.success());
    fs::write(
        results.join("toolchain.log"),
        [&version.stdout[..], &version.stderr[..]].concat(),
    )
    .unwrap();
    let mut setup_args = vec![
        "init",
        "--workspace",
        root.to_str().unwrap(),
        "--executable",
        &executable,
    ];
    if std::env::var_os("SIEVE_UPSTREAM_OFFLINE").is_some() {
        setup_args.extend(["--", "--offline"]);
    }
    let setup = cli(&setup_args);
    assert!(
        setup.status.success(),
        "{}",
        String::from_utf8_lossy(&setup.stderr)
    );
    let mut config: Value =
        serde_json::from_slice(&fs::read(root.join("impact.json")).unwrap()).unwrap();
    config["class_level"] = json!(true);
    write(
        &root,
        "impact.json",
        serde_json::to_vec_pretty(&config).unwrap(),
    );
    git(root.to_str().unwrap(), &["init", "-q"]);
    let base = commit(
        &root,
        "Unchanged upstream TEXT-158 fixed sources and regression tests",
    );
    let observe = |name: &str, broken: bool, full: bool| -> (bool, Value) {
        let output_path = results.join(format!("{name}.json"));
        if output_path.exists() {
            fs::remove_file(&output_path).unwrap();
        }
        let mut command = Command::new(BIN);
        command.args([
            "run",
            "--workspace",
            root.to_str().unwrap(),
            "--executable",
            &executable,
            "--observe",
            "--output",
            output_path.to_str().unwrap(),
        ]);
        if full {
            command.arg("--full");
        } else {
            command.args(["--base", &base]);
        }
        if broken {
            command.env("SIEVE_BROKEN_SELECTOR", "1");
        } else {
            command.env_remove("SIEVE_BROKEN_SELECTOR");
        }
        if std::env::var_os("SIEVE_UPSTREAM_OFFLINE").is_some() {
            command.args(["--", "--offline"]);
        }
        let output = command.output().unwrap();
        fs::write(
            results.join(format!("{name}.log")),
            [&output.stdout[..], &output.stderr[..]].concat(),
        )
        .unwrap();
        let selection: Value =
            serde_json::from_slice(&fs::read(&output_path).unwrap_or_else(|error| {
                panic!(
                    "{error}; inspect {}",
                    results.join(format!("{name}.log")).display()
                )
            }))
            .unwrap();
        assert!(
            selection["observation"].is_object(),
            "missing observation; inspect {}",
            results.join(format!("{name}.log")).display()
        );
        (output.status.success(), selection)
    };
    let (success, baseline) = observe("fixed", false, true);
    assert!(
        success,
        "fixed upstream tests must pass; inspect {}",
        results.display()
    );
    assert_eq!(
        baseline["observation"]["executed_classes"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(baseline["observation"]["failed_classes"], json!([]));
    assert!(baseline["observation"]["evidence"]
        .as_str()
        .unwrap()
        .starts_with("no failure-retention evidence"));
    fs::copy(
        root.join("regression/JaccardSimilarity.java"),
        root.join(PRODUCTION),
    )
    .unwrap();
    assert_eq!(
        git(root.to_str().unwrap(), &["diff", "--name-only"]),
        PRODUCTION,
        "only the actual upstream production fix may be reverted; tests must stay unchanged"
    );
    let (success, regressed) = observe("regressed", false, false);
    assert!(
        !success,
        "the unchanged upstream regression tests must fail on the buggy source"
    );
    assert_eq!(regressed["observation"]["failed_classes"], json!(FAILURES));
    assert_eq!(regressed["observation"]["proposed"]["mode"], "SUBSET");
    assert_eq!(
        regressed["observation"]["proposed"]["tests"],
        json!([
            "org.apache.commons.text.similarity.JaccardDistanceTest",
            "org.apache.commons.text.similarity.JaccardSimilarityTest",
        ])
    );
    assert_eq!(
        regressed["observation"]["executed_classes"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(regressed["observation"]["missed_failed_classes"], json!([]));
    assert_eq!(regressed["observation"]["failure_retention"], 1.0);
    let (success, broken) = observe("broken-selector", true, false);
    assert!(!success);
    assert_eq!(broken["observation"]["proposed"]["mode"], "NONE");
    assert_eq!(broken["observation"]["failed_classes"], json!(FAILURES));
    assert_eq!(
        broken["observation"]["missed_failed_classes"],
        json!(FAILURES)
    );
    assert_eq!(broken["observation"]["failure_retention"], 0.0);
    assert_eq!(broken["observation"]["would_detect_test_failure"], false);
    assert_eq!(
        git(root.to_str().unwrap(), &["diff", "--name-only"]),
        PRODUCTION
    );
    fs::write(results.join("evidence.json"), serde_json::to_vec_pretty(&json!({
        "case": "Apache Commons Text TEXT-158", "fix_revision": "38727e31d978350b76edba1683a081a114fe345f",
        "buggy_revision": "ac09fd74ed1c71de634f58f5ec89e66d729e0ffc", "unchanged_test_classes": 3,
        "actual_upstream_failures": FAILURES, "normal_retention": 1.0, "broken_selector_retention": 0.0,
        "scope": "one static selection case; no runtime-record or performance claim"
    })).unwrap()).unwrap();
}
