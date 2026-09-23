mod support;
use serde_json::{json, Value};
use std::{fs, path::Path, process::Command};
use support::*;

fn decision(root: &Path) -> Value {
    select(root.to_str().unwrap(), "HEAD")
}

#[test]
fn git_input_matrix_and_net_changes() {
    // GIT-01/02/07/08/10/11: classifications are declared independently here.
    for tool in ["maven", "gradle"] {
        let temp = fixture(tool);
        let root = temp.path().join("project");
        for (path, mode, modules) in [
            (
                "pricing/src/main/java/example/odd ü\nname.txt",
                "MODULES",
                vec!["checkout", "pricing"],
            ),
            (
                "checkout/src/test/resources/input.txt",
                "MODULES",
                vec!["checkout"],
            ),
            (
                "runtime/src/main/resources/docs/README.md",
                "MODULES",
                vec!["runtime"],
            ),
            ("docs/notes.md", "NONE", vec![]),
            ("VALIDATION.md", "NONE", vec![]),
            (
                "gradle.lockfile",
                "ALL",
                vec!["checkout", "pricing", "runtime"],
            ),
            (
                ".mvn/maven.config",
                "ALL",
                vec!["checkout", "pricing", "runtime"],
            ),
            (
                "pricing/build.gradle.kts",
                "ALL",
                vec!["checkout", "pricing", "runtime"],
            ),
            (
                ".github/workflows/check.yml",
                "ALL",
                vec!["checkout", "pricing", "runtime"],
            ),
            ("unknown.txt", "ALL", vec!["checkout", "pricing", "runtime"]),
        ] {
            write(&root, path, "changed");
            let selected = decision(&root);
            assert_eq!(selected["mode"], mode, "{tool}: {path}: {selected}");
            assert_eq!(selected["modules"], json!(modules), "{path}");
            fs::remove_file(root.join(path)).unwrap();
        }
        write(&root, "checkout/src/test/resources/a", "a");
        write(&root, "runtime/src/main/resources/b", "b");
        write(&root, "docs/change.md", "documentation");
        assert_eq!(decision(&root)["modules"], json!(["checkout", "runtime"]));
        let base = git(root.to_str().unwrap(), &["rev-parse", "HEAD"]);
        commit(&root, "two modules");
        git(root.to_str().unwrap(), &["revert", "--no-edit", "HEAD"]);
        assert_eq!(select(root.to_str().unwrap(), &base)["mode"], "NONE");
        for path in [
            "pricing/target/report.xml",
            "runtime/build/report.xml",
            "checkout/.gradle/cache",
        ] {
            write(&root, path, "ignored");
        }
        let report = temp.path().join("selection.json");
        success(&[
            "select",
            "--workspace",
            root.to_str().unwrap(),
            "--base",
            "HEAD",
            "--output",
            report.to_str().unwrap(),
        ]);
        assert_eq!(decision(&root)["mode"], "NONE");
        write(&root, "selection.json", fs::read(report).unwrap());
        assert_eq!(decision(&root)["mode"], "ALL");
    }
}

#[test]
fn committed_moves_and_isolated_deletions() {
    // GIT-03/04: no second edit can accidentally select the deleted file's owner.
    for path in [
        "checkout/src/main/java/example/LegacyReceipt.java",
        "pricing/src/test/java/example/TaxRulesTest.java",
        "runtime/src/main/resources/application.properties",
    ] {
        let temp = fixture("maven");
        let root = temp.path().join("project");
        let module = path.split('/').next().unwrap();
        let expected = if module == "pricing" {
            json!(["checkout", "pricing"])
        } else {
            json!([module])
        };
        fs::remove_file(root.join(path)).unwrap();
        assert_eq!(decision(&root)["modules"], expected);
        let base = git(root.to_str().unwrap(), &["rev-parse", "HEAD"]);
        commit(&root, "delete only");
        assert_eq!(select(root.to_str().unwrap(), &base)["modules"], expected);
    }
    for destination in [
        "pricing/src/main/java/example/Renamed.java",
        "runtime/src/main/java/example/UnusedDiscount.java",
    ] {
        let temp = fixture("maven");
        let root = temp.path().join("project");
        let base = git(root.to_str().unwrap(), &["rev-parse", "HEAD"]);
        git(
            root.to_str().unwrap(),
            &[
                "mv",
                "pricing/src/main/java/example/UnusedDiscount.java",
                destination,
            ],
        );
        let expected = if destination.starts_with("pricing") {
            json!(["checkout", "pricing"])
        } else {
            json!(["checkout", "pricing", "runtime"])
        };
        assert_eq!(decision(&root)["modules"], expected);
        commit(&root, "move source");
        assert_eq!(select(root.to_str().unwrap(), &base)["modules"], expected);
    }
}

