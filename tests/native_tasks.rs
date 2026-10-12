mod support;
use serde_json::{json, Value};
use std::{fs, path::Path, process::Command};
use support::*;

fn project(root: &Path, tool: &str, policy: bool) {
    write(root, ".gitignore", "**/target/\n**/build/\n**/.gradle/\n");
    write(
        root,
        "impact.json",
        serde_json::to_vec(&json!({
            "tool": tool, "modules": {".": []}, "class_level": true,
            "must_run": if policy {vec!["example.Smoke*"]} else {vec![]},
        }))
        .unwrap(),
    );
    if tool == "maven" {
        write(
            root,
            "pom.xml",
            r#"<project><modelVersion>4.0.0</modelVersion><groupId>example</groupId><artifactId>tasks</artifactId><version>1</version>
<properties><maven.compiler.release>17</maven.compiler.release><impact.skip.root>false</impact.skip.root></properties>
<dependencies><dependency><groupId>org.junit.jupiter</groupId><artifactId>junit-jupiter</artifactId><version>5.11.4</version><scope>test</scope></dependency></dependencies>
<build><plugins>
<plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-compiler-plugin</artifactId><version>3.13.0</version></plugin>
<plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-surefire-plugin</artifactId><version>3.5.2</version><configuration><skipTests>${impact.skip.root}</skipTests></configuration></plugin>
<plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-failsafe-plugin</artifactId><version>3.5.2</version><configuration><skipTests>${impact.skip.root}</skipTests></configuration><executions><execution><goals><goal>integration-test</goal><goal>verify</goal></goals></execution></executions></plugin>
<plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-jar-plugin</artifactId><version>3.4.2</version></plugin>
</plugins></build></project>"#,
        );
    } else {
        write(root, "settings.gradle", "rootProject.name = 'tasks'\n");
        write(
            root,
            "build.gradle",
            r#"plugins { id 'java' }
repositories { mavenCentral() }
dependencies {
    testImplementation 'org.junit.jupiter:junit-jupiter:5.11.4'
    testRuntimeOnly 'org.junit.platform:junit-platform-launcher:1.11.4'
}
tasks.withType(Test).configureEach { useJUnitPlatform() }
test { exclude '**/*IT.class' }
tasks.register('integrationTest', Test) {
    testClassesDirs = sourceSets.test.output.classesDirs
    classpath = sourceSets.test.runtimeClasspath
    include '**/*IT.class'
}
// A consumer report task still runs for NONE and observes only this invocation's XML.
tasks.register('auditReport') {
    dependsOn 'check', 'integrationTest', 'jar'
    doLast {
        file('build/audit.txt').text = fileTree('build/test-results').matching { include '**/TEST-*.xml' }.files.collect { it.name }.sort().join('\n')
    }
}
"#,
        );
    }
    write(
        root,
        "src/main/java/example/Price.java",
        "package example; public class Price { public static int value() { return 1; } }\n",
    );
    write(
        root,
        "src/main/java/example/Unused.java",
        "package example; public class Unused { int value = 1; }\n",
    );
    for name in ["PriceTest", "PriceIT", "SmokeTest"] {
        let assertion = if name == "SmokeTest" {
            "assertTrue(true);"
        } else {
            "assertEquals(1, Price.value());"
        };
        write(root, &format!("src/test/java/example/{name}.java"), format!("package example; import org.junit.jupiter.api.Test; import static org.junit.jupiter.api.Assertions.*; public class {name} {{ @Test void verifies() {{ {assertion} }} }}\n"));
    }
    let executable = std::env::var(format!("IMPACT_{}", tool.to_uppercase()))
        .unwrap_or_else(|_| if tool == "maven" { "mvn" } else { "gradle" }.into());
    checked(&cli(&[
        "refresh",
        "--workspace",
        root.to_str().unwrap(),
        "--executable",
        &executable,
    ]));
    git(root.to_str().unwrap(), &["init", "-q"]);
    commit(root, "baseline");
}

