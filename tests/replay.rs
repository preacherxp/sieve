#![cfg(unix)]
mod support;
use serde_json::Value;
use std::fs;
use support::*;

#[test]
fn replay_rejects_unusable_reference_and_unreproduced_test_failures() {
    for inconclusive in [true, false] {
        let temp = fixture("maven");
        let root = temp.path().join("project");
        if inconclusive {
            write(&root, "README.md", "Documentation only.\n");
        } else {
            write(
                &root,
                "pricing/src/main/java/example/NewPrice.java",
                "package example; class NewPrice {}\n",
            );
        }
        commit(&root, "edit");
        let build = temp.path().join("build-tool");
        executable(
            &build,
            if inconclusive {
                "#!/bin/sh\nfor arg do\n  if [ \"$arg\" = verify ]; then exit 1; fi\ndone\nexit 0\n"
            } else {
                r#"#!/bin/sh
selected=0
for arg do
  if [ "$arg" = -pl ]; then selected=1; fi
done
mkdir -p pricing/target/surefire-reports
if [ "$selected" = 1 ]; then
  echo '<testsuite><testcase classname="example.PriceTest" name="price"/></testsuite>' > pricing/target/surefire-reports/TEST-example.PriceTest.xml
  exit 0
fi
echo '<testsuite><testcase classname="example.PriceTest" name="price"><failure/></testcase></testsuite>' > pricing/target/surefire-reports/TEST-example.PriceTest.xml
exit 1
"#
            },
        );
        let report = temp.path().join("replay.json");
        let output = cli(&[
            "replay",
            "--workspace",
            root.to_str().unwrap(),
            "--commits",
            "1",
            "--run",
            "--executable",
            build.to_str().unwrap(),
            "--output",
            report.to_str().unwrap(),
        ]);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty());
        let markdown = fs::read_to_string(format!("{}.md", report.display())).unwrap();
        assert!(String::from_utf8_lossy(&output.stderr).contains(&markdown));
        assert!(markdown.contains("No usable timing pairs; no savings can be reported."));
        assert!(markdown.contains("Excluded from savings: 1 replayed commits"));
        let report: Value = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
        let record = &report["records"][0];
        assert_eq!(record["full"]["exit"], 1);
        assert_eq!(record["selected"]["exit"], 0);
        assert_eq!(record["inconclusive"], inconclusive);
        assert!(report["summary"]["missed_failures"].as_u64().unwrap() > 0);
        if inconclusive {
            assert_eq!(record["selection"]["mode"], "NONE");
            assert_eq!(report["summary"]["inconclusive_runs"], 1);
        } else {
            assert_eq!(
                record["selected"]["executed"],
                serde_json::json!(["pricing:unit:example.PriceTest"])
            );
            assert!(record["missed"]
                .as_array()
                .unwrap()
                .iter()
                .any(|id| id == "pricing:unit:example.PriceTest"));
        }
    }
}

#[test]
fn replay_preview_keeps_json_stdout_and_explains_absent_measurements() {
    let temp = fixture("maven");
    let root = temp.path().join("project");
    write(&root, "README.md", "Documentation only.\n");
    commit(&root, "edit");
    let output = cli(&[
        "replay",
        "--workspace",
        root.to_str().unwrap(),
        "--commits",
        "1",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["summary"]["replayed"], 1);
    assert_eq!(report["summary"]["modes"]["NONE"], 1);
    assert!(report["summary"].get("full_seconds").is_none());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Preview only: no builds ran"));
}
