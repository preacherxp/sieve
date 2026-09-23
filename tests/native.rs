mod support;
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use support::*;

fn tools() -> Vec<(&'static str, String)> {
    let requested = std::env::var("IMPACT_TOOL").unwrap_or_else(|_| "both".into());
    assert!(["both", "maven", "gradle"].contains(&requested.as_str()));
    ["maven", "gradle"]
        .into_iter()
        .filter(|tool| requested == "both" || requested == *tool)
        .map(|tool| {
            (
                tool,
                std::env::var(format!("IMPACT_{}", tool.to_uppercase()))
                    .unwrap_or_else(|_| if tool == "maven" { "mvn" } else { "gradle" }.into()),
            )
        })
        .collect()
}

fn checked(output: Output) {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn reports(root: &Path, tool: &str) -> Value {
    success(&[
        "fixtures",
        "reports",
        "--tool",
        tool,
        "--workspace",
        root.to_str().unwrap(),
    ])
}

fn run(root: &Path, executable: &str, full: bool, extra: &[&str]) -> Output {
    let mut args = vec![
        "run",
        "--workspace",
        root.to_str().unwrap(),
        "--executable",
        executable,
    ];
    args.extend(if full {
        vec!["--full"]
    } else {
        vec!["--base", "HEAD"]
    });
    args.push("--");
    args.extend(extra);
    cli(&args)
}

fn graph(root: &Path, tool: &str) {
    let modules = [
        "provider",
        "left",
        "right",
        "testkit",
        "consumer",
        "app",
        "unrelated",
    ];
    write(
        root,
        ".gitignore",
        "**/target/\n**/build/\n**/.gradle/\n**/.kotlin/\n",
    );
    if tool == "maven" {
        write(
            root,
            "pom.xml",
            format!(
                r#"<project><modelVersion>4.0.0</modelVersion><groupId>example</groupId><artifactId>graph</artifactId><version>1</version><packaging>pom</packaging>
<properties><maven.compiler.release>17</maven.compiler.release><edge.group>example</edge.group></properties>
<modules>{}</modules><dependencies><dependency><groupId>org.junit.jupiter</groupId><artifactId>junit-jupiter</artifactId><version>5.11.4</version><scope>test</scope></dependency></dependencies>
<build><plugins>
<plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-compiler-plugin</artifactId><version>3.13.0</version></plugin>
<plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-surefire-plugin</artifactId><version>3.5.2</version><configuration><skipTests>false</skipTests></configuration></plugin>
<plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-failsafe-plugin</artifactId><version>3.5.2</version><executions><execution><goals><goal>integration-test</goal><goal>verify</goal></goals></execution></executions></plugin>
</plugins></build></project>"#,
                modules
                    .iter()
                    .map(|m| format!("<module>{m}</module>"))
                    .collect::<String>()
            ),
        );
        for module in modules {
            let edges: &[(&str, &str)] = match module {
                "left" | "right" => &[("provider", "runtime")],
                "consumer" => &[
                    ("left", "compile"),
                    ("right", "compile"),
                    ("testkit", "test"),
                ],
                "app" => &[("consumer", "compile")],
                _ => &[],
            };
            let dependencies = edges.iter().map(|(dep, scope)| format!("<dependency><groupId>${{edge.group}}</groupId><artifactId>{dep}</artifactId><version>1</version><scope>{scope}</scope>{}</dependency>", if *dep == "testkit" { "<type>test-jar</type><classifier>tests</classifier>" } else { "" })).collect::<String>();
            let dependencies = if matches!(module, "left" | "right") {
                format!("<profiles><profile><id>runtime-edge</id><activation><activeByDefault>true</activeByDefault></activation><dependencies>{dependencies}</dependencies></profile></profiles>")
            } else {
                format!("<dependencies>{dependencies}</dependencies>")
            };
            let build = if module == "testkit" {
                "<build><plugins><plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-jar-plugin</artifactId><version>3.4.2</version><executions><execution><goals><goal>test-jar</goal></goals></execution></executions></plugin></plugins></build>"
            } else {
                ""
            };
            write(root, &format!("{module}/pom.xml"), format!("<project><modelVersion>4.0.0</modelVersion><parent><groupId>example</groupId><artifactId>graph</artifactId><version>1</version></parent><artifactId>{module}</artifactId>{dependencies}{build}</project>"));
        }
    } else {
        write(root, "settings.gradle", "rootProject.name = 'graph'\ninclude 'provider', 'left', 'right', 'testkit', 'consumer', 'app', 'unrelated'\n");
        write(
            root,
            "build.gradle",
            r#"
plugins { id 'java' }
allprojects {
    apply plugin: 'java'
    group = 'example'; version = '1'
    repositories { mavenCentral() }
    dependencies {
        testImplementation 'org.junit.jupiter:junit-jupiter:5.11.4'
        testRuntimeOnly 'org.junit.platform:junit-platform-launcher:1.11.4'
    }
    tasks.withType(Test).configureEach {
        useJUnitPlatform()
        ignoreFailures = providers.gradleProperty('fixtureIgnoreFailures').map { it.toBoolean() }.getOrElse(false)
    }
    test { exclude '**/*IT.class' }
}
project(':left') { dependencies { runtimeOnly project(':provider') } }
project(':right') { dependencies { runtimeOnly project(':provider') } }
project(':testkit') { apply plugin: 'java-test-fixtures' }
project(':consumer') {
    dependencies {
        implementation project(':left'); implementation project(':right')
        testImplementation testFixtures(project(':testkit'))
    }
    tasks.register('smokeTest', Test) {
        testClassesDirs = sourceSets.test.output.classesDirs
        classpath = sourceSets.test.runtimeClasspath
        include '**/*IT.class'
    }
    check.dependsOn smokeTest
}
project(':app') { dependencies { implementation project(':consumer') } }
dependencies { implementation project(':app') }
check.dependsOn subprojects.collect { it.tasks.named('check') }
"#,
        );
        write(root, "src/test/java/example/RootTest.java", "package example; import org.junit.jupiter.api.Test; import static org.junit.jupiter.api.Assertions.*; class RootTest { @Test void works() throws Exception { assertEquals(2, App.value()); } }");
    }
    for (module, class, body, expected) in [
        ("provider", "Provider", "return 1;", 1),
        ("left", "Left", "return (Integer) Class.forName(\"example.Provider\").getMethod(\"value\").invoke(null);", 1),
        ("right", "Right", "java.util.Properties p = new java.util.Properties(); try (var in = Right.class.getResourceAsStream(\"/provider.properties\")) { p.load(in); } return Integer.parseInt(p.getProperty(\"value\"));", 1),
        ("consumer", "Consumer", "return Left.value() + Right.value();", 2),
        ("app", "App", "return Consumer.value();", 2),
        ("testkit", "Testkit", "return 1;", 1),
        ("unrelated", "Unrelated", "return 1;", 1),
    ] {
        write(root, &format!("{module}/src/main/java/example/{class}.java"), format!("package example; public class {class} {{ public static int value() throws Exception {{ {body} }} }}"));
        let extra = if module == "consumer" || module == "testkit" { "assertEquals(1, FixtureValue.value());" } else { "" };
        write(root, &format!("{module}/src/test/java/example/{class}Test.java"), format!("package example; import org.junit.jupiter.api.Test; import static org.junit.jupiter.api.Assertions.*; class {class}Test {{ @Test void works() throws Exception {{ assertEquals({expected}, {class}.value()); {extra} }} }}"));
    }
    write(
        root,
        "provider/src/main/resources/provider.properties",
        "value=1\n",
    );
    let fixtures = if tool == "maven" {
        "test"
    } else {
        "testFixtures"
    };
    write(
        root,
        &format!("testkit/src/{fixtures}/java/example/FixtureValue.java"),
        "package example; public class FixtureValue { public static int value() { return 1; } }",
    );
    write(root, "consumer/src/test/java/example/ConsumerIT.java", "package example; import org.junit.jupiter.api.Test; import static org.junit.jupiter.api.Assertions.*; class ConsumerIT { @Test void integrates() throws Exception { assertEquals(2, Consumer.value()); } }");
}

#[test]
#[ignore = "requires Java 17 and Maven/Gradle; run by fixture CI"]
fn native_graph_runtime_resources_test_artifacts_and_failures() {
    // DEP-02..08, BUILD-01/02/06..08, RUN-01..03: use the real model and real reports.
    for (tool, executable) in tools() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        graph(root, tool);
        success(&[
            "init",
            "--workspace",
            root.to_str().unwrap(),
            "--executable",
            &executable,
        ]);
        let config: Value =
            serde_json::from_slice(&fs::read(root.join("impact.json")).unwrap()).unwrap();
        for (module, deps) in [
            ("provider", json!([])),
            ("left", json!(["provider"])),
            ("right", json!(["provider"])),
            ("consumer", json!(["left", "right", "testkit"])),
            ("app", json!(["consumer"])),
        ] {
            assert_eq!(config["modules"][module], deps, "{tool}: {config}");
        }
        if tool == "gradle" {
            assert_eq!(config["modules"]["."], json!(["app"]));
        }
        git(root.to_str().unwrap(), &["init", "-q"]);
        commit(root, "installed graph");
        let native_args = if tool == "maven" {
            vec!["-B", "-ntp", "clean", "verify"]
        } else {
            vec!["--no-daemon", "--console=plain", "clean", "check"]
        };
        checked(
            Command::new(&executable)
                .current_dir(root)
                .args(native_args)
                .output()
                .unwrap(),
        );
        assert_eq!(
            reports(root, tool)["cases"],
            if tool == "maven" { 8 } else { 9 }
        );
        let ignore = if tool == "maven" {
            "-Dmaven.test.failure.ignore=true"
        } else {
            "-PfixtureIgnoreFailures=true"
        };
        for (path, before, after, affected) in [
            (
                "provider/src/main/java/example/Provider.java".to_owned(),
                "return 1;",
                "return 2;",
                vec!["app", "consumer", "left", "provider", "right"],
            ),
            (
                "provider/src/main/resources/provider.properties".to_owned(),
                "value=1",
                "value=2",
                vec!["app", "consumer", "left", "provider", "right"],
            ),
            (
                format!(
                    "testkit/src/{}/java/example/FixtureValue.java",
                    if tool == "maven" {
                        "test"
                    } else {
                        "testFixtures"
                    }
                ),
                "return 1;",
                "return 2;",
                vec!["app", "consumer", "testkit"],
            ),
        ] {
            let original = fs::read_to_string(root.join(&path)).unwrap();
            write(root, &path, original.replace(before, after));
            let mut affected = affected;
            if tool == "gradle" {
                affected.insert(0, ".");
            }
            assert_eq!(
                select(root.to_str().unwrap(), "HEAD")["modules"],
                json!(affected)
            );
            assert!(
                !run(root, &executable, false, &[]).status.success(),
                "{tool}: must propagate assertion failure"
            );
            checked(run(root, &executable, false, &[ignore]));
            let actual = reports(root, tool);
            assert!(!actual["failed"].as_array().unwrap().is_empty(), "{actual}");
            assert!(actual["errors"].as_array().unwrap().is_empty(), "{actual}");
            let executed: std::collections::BTreeSet<_> = actual["executed"]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| id.as_str().unwrap().split(':').next().unwrap())
                .collect();
            assert_eq!(executed, affected.into_iter().collect(), "{actual}");
            write(root, &path, original);
        }
        // The consumer needs the excluded testkit's test JAR / Gradle test fixtures.
        let consumer = "consumer/src/main/java/example/Consumer.java";
        let original = fs::read_to_string(root.join(consumer)).unwrap();
        write(
            root,
            consumer,
            format!("{original}\n// only the consumer changed\n"),
        );
        checked(run(root, &executable, false, &[]));
        let actual = reports(root, tool);
        assert_eq!(actual["cases"], if tool == "maven" { 3 } else { 4 });
        if tool == "maven" {
            assert!(root.join("testkit/target/testkit-1-tests.jar").exists());
        }
        write(root, consumer, &original);
        write(root, "README.md", "docs");
        assert_eq!(select(root.to_str().unwrap(), "HEAD")["mode"], "NONE");
        checked(run(root, &executable, false, &[]));
        assert_eq!(
            reports(root, tool)["cases"],
            0,
            "stale reports survived clean"
        );
        fs::remove_file(root.join("README.md")).unwrap();
        write(root, consumer, "not valid Java");
        assert!(!run(root, &executable, false, &[]).status.success());
        write(root, consumer, original);
        // A committed change to the native graph cannot silently reuse the old graph.
        let build = if tool == "maven" {
            "unrelated/pom.xml"
        } else {
            "build.gradle"
        };
        let original = fs::read_to_string(root.join(build)).unwrap();
        let changed = if tool == "maven" {
            original.replace("<dependencies>", "<dependencies><dependency><groupId>example</groupId><artifactId>provider</artifactId><version>1</version><scope>runtime</scope></dependency>")
        } else {
            format!("{original}\nproject(':unrelated') {{ dependencies {{ runtimeOnly project(':provider') }} }}\n")
        };
        write(root, build, changed);
        commit(root, "new runtime edge without refreshed graph");
        write(root, "provider/src/main/resources/next", "change");
        assert_eq!(select(root.to_str().unwrap(), "HEAD")["mode"], "ALL");
        success(&[
            "refresh",
            "--workspace",
            root.to_str().unwrap(),
            "--executable",
            &executable,
        ]);
        let updated: Value =
            serde_json::from_slice(&fs::read(root.join("impact.json")).unwrap()).unwrap();
        assert_eq!(updated["modules"]["unrelated"], json!(["provider"]));
        commit(root, "refresh graph");
        write(root, "provider/src/main/resources/next", "another change");
        assert!(select(root.to_str().unwrap(), "HEAD")["modules"]
            .as_array()
            .unwrap()
            .contains(&json!("unrelated")));
        checked(run(root, &executable, true, &[]));
        // Offline dependency failure and native cycle rejection must never pass.
        let root_build = if tool == "maven" {
            "pom.xml"
        } else {
            "build.gradle"
        };
        let original = fs::read_to_string(root.join(root_build)).unwrap();
        let broken = if tool == "maven" {
            original.replacen("<dependencies>", "<dependencies><dependency><groupId>invalid.sieve.fixture</groupId><artifactId>absent</artifactId><version>0</version></dependency>", 1)
        } else {
            format!("{original}\nallprojects {{ dependencies {{ implementation 'invalid.sieve.fixture:absent:0' }} }}")
        };
        write(root, root_build, broken);
        assert!(!run(root, &executable, true, &["--offline"])
            .status
            .success());
        write(root, root_build, original);
        let cycle_build = if tool == "maven" {
            "provider/pom.xml"
        } else {
            "build.gradle"
        };
        let original = fs::read_to_string(root.join(cycle_build)).unwrap();
        let cyclic = if tool == "maven" {
            original.replace("<dependencies>", "<dependencies><dependency><groupId>example</groupId><artifactId>app</artifactId><version>1</version></dependency>")
        } else {
            format!("{original}\nproject(':provider') {{ dependencies {{ implementation project(':app') }} }}\nproject(':left') {{ dependencies {{ implementation project(':provider') }} }}")
        };
        write(root, cycle_build, cyclic);
        let output = run(root, &executable, true, &[]);
        assert!(
            !output.status.success(),
            "{tool}: native compile cycle unexpectedly succeeded: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        write(root, cycle_build, original);
        let missing_jdk = Command::new(BIN)
            .args([
                "run",
                "--workspace",
                root.to_str().unwrap(),
                "--full",
                "--executable",
                &executable,
            ])
            .env("JAVA_HOME", root.join("missing-jdk"))
            .output()
            .unwrap();
        assert!(!missing_jdk.status.success());
    }
}

