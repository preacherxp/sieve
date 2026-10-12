mod support;
use serde_json::{json, Value};
use std::{fs, path::Path, process::Command};
use support::*;

fn convention_project(root: &Path) {
    write(root, ".gitignore", "**/build/\n**/.gradle/\n");
    write(root, "settings.gradle", "pluginManagement { includeBuild 'tools/conventions' }\nrootProject.name = 'consumer'\ninclude 'app'\n");
    write(root, "build.gradle", "plugins { id 'java' }\n");
    write(
        root,
        "app/build.gradle",
        "plugins { id 'example.convention' }\n",
    );
    write(
        root,
        "app/src/main/java/App.java",
        "public class App { int value = 1; }",
    );
    write(
        root,
        "tools/conventions/settings.gradle",
        "rootProject.name = 'conventions'\n",
    );
    write(
        root,
        "tools/conventions/build.gradle",
        r#"plugins { id 'java-gradle-plugin' }
repositories { mavenCentral() }
gradlePlugin { plugins { convention { id = 'example.convention'; implementationClass = 'Convention' } } }
dependencies {
    testImplementation 'org.junit.jupiter:junit-jupiter:5.11.4'
    testRuntimeOnly 'org.junit.platform:junit-platform-launcher:1.11.4'
}
tasks.withType(Test).configureEach {
    useJUnitPlatform()
    systemProperty 'failConventionChecks', providers.gradleProperty('failConventionChecks').getOrElse('false')
}
"#,
    );
    write(root, "tools/conventions/src/main/java/Convention.java", "import org.gradle.api.*; public class Convention implements Plugin<Project> { public void apply(Project p) { p.getPluginManager().apply(\"java\"); } }");
    write(
        root,
        "tools/conventions/src/main/resources/policy.md",
        "plugin resource",
    );
    write(root, "tools/conventions/src/test/java/ConventionTest.java", "import org.junit.jupiter.api.Test; import static org.junit.jupiter.api.Assertions.*; public class ConventionTest { @Test void verifiesConvention() { assertFalse(Boolean.getBoolean(\"failConventionChecks\")); } }");
}

#[test]
#[ignore = "requires Java 17+ and Gradle"]
fn native_gradle_convention_setup_and_conservative_selection() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    convention_project(root);
    let executable = std::env::var("IMPACT_GRADLE").unwrap_or_else(|_| "gradle".into());
    let result = cli(&[
        "init",
        "--workspace",
        root.to_str().unwrap(),
        "--executable",
        &executable,
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let config: Value =
        serde_json::from_slice(&fs::read(root.join("impact.json")).unwrap()).unwrap();
    assert_eq!(config["build_logic"], json!(["tools/conventions"]));
    assert_eq!(config["modules"], json!({".": [], "app": []}));
    git(root.to_str().unwrap(), &["init", "-q"]);
    let base = commit(root, "baseline");
    for path in [
        "tools/conventions/src/main/java/Convention.java",
        "tools/conventions/src/main/resources/policy.md",
        "tools/conventions/gradle.lockfile",
    ] {
        let original = fs::read(root.join(path)).ok();
        write(root, path, "changed plugin input");
        let selection = select(root.to_str().unwrap(), &base);
        assert_eq!(selection["mode"], "ALL", "{path}: {selection}");
        match original {
            Some(bytes) => write(root, path, bytes),
            None => fs::remove_file(root.join(path)).unwrap(),
        }
    }
    write(
        root,
        "app/src/main/java/App.java",
        "public class App { int value = 2; }",
    );
    let selection = select(root.to_str().unwrap(), &base);
    assert_eq!(selection["modules"], json!(["app"]));
    assert_eq!(selection["mode"], "MODULES");
}

#[test]
#[ignore = "requires Java 17+ and Gradle"]
fn native_gradle_plugin_verification_survives_selection_and_configuration_cache() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    convention_project(root);
    let executable = std::env::var("IMPACT_GRADLE").unwrap_or_else(|_| "gradle".into());
    for repeat in 0..2 {
        let result = Command::new(&executable)
            .current_dir(root)
            .args([
                "--console=plain",
                "--configuration-cache",
                "--init-script",
                &format!("{ROOT}/src/gradle.init.gradle"),
                "-Pimpact.modules=app",
                ":app:check",
                ":conventions:check",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(root
            .join("tools/conventions/build/test-results/test/TEST-ConventionTest.xml")
            .is_file());
        if repeat == 1 {
            assert!(String::from_utf8_lossy(&result.stdout).contains("Reusing configuration cache"));
        }
    }
    let setup = cli(&[
        "init",
        "--workspace",
        root.to_str().unwrap(),
        "--executable",
        &executable,
    ]);
    assert!(
        setup.status.success(),
        "{}",
        String::from_utf8_lossy(&setup.stderr)
    );
    git(root.to_str().unwrap(), &["init", "-q"]);
    let base = commit(root, "baseline");
    write(
        root,
        "app/src/main/java/App.java",
        "public class App { int value = 2; }",
    );
    let result = cli(&[
        "run",
        "--workspace",
        root.to_str().unwrap(),
        "--base",
        &base,
        "--executable",
        &executable,
        "--task",
        ":app:check",
        "--task",
        ":conventions:check",
        "--",
        "-PfailConventionChecks=true",
    ]);
    assert!(
        !result.status.success(),
        "included plugin test failure must propagate"
    );
    let report = fs::read_to_string(
        root.join("tools/conventions/build/test-results/test/TEST-ConventionTest.xml"),
    )
    .unwrap();
    assert!(report.contains("<failure"));
}

#[test]
#[ignore = "requires Java 17+ and Gradle"]
fn native_gradle_rejects_application_composites_even_when_named_build_logic() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(
        root,
        "settings.gradle",
        "rootProject.name = 'consumer'\nincludeBuild 'build-logic'\n",
    );
    write(root, "build.gradle", "plugins { id 'java' }");
    write(
        root,
        "build-logic/settings.gradle",
        "rootProject.name = 'build-logic'",
    );
    write(root, "build-logic/build.gradle", "plugins { id 'java' }");
    let executable = std::env::var("IMPACT_GRADLE").unwrap_or_else(|_| "gradle".into());
    let result = cli(&[
        "init",
        "--workspace",
        root.to_str().unwrap(),
        "--executable",
        &executable,
    ]);
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("Composite builds"),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!root.join("impact.json").exists());
}