fn run(root: &Path, tool: &str, full: bool) -> (std::process::Output, Value) {
    let tool_exe = std::env::var(format!("IMPACT_{}", tool.to_uppercase()))
        .unwrap_or_else(|_| if tool == "maven" { "mvn" } else { "gradle" }.into());
    let report = root
        .parent()
        .unwrap()
        .join(format!("{tool}-selection.json"));
    let mut command = Command::new(BIN);
    command.args([
        "run",
        "--workspace",
        root.to_str().unwrap(),
        "--executable",
        &tool_exe,
        "--task",
        if tool == "maven" {
            "verify"
        } else {
            "auditReport"
        },
        "--output",
        report.to_str().unwrap(),
    ]);
    if full {
        command.arg("--full");
    } else {
        command.args(["--base", "HEAD"]);
    }
    let result = command.output().unwrap();
    let selection = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
    (result, selection)
}
fn checked(result: &std::process::Output) {
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
fn executed(root: &Path, tool: &str) -> Value {
    success(&[
        "fixtures",
        "reports",
        "--workspace",
        root.to_str().unwrap(),
        "--tool",
        tool,
    ])["executed"]
        .clone()
}

#[test]
#[ignore = "requires Java 17+, Maven and Gradle"]
fn native_explicit_tasks_preserve_artifacts_and_report_aggregation_on_none() {
    for tool in ["maven", "gradle"].into_iter().filter(|tool| {
        std::env::var("IMPACT_TOOL")
            .map_or(true, |requested| requested == "both" || requested == *tool)
    }) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        project(&root, tool, false);
        let (result, _) = run(&root, tool, true);
        checked(&result);
        assert_eq!(executed(&root, tool).as_array().unwrap().len(), 3);
        let (result, selection) = run(&root, tool, false);
        checked(&result);
        assert_eq!(selection["mode"], "NONE");
        assert!(executed(&root, tool).as_array().unwrap().is_empty());
        assert!(root
            .join(if tool == "maven" {
                "target/tasks-1.jar"
            } else {
                "build/libs/tasks.jar"
            })
            .is_file());
        if tool == "gradle" {
            assert_eq!(
                fs::read_to_string(root.join("build/audit.txt")).unwrap(),
                ""
            );
        }
    }
}

#[test]
#[ignore = "requires Java 17+, Maven and Gradle"]
fn native_static_must_run_is_additive_for_none_and_relevant_faults() {
    for tool in ["maven", "gradle"].into_iter().filter(|tool| {
        std::env::var("IMPACT_TOOL")
            .map_or(true, |requested| requested == "both" || requested == *tool)
    }) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        project(&root, tool, true);
        let (result, selection) = run(&root, tool, false);
        checked(&result);
        assert_eq!(
            selection["tests"],
            json!(["example.SmokeTest"]),
            "{tool}: {selection}"
        );
        assert_eq!(executed(&root, tool), json!([".:unit:example.SmokeTest"]));
        let executable = std::env::var(format!("IMPACT_{}", tool.to_uppercase()))
            .unwrap_or_else(|_| if tool == "maven" { "mvn" } else { "gradle" }.into());
        let mut command = Command::new(BIN);
        command.args([
            "run",
            "--workspace",
            root.to_str().unwrap(),
            "--base",
            "HEAD",
            "--executable",
            &executable,
            "--task",
            "test",
            "--",
        ]);
        if tool == "maven" {
            command.arg("-Dtest=example.PriceTest");
        } else {
            command.args(["--tests", "example.PriceTest"]);
        }
        checked(&command.output().unwrap());
        assert_eq!(executed(&root, tool), json!([".:unit:example.PriceTest"]));
        write(
            &root,
            "src/main/java/example/Price.java",
            "package example; public class Price { public static int value() { return 2; } }\n",
        );
        let (result, selection) = run(&root, tool, false);
        assert!(
            !result.status.success(),
            "{tool}: planted failure must fail build"
        );
        assert!(
            selection["tests"]
                .as_array()
                .unwrap()
                .contains(&json!("example.SmokeTest")),
            "{selection}"
        );
        assert!(
            selection["tests"]
                .as_array()
                .unwrap()
                .contains(&json!("example.PriceTest")),
            "{selection}"
        );
    }
}