#[test]
fn real_merge_checkout_shallow_history_and_unrelated_roots() {
    // GIT-05/06 and CI-01: reproduce the default PR merge checkout.
    let temp = fixture("maven");
    let root = temp.path().join("project");
    let root_s = root.to_str().unwrap();
    git(root_s, &["checkout", "-qb", "feature"]);
    write(&root, "checkout/src/main/resources/change", "feature");
    commit(&root, "feature");
    git(root_s, &["checkout", "-q", "--detach", "HEAD~1"]);
    write(&root, "runtime/src/main/resources/change", "base");
    let base = commit(&root, "base advanced");
    git(root_s, &["merge", "--no-ff", "-m", "PR merge", "feature"]);
    assert_eq!(select(root_s, &base)["modules"], json!(["checkout"]));
    let shallow = temp.path().join("shallow");
    git(
        temp.path().to_str().unwrap(),
        &[
            "clone",
            "-q",
            "--depth=1",
            &format!("file://{}", root.display()),
            shallow.to_str().unwrap(),
        ],
    );
    assert_eq!(select(shallow.to_str().unwrap(), &base)["mode"], "ALL");
    git(root_s, &["checkout", "--orphan", "unrelated"]);
    commit(&root, "different root");
    assert_eq!(select(root_s, &base)["mode"], "ALL");
    assert_eq!(select(root_s, "--help")["mode"], "ALL");
}

#[test]
fn workspace_below_repository_and_paths_with_spaces() {
    // GIT-09/10: changing a sibling must not disappear from the diff.
    let temp = fixture("maven");
    let root = temp.path().join("project");
    fs::remove_dir_all(root.join(".git")).unwrap();
    let nested = temp.path().join("nested project ü");
    fs::rename(&root, &nested).unwrap();
    git(temp.path().to_str().unwrap(), &["init", "-q"]);
    commit(temp.path(), "repository");
    assert_eq!(decision(&nested)["mode"], "NONE");
    write(temp.path(), "docs/review.md", "doc");
    assert_eq!(decision(&nested)["mode"], "NONE");
    write(temp.path(), "sibling/src/input", "shared");
    let selected = decision(&nested);
    assert_eq!(selected["mode"], "ALL");
    assert!(selected["reason"]
        .as_str()
        .unwrap()
        .contains("@repository/sibling"));
}

#[cfg(unix)]
#[test]
fn unavailable_git_and_non_utf8_names_fall_back() {
    let temp = fixture("maven");
    let root = temp.path().join("project");
    let empty = temp.path().join("empty");
    fs::create_dir(&empty).unwrap();
    let output = Command::new(BIN)
        .args([
            "select",
            "--workspace",
            root.to_str().unwrap(),
            "--base",
            "HEAD",
        ])
        .env("PATH", &empty)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["mode"],
        "ALL"
    );
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStringExt;
        fs::write(
            root.join("pricing/src")
                .join(std::ffi::OsString::from_vec(vec![b'x', 255])),
            "bad name",
        )
        .unwrap();
        assert_eq!(decision(&root)["mode"], "ALL");
    }
}

#[test]
fn invalid_configuration_and_cli_usage_fail_before_execution() {
    // BUILD-04 / RUN-06: invalid configuration must never become a green NONE.
    let temp = fixture("maven");
    let root = temp.path().join("project");
    for config in [
        json!({"tool":"unknown","modules":{"pricing":[]}}),
        json!({"tool":"maven","modules":{}}),
        json!({"tool":"maven","modules":{"missing":[]}}),
        json!({"tool":"maven","modules":{"pricing":["missing"]}}),
        json!({"tool":"maven","modules":{"../project":[]}}),
        json!({"tool":"maven","modules":{"pricing":[]},"typo":true}),
    ] {
        write(&root, "impact.json", serde_json::to_vec(&config).unwrap());
        let output = cli(&[
            "select",
            "--workspace",
            root.to_str().unwrap(),
            "--base",
            "missing",
        ]);
        assert_eq!(output.status.code(), Some(2), "{config}");
        assert!(!output.stderr.is_empty());
    }
    write(&root, "impact.json", b"{");
    assert_eq!(
        cli(&["select", "--workspace", root.to_str().unwrap(), "--full"])
            .status
            .code(),
        Some(2)
    );
    for args in [
        vec!["run"],
        vec!["select", "--workspace"],
        vec!["run", "--bad", "value"],
        vec!["run", "--full", "--base", "HEAD"],
    ] {
        assert_eq!(cli(&args).status.code(), Some(2), "{args:?}");
    }
}

