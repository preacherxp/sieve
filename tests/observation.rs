#![cfg(unix)]
mod support;
use serde_json::{json, Value};
use std::{fs, process::Command};
use support::*;

#[test]
fn shadow_runs_full_once_and_exposes_a_broken_selector_miss() {
    let temp = fixture("maven");
    let root = temp.path().join("project");
    write(
        &root,
        "pricing/src/main/java/example/NewPrice.java",
        "package example; class NewPrice {}\n",
    );
    let build = temp.path().join("build-tool");
    executable(
        &build,
        r#"#!/bin/sh
for arg do
  case "$arg" in -pl|-Dimpact.skip.*=true) exit 19;; esac
done
mkdir -p pricing/target/surefire-reports
echo '<testsuite tests="1" failures="1" errors="0" skipped="0"><testcase classname="example.PriceTest" name="price"><failure/></testcase></testsuite>' > pricing/target/surefire-reports/TEST-example.PriceTest.xml
exit 1
"#,
    );
    let report = temp.path().join("observation.json");
    let output = Command::new(BIN)
        .args([
            "run",
            "--workspace",
            root.to_str().unwrap(),
            "--base",
            "HEAD",
            "--observe",
            "--output",
            report.to_str().unwrap(),
            "--executable",
            build.to_str().unwrap(),
        ])
        .env("SIEVE_BROKEN_SELECTOR", "1")
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
    let observation = &report["observation"];
    assert_eq!(observation["proposed"]["mode"], "NONE");
    assert_eq!(
        observation["missed_failed_classes"],
        json!(["pricing:unit:example.PriceTest"])
    );
    assert_eq!(observation["failure_retention"], 0.0);
    assert_eq!(observation["would_detect_test_failure"], false);
}

#[test]
fn shadow_reports_no_failure_evidence_for_a_passing_full_run() {
    let temp = fixture("maven");
    let root = temp.path().join("project");
    let build = temp.path().join("build-tool");
    executable(&build, "#!/bin/sh\nmkdir -p pricing/target/surefire-reports\necho '<testsuite><testcase classname=\"example.PriceTest\" name=\"price\"/></testsuite>' > pricing/target/surefire-reports/TEST-example.PriceTest.xml\n");
    let report = temp.path().join("observation.json");
    let output = cli(&[
        "run",
        "--workspace",
        root.to_str().unwrap(),
        "--base",
        "HEAD",
        "--observe",
        "--output",
        report.to_str().unwrap(),
        "--executable",
        build.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
    assert!(report["observation"]["failure_retention"].is_null());
    assert!(report["observation"]["evidence"]
        .as_str()
        .unwrap()
        .starts_with("no failure-retention evidence"));
}
