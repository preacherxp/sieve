#![cfg(feature = "agent")]

mod support;

use std::{fs, path::PathBuf, process::Command};
use support::*;

#[test]
fn overlapping_classes_and_cached_contexts_retain_async_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    write(
        &root,
        "impact.json",
        r#"{"tool":"maven","modules":{".":[]}}"#,
    );
    write(&root, "pom.xml", "<project/>");
    let output = Command::new(BIN)
        .args(["env", "--workspace", root.to_str().unwrap()])
        .env("SIEVE_CACHE_DIR", temp.path().join("cache"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let option = String::from_utf8(output.stdout).unwrap();
    let jar = PathBuf::from(
        option
            .trim()
            .strip_prefix("-javaagent:")
            .unwrap()
            .split_once('=')
            .unwrap()
            .0,
    );
    let classes = temp.path().join("classes");
    fs::create_dir(&classes).unwrap();
    let classpath = std::env::join_paths([
        jar.clone(),
        jar.with_file_name("sieve-probe.jar"),
        classes.clone(),
    ])
    .unwrap();
    let output = Command::new("javac")
        .args(["--release", "17", "-d"])
        .arg(&classes)
        .arg("-classpath")
        .arg(&classpath)
        .arg(PathBuf::from(ROOT).join("agent/tests/sieve/agent/ParallelEvidence.java"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new("java")
        .arg("-classpath")
        .arg(classpath)
        .arg("sieve.agent.ParallelEvidence")
        .arg(&root)
        .arg(BIN)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let plans: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let raw = &plans[0];
    assert_eq!(raw["parallel"], true);
    assert_eq!(raw["parallel_safe"], true);
    assert_eq!(raw["errors"], serde_json::json!([]));
    let tests = raw["tests"].as_array().unwrap();
    assert_eq!(tests.len(), 4);
    for name in ["FirstTest", "SecondTest", "CachedContextTest"] {
        let test = tests
            .iter()
            .find(|test| test["name"] == format!("example.{name}"))
            .unwrap();
        let methods = test["methods"].as_array().unwrap();
        for method in [
            "example/First#startup()V",
            "example/First#test()V",
            "example/Second#worker()V",
            "example/Lazy#initialize()V",
        ] {
            assert!(methods.contains(&serde_json::json!(method)), "{test:#}");
        }
        assert!(
            test["files"]
                .as_array()
                .unwrap()
                .iter()
                .any(|file| file.as_str().unwrap().ends_with("/async-input.txt")),
            "{test:#}"
        );
    }
    let independent = tests
        .iter()
        .find(|test| test["name"] == "example.IndependentTest")
        .unwrap();
    assert_eq!(
        independent["methods"],
        serde_json::json!(["example/Independent#test()V"])
    );
    assert_eq!(independent["files"], serde_json::json!([]));
    let repeated = &plans[1]["tests"];
    assert_eq!(repeated.as_array().unwrap().len(), 1, "{repeated:#}");
    assert_eq!(repeated[0]["name"], "example.RepeatedTest");
    assert_eq!(repeated[0]["passed"], false);
    assert_eq!(
        repeated[0]["methods"],
        serde_json::json!([
            "example/Partner#test()V",
            "example/Repeated#first()V",
            "example/Repeated#second()V"
        ])
    );
}