#[test]
#[ignore = "requires Java 17+ and Gradle"]
fn native_gradle_observation_preserves_plugin_failures_and_discards_stale_reports() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    convention_project(&root);
    let executable = std::env::var("IMPACT_GRADLE").unwrap_or_else(|_| "gradle".into());
    let setup = cli(&[
        "init",
        "--workspace",
        root.to_str().unwrap(),
        "--executable",
        &executable,
    ]);
    assert!(
        setup.status.success(),
        "{}",
        String::from_utf8_lossy(&setup.stderr)
    );
    git(root.to_str().unwrap(), &["init", "-q"]);
    let base = commit(&root, "baseline");
    write(
        &root,
        "app/src/main/java/App.java",
        "public class App { int value = 2; }",
    );
    let output = temp.path().join("observation.json");
    let arguments = [
        "run",
        "--workspace",
        root.to_str().unwrap(),
        "--base",
        &base,
        "--executable",
        &executable,
        "--observe",
        "--output",
        output.to_str().unwrap(),
        "--task",
        ":app:check",
    ];
    let mut failing = arguments.to_vec();
    failing.extend([
        "--task",
        ":conventions:check",
        "--",
        "-PfailConventionChecks=true",
    ]);
    let result = cli(&failing);
    assert!(!result.status.success());
    let selection: Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(
        selection["observation"]["failed_classes"],
        json!(["tools/conventions:unit:ConventionTest"])
    );
    assert_eq!(
        selection["observation"]["missed_failed_classes"],
        json!([]),
        "included plugin checks always retain native execution"
    );
    assert_eq!(selection["observation"]["failure_retention"], 1.0);
    let result = cli(&arguments);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let selection: Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(
        selection["observation"]["failed_classes"],
        json!([]),
        "unrequested plugin tests cannot leave stale failures in this observation"
    );
    assert_eq!(selection["observation"]["executed_classes"], json!([]));
}