#[cfg(unix)]
#[test]
fn wrappers_arguments_failure_and_output_order() {
    // BUILD-03/10, RUN-01/02/06/07. The executable records actual argv, not a reconstructed string.
    let temp = fixture("maven");
    let root = temp.path().join("project");
    let script = "#!/bin/sh\nprintf '%s\\n' \"$@\" > argv.txt\nexit 0\n";
    executable(&root.join("mvnw"), script);
    // Wrapper is a build input, so this intentionally uses full execution.
    for flags in [vec!["--full"], vec![]] {
        let mut args = vec!["run", "--workspace", root.to_str().unwrap()];
        args.extend(flags);
        args.extend(["--", "-Pci", "-Dmessage=a b;$literal", "package"]);
        success(&args);
        assert_eq!(
            fs::read_to_string(root.join("argv.txt")).unwrap(),
            "-B\n-ntp\nclean\nverify\n-Pci\n-Dmessage=a b;$literal\npackage\n"
        );
    }
    fs::remove_file(root.join("argv.txt")).unwrap();
    success(&["select", "--workspace", root.to_str().unwrap(), "--full"]);
    assert!(!root.join("argv.txt").exists());
    let output = cli(&[
        "run",
        "--workspace",
        root.to_str().unwrap(),
        "--full",
        "--output",
        root.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(!root.join("argv.txt").exists());
    let override_path = temp.path().join("override");
    for body in ["#!/bin/sh\nexit 17\n", "#!/bin/sh\nkill -TERM $$\n"] {
        executable(&override_path, body);
        let output = cli(&[
            "run",
            "--workspace",
            root.to_str().unwrap(),
            "--full",
            "--executable",
            override_path.to_str().unwrap(),
        ]);
        assert_eq!(output.status.code(), Some(1));
    }
    fs::remove_file(override_path).unwrap();
    assert_eq!(
        cli(&[
            "run",
            "--workspace",
            root.to_str().unwrap(),
            "--full",
            "--executable",
            "/nonexistent/sieve-build"
        ])
        .status
        .code(),
        Some(2)
    );
}

#[test]
fn committed_build_change_keeps_falling_back_until_refresh() {
    // DEP-07: the unsafe case is the *next* source-only commit, not the build-file diff itself.
    for tool in ["maven", "gradle"] {
        let temp = fixture(tool);
        let root = temp.path().join("project");
        let build = if tool == "maven" {
            "pom.xml"
        } else {
            "build.gradle"
        };
        let contents = fs::read_to_string(root.join(build)).unwrap();
        write(&root, build, format!("{contents}\n"));
        commit(&root, "build graph changed without refreshing");
        write(
            &root,
            "pricing/src/main/resources/next-change",
            "next commit",
        );
        let actual = decision(&root);
        assert_eq!(actual["mode"], "ALL");
        assert!(actual["reason"].as_str().unwrap().contains("refresh"));
        let mut config: Value =
            serde_json::from_slice(&fs::read(root.join("impact.json")).unwrap()).unwrap();
        config.as_object_mut().unwrap().remove("build_fingerprint");
        write(&root, "impact.json", serde_json::to_vec(&config).unwrap());
        commit(&root, "legacy graph");
        assert_eq!(decision(&root)["mode"], "ALL");
    }
}

#[test]
fn class_level_selection_single_module_workspace() {
    // GIT-CT: end-to-end git integration for class-level (SUBSET) selection.
    // Uses `fixtures prepare --tool single-maven` so impact.json carries the
    // correct build_fingerprint for the workspace.
    let temp = fixture("single-maven");
    let root = temp.path().join("project");
    let root_str = root.to_str().unwrap();

    // GIT-CT-01: changing a covered source file produces SUBSET with its test.
    write(
        &root,
        "src/main/java/example/Calculator.java",
        b"class Calculator{ int v=1; }".as_ref(),
    );
    let s = decision(&root);
    assert_eq!(s["mode"], "SUBSET", "covered source → SUBSET: {s}");
    assert_eq!(s["tests"], json!([".:unit:example.CalculatorTest"]), "{s}");
    git(
        root_str,
        &["checkout", "--", "src/main/java/example/Calculator.java"],
    );

    // GIT-CT-02: changing an unused source file (empty test list) produces NONE.
    write(
        &root,
        "src/main/java/example/Unused.java",
        b"class Unused{ int v=2; }".as_ref(),
    );
    let s = decision(&root);
    assert_eq!(s["mode"], "NONE", "empty test list → NONE: {s}");
    assert_eq!(
        s["modules"],
        json!(["."]),
        "NONE keeps the module for compilation: {s}"
    );
    git(
        root_str,
        &["checkout", "--", "src/main/java/example/Unused.java"],
    );

    // GIT-CT-03: changing a file absent from class_tests falls back to MODULES.
    write(
        &root,
        "src/main/java/example/New.java",
        b"class New{}".as_ref(),
    );
    let s = decision(&root);
    assert_eq!(s["mode"], "MODULES", "absent from map → MODULES: {s}");
    let _ = fs::remove_file(root.join("src/main/java/example/New.java"));

    // GIT-CT-04: changing a build input (pom.xml) still produces ALL.
    write(
        &root,
        "pom.xml",
        b"<project><!-- changed --></project>".as_ref(),
    );
    let s = decision(&root);
    assert_eq!(s["mode"], "ALL", "build input → ALL: {s}");
    git(root_str, &["checkout", "--", "pom.xml"]);

    // GIT-CT-05: changing a covered test file produces SUBSET with its own test.
    write(
        &root,
        "src/test/java/example/CalculatorTest.java",
        b"class CalculatorTest{ int v=1; }".as_ref(),
    );
    let s = decision(&root);
    assert_eq!(s["mode"], "SUBSET", "changed test file → SUBSET: {s}");
    assert_eq!(s["tests"], json!([".:unit:example.CalculatorTest"]), "{s}");
}

#[test]
fn class_level_selection_single_module_gradle() {
    // GIT-CT-G: mirrors class_level_selection_single_module_workspace for the
    // Gradle path, exercising the impact.tests property in the init script.
    let temp = fixture("single-gradle");
    let root = temp.path().join("project");
    let root_str = root.to_str().unwrap();

    // Covered source → SUBSET.
    write(
        &root,
        "src/main/java/example/Calculator.java",
        b"class Calculator{ int v=1; }".as_ref(),
    );
    let s = decision(&root);
    assert_eq!(s["mode"], "SUBSET", "covered source → SUBSET: {s}");
    assert_eq!(s["tests"], json!([".:unit:example.CalculatorTest"]), "{s}");
    git(
        root_str,
        &["checkout", "--", "src/main/java/example/Calculator.java"],
    );

    // Unused source (empty test list) → NONE.
    write(
        &root,
        "src/main/java/example/Unused.java",
        b"class Unused{ int v=2; }".as_ref(),
    );
    let s = decision(&root);
    assert_eq!(s["mode"], "NONE", "empty test list → NONE: {s}");
    assert_eq!(
        s["modules"],
        json!(["."]),
        "NONE keeps the module for compilation: {s}"
    );
    git(
        root_str,
        &["checkout", "--", "src/main/java/example/Unused.java"],
    );

    // File absent from class_tests → MODULES fallback.
    write(
        &root,
        "src/main/java/example/New.java",
        b"class New{}".as_ref(),
    );
    let s = decision(&root);
    assert_eq!(s["mode"], "MODULES", "absent from map → MODULES: {s}");
    let _ = fs::remove_file(root.join("src/main/java/example/New.java"));

    // Build input change → ALL.
    write(&root, "build.gradle", b"// changed".as_ref());
    let s = decision(&root);
    assert_eq!(s["mode"], "ALL", "build input → ALL: {s}");
    git(root_str, &["checkout", "--", "build.gradle"]);
}