#[test]
#[ignore = "requires Docker, Java 17 and Maven; run by container CI"]
fn selected_containers_start_only_for_affected_modules() {
    let service = std::env::var("IMPACT_SERVICE").unwrap_or_else(|_| "Redis".into());
    let (variable, image) = match service.as_str() {
        "Kafka" => ("kafka", "apache/kafka:3.9.2"),
        "Redis" => ("redis", "redis:7.2.16-alpine"),
        "MongoDB" => ("mongo", "mongo:7.0.43"),
        "PostgreSQL" => ("postgres", "postgres:17-alpine"),
        "MySQL" => ("mysql", "mysql:8.4"),
        "RabbitMQ" => ("rabbit", "rabbitmq:4.1-alpine"),
        _ => panic!("Unsupported container service: {service}"),
    };
    let executable = std::env::var("IMPACT_MAVEN").unwrap_or_else(|_| "mvn".into());
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let pom = fs::read_to_string(Path::new(ROOT).join("projects/containers/pom.xml")).unwrap();
    write(root, "pom.xml", pom.replace("<artifactId>container-fixtures</artifactId>", "<artifactId>container-fixtures</artifactId><packaging>pom</packaging><modules><module>provider</module><module>service</module><module>independent</module></modules>"));
    for module in ["provider", "service", "independent"] {
        let dependency = if module == "service" {
            "<dependencies><dependency><groupId>example</groupId><artifactId>provider</artifactId><version>1.0-SNAPSHOT</version></dependency></dependencies>"
        } else {
            ""
        };
        write(root, &format!("{module}/pom.xml"), format!("<project><modelVersion>4.0.0</modelVersion><parent><groupId>example</groupId><artifactId>container-fixtures</artifactId><version>1.0-SNAPSHOT</version></parent><artifactId>{module}</artifactId>{dependency}</project>"));
    }
    write(root, ".gitignore", "**/target/\n");
    let provider = "provider/src/main/java/example/Provider.java";
    let code = "package example; public class Provider { public static int value() { return 1; } }";
    write(root, provider, code);
    write(root, "independent/src/test/java/example/IndependentTest.java", "package example; class IndependentTest { @org.junit.jupiter.api.Test void works() { org.junit.jupiter.api.Assertions.assertTrue(true); } }");
    let original = fs::read_to_string(Path::new(ROOT).join(format!(
        "projects/containers/src/test/java/example/{service}Test.java"
    )))
    .unwrap();
    // Record an attempt before startup, plus the real container ID after startup.
    // Both observation files are ignored so they cannot affect selection.
    let marker = temp.path().join("starts");
    let ids = temp.path().join("ids");
    let java_path = |p: &Path| serde_json::to_string(p.to_str().unwrap()).unwrap();
    let instrumented = original.replace(&format!("{variable}.start();"), &format!(r#"
org.junit.jupiter.api.Assertions.assertDoesNotThrow(() -> java.nio.file.Files.writeString(java.nio.file.Path.of({}), "start\n", java.nio.file.StandardOpenOption.CREATE, java.nio.file.StandardOpenOption.APPEND));
{variable}.start();
org.junit.jupiter.api.Assertions.assertDoesNotThrow(() -> java.nio.file.Files.writeString(java.nio.file.Path.of({}), {variable}.getContainerId() + "\n", java.nio.file.StandardOpenOption.CREATE, java.nio.file.StandardOpenOption.APPEND));
org.junit.jupiter.api.Assertions.assertEquals(1, Provider.value());
"#, java_path(&marker), java_path(&ids)));
    assert_ne!(instrumented, original);
    write(
        root,
        &format!("service/src/test/java/example/{service}Test.java"),
        &instrumented,
    );
    // Markers are ignored observations, never build inputs.
    write(root, ".gitignore", "**/target/\n/starts\n/ids\n");
    success(&[
        "init",
        "--workspace",
        root.to_str().unwrap(),
        "--executable",
        &executable,
    ]);
    git(root.to_str().unwrap(), &["init", "-q"]);
    commit(root, "container graph");
    checked(run(root, &executable, true, &[]));
    assert_eq!(reports(root, "maven")["cases"], 2);
    assert_eq!(fs::read_to_string(&marker).unwrap().lines().count(), 1);
    write(root, "independent/src/main/resources/change", "independent");
    assert_eq!(
        select(root.to_str().unwrap(), "HEAD")["modules"],
        json!(["independent"])
    );
    checked(run(root, &executable, false, &[]));
    assert_eq!(reports(root, "maven")["cases"], 1);
    assert_eq!(
        fs::read_to_string(&marker).unwrap().lines().count(),
        1,
        "excluded container attempted startup"
    );
    fs::remove_file(root.join("independent/src/main/resources/change")).unwrap();
    write(root, provider, code.replace("return 1", "return 2"));
    assert_eq!(
        select(root.to_str().unwrap(), "HEAD")["modules"],
        json!(["provider", "service"])
    );
    assert!(!run(root, &executable, false, &[]).status.success());
    let actual = reports(root, "maven");
    assert_eq!(
        actual["failed"],
        json!([format!("service:unit:example.{service}Test")])
    );
    assert_eq!(actual["errors"], json!([]));
    write(root, provider, code);
    write(
        root,
        "service/src/test/resources/service.properties",
        "test=changed",
    );
    assert_eq!(
        select(root.to_str().unwrap(), "HEAD")["modules"],
        json!(["service"])
    );
    checked(run(root, &executable, false, &[]));
    assert_eq!(fs::read_to_string(&marker).unwrap().lines().count(), 3);
    let build = fs::read_to_string(root.join("service/pom.xml")).unwrap();
    write(
        root,
        "service/pom.xml",
        format!("{build}\n<!-- image configuration changed -->\n"),
    );
    assert_eq!(select(root.to_str().unwrap(), "HEAD")["mode"], "ALL");
    write(root, "service/pom.xml", build);
    fs::remove_file(root.join("service/src/test/resources/service.properties")).unwrap();
    let service_test = format!("service/src/test/java/example/{service}Test.java");
    let label = format!("sieve-{}", root.file_name().unwrap().to_string_lossy());
    let missing_image = format!(
        "{}:sieve-nonexistent-fixture-tag",
        image.split_once(':').unwrap().0
    );
    let missing_image_test = instrumented.replace(image, &missing_image);
    assert_ne!(missing_image_test, instrumented);
    let mut broken = vec![("missing image", missing_image_test)];
    // MySQLContainer replaces custom wait strategies during startup.
    if service != "MySQL" {
        broken.push(("startup timeout", instrumented.replace(&format!("{variable}.start();"), &format!("{variable}.withLabel(\"sieve.fixture\", \"{label}\").waitingFor(org.testcontainers.containers.wait.strategy.Wait.forLogMessage(\"sieve-never-ready\\n\", 1)).withStartupTimeout(java.time.Duration.ofSeconds(2)); {variable}.start();"))));
    }
    for (failure, broken) in broken {
        write(root, &service_test, broken);
        let output = run(
            root,
            &executable,
            false,
            &["-Dpull.timeout=10", "-Dpull.pause.timeout=5"],
        );
        assert!(
            !output.status.success(),
            "{failure} returned success: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let actual = reports(root, "maven");
        assert_eq!(
            actual["errors"],
            json!([format!("service:unit:example.{service}Test")]),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let containers = Command::new("docker")
            .args([
                "ps",
                "-q",
                "--filter",
                &format!("label=sieve.fixture={label}"),
            ])
            .output()
            .unwrap();
        assert!(containers.status.success());
        assert!(
            containers.stdout.is_empty(),
            "failed startup left a running container"
        );
    }
    for id in fs::read_to_string(ids).unwrap().lines() {
        let inspected = Command::new("docker")
            .args(["inspect", "--format", "{{.State.Running}}", id])
            .output()
            .unwrap();
        assert!(
            !inspected.status.success()
                || String::from_utf8_lossy(&inspected.stdout).trim() == "false",
            "container survived scoped cleanup: {id}"
        );
    }
}

#[test]
#[ignore = "requires Java 17 and Gradle; run by fixture CI"]
fn unsupported_gradle_layouts_fail_without_configuration() {
    let executable = std::env::var("IMPACT_GRADLE").unwrap_or_else(|_| "gradle".into());
    for (settings, expected) in [
        ("include 'outer:inner'", "direct child modules"),
        (
            "include 'child'\nproject(':child').projectDir = file('elsewhere')",
            "direct child modules",
        ),
        ("includeBuild 'included'", "Composite builds"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(
            root,
            "settings.gradle",
            format!("rootProject.name = 'unsupported'\n{settings}"),
        );
        write(root, "build.gradle", "plugins { id 'java' }");
        for dir in ["outer/inner", "elsewhere", "included"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        write(
            root,
            "included/settings.gradle",
            "rootProject.name = 'included'",
        );
        write(root, "included/build.gradle", "plugins { id 'java' }");
        let result = cli(&[
            "init",
            "--workspace",
            root.to_str().unwrap(),
            "--executable",
            &executable,
        ]);
        assert!(!result.status.success());
        assert!(
            String::from_utf8_lossy(&result.stderr).contains(expected),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!root.join("impact.json").exists());
        assert_eq!(
            fs::read_to_string(root.join("build.gradle")).unwrap(),
            "plugins { id 'java' }"
        );
    }
    // Minimal local plugins exercise the real plugin-ID rejection without an Android SDK.
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "settings.gradle", "rootProject.name = 'unsupported'");
    write(
        root,
        "buildSrc/build.gradle",
        "plugins { id 'java' }; dependencies { implementation gradleApi() }",
    );
    write(root, "buildSrc/src/main/java/Unsupported.java", "import org.gradle.api.*; public class Unsupported implements Plugin<Project> { public void apply(Project p) {} }");
    for plugin in [
        "com.android.application",
        "com.android.library",
        "org.jetbrains.kotlin.multiplatform",
    ] {
        write(
            root,
            &format!("buildSrc/src/main/resources/META-INF/gradle-plugins/{plugin}.properties"),
            "implementation-class=Unsupported",
        );
        write(
            root,
            "build.gradle",
            format!("plugins {{ id 'java'; id '{plugin}' }}"),
        );
        let result = cli(&[
            "init",
            "--workspace",
            root.to_str().unwrap(),
            "--executable",
            &executable,
        ]);
        assert!(!result.status.success());
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("not Android or multiplatform"),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!root.join("impact.json").exists());
    }
}

#[test]
#[ignore = "requires Java 17 and Maven/Gradle; run by fixture CI"]
fn native_java_consumer_of_kotlin_provider() {
    for (tool, executable) in tools() {
        let temp = fixture(tool);
        let root = temp.path().join("project");
        write(&root, "checkout/src/test/java/example/JavaKotlinTest.java", "package example; class JavaKotlinTest { @org.junit.jupiter.api.Test void formats() { org.junit.jupiter.api.Assertions.assertEquals(\"EUR:120\", new KotlinPriceFormatter().format(100)); } }");
        commit(&root, "Java consumer of Kotlin provider");
        checked(run(&root, &executable, true, &[]));
        assert_eq!(reports(&root, tool)["cases"], 18);
        success(&[
            "fixtures",
            "apply",
            "kotlin-source",
            "--workspace",
            root.to_str().unwrap(),
        ]);
        assert_eq!(
            select(root.to_str().unwrap(), "HEAD")["modules"],
            json!(["checkout", "pricing"])
        );
        assert!(!run(&root, &executable, false, &[]).status.success());
        let ignore = if tool == "maven" {
            "-Dmaven.test.failure.ignore=true"
        } else {
            "-PfixtureIgnoreFailures=true"
        };
        checked(run(&root, &executable, false, &[ignore]));
        let actual = reports(&root, tool);
        assert_eq!(actual["cases"], 13);
        assert_eq!(
            actual["failed"],
            json!([
                "checkout:unit:example.JavaKotlinTest",
                "pricing:unit:example.KotlinPriceFormatterTest"
            ])
        );
        assert_eq!(actual["errors"], json!([]));
    }
}

#[test]
#[ignore = "requires Java 17 and Maven/Gradle; run by fixture CI"]
fn native_class_level_selection_runs_reaching_test_classes() {
    for (tool, executable) in tools() {
        let temp = fixture(&format!("single-{tool}"));
        let root = temp.path().join("project");
        let workspace = root.to_str().unwrap();
        let output = temp.path().join("selection.json");
        let integration = if tool == "maven" {
            "integration"
        } else {
            "unit"
        };
        let unit = |names: &[&str]| -> Vec<String> {
            names
                .iter()
                .map(|n| {
                    let suite = if n.ends_with("IT") {
                        integration
                    } else {
                        "unit"
                    };
                    format!(".:{suite}:example.{n}")
                })
                .collect()
        };
        let all = unit(&[
            "CalculatorIT",
            "CalculatorTest",
            "DiscountTest",
            "GreeterTest",
            "LimitsTest",
            "StringUtilsTest",
        ]);
        let cases: [(&str, &str, &str, &str, Vec<String>); 5] = [
            // Direct and transitive callers, across both Maven test plugins.
            (
                "src/main/java/example/Calculator.java",
                "a + b;",
                "b + a;",
                "SUBSET",
                unit(&["CalculatorIT", "CalculatorTest", "DiscountTest"]),
            ),
            (
                "src/main/java/example/StringUtils.java",
                "toUpperCase()",
                "toUpperCase(java.util.Locale.ROOT)",
                "SUBSET",
                unit(&["StringUtilsTest"]),
            ),
            // Reached only through Class.forName and its interface.
            (
                "src/main/java/example/EnglishGreeter.java",
                "\"Hello \"",
                "\"Hello\" + \" \"",
                "SUBSET",
                unit(&["GreeterTest"]),
            ),
            (
                "src/main/java/example/Unused.java",
                "42",
                "43",
                "NONE",
                vec![],
            ),
            // Inlined constants leave no reference, so the module runs.
            (
                "src/main/java/example/Limits.java",
                "public static",
                "/* edited */ public static",
                "MODULES",
                all.clone(),
            ),
        ];
        for (path, from, to, mode, executed) in cases {
            let file = root.join(path);
            let text = fs::read_to_string(&file).unwrap();
            assert!(text.contains(from), "{path}");
            fs::write(&file, text.replacen(from, to, 1)).unwrap();
            let args = [
                "run",
                "--workspace",
                workspace,
                "--executable",
                &executable,
                "--base",
                "HEAD",
                "--output",
                output.to_str().unwrap(),
            ];
            checked(cli(&args));
            let selection: Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
            assert_eq!(selection["mode"], mode, "{tool} {path}: {selection}");
            assert_eq!(
                reports(&root, tool)["executed"],
                json!(executed),
                "{tool} {path}"
            );
            git(workspace, &["checkout", "--", path]);
        }
        // A behavior change fails the selected tests, and the failure propagates.
        let file = root.join("src/main/java/example/Calculator.java");
        let text = fs::read_to_string(&file).unwrap();
        fs::write(&file, text.replacen("a + b;", "a + b + 1;", 1)).unwrap();
        assert!(!run(&root, &executable, false, &[]).status.success());
        // A compile error fails in the compile step.
        fs::write(&file, "package example; class Calculator {").unwrap();
        assert!(!run(&root, &executable, false, &[]).status.success());
    }
}
