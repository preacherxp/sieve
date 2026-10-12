//! Engine compatibility is checked against an independent native build and planted failures.
//! Run with Maven and JDK 17+: cargo test --test compatibility_modes -- --ignored --test-threads=1
mod support;

use serde_json::Value;
use std::{collections::BTreeSet, fs, path::Path, process::Command};
use support::*;

enum Engine {
    Junit6,
    Spock,
    Kotest,
    TestNg { platform: bool },
}

impl Engine {
    fn dependencies(&self) -> &'static str {
        match self {
            Self::Junit6 => "<dependency><groupId>org.junit.jupiter</groupId><artifactId>junit-jupiter</artifactId><version>6.0.3</version><scope>test</scope></dependency>",
            Self::Spock => "<dependency><groupId>org.spockframework</groupId><artifactId>spock-core</artifactId><version>2.4-groovy-4.0</version><scope>test</scope></dependency>",
            Self::Kotest => "<dependency><groupId>io.kotest</groupId><artifactId>kotest-runner-junit5-jvm</artifactId><version>6.0.4</version><scope>test</scope></dependency>",
            Self::TestNg { .. } => "<dependency><groupId>org.testng</groupId><artifactId>testng</artifactId><version>7.11.0</version><scope>test</scope></dependency>",
        }
    }

    fn compiler(&self) -> &'static str {
        match self {
            Self::Spock => "<plugin><groupId>org.codehaus.gmavenplus</groupId><artifactId>gmavenplus-plugin</artifactId><version>4.2.1</version><executions><execution><goals><goal>compileTests</goal></goals></execution></executions><configuration><targetBytecode>17</targetBytecode></configuration></plugin>",
            Self::Kotest => "<plugin><groupId>org.jetbrains.kotlin</groupId><artifactId>kotlin-maven-plugin</artifactId><version>2.2.21</version><executions><execution><id>kotlin-tests</id><phase>test-compile</phase><goals><goal>test-compile</goal></goals><configuration><jvmTarget>17</jvmTarget><sourceDirs><sourceDir>src/test/kotlin</sourceDir></sourceDirs></configuration></execution></executions></plugin>",
            _ => "",
        }
    }

    fn sources(&self, root: &Path) {
        for (test, production) in [("ServiceTest", "Service"), ("PeerTest", "Peer")] {
            let (language, extension, source) = match self {
                Self::Junit6 => ("java", "java", format!(r#"package example;
import org.junit.jupiter.api.*;
class {test} {{
    @Nested class Cases {{ @Test void nested() {{ Assertions.assertEquals(1, {production}.value()); }} }}
    @TestFactory java.util.stream.Stream<DynamicNode> dynamic() {{
        var source = java.net.URI.create("class:example.{test}");
        return java.util.stream.Stream.of(DynamicContainer.dynamicContainer("group", source, java.util.stream.Stream.of(
            DynamicTest.dynamicTest("value", source, () -> Assertions.assertEquals(1, {production}.value())))));
    }}
}}
"#)),
                Self::Spock => ("groovy", "groovy", format!("package example\nclass {test} extends spock.lang.Specification {{\n def 'checks production value'() {{ expect: {production}.value() == 1 }}\n}}\n")),
                Self::Kotest => ("kotlin", "kt", format!("package example\nclass {test} : io.kotest.core.spec.style.StringSpec({{\n \"checks production value\" {{ check({production}.value() == 1) }}\n}})\n")),
                Self::TestNg { .. } => ("java", "java", format!("package example;\npublic class {test} {{ @org.testng.annotations.Test public void checks() {{ org.testng.Assert.assertEquals({production}.value(), 1); }} }}\n")),
            };
            write(
                root,
                &format!("src/test/{language}/example/{test}.{extension}"),
                source,
            );
        }
    }
}

fn native(root: &Path, maven: &str) -> std::process::Output {
    let mut command = Command::new(maven);
    command
        .current_dir(root)
        .args(["-q", "-B", "-ntp", "clean", "verify"]);
    if std::env::var_os("IMPACT_OFFLINE").is_some() {
        command.arg("-o");
    }
    command.output().unwrap()
}

fn selected(root: &Path, maven: &str, records: bool) -> (bool, Value) {
    let output_path = root.parent().unwrap().join("selection.json");
    let mut command = Command::new(BIN);
    command
        .args(["run", "--workspace"])
        .arg(root)
        .args(["--executable", maven, "--output"])
        .arg(&output_path)
        .env("SIEVE_CACHE_DIR", root.parent().unwrap().join("cache"));
    if records {
        command.arg("--records");
    } else {
        command.args(["--base", "HEAD"]);
    }
    command.args(["--", "-q"]);
    if std::env::var_os("IMPACT_OFFLINE").is_some() {
        command.arg("-o");
    }
    let output = command.output().unwrap();
    let selection: Value = serde_json::from_slice(&fs::read(&output_path).unwrap_or_default())
        .unwrap_or_else(|_| {
            panic!(
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
    (output.status.success(), selection)
}

fn assert_service_failed(root: &Path) {
    use quick_xml::events::Event;
    let mut failed = BTreeSet::new();
    for file in fs::read_dir(root.join("target/surefire-reports")).unwrap() {
        let path = file.unwrap().path();
        if !path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("TEST-")
            || path.extension().is_none_or(|extension| extension != "xml")
        {
            continue;
        }
        let xml = fs::read_to_string(path).unwrap();
        let mut reader = quick_xml::Reader::from_str(&xml);
        let mut class = None;
        loop {
            match reader.read_event().unwrap() {
                Event::Start(tag) if tag.name().as_ref() == b"testcase" => {
                    class = tag
                        .attributes()
                        .map(Result::unwrap)
                        .find(|attribute| attribute.key.as_ref() == b"classname")
                        .map(|attribute| String::from_utf8(attribute.value.into_owned()).unwrap());
                }
                Event::Start(tag) | Event::Empty(tag)
                    if matches!(tag.name().as_ref(), b"failure" | b"error") =>
                {
                    let class = class.as_ref().expect("failure outside a test case");
                    failed.insert(class.split('$').next().unwrap().to_owned());
                }
                Event::End(tag) if tag.name().as_ref() == b"testcase" => class = None,
                Event::Eof => break,
                _ => {}
            }
        }
    }
    assert_eq!(failed, BTreeSet::from(["example.ServiceTest".to_owned()]));
}

fn check(engine: Engine) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let maven = std::env::var("IMPACT_MAVEN").unwrap_or_else(|_| "mvn".into());
    let inactive = matches!(engine, Engine::TestNg { platform: false });
    let surefire = if inactive { "3.5.2" } else { "3.6.0" };
    let junit = if matches!(engine, Engine::Junit6) {
        "6.0.3"
    } else {
        "5.13.4"
    };
    write(
        &root,
        "pom.xml",
        format!(
            r#"<project xmlns="http://maven.apache.org/POM/4.0.0">
<modelVersion>4.0.0</modelVersion><groupId>example</groupId><artifactId>engine-compatibility</artifactId><version>1</version>
<properties><maven.compiler.release>17</maven.compiler.release><project.build.sourceEncoding>UTF-8</project.build.sourceEncoding></properties>
<dependencyManagement><dependencies><dependency><groupId>org.junit</groupId><artifactId>junit-bom</artifactId><version>{junit}</version><type>pom</type><scope>import</scope></dependency></dependencies></dependencyManagement>
<dependencies>{}</dependencies>
<build><plugins><plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-compiler-plugin</artifactId><version>3.13.0</version></plugin>{}
<plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-surefire-plugin</artifactId><version>{surefire}</version></plugin>
</plugins></build></project>
"#,
            engine.dependencies(),
            engine.compiler()
        ),
    );
    write(
        &root,
        "impact.json",
        r#"{"tool":"maven","modules":{".":[]},"class_level":true}"#,
    );
    write(&root, ".gitignore", "target/\n.sieve/\n");
    for name in ["Service", "Peer", "Unused"] {
        write(&root, &format!("src/main/java/example/{name}.java"), format!("package example;\npublic class {name} {{ public static int value() {{ return 1; }} }}\n"));
    }
    engine.sources(&root);
    git(root.to_str().unwrap(), &["init", "-q"]);
    let baseline = native(&root, &maven);
    assert!(
        baseline.status.success(),
        "native baseline: {}\n{}",
        String::from_utf8_lossy(&baseline.stdout),
        String::from_utf8_lossy(&baseline.stderr)
    );
    let mut refresh = Command::new(BIN);
    refresh
        .args(["refresh", "--workspace"])
        .arg(&root)
        .args(["--executable", &maven, "--", "-q"]);
    if std::env::var_os("IMPACT_OFFLINE").is_some() {
        refresh.arg("-o");
    }
    let refresh = refresh.output().unwrap();
    assert!(
        refresh.status.success(),
        "{}",
        String::from_utf8_lossy(&refresh.stderr)
    );
    git(root.to_str().unwrap(), &["add", "."]);
    git(root.to_str().unwrap(), &["commit", "-qm", "green baseline"]);
    let (passed, prime) = selected(&root, &maven, true);
    assert!(passed, "prime: {prime:#}");
    assert_eq!(
        prime["tests"],
        serde_json::json!(["example.PeerTest", "example.ServiceTest"]),
        "{prime:#}"
    );
    if inactive {
        assert!(
            prime["reason"]
                .as_str()
                .unwrap()
                .contains("without Sieve execution-listener events"),
            "{prime:#}"
        );
    }
    write(
        &root,
        "src/main/java/example/Unused.java",
        "package example;\npublic class Unused { public static int value() { return 2; } }\n",
    );
    let (passed, unrelated) = selected(&root, &maven, true);
    assert!(passed, "unrelated: {unrelated:#}");
    if inactive {
        assert_eq!(unrelated["tests"], prime["tests"], "{unrelated:#}");
    } else {
        assert_eq!(unrelated["mode"], "NONE", "{unrelated:#}");
        assert_eq!(unrelated["skipped"], prime["tests"], "{unrelated:#}");
    }
    write(
        &root,
        "src/main/java/example/Service.java",
        "package example;\npublic class Service { public static int value() { return 2; } }\n",
    );
    let (passed, fault) = selected(&root, &maven, true);
    assert!(!passed, "missed planted fault: {fault:#}");
    assert_service_failed(&root);
    if !inactive {
        assert_eq!(
            fault["tests"],
            serde_json::json!(["example.ServiceTest"]),
            "{fault:#}"
        );
        assert_eq!(
            fault["skipped"],
            serde_json::json!(["example.PeerTest"]),
            "{fault:#}"
        );
    }
    let (passed, static_fault) = selected(&root, &maven, false);
    assert!(
        !passed,
        "static mode missed planted fault: {static_fault:#}"
    );
    assert_service_failed(&root);
    assert_eq!(
        static_fault["tests"],
        serde_json::json!(["example.ServiceTest"]),
        "{static_fault:#}"
    );
    let full_fault = native(&root, &maven);
    assert!(
        !full_fault.status.success(),
        "native oracle did not see planted fault"
    );
    assert_service_failed(&root);
}

#[test]
#[ignore = "requires Java 17+, Maven and JUnit 6.0.3 dependencies"]
fn junit6_nested_dynamic_records_and_classes() {
    check(Engine::Junit6);
}

#[test]
#[ignore = "requires Java 17+, Maven and Spock 2.4/Groovy 4 dependencies"]
fn spock_records_and_classes() {
    check(Engine::Spock);
}

#[test]
#[ignore = "requires Java 17+, Maven and Kotest 6.0.4/Kotlin 2.2.21 dependencies"]
fn kotest_records_and_classes() {
    check(Engine::Kotest);
}

#[test]
#[ignore = "requires Java 17+, Maven and TestNG 7.11/Surefire 3.6 dependencies"]
fn testng_platform_records_and_classes() {
    check(Engine::TestNg { platform: true });
}

#[test]
#[ignore = "requires Java 17+, Maven and TestNG 7.11/Surefire 3.5 dependencies"]
fn testng_legacy_fails_open_and_classes_select() {
    check(Engine::TestNg { platform: false });
}
