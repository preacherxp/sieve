#![cfg(unix)]
mod support;
use serde_json::{json, Value};
use std::fs;
use support::*;

#[test]
fn verifier_rejects_missing_extra_skipped_failed_and_malformed_execution() {
    // RUN-04/05: a fake native executable is intentional; it attempts to fool the real verifier.
    let temp = tempfile::tempdir().unwrap();
    let catalog = temp.path().join("catalog");
    success(&[
        "fixtures",
        "prepare",
        "--tool",
        "maven",
        "--dest",
        catalog.join("projects/maven").to_str().unwrap(),
    ]);
    write(
        &catalog,
        "scenarios.json",
        serde_json::to_vec(
            &json!({"baseline_tests":["pricing:unit:example.ExpectedTest"],"scenarios":[]}),
        )
        .unwrap(),
    );
    let executable_path = temp.path().join("build");
    let output = temp.path().join("results");
    let valid =
        "<testsuite><testcase classname=\"example.ExpectedTest\" name=\"works\"/></testsuite>";
    for (report, exit, passes) in [
        (valid, 0, true), (valid, 1, false), ("", 0, false), ("<testsuite/>", 0, false),
        ("<testsuite><testcase classname=\"example.OtherTest\" name=\"works\"/></testsuite>", 0, false),
        ("<testsuite><testcase classname=\"example.ExpectedTest\" name=\"works\"/><testcase classname=\"example.ExtraTest\" name=\"extra\"/></testsuite>", 0, false),
        ("<testsuite><testcase classname=\"example.ExpectedTest\" name=\"one\"/><testcase classname=\"example.ExpectedTest\" name=\"two\"/></testsuite>", 0, false),
        ("<testsuite><testcase classname=\"example.ExpectedTest\" name=\"works\"><skipped/></testcase></testsuite>", 0, false),
        ("<testsuite><testcase classname=\"example.ExpectedTest\" name=\"works\"><failure/></testcase></testsuite>", 0, false),
        ("<testsuite><testcase classname=\"example.ExpectedTest\" name=\"works\"><error/></testcase></testsuite>", 0, false),
        ("<testsuite><testcase classname=\"example.ExpectedTest\" name=\"works\"/><testcase classname=\"example.ExpectedTest\" name=\"works\"/></testsuite>", 0, false),
        ("<testsuite><testcase", 0, false), ("<other/>", 0, false),
    ] {
        executable(&executable_path, &format!("#!/bin/sh\nmkdir -p pricing/target/surefire-reports\ncat > pricing/target/surefire-reports/TEST-result.xml <<'XML'\n{report}\nXML\nexit {exit}\n"));
        let result = cli(&["fixtures", "verify", "--root", catalog.to_str().unwrap(), "--tool", "maven", "--maven", executable_path.to_str().unwrap(), "--output", output.to_str().unwrap()]);
        assert_eq!(result.status.success(), passes, "report={report}, exit={exit}: {}", String::from_utf8_lossy(&result.stderr));
    }
    executable(&executable_path, "#!/bin/sh\nexit 0\n");
    assert!(!cli(&[
        "fixtures",
        "verify",
        "--root",
        catalog.to_str().unwrap(),
        "--tool",
        "maven",
        "--maven",
        executable_path.to_str().unwrap(),
        "--output",
        output.to_str().unwrap()
    ])
    .status
    .success());
}

#[test]
fn mutations_are_atomic_and_refuse_symlink_escape() {
    // RUN-08: validate every mutation before writing even the first file.
    let temp = fixture("maven");
    let root = temp.path().join("project");
    let catalog = temp.path().join("catalog");
    let file = "pricing/src/main/java/example/TaxRules.java";
    let original = fs::read(root.join(file)).unwrap();
    let outside = temp.path().join("outside");
    fs::write(&outside, "untouched").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("linked")).unwrap();
    for bad in [
        json!({"path":"linked", "before":"untouched","after":"bad"}),
        json!({"path":file,"before":"not present","after":"bad"}),
        json!({"path":"../outside","delete":true}),
    ] {
        write(&catalog, "scenarios.json", serde_json::to_vec(&json!({"baseline_tests":[],"scenarios":[{"id":"bad","description":"must refuse","required":[],"expected_failures":[],"changes":[{"path":file,"before":"return 20;","after":"return 30;"},bad]}]})).unwrap());
        let result = cli(&[
            "fixtures",
            "apply",
            "bad",
            "--root",
            catalog.to_str().unwrap(),
            "--workspace",
            root.to_str().unwrap(),
        ]);
        assert!(!result.status.success());
        assert_eq!(fs::read(root.join(file)).unwrap(), original);
        assert_eq!(fs::read_to_string(&outside).unwrap(), "untouched");
        let marker: Value =
            serde_json::from_slice(&fs::read(root.join(".fixture-workspace.json")).unwrap())
                .unwrap();
        assert_eq!(marker["scenario"], "baseline");
    }
}

#[test]
fn benchmark_requires_correct_results_and_keeps_raw_samples() {
    let temp = tempfile::tempdir().unwrap();
    let catalog = temp.path().join("catalog");
    success(&[
        "fixtures",
        "prepare",
        "--tool",
        "maven",
        "--dest",
        catalog.join("projects/maven").to_str().unwrap(),
    ]);
    write(
        &catalog,
        "scenarios.json",
        serde_json::to_vec(
            &json!({"baseline_tests":["pricing:unit:example.ExpectedTest"],"scenarios":[]}),
        )
        .unwrap(),
    );
    let build = temp.path().join("build");
    executable(&build, "#!/bin/sh\nif [ \"$1\" = --version ]; then echo fake-build; exit 0; fi\nmkdir -p pricing/target/surefire-reports\nprintf '<testsuite><testcase classname=\"example.ExpectedTest\" name=\"works\"/></testsuite>' > pricing/target/surefire-reports/TEST-result.xml\n");
    let output = temp.path().join("results");
    let args = [
        "fixtures",
        "benchmark",
        "--root",
        catalog.to_str().unwrap(),
        "--tool",
        "maven",
        "--maven",
        build.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ];
    success(&args);
    let report: Value =
        serde_json::from_slice(&fs::read(output.join("maven-baseline-benchmark.json")).unwrap())
            .unwrap();
    assert_eq!(report["records"].as_array().unwrap().len(), 18);
    for mode in ["native", "full", "selected"] {
        let summary = &report["summaries"][mode];
        let mut samples: Vec<f64> = summary["samples_seconds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        assert_eq!(samples.len(), 5);
        samples.sort_by(f64::total_cmp);
        assert_eq!(summary["median_seconds"].as_f64().unwrap(), samples[2]);
    }
    let mut too_few = args.to_vec();
    too_few.extend(["--runs", "4"]);
    assert!(!cli(&too_few).status.success());
    executable(&build, "#!/bin/sh\nexit 0\n");
    assert!(!cli(&args).status.success());
    assert!(!output.join("maven-baseline-benchmark.json").exists());
}
