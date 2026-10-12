mod support;
use serde_json::Value;
use std::{fs, process::Output};
use support::*;

fn selected_then_remaining(
    root: &std::path::Path,
    tool: &str,
    executable: &str,
    receipt: &std::path::Path,
) {
    let config_path = root.join("impact.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["class_level"] = Value::Bool(true);
    fs::write(config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    commit(root, "enable class selection");
    write(root, "src/test/java/example/PassingTest.java", "package example; public class PassingTest { @org.junit.jupiter.api.Test void passes() { org.junit.jupiter.api.Assertions.assertEquals(2, 1 + 1); } }\n");
    let selected = cli(&[
        "run",
        "--workspace",
        root.to_str().unwrap(),
        "--base",
        "HEAD",
        "--receipt",
        receipt.to_str().unwrap(),
        "--executable",
        executable,
    ]);
    let selected_log = String::from_utf8_lossy(&selected.stderr).into_owned();
    checked(selected);
    let saved: Value = serde_json::from_slice(&fs::read(receipt).unwrap()).unwrap();
    assert_eq!(
        saved["completed"],
        serde_json::json!([".:unit:example.PassingTest"]),
        "{saved}\n{selected_log}"
    );
    checked(cli(&[
        "run",
        "--workspace",
        root.to_str().unwrap(),
        "--remaining",
        receipt.to_str().unwrap(),
        "--executable",
        executable,
    ]));
    let reports = success(&[
        "fixtures",
        "reports",
        "--tool",
        tool,
        "--workspace",
        root.to_str().unwrap(),
    ]);
    assert_eq!(
        reports["executed"],
        serde_json::json!([".:unit:example.NewTest"]),
        "{reports}"
    );
    let observation = receipt.with_file_name("observation.json");
    checked(cli(&[
        "run",
        "--workspace",
        root.to_str().unwrap(),
        "--base",
        "HEAD",
        "--observe",
        "--output",
        observation.to_str().unwrap(),
        "--executable",
        executable,
    ]));
    let report: Value = serde_json::from_slice(&fs::read(observation).unwrap()).unwrap();
    assert_eq!(
        report["observation"]["proposed"]["tests"],
        serde_json::json!(["example.PassingTest"]),
        "{report}"
    );
    assert_eq!(
        report["observation"]["executed_classes"],
        serde_json::json!([".:unit:example.NewTest", ".:unit:example.PassingTest"]),
        "{report}"
    );
    let classes = if tool == "maven" {
        "target/classes"
    } else {
        "build/classes/java/main"
    };
    write(root, "src/test/java/example/PassingTest.java", format!("package example; public class PassingTest {{ @org.junit.jupiter.api.Test void passes() throws Exception {{ java.nio.file.Files.createDirectories(java.nio.file.Paths.get(\"{classes}\")); java.nio.file.Files.writeString(java.nio.file.Paths.get(\"{classes}/changed.txt\"), \"changed during tests\"); }} }}\n"));
    checked(cli(&[
        "run",
        "--workspace",
        root.to_str().unwrap(),
        "--full",
        "--receipt",
        receipt.to_str().unwrap(),
        "--executable",
        executable,
    ]));
    let saved: Value = serde_json::from_slice(&fs::read(receipt).unwrap()).unwrap();
    assert_eq!(
        saved["completed"],
        serde_json::json!([]),
        "changed runtime inputs cannot earn a receipt: {saved}"
    );
}

fn checked(output: Output) {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "requires Java 17 and Maven; run with IMPACT_MAVEN set when Maven is not on PATH"]
fn native_maven_receipt_runs_only_remaining_tests_and_invalidates_on_new_inputs() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    write(&root, ".gitignore", "target/\n");
    write(
        &root,
        "pom.xml",
        r#"<project><modelVersion>4.0.0</modelVersion><groupId>example</groupId><artifactId>receipt</artifactId><version>1</version>
<properties><maven.compiler.release>17</maven.compiler.release></properties>
<dependencies><dependency><groupId>org.junit.jupiter</groupId><artifactId>junit-jupiter</artifactId><version>5.11.4</version><scope>test</scope></dependency></dependencies>
<build><plugins><plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-compiler-plugin</artifactId><version>3.13.0</version></plugin><plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-surefire-plugin</artifactId><version>3.5.2</version></plugin><plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-failsafe-plugin</artifactId><version>3.5.2</version></plugin></plugins></build></project>"#,
    );
    write(&root, "src/test/java/example/PassingTest.java", "package example; public class PassingTest { @org.junit.jupiter.api.Test void passes() {} }\n");
    let executable = std::env::var("IMPACT_MAVEN").unwrap_or_else(|_| "mvn".into());
    checked(cli(&[
        "init",
        "--workspace",
        root.to_str().unwrap(),
        "--executable",
        &executable,
    ]));
    git(root.to_str().unwrap(), &["init", "-q"]);
    commit(&root, "initial");
    let receipt = temp.path().join("receipt.json");
    checked(cli(&[
        "run",
        "--workspace",
        root.to_str().unwrap(),
        "--full",
        "--receipt",
        receipt.to_str().unwrap(),
        "--executable",
        &executable,
    ]));
    let saved: Value = serde_json::from_slice(&fs::read(&receipt).unwrap()).unwrap();
    assert_eq!(
        saved["completed"],
        serde_json::json!([".:unit:example.PassingTest"]),
        "{saved}"
    );
    checked(cli(&[
        "run",
        "--workspace",
        root.to_str().unwrap(),
        "--remaining",
        receipt.to_str().unwrap(),
        "--executable",
        &executable,
    ]));
    let reports = success(&[
        "fixtures",
        "reports",
        "--tool",
        "maven",
        "--workspace",
        root.to_str().unwrap(),
    ]);
    assert_eq!(reports["cases"], 0, "{reports}");
    write(
        &root,
        "src/test/java/example/NewTest.java",
        "package example; public class NewTest { @org.junit.jupiter.api.Test void passes() {} }\n",
    );
    checked(cli(&[
        "run",
        "--workspace",
        root.to_str().unwrap(),
        "--remaining",
        receipt.to_str().unwrap(),
        "--executable",
        &executable,
    ]));
    let reports = success(&[
        "fixtures",
        "reports",
        "--tool",
        "maven",
        "--workspace",
        root.to_str().unwrap(),
    ]);
    assert_eq!(reports["cases"], 2, "{reports}");
    selected_then_remaining(&root, "maven", &executable, &receipt);
}

#[test]
#[ignore = "requires Java 17 and Maven; run with IMPACT_MAVEN set when Maven is not on PATH"]
fn native_maven_remaining_preserves_custom_nested_class_discovery() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    write(
        &root,
        ".gitignore",
        "target/\nstate/\nsrc/test/resources/ignored.txt\n",
    );
    write(
        &root,
        "impact.json",
        r#"{"tool":"maven","modules":{".":[]}}"#,
    );
    write(
        &root,
        "pom.xml",
        r#"<project><modelVersion>4.0.0</modelVersion><groupId>example</groupId><artifactId>receipt</artifactId><version>1</version>
<properties><maven.compiler.release>17</maven.compiler.release></properties>
<dependencies><dependency><groupId>org.junit.jupiter</groupId><artifactId>junit-jupiter</artifactId><version>5.11.4</version><scope>test</scope></dependency></dependencies>
<build><plugins><plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-compiler-plugin</artifactId><version>3.13.0</version></plugin><plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-surefire-plugin</artifactId><version>3.5.2</version><configuration><excludes><exclude>**/Never.class</exclude></excludes></configuration></plugin></plugins></build></project>"#,
    );
    write(&root, "src/test/java/example/PassingTest.java", "package example; public class PassingTest { @org.junit.jupiter.api.Test void passes() {} }\n");
    write(
        &root,
        "src/test/java/example/Dollar$FailureTest.java",
        r#"package example;
public class Dollar$FailureTest {
  @org.junit.jupiter.api.Test void checksState() { org.junit.jupiter.api.Assertions.assertFalse(java.nio.file.Files.exists(java.nio.file.Paths.get("state/fail"))); }
  @org.junit.jupiter.api.Test @org.junit.jupiter.api.Disabled void incomplete() {}
}
"#,
    );
    git(root.to_str().unwrap(), &["init", "-q"]);
    commit(&root, "native dollar class discovery");
    let executable = std::env::var("IMPACT_MAVEN").unwrap_or_else(|_| "mvn".into());
    let receipt = temp.path().join("receipt.json");
    checked(cli(&[
        "run",
        "--workspace",
        root.to_str().unwrap(),
        "--full",
        "--receipt",
        receipt.to_str().unwrap(),
        "--executable",
        &executable,
    ]));
    let saved: Value = serde_json::from_slice(&fs::read(&receipt).unwrap()).unwrap();
    assert_eq!(
        saved["completed"],
        serde_json::json!([".:unit:example.PassingTest"]),
        "{saved}"
    );
    write(
        &root,
        "state/fail",
        "external state changes must still reach uncredited tests",
    );
    for changed_inventory in [false, true] {
        if changed_inventory {
            write(&root, "src/test/resources/ignored.txt", "new runtime input");
        }
        let output = cli(&[
            "run",
            "--workspace",
            root.to_str().unwrap(),
            "--remaining",
            receipt.to_str().unwrap(),
            "--executable",
            &executable,
        ]);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{changed_inventory}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let reports = success(&[
            "fixtures",
            "reports",
            "--tool",
            "maven",
            "--workspace",
            root.to_str().unwrap(),
        ]);
        assert_eq!(
            reports["failed"],
            serde_json::json!([".:unit:example.Dollar$FailureTest"]),
            "{reports}"
        );
    }
}