#[test]
#[ignore = "requires Java 17+ and Gradle"]
fn native_gradle_selection_intersects_existing_task_includes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    convention_project(root);
    write(
        root,
        "app/build.gradle",
        r#"plugins { id 'example.convention' }
repositories { mavenCentral() }
dependencies {
    testImplementation 'org.junit.jupiter:junit-jupiter:5.11.4'
    testRuntimeOnly 'org.junit.platform:junit-platform-launcher:1.11.4'
}
test { useJUnitPlatform(); filter.includeTestsMatching('AllowedTest') }
"#,
    );
    write(
        root,
        "app/src/test/java/AllowedTest.java",
        "import org.junit.jupiter.api.Test; public class AllowedTest { @Test void allowed() {} }",
    );
    write(root, "app/src/test/java/ForbiddenTest.java", "import org.junit.jupiter.api.Test; import static org.junit.jupiter.api.Assertions.*; public class ForbiddenTest { @Test void forbidden() { assertEquals(1, new App().value); } }");
    let executable = std::env::var("IMPACT_GRADLE").unwrap_or_else(|_| "gradle".into());
    let setup = cli(&[
        "init",
        "--workspace",
        root.to_str().unwrap(),
        "--executable",
        &executable,
    ]);
    assert!(
        setup.status.success(),
        "{}",
        String::from_utf8_lossy(&setup.stderr)
    );
    let mut config: Value =
        serde_json::from_slice(&fs::read(root.join("impact.json")).unwrap()).unwrap();
    config["class_level"] = json!(true);
    write(root, "impact.json", serde_json::to_vec(&config).unwrap());
    git(root.to_str().unwrap(), &["init", "-q"]);
    let base = commit(root, "baseline with a native task filter");
    write(
        root,
        "app/src/main/java/App.java",
        "public class App { int value = 2; }",
    );
    let selection_file = tempfile::NamedTempFile::new().unwrap();
    let result = cli(&[
        "run",
        "--workspace",
        root.to_str().unwrap(),
        "--base",
        &base,
        "--executable",
        &executable,
        "--output",
        selection_file.path().to_str().unwrap(),
        "--task",
        ":app:test",
    ]);
    assert!(result.status.success(), "selection must not enable the deliberately failing ForbiddenTest excluded by the native include filter: {}", String::from_utf8_lossy(&result.stderr));
    assert!(!root
        .join("app/build/test-results/test/TEST-ForbiddenTest.xml")
        .exists());
    let selection: Value =
        serde_json::from_slice(&fs::read(selection_file.path()).unwrap()).unwrap();
    assert_eq!(selection["mode"], "SUBSET");
    assert_eq!(selection["tests"], json!(["ForbiddenTest"]));
}

#[test]
#[ignore = "requires Java 17+ and Gradle"]
fn native_gradle_observation_executes_tests_instead_of_restoring_cached_reports() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let flag = temp.path().join("external-failure");
    write(&root, ".gitignore", "/build/\n/.gradle/\n/.cache/\n");
    write(
        &root,
        "settings.gradle",
        "rootProject.name = 'observed'\nbuildCache { local { directory = file('.cache') } }\n",
    );
    write(
        &root,
        "build.gradle",
        r#"plugins { id 'java' }
repositories { mavenCentral() }
dependencies {
    testImplementation 'org.junit.jupiter:junit-jupiter:5.11.4'
    testRuntimeOnly 'org.junit.platform:junit-platform-launcher:1.11.4'
}
test { useJUnitPlatform() }
"#,
    );
    write(&root, "src/test/java/ExternalStateTest.java", format!("import org.junit.jupiter.api.Test; import static org.junit.jupiter.api.Assertions.*; public class ExternalStateTest {{ @Test void readsExternalState() {{ assertFalse(java.nio.file.Files.exists(java.nio.file.Path.of({}))); }} }}", serde_json::to_string(&flag.to_string_lossy()).unwrap()));
    let executable = std::env::var("IMPACT_GRADLE").unwrap_or_else(|_| "gradle".into());
    let setup = cli(&[
        "init",
        "--workspace",
        root.to_str().unwrap(),
        "--executable",
        &executable,
    ]);
    assert!(
        setup.status.success(),
        "{}",
        String::from_utf8_lossy(&setup.stderr)
    );
    for repeat in 0..2 {
        let result = Command::new(&executable)
            .current_dir(&root)
            .args(["--console=plain", "--build-cache", "clean", "test"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        if repeat == 1 {
            assert!(
                String::from_utf8_lossy(&result.stdout).contains(":test FROM-CACHE"),
                "fixture must first demonstrate native test caching: {}",
                String::from_utf8_lossy(&result.stdout)
            );
        }
    }
    fs::write(flag, "changed outside Gradle's declared inputs").unwrap();
    let output = temp.path().join("observation.json");
    let result = cli(&[
        "run",
        "--workspace",
        root.to_str().unwrap(),
        "--executable",
        &executable,
        "--full",
        "--observe",
        "--output",
        output.to_str().unwrap(),
        "--task",
        "test",
        "--",
        "--build-cache",
    ]);
    assert!(
        !result.status.success(),
        "observation must execute the fresh external failure instead of reusing passing XML"
    );
    let selection: Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    assert_eq!(
        selection["observation"]["failed_classes"],
        json!([".:unit:ExternalStateTest"])
    );
    assert_eq!(selection["observation"]["missed_failed_classes"], json!([]));
}
