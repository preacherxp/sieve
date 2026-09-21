use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

const BIN: &str = env!("CARGO_BIN_EXE_sieve");
const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn cli(args: &[&str]) -> Output {
    Command::new(BIN)
        .current_dir(ROOT)
        .args(args)
        .output()
        .unwrap()
}

fn success(args: &[&str]) -> Value {
    let output = cli(args);
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or(Value::Null)
}

fn git(workspace: &str, args: &[&str]) -> String {
    let output = Command::new("git")
        .args([
            "-C",
            workspace,
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

fn select(workspace: &str, base: &str) -> Value {
    success(&["select", "--workspace", workspace, "--base", base])
}

#[test]
fn every_mutation_is_safe_for_both_builds() {
    let catalog: Value =
        serde_json::from_slice(&fs::read(Path::new(ROOT).join("scenarios.json")).unwrap()).unwrap();
    for tool in ["maven", "gradle"] {
        for scenario in catalog["scenarios"].as_array().unwrap() {
            let temp = tempfile::tempdir().unwrap();
            let workspace = temp.path().join("project");
            let workspace = workspace.to_str().unwrap();
            let name = scenario["id"].as_str().unwrap();
            success(&[
                "fixtures", "prepare", "--tool", tool, "--dest", workspace, "--git",
            ]);
            assert!(
                !cli(&["fixtures", "prepare", "--tool", tool, "--dest", workspace])
                    .status
                    .success()
            );
            success(&["fixtures", "apply", name, "--workspace", workspace]);
            assert!(!cli(&["fixtures", "apply", name, "--workspace", workspace])
                .status
                .success());
            let actual = select(workspace, "HEAD");
            let file = temp.path().join("actual.json");
            fs::write(&file, serde_json::to_vec(&actual).unwrap()).unwrap();
            let result = success(&[
                "fixtures",
                "check-selection",
                name,
                "--actual",
                file.to_str().unwrap(),
            ]);
            assert_eq!(result["ok"], true, "{tool}: {name}: {actual}");
            match name {
                "docs-only" => assert_eq!(actual["mode"], "NONE"),
                "tax-transitive" | "kotlin-source" => {
                    assert_eq!(actual["modules"], json!(["checkout", "pricing"]))
                }
                "reflection" => assert_eq!(actual["modules"], json!(["runtime"])),
                _ => {}
            }
        }
    }
}

#[test]
fn git_changes_and_missing_history_are_handled() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("project");
    let workspace = workspace.to_str().unwrap();
    success(&[
        "fixtures", "prepare", "--tool", "maven", "--dest", workspace, "--git",
    ]);
    let baseline = git(workspace, &["rev-parse", "HEAD"]);
    assert_eq!(select(workspace, "HEAD")["mode"], "NONE");
    assert_eq!(select(workspace, "unknown-revision")["mode"], "ALL");
    success(&[
        "fixtures",
        "apply",
        "tax-transitive",
        "--workspace",
        workspace,
    ]);
    git(workspace, &["add", "."]);
    assert_eq!(
        select(workspace, "HEAD")["modules"],
        json!(["checkout", "pricing"])
    );
    git(workspace, &["commit", "-qm", "Change tax"]);
    assert_eq!(
        select(workspace, &baseline)["modules"],
        json!(["checkout", "pricing"])
    );
    assert_eq!(select(workspace, "HEAD")["mode"], "NONE");
    git(workspace, &["branch", "feature"]);
    git(workspace, &["checkout", "-q", "--detach", &baseline]);
    fs::write(Path::new(workspace).join("README.md"), "Main advanced").unwrap();
    git(workspace, &["add", "."]);
    git(workspace, &["commit", "-qm", "Change docs"]);
    let main = git(workspace, &["rev-parse", "HEAD"]);
    git(workspace, &["checkout", "-q", "feature"]);
    assert_eq!(
        select(workspace, &main)["modules"],
        json!(["checkout", "pricing"])
    );
    fs::rename(
        Path::new(workspace).join("pricing/src/main/java/example/UnusedDiscount.java"),
        Path::new(workspace).join("runtime/src/main/java/example/UnusedDiscount.java"),
    )
    .unwrap();
    assert_eq!(
        select(workspace, "HEAD")["modules"],
        json!(["checkout", "pricing", "runtime"])
    );
    fs::write(Path::new(workspace).join("unknown input.txt"), "changed").unwrap();
    assert_eq!(select(workspace, "HEAD")["mode"], "ALL");
}

#[test]
fn source_and_resource_parity() {
    fn compare(first: &Path, second: &Path) {
        for entry in fs::read_dir(first).unwrap() {
            let entry = entry.unwrap();
            let other = second.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                compare(&entry.path(), &other);
            } else {
                assert_eq!(
                    fs::read(entry.path()).unwrap(),
                    fs::read(&other).unwrap(),
                    "{}",
                    other.display()
                );
            }
        }
    }
    for module in ["pricing", "checkout", "runtime"] {
        let maven = Path::new(ROOT)
            .join("projects/maven")
            .join(module)
            .join("src");
        let gradle = Path::new(ROOT)
            .join("projects/gradle")
            .join(module)
            .join("src");
        compare(&maven, &gradle);
        compare(&gradle, &maven);
    }
}

#[cfg(unix)]
#[test]
fn runner_propagates_failure_and_records_selection_first() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("project");
    let workspace = workspace.to_str().unwrap();
    success(&[
        "fixtures", "prepare", "--tool", "maven", "--dest", workspace,
    ]);
    let build = temp.path().join("failing-build");
    fs::write(&build, "#!/bin/sh\nexit 7\n").unwrap();
    fs::set_permissions(&build, fs::Permissions::from_mode(0o755)).unwrap();
    let output = temp.path().join("selection.json");
    let result = cli(&[
        "run",
        "--workspace",
        workspace,
        "--full",
        "--executable",
        build.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ]);
    assert_eq!(result.status.code(), Some(1));
    let selection: Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    assert_eq!(selection["mode"], "ALL");
}

