#![cfg(unix)]
mod support;
use serde_json::Value;
use std::{collections::BTreeMap, fs, path::Path, process::Command};
use support::*;

#[test]
fn every_fixture_has_one_selected_variant_and_full_verification_remains() {
    // CI-07: adding a manifest scenario without scheduling it must break this check.
    let workflow = fs::read_to_string(Path::new(ROOT).join(".github/workflows/ci.yml")).unwrap();
    let mut scheduled = BTreeMap::new();
    for line in workflow.lines() {
        if let Some((_, cases)) = line.split_once(") scenarios=(") {
            for case in cases.split(')').next().unwrap().split_whitespace() {
                *scheduled.entry(case.to_owned()).or_insert(0) += 1;
            }
        }
    }
    let manifest: Value =
        serde_json::from_slice(&fs::read(Path::new(ROOT).join("scenarios.json")).unwrap()).unwrap();
    let expected: BTreeMap<_, _> = manifest["scenarios"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (s["id"].as_str().unwrap().to_owned(), 1))
        .collect();
    assert_eq!(scheduled, expected);
    assert!(workflow.contains("needs: selected-tests\n    if: ${{ !cancelled() }}"));
    assert!(!workflow.contains("pull_request_target"));
    let tests =
        fs::read_to_string(Path::new(ROOT).join(".github/workflows/java-tests.yml")).unwrap();
    assert!(tests.contains("--scenario all"));
    assert!(tests.contains("fetch-depth: 0"));
    assert!(tests.contains("persist-credentials: false"));
    assert!(tests.contains("if: always()"));
    assert!(!tests.contains("continue-on-error"));
}

#[test]
fn event_modes_quote_inputs_and_propagate_build_failures() {
    // CI-01..06: execute the *same* shell script used by the real workflow.
    let temp = tempfile::tempdir().unwrap();
    let sieve = temp.path().join("sieve");
    executable(
        &sieve,
        r#"#!/bin/sh
printf '%s\000' "$@" >> "$CAPTURE"
command=$1
for arg do
 if [ "${next:-0}" = 1 ]; then output=$arg; next=0; fi
 if [ "$arg" = --output ]; then next=1; fi
done
printf '{"mode":"%s"}\n' "$DECISION" > "$output"
if [ "$command" = run ]; then exit "$BUILD_EXIT"; fi
"#,
    );
    for (event, pr, push, mode, decision, exit, expected_base, expected_calls) in [
        (
            "pull_request",
            "pr-base",
            "push-base",
            "selected",
            "MODULES",
            0,
            Some("pr-base"),
            2,
        ),
        (
            "pull_request",
            "$(touch injected); spaced",
            "",
            "selected",
            "MODULES",
            0,
            Some("$(touch injected); spaced"),
            2,
        ),
        (
            "push",
            "",
            "previous",
            "selected",
            "MODULES",
            0,
            Some("previous"),
            2,
        ),
        ("push", "", "", "selected", "ALL", 0, None, 1),
        (
            "push",
            "",
            "0000000000000000000000000000000000000000",
            "selected",
            "ALL",
            0,
            None,
            1,
        ),
        ("workflow_dispatch", "", "", "selected", "ALL", 0, None, 1),
        ("pull_request", "", "", "selected", "ALL", 0, None, 1),
        ("pull_request", "pr-base", "", "full", "ALL", 0, None, 1),
        (
            "pull_request",
            "pr-base",
            "",
            "selected",
            "NONE",
            0,
            Some("pr-base"),
            2,
        ),
        (
            "pull_request",
            "pr-base",
            "",
            "selected",
            "MODULES",
            7,
            Some("pr-base"),
            2,
        ),
    ] {
        let capture = temp.path().join("argv");
        fs::write(&capture, "").unwrap();
        let result = Command::new("bash")
            .arg(Path::new(ROOT).join("scripts/ci-test-stage.sh"))
            .current_dir(temp.path())
            .env("BUILD_TOOL", "maven")
            .env("MODE", mode)
            .env("EVENT_NAME", event)
            .env("PR_BASE", pr)
            .env("PUSH_BASE", push)
            .env("SIEVE_BIN", &sieve)
            .env("SELECTION_DIR", temp.path().join("reports"))
            .env("CAPTURE", &capture)
            .env("DECISION", decision)
            .env("BUILD_EXIT", exit.to_string())
            .output()
            .unwrap();
        assert_eq!(
            result.status.code(),
            Some(exit),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let bytes = fs::read(&capture).unwrap();
        let args: Vec<_> = bytes
            .split(|b| *b == 0)
            .filter(|v| !v.is_empty())
            .map(|v| std::str::from_utf8(v).unwrap())
            .collect();
        assert_eq!(
            args.iter()
                .filter(|a| **a == "select" || **a == "run")
                .count(),
            expected_calls
        );
        if let Some(base) = expected_base {
            assert!(args.windows(2).any(|a| a == ["--base", base]));
        } else {
            assert!(args.contains(&"--full"));
            assert!(!args.contains(&"--base"));
        }
        assert!(!temp.path().join("injected").exists());
        assert!(temp.path().join("reports/maven-selection.json").exists());
    }
    let invalid = Command::new("bash")
        .arg(Path::new(ROOT).join("scripts/ci-test-stage.sh"))
        .current_dir(temp.path())
        .env("BUILD_TOOL", "maven")
        .env("MODE", "invalid")
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
}

#[test]
fn ci_cost_counts_parallel_and_failed_jobs() {
    let temp = tempfile::tempdir().unwrap();
    write(
        temp.path(),
        "jobs.json",
        r#"{"jobs":[
      {"name":"a","status":"completed","conclusion":"success","started_at":"2026-01-01T00:00:00Z","completed_at":"2026-01-01T00:00:20Z"},
      {"name":"b","status":"completed","conclusion":"failure","started_at":"2026-01-01T00:00:10Z","completed_at":"2026-01-01T00:00:30Z"},
      {"name":"skipped","status":"completed","conclusion":"skipped","started_at":null,"completed_at":null}
    ]}"#,
    );
    let result = Command::new("bash")
        .arg(Path::new(ROOT).join("scripts/ci-cost.sh"))
        .arg("--input")
        .arg(temp.path().join("jobs.json"))
        .output()
        .unwrap();
    assert!(result.status.success());
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["aggregate_runner_seconds"], 40);
    assert_eq!(report["observed_wall_seconds"], 30);
    assert_eq!(report["completed_jobs"], 3);
    assert_eq!(report["timed_jobs"], 2);
}