#[test]
#[ignore = "requires Java 17 and Gradle; run with IMPACT_GRADLE set when Gradle is not on PATH"]
fn native_gradle_receipt_skips_completed_suite_and_retains_new_tests() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    write(&root, ".gitignore", "build/\n.gradle/\n");
    write(&root, "settings.gradle", "rootProject.name = 'receipt'\n");
    write(
        &root,
        "build.gradle",
        r#"plugins { id 'java' }
repositories { mavenCentral() }
dependencies { testImplementation 'org.junit.jupiter:junit-jupiter:5.11.4'; testRuntimeOnly 'org.junit.platform:junit-platform-launcher:1.11.4' }
test { useJUnitPlatform() }
"#,
    );
    write(&root, "src/test/java/example/PassingTest.java", "package example; public class PassingTest { @org.junit.jupiter.api.Test void passes() {} }\n");
    let executable = std::env::var("IMPACT_GRADLE").unwrap_or_else(|_| "gradle".into());
    checked(cli(&[
        "init",
        "--workspace",
        root.to_str().unwrap(),
        "--executable",
        &executable,
    ]));
    git(root.to_str().unwrap(), &["init", "-q"]);
    commit(&root, "initial");
    let receipt = temp.path().join("receipt.json");
    checked(cli(&[
        "run",
        "--workspace",
        root.to_str().unwrap(),
        "--full",
        "--receipt",
        receipt.to_str().unwrap(),
        "--executable",
        &executable,
    ]));
    let saved: Value = serde_json::from_slice(&fs::read(&receipt).unwrap()).unwrap();
    assert_eq!(
        saved["completed"],
        serde_json::json!([".:unit:example.PassingTest"]),
        "{saved}"
    );
    checked(cli(&[
        "run",
        "--workspace",
        root.to_str().unwrap(),
        "--remaining",
        receipt.to_str().unwrap(),
        "--executable",
        &executable,
    ]));
    let reports = success(&[
        "fixtures",
        "reports",
        "--tool",
        "gradle",
        "--workspace",
        root.to_str().unwrap(),
    ]);
    assert_eq!(reports["cases"], 0, "{reports}");
    write(
        &root,
        "src/test/java/example/NewTest.java",
        "package example; public class NewTest { @org.junit.jupiter.api.Test void passes() {} }\n",
    );
    checked(cli(&[
        "run",
        "--workspace",
        root.to_str().unwrap(),
        "--remaining",
        receipt.to_str().unwrap(),
        "--executable",
        &executable,
    ]));
    let reports = success(&[
        "fixtures",
        "reports",
        "--tool",
        "gradle",
        "--workspace",
        root.to_str().unwrap(),
    ]);
    assert_eq!(reports["cases"], 2, "{reports}");
    selected_then_remaining(&root, "gradle", &executable, &receipt);
}