#[test]
#[ignore = "requires Java 17 and Maven/Gradle; exercised by the fixture CI workflow"]
fn installs_build_adapters() {
    let requested = std::env::var("IMPACT_TOOL").unwrap_or_else(|_| "both".into());
    for tool in ["maven", "gradle"] {
        if requested != "both" && requested != tool {
            continue;
        }
        let executable = std::env::var(format!("IMPACT_{}", tool.to_uppercase()))
            .unwrap_or_else(|_| if tool == "maven" { "mvn" } else { "gradle" }.into());
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("project");
        let workspace = workspace.to_str().unwrap();
        success(&[
            "fixtures", "prepare", "--tool", tool, "--dest", workspace, "--git",
        ]);
        fs::remove_file(Path::new(workspace).join("impact.json")).unwrap();
        if tool == "maven" {
            let pom = Path::new(workspace).join("pom.xml");
            let original = fs::read_to_string(&pom).unwrap();
            fs::write(
                pom,
                original.replace(
                    "<configuration><skipTests>${impact.skip}</skipTests></configuration>",
                    "",
                ),
            )
            .unwrap();
        }
        success(&[
            "init",
            "--workspace",
            workspace,
            "--executable",
            &executable,
        ]);
        let config_path = Path::new(workspace).join("impact.json");
        let installed = fs::read(&config_path).unwrap();
        let config: Value = serde_json::from_slice(&installed).unwrap();
        assert_eq!(config["modules"]["checkout"], json!(["pricing"]));
        assert!(!cli(&[
            "init",
            "--workspace",
            workspace,
            "--executable",
            &executable
        ])
        .status
        .success());
        assert_eq!(fs::read(config_path).unwrap(), installed);
        git(workspace, &["add", "."]);
        git(workspace, &["commit", "-qm", "Install test selection"]);
        success(&[
            "fixtures",
            "apply",
            "tax-transitive",
            "--workspace",
            workspace,
        ]);
        let ignore = if tool == "maven" {
            "-Dmaven.test.failure.ignore=true"
        } else {
            "-PfixtureIgnoreFailures=true"
        };
        success(&[
            "run",
            "--workspace",
            workspace,
            "--base",
            "HEAD",
            "--executable",
            &executable,
            "--",
            ignore,
        ]);
        let reports = success(&[
            "fixtures",
            "reports",
            "--tool",
            tool,
            "--workspace",
            workspace,
        ]);
        assert_eq!(reports["cases"], 9, "{tool}: {reports}");
        assert_eq!(reports["failed"].as_array().unwrap().len(), 5);
        assert!(reports["executed"]
            .as_array()
            .unwrap()
            .iter()
            .all(|id| !id.as_str().unwrap().starts_with("runtime:")));
    }
}

#[test]
#[ignore = "requires Java 17 and Gradle; exercised by the fixture CI workflow"]
fn installs_kotlin_dsl_package() {
    let executable = std::env::var("IMPACT_GRADLE").unwrap_or_else(|_| "gradle".into());
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().to_str().unwrap();
    fs::write(
        temp.path().join("settings.gradle.kts"),
        "rootProject.name = \"kotlin-package\"\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("build.gradle.kts"),
        r#"
plugins { kotlin("jvm") version "2.2.21" }
repositories { mavenCentral() }
kotlin { jvmToolchain(17) }
dependencies {
    testImplementation("org.junit.jupiter:junit-jupiter:5.11.4")
    testRuntimeOnly("org.junit.platform:junit-platform-launcher:1.11.4")
}
tasks.test { useJUnitPlatform() }
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join(".gitignore"),
        ".gradle/\nbuild/\n.kotlin/\n",
    )
    .unwrap();
    let main = temp.path().join("src/main/kotlin/example");
    let tests = temp.path().join("src/test/kotlin/example");
    fs::create_dir_all(&main).unwrap();
    fs::create_dir_all(&tests).unwrap();
    fs::write(
        main.join("Greeting.kt"),
        "package example\nfun greeting() = \"hello\"\n",
    )
    .unwrap();
    fs::write(tests.join("GreetingTest.kt"), "package example\nimport org.junit.jupiter.api.Test\nimport org.junit.jupiter.api.Assertions.assertEquals\nclass GreetingTest { @Test fun greets() { assertEquals(\"hello\", greeting()) } }\n").unwrap();
    success(&[
        "init",
        "--workspace",
        workspace,
        "--executable",
        &executable,
    ]);
    git(workspace, &["init", "-q"]);
    git(workspace, &["add", "."]);
    git(workspace, &["commit", "-qm", "Kotlin baseline"]);
    fs::write(
        main.join("Greeting.kt"),
        "package example\nfun greeting() = \"bye\"\n",
    )
    .unwrap();
    assert_eq!(select(workspace, "HEAD")["modules"], json!(["."]));
    let result = cli(&[
        "run",
        "--workspace",
        workspace,
        "--base",
        "HEAD",
        "--executable",
        &executable,
    ]);
    assert_eq!(result.status.code(), Some(1));
    let report = fs::read_to_string(
        temp.path()
            .join("build/test-results/test/TEST-example.GreetingTest.xml"),
    )
    .unwrap();
    assert!(report.contains("<failure"), "{report}");
}
