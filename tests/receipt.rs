mod support;
use serde_json::Value;
use std::{fs, path::Path, process::Command, process::Output};
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
fn native_gradle_receipt_inventory_tracks_default_and_explicit_test_jvms() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(
        root,
        "settings.gradle",
        "rootProject.name = 'receipt-jvms'\n",
    );
    write(
        root,
        "build.gradle",
        r#"plugins { id 'java' }
def launcher = javaToolchains.launcherFor {
    languageVersion = JavaLanguageVersion.of(JavaVersion.current().majorVersion)
}
tasks.register('legacyExecutable', Test) {
    executable = launcher.get().executablePath.asFile.absolutePath
}
tasks.register('explicitLauncher', Test) {
    javaLauncher = launcher
}
// Ask Gradle's public fork-options API which binary each Test actually uses.
def probe = tasks.create('nativeJvmProbe', JavaExec)
def expected = [:]
tasks.withType(Test).each { task ->
    task.copyTo(probe)
    def suite = task.name == 'test' ? 'unit' : task.name
    expected['.:' + suite] = probe.executable
}
file('expected-jvms.json').text = groovy.json.JsonOutput.toJson(expected)
"#,
    );
    write(root, "remaining.json", "{}\n");
    let executable = std::env::var("IMPACT_GRADLE").unwrap_or_else(|_| "gradle".into());
    let classes = root.join("classes.json");
    for attempt in 0..2 {
        let output = Command::new(&executable)
            .current_dir(root)
            .args([
                "--no-daemon",
                "--console=plain",
                "--offline",
                "--configuration-cache",
                "--init-script",
            ])
            .arg(Path::new(ROOT).join("src/gradle.init.gradle"))
            .arg(format!("-Pimpact.classes={}", classes.display()))
            .arg(format!(
                "-Pimpact.remaining={}",
                root.join("remaining.json").display()
            ))
            .arg("impactClasses")
            .output()
            .unwrap();
        let log = String::from_utf8_lossy(&output.stdout).into_owned();
        checked(output);
        if attempt == 1 {
            assert!(log.contains("Reusing configuration cache."), "{log}");
        }
        let expected: Value =
            serde_json::from_slice(&fs::read(root.join("expected-jvms.json")).unwrap()).unwrap();
        let listing: Value = serde_json::from_slice(&fs::read(&classes).unwrap()).unwrap();
        assert_eq!(expected.as_object().unwrap().len(), 3, "{expected}");
        for (suite, java) in expected.as_object().unwrap() {
            let java = Path::new(java.as_str().unwrap()).canonicalize().unwrap();
            let release = java.parent().unwrap().parent().unwrap().join("release");
            assert!(release.is_file(), "{}", release.display());
            let inputs = listing["suites"][suite]
                .as_array()
                .unwrap_or_else(|| panic!("Missing JVM inventory for {suite}: {listing}"));
            for required in [&java, &release] {
                assert!(
                    inputs.iter().any(|input| {
                        Path::new(input.as_str().unwrap())
                            .canonicalize()
                            .is_ok_and(|path| path == *required)
                    }),
                    "{suite} omitted {}: {listing}",
                    required.display()
                );
            }
        }
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
test {
    useJUnitPlatform()
    // Gradle 7 on macOS forwards a launcher PID marker; keep the fixture's actual environment stable.
    environment = environment.findAll { key, value -> !(key ==~ /JAVA_MAIN_CLASS_\d+/) }
}
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
