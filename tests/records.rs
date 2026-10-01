mod support;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use support::*;

/// A copy of `projects/records` in a Git repository, run through a Maven wrapper that counts
/// its starts.
struct Project {
    temp: tempfile::TempDir,
}

impl Project {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        copy(&Path::new(ROOT).join("projects/records"), &root);
        let maven = std::env::var("IMPACT_MAVEN").unwrap_or_else(|_| "mvn".into());
        let wrapper = temp.path().join("mvn-counting");
        executable(
            &wrapper,
            &format!(
                "#!/bin/sh\necho start >> '{}'\nexec '{maven}' \"$@\"\n",
                temp.path().join("starts").display()
            ),
        );
        let project = Self { temp };
        git(root.to_str().unwrap(), &["init", "-q"]);
        commit(&project.root(), "base");
        project
    }

    fn root(&self) -> PathBuf {
        self.temp.path().join("project")
    }

    fn starts(&self) -> usize {
        fs::read_to_string(self.temp.path().join("starts"))
            .map(|s| s.lines().count())
            .unwrap_or(0)
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(BIN);
        command
            .args(args)
            .env("SIEVE_CACHE_DIR", self.temp.path().join("cache"));
        command
    }

    /// Runs `sieve run` and returns the exit code, the selection, and the Maven starts it took.
    fn run(&self, extra: &[&str]) -> (i32, Value, usize) {
        self.run_with(&[], extra)
    }

    fn run_with(&self, options: &[&str], extra: &[&str]) -> (i32, Value, usize) {
        let before = self.starts();
        let selection = self.temp.path().join("selection.json");
        let _ = fs::remove_file(&selection);
        let wrapper = self.temp.path().join("mvn-counting");
        let root = self.root();
        let mut args = vec![
            "run",
            "--workspace",
            root.to_str().unwrap(),
            "--executable",
            wrapper.to_str().unwrap(),
            "--output",
            selection.to_str().unwrap(),
        ];
        args.extend(options);
        args.extend(["--", "-q"]);
        let offline = std::env::var_os("IMPACT_OFFLINE").is_some();
        if offline {
            args.push("-o");
        }
        args.extend(extra);
        let output = self.command(&args).output().unwrap();
        let value = serde_json::from_slice(&fs::read(&selection).unwrap_or_default())
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)));
        (output.status.code().unwrap(), value, self.starts() - before)
    }

    fn edit(&self, path: &str, from: &str, to: &str) {
        let path = self.root().join(path);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains(from), "{from}");
        fs::write(path, text.replacen(from, to, 1)).unwrap();
    }

    /// Asserts which test classes ran (simple names), that the build passed, and that Maven
    /// started once.
    fn expect(&self, label: &str, ran: &[&str]) -> Value {
        let (code, selection, starts) = self.run(&[]);
        assert_eq!(code, 0, "{label}: {selection:#}");
        assert_eq!(starts, 1, "{label}: Maven starts");
        assert_eq!(
            names(&selection["tests"]),
            set(ran),
            "{label}: {selection:#}"
        );
        selection
    }
}

fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if matches!(name.to_str(), Some("target" | ".sieve")) {
            continue;
        }
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &to.join(name));
        } else {
            fs::copy(entry.path(), to.join(name)).unwrap();
        }
    }
}

fn names(tests: &Value) -> BTreeSet<String> {
    tests
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| t.as_str().unwrap().rsplit('.').next().unwrap().to_owned())
        .collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

const ALL: &[&str] = &[
    "AdditionTest",
    "ComposedGreetingTest",
    "DefaultsFirstTest",
    "DefaultsSecondTest",
    "FormatterTest",
    "GreeterTest",
    "GreetingControllerTest",
    "GreetingTest",
    "HierarchyTest",
    "LimitsTest",
    "MultiplicationTest",
    "OrderFlowTest",
    "PriceTest",
    "PropertySourceFirstTest",
    "PropertySourceSecondTest",
    "ShapeTest",
];

/// Full-context tests that load the application's configuration and services.
const BOOT: &[&str] = &[
    "ComposedGreetingTest",
    "GreetingTest",
    "OrderFlowTest",
    "PropertySourceFirstTest",
    "PropertySourceSecondTest",
];

fn with(names: &[&str], more: &[&'static str]) -> Vec<&'static str> {
    let mut all: Vec<&'static str> = names
        .iter()
        .map(|n| *ALL.iter().chain(more).find(|a| *a == n).unwrap())
        .collect();
    all.extend(more);
    all
}

#[test]
#[ignore = "requires a Java 24+ JDK and Maven with the fixture's dependencies"]
fn records_skip_unchanged_tests_and_rerun_what_changed() {
    let p = Project::new();
    let first = p.expect("first run", ALL);
    assert!(first["reason"].as_str().unwrap().contains("clean"));
    let record: Value = serde_json::from_slice(
        &fs::read(p.root().join(".sieve/records/example.OrderFlowTest.json")).unwrap(),
    )
    .unwrap();
    assert!(record["methods"]
        .as_object()
        .unwrap()
        .contains_key("example/OrderListener#on(Lexample/OrderPlaced;)V"));
    assert_eq!(
        fs::read_to_string(p.root().join(".sieve/.gitignore")).unwrap(),
        "*\n"
    );

    // Nothing changed since a passing run: no build at all.
    let (code, selection, starts) = p.run(&[]);
    assert_eq!((code, starts), (0, 0), "{selection:#}");
    assert_eq!(selection["mode"], "NONE");

    // A build without edits runs no tests, and without `clean`.
    let calculator = "src/main/java/example/Calculator.java";
    fs::write(
        p.root().join(calculator),
        fs::read(p.root().join(calculator)).unwrap(),
    )
    .unwrap();
    let selection = p.expect("touched", &[]);
    assert!(selection["reason"]
        .as_str()
        .unwrap()
        .contains("incremental"));

    p.edit(
        calculator,
        "return a + b;",
        "int sum = a + b;\n        return sum;",
    );
    let selection = p.expect("body edit", &["AdditionTest"]);
    assert_eq!(
        selection["reasons"]["example.AdditionTest"],
        "Changed method: example/Calculator#add(II)I"
    );
    assert_eq!(
        selection["reasons"]["example.FormatterTest"],
        "Unchanged test record"
    );
    p.edit(
        calculator,
        "    public int multiply",
        "    // Comments leave bytecode alone.\n    public int multiply",
    );
    p.expect("comment", &[]);

    // A failing test runs until it passes.
    p.edit(calculator, "return sum;", "return sum + 1;");
    let (code, selection, _) = p.run(&[]);
    assert_eq!(
        (code, names(&selection["tests"])),
        (1, set(&["AdditionTest"]))
    );
    p.edit(calculator, "return sum + 1;", "return sum;");
    let selection = p.expect("after failure", &["AdditionTest"]);
    assert_eq!(
        selection["reasons"]["example.AdditionTest"],
        "Failed last time"
    );

    // Explicitly requested tests always run.
    let (code, selection, _) = p.run(&["-Dtest=FormatterTest"]);
    assert_eq!(
        (code, names(&selection["tests"])),
        (0, set(&["FormatterTest"]))
    );
    // Restoring the default invocation reruns records made under different Maven properties.
    p.expect(
        "default invocation after explicit selection",
        &["FormatterTest"],
    );

    // Framework dispatch, context startup, and resources read at startup.
    p.edit(
        "src/main/java/example/OrderListener.java",
        "log.add(\"placed \" + event.id());",
        "String entry = \"placed \" + event.id();\n        log.add(entry);",
    );
    p.expect("listener", &["OrderFlowTest"]);
    p.edit(
        "src/main/java/example/GreetingService.java",
        "this.prefix = prefix;",
        "this.prefix = prefix.trim();",
    );
    p.expect("bean constructor", BOOT);
    p.edit(
        "src/main/resources/application.properties",
        "app.greeting.prefix=Hello",
        "app.greeting.prefix=Hello\napp.unused=1",
    );
    p.expect("configuration", &with(BOOT, &["GreetingControllerTest"]));
    p.edit(
        "src/test/resources/prices.json",
        "\"pear\": 4",
        "\"pear\": 5",
    );
    p.expect("classpath fixture", &["PriceTest"]);
    p.edit("src/test/resources/limits.txt", "max=10", "max=10\n");
    p.expect("fixture read by path", &["LimitsTest"]);

    // Structural changes and wiring.
    p.edit(
        "src/main/java/example/LoudGreeter.java",
        "public class LoudGreeter extends Greeter {",
        "public class LoudGreeter extends Greeter {\n    @Override\n    public String greet(String name) {\n        return super.greet(name);\n    }\n",
    );
    let selection = p.expect("override", &["GreeterTest"]);
    assert_eq!(
        selection["reasons"]["example.GreeterTest"],
        "Structural change: example/LoudGreeter"
    );
    write(
        &p.root(),
        "src/main/java/example/Clock.java",
        "package example;\n\n@org.springframework.stereotype.Component\npublic class Clock {}\n",
    );
    p.expect("added component", &with(BOOT, &["HierarchyTest"]));

    // A resource that was looked up but absent counts once it appears.
    write(
        &p.root(),
        "src/main/resources/application-default.properties",
        "app.greeting.prefix=Hello\n",
    );
    p.expect("new profile file", &with(BOOT, &["GreetingControllerTest"]));
    // A new fixture with the test that reads it runs only that test.
    write(&p.root(), "src/test/resources/extra.json", "{}\n");
    write(
        &p.root(),
        "src/test/java/example/ExtraTest.java",
        "package example;\n\nclass ExtraTest {\n    @org.junit.jupiter.api.Test\n    void reads() throws Exception {\n        try (var in = ExtraTest.class.getResourceAsStream(\"/extra.json\")) {\n            org.junit.jupiter.api.Assertions.assertNotNull(in);\n        }\n    }\n}\n",
    );
    p.expect("new fixture and test", &["ExtraTest"]);
    // An ignored test failure does not count as a pass.
    let formatter = "src/main/java/example/Formatter.java";
    p.edit(formatter, "\"#\" + value", "\"!\" + value");
    let (code, selection, _) = p.run(&["-Dmaven.test.failure.ignore=true"]);
    assert_eq!(code, 0, "Maven's status stays");
    assert!(
        selection["reason"].as_str().unwrap().contains("failed: 1"),
        "{selection:#}"
    );
    p.edit(formatter, "\"!\" + value", "\"#\" + value");
    let selection = p.expect("after an ignored failure", &with(ALL, &["ExtraTest"]));
    assert_eq!(
        selection["reasons"]["example.FormatterTest"],
        "Failed last time"
    );

    // A deleted resource cleans the build output, so the test that needs it fails.
    fs::remove_file(p.root().join("src/test/resources/prices.json")).unwrap();
    let (code, selection, starts) = p.run(&[]);
    assert_eq!((code, starts), (1, 1), "{selection:#}");
    assert!(selection["reason"]
        .as_str()
        .unwrap()
        .contains("clean: Deleted"));
}

#[test]
#[ignore = "requires a Java 24+ JDK and Maven with the fixture's dependencies"]
fn plain_maven_gets_the_same_selection_through_java_options() {
    let p = Project::new();
    p.expect("first run", ALL);
    p.edit(
        "src/main/java/example/Formatter.java",
        "return \"#\" + value;",
        "String text = \"#\" + value;\n        return text;",
    );
    let root = p.root();
    let env = p
        .command(&["env", "--workspace", root.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        env.status.success(),
        "{}",
        String::from_utf8_lossy(&env.stderr)
    );
    let options = String::from_utf8(env.stdout).unwrap();
    let maven = std::env::var("IMPACT_MAVEN").unwrap_or_else(|_| "mvn".into());
    let output = Command::new(maven)
        .current_dir(&root)
        .args(["-B", "test"])
        .args(std::env::var_os("IMPACT_OFFLINE").map(|_| "-o"))
        .env("JDK_JAVA_OPTIONS", options.trim())
        .output()
        .unwrap();
    let log = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{log}");
    assert!(log.contains("Tests run: 1, Failures: 0"), "{log}");
    assert!(log.contains("in example.FormatterTest"), "{log}");
}

#[test]
#[ignore = "requires a Java 24+ JDK and Maven with the fixture's dependencies"]
fn tests_without_records_fall_back_to_static_selection_against_the_base() {
    let p = Project::new();
    p.expect("first run", ALL);
    fs::remove_dir_all(p.root().join(".sieve/records")).unwrap();
    p.edit(
        "src/main/java/example/Calculator.java",
        "return a * b;",
        "int product = a * b;\n        return product;",
    );
    let (code, selection, _) = p.run_with(&["--base", "HEAD"], &[]);
    assert_eq!(code, 0, "{selection:#}");
    // Both tests reach the changed class; neither has a record.
    assert_eq!(
        names(&selection["tests"]),
        set(&["AdditionTest", "MultiplicationTest"])
    );
    assert_eq!(
        selection["reasons"]["example.FormatterTest"],
        "No record; reaches no change since the base"
    );
    // Without a base, tests without a record run.
    fs::remove_dir_all(p.root().join(".sieve/records")).unwrap();
    p.edit(
        "src/main/java/example/Calculator.java",
        "int product",
        "int result",
    );
    p.edit(
        "src/main/java/example/Calculator.java",
        "return product;",
        "return result;",
    );
    p.expect("no base", ALL);
}

#[test]
fn extracted_agent_is_verified_and_replaced() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    write(
        &root,
        "impact.json",
        r#"{"tool":"maven","modules":{".":[]},"records":true}"#,
    );
    write(&root, "pom.xml", "<project/>");
    let env = || {
        Command::new(BIN)
            .args(["env", "--workspace", root.to_str().unwrap()])
            .env("SIEVE_CACHE_DIR", temp.path().join("cache"))
            .output()
            .unwrap()
    };
    let output = env();
    if !output.status.success() {
        // Built without the agent.
        assert!(String::from_utf8_lossy(&output.stderr).contains("without the agent"));
        return;
    }
    let line = String::from_utf8(output.stdout).unwrap();
    let jar = line
        .trim()
        .strip_prefix("-javaagent:")
        .and_then(|l| l.split_once('='))
        .map(|(jar, _)| PathBuf::from(jar))
        .unwrap();
    let original = fs::read(&jar).unwrap();
    fs::write(&jar, b"tampered").unwrap();
    assert!(env().status.success());
    assert_eq!(fs::read(&jar).unwrap(), original);
    assert!(jar.with_file_name("sieve-probe.jar").is_file());
}

#[test]
#[ignore = "requires a Java 24+ JDK and Maven with the sample's dependencies"]
fn local_records_invalidate_properties_environment_and_preserved_metadata_edits() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    copy(
        &Path::new(ROOT).join("samples/selective-performance"),
        &root,
    );
    write(
        &root,
        "impact.json",
        r#"{"tool":"maven","modules":{".":[]},"records":true,"record_env":["SIEVE_SAMPLE_ENV"]}"#,
    );
    let test = root.join("src/test/java/example/PriceTest.java");
    let text = fs::read_to_string(&test).unwrap();
    assert!(text.contains("assertEquals(12, new Price().total(10, 2));"));
    fs::write(&test, text.replace("assertEquals(12, new Price().total(10, 2));", "assertEquals(12, new Price().total(10, 2));\n        org.junit.jupiter.api.Assertions.assertNotEquals(\"unsafe\", System.getProperty(\"sample.flavor\"));\n        org.junit.jupiter.api.Assertions.assertNotEquals(\"unsafe\", System.getenv(\"SIEVE_SAMPLE_ENV\"));")).unwrap();
    git(root.to_str().unwrap(), &["init", "-q"]);
    commit(&root, "base");
    let maven = std::env::var("IMPACT_MAVEN").unwrap_or_else(|_| "mvn".into());
    let run = |flavor: &str, environment: &str, options: &[&str]| {
        let selection = temp.path().join("selection.json");
        let output = Command::new(BIN)
            .args([
                "run",
                "--workspace",
                root.to_str().unwrap(),
                "--executable",
                &maven,
                "--output",
                selection.to_str().unwrap(),
            ])
            .args(options)
            .args([
                "--",
                "-q",
                "-Dsample.delay.ms=0",
                &format!("-Dsample.flavor={flavor}"),
            ])
            .args(std::env::var_os("IMPACT_OFFLINE").map(|_| "-o"))
            .env("SIEVE_SAMPLE_ENV", environment)
            .env("SIEVE_CACHE_DIR", temp.path().join("cache"))
            .output()
            .unwrap();
        let value: Value = serde_json::from_slice(&fs::read(&selection).unwrap_or_default())
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)));
        (output.status.code().unwrap(), value)
    };
    let (code, first) = run("safe", "safe", &[]);
    assert_eq!(code, 0, "{first:#}");
    let (code, unchanged) = run("safe", "safe", &[]);
    assert_eq!(code, 0, "{unchanged:#}");
    assert_eq!(unchanged["mode"], "NONE");
    assert!(unchanged["reason"]
        .as_str()
        .unwrap()
        .contains("no build started"));
    let (code, properties) = run("unsafe", "safe", &[]);
    assert_eq!(code, 1, "{properties:#}");
    assert!(names(&properties["tests"]).contains("PriceTest"));
    assert_eq!(run("safe", "safe", &[]).0, 0);
    fs::remove_dir_all(root.join(".sieve/records")).unwrap();
    let (code, absent_records) = run("unsafe", "safe", &["--base", "HEAD"]);
    assert_eq!(code, 1, "{absent_records:#}");
    assert!(names(&absent_records["tests"]).contains("PriceTest"));
    assert_eq!(run("safe", "safe", &[]).0, 0);
    let (code, environment) = run("safe", "unsafe", &[]);
    assert_eq!(code, 1, "{environment:#}");
    assert!(names(&environment["tests"]).contains("PriceTest"));
    assert_eq!(run("safe", "safe", &[]).0, 0);
    // Old record schemas cannot authorize dropping a test, even when its bytecode matches.
    let record = root.join(".sieve/records/example.PriceTest.json");
    let mut old: Value = serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();
    old.as_object_mut().unwrap().remove("context");
    fs::write(&record, serde_json::to_vec(&old).unwrap()).unwrap();
    fs::write(&test, fs::read(&test).unwrap()).unwrap();
    let (code, migrated) = run("safe", "safe", &[]);
    assert_eq!(code, 0, "{migrated:#}");
    assert!(names(&migrated["tests"]).contains("PriceTest"));
    let source = root.join("src/main/java/example/Price.java");
    let metadata = fs::metadata(&source).unwrap();
    let text = fs::read_to_string(&source).unwrap();
    fs::write(&source, text.replace("price + tax", "price - tax")).unwrap();
    fs::File::options()
        .write(true)
        .open(source)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(metadata.modified().unwrap()))
        .unwrap();
    let (code, content) = run("safe", "safe", &[]);
    assert_eq!(code, 1, "{content:#}");
    assert!(content["reason"]
        .as_str()
        .unwrap()
        .contains("unchanged metadata"));
    assert_eq!(names(&content["tests"]), set(&["PriceTest"]));
    assert_eq!(names(&content["skipped"]), set(&["SlowUnrelatedTest"]));
}

#[test]
#[ignore = "requires Docker, a Java 24+ JDK, and Maven"]
fn container_reuse_keeps_containers_between_runs() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    copy(&Path::new(ROOT).join("projects/records-reuse"), &root);
    let maven = std::env::var("IMPACT_MAVEN").unwrap_or_else(|_| "mvn".into());
    let ids = || -> Vec<String> {
        fs::read_to_string(root.join("target/container-ids.txt"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    };
    let run = |options: &[&str]| {
        let mut command = Command::new(BIN);
        command
            .args([
                "run",
                "--workspace",
                root.to_str().unwrap(),
                "--executable",
                &maven,
            ])
            .args(options)
            .args(["--", "-q", "-Dtest=ReusedContainerTest"])
            .args(std::env::var_os("IMPACT_OFFLINE").map(|_| "-o"))
            .env("SIEVE_CACHE_DIR", temp.path().join("cache"))
            .env_remove("TESTCONTAINERS_REUSE_ENABLE");
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run(&["--with", "reuse"]);
    run(&["--with", "reuse"]);
    run(&["--without", "reuse"]);
    let ids = ids();
    for id in BTreeSet::<&String>::from_iter(&ids) {
        let _ = Command::new("docker").args(["rm", "-f", id]).output();
    }
    assert_eq!(ids.len(), 3, "{ids:?}");
    assert_eq!(ids[0], ids[1], "reuse keeps the container between runs");
    assert_ne!(ids[1], ids[2], "--without reuse starts a new one");
}

#[test]
#[ignore = "requires a Java 24+ JDK and Maven with the fixture's dependencies"]
fn catalog_times_edits_and_planted_bugs_catch_a_broken_selector() {
    let temp = tempfile::tempdir().unwrap();
    let catalog = temp.path().join("catalog.json");
    fs::write(
        &catalog,
        r#"{"edits": [
            {"name": "add", "kind": "body", "file": "src/main/java/example/Calculator.java",
             "search": "        return a + b;", "replace": "        int sum = a + b;\n        return sum;"},
            {"name": "docs", "kind": "docs", "file": "README.md", "content": "notes\n"}
        ]}"#,
    )
    .unwrap();
    let catalog_run = |output: &str, broken: bool| {
        let output = temp.path().join(output);
        let mut command = Command::new(BIN);
        command
            .args(["catalog", "--workspace"])
            .arg(Path::new(ROOT).join("projects/records"))
            .arg("--catalog")
            .arg(&catalog)
            .args(["--samples", "1", "--plant", "--commits", "5", "--output"])
            .arg(&output)
            .arg("--")
            .args(std::env::var_os("IMPACT_OFFLINE").map(|_| "-o"))
            .env("SIEVE_CACHE_DIR", temp.path().join("cache"));
        if broken {
            command.env("SIEVE_BROKEN_SELECTOR", "1");
        }
        let status = command.output().unwrap();
        let report: Value =
            serde_json::from_slice(&fs::read(output.join("catalog.json")).unwrap_or_default())
                .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&status.stderr)));
        (status.status.code().unwrap(), report)
    };
    let (code, report) = catalog_run("good", false);
    assert_eq!(code, 0, "{report:#}");
    let edit = &report["edits"][0];
    assert_eq!(edit["selection"]["mode"], "SUBSET");
    assert_eq!(
        edit["selection"]["tests"],
        serde_json::json!(["example.AdditionTest"])
    );
    for run in edit["result"]["runs"]
        .as_array()
        .unwrap()
        .iter()
        .chain(report["native"]["runs"].as_array().unwrap())
    {
        let phases: f64 = run["phases"]
            .as_object()
            .unwrap()
            .values()
            .map(|v| v.as_f64().unwrap())
            .sum();
        assert!(
            (phases - run["seconds"].as_f64().unwrap()).abs() < 1e-6,
            "{run:#}"
        );
    }
    assert_eq!(report["edits"][1]["selection"]["mode"], "NONE");
    let planted = &report["planted"][0];
    assert!(planted["detected"].as_bool().unwrap(), "{planted:#}");
    // The planted run decides with the records of the normal runs, so it runs fewer tests;
    // selecting everything would make "no missed failure" hold trivially.
    assert!(
        planted["selected_cases"].as_u64().unwrap() < planted["full_cases"].as_u64().unwrap(),
        "{planted:#}"
    );
    assert_eq!(report["missed_failures"], 0);
    // The fixture itself is never edited.
    assert!(!Path::new(ROOT).join("projects/records/README.md").exists());
    let (code, report) = catalog_run("broken", true);
    assert_eq!(code, 1, "{report:#}");
    assert!(report["missed_failures"].as_u64().unwrap() > 0);
}

#[test]
#[ignore = "requires a Java 24+ JDK and Maven with the fixture's dependencies"]
fn commit_walk_carries_records_forward_and_cleans_after_deletions() {
    let p = Project::new();
    let root = p.root();
    p.edit(
        "src/main/java/example/Calculator.java",
        "return a + b;",
        "int sum = a + b;\n        return sum;",
    );
    commit(&root, "body edit");
    fs::remove_file(root.join("src/test/java/example/LimitsTest.java")).unwrap();
    fs::remove_file(root.join("src/test/resources/limits.txt")).unwrap();
    commit(&root, "delete a test and its fixture");
    p.edit(
        "src/test/resources/prices.json",
        "\"pear\": 4",
        "\"pear\": 5",
    );
    commit(&root, "fixture edit");
    write(&root, "README.md", "Notes\n");
    commit(&root, "docs");
    let walk = |broken: bool| {
        let output = p
            .temp
            .path()
            .join(if broken { "broken.json" } else { "walk.json" });
        let mut command = p.command(&[
            "replay",
            "--workspace",
            root.to_str().unwrap(),
            "--walk",
            "--plant",
            "--commits",
            "4",
            "--output",
            output.to_str().unwrap(),
            "--",
        ]);
        command.args(std::env::var_os("IMPACT_OFFLINE").map(|_| "-o"));
        if broken {
            command.env("SIEVE_BROKEN_SELECTOR", "1");
        }
        let status = command.output().unwrap();
        let report: Value = serde_json::from_slice(&fs::read(&output).unwrap_or_default())
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&status.stderr)));
        (status.status.code().unwrap(), report)
    };
    let (code, report) = walk(false);
    assert_eq!(code, 0, "{:#}", report["summary"]);
    let records = report["records"].as_array().unwrap();
    let tests = |i: usize| names(&records[i]["selection"]["tests"]);
    assert_eq!(
        tests(0),
        set(&["AdditionTest"]),
        "{:#}",
        records[0]["selection"]
    );
    assert_eq!(records[0]["clean"], false);
    assert_eq!(records[1]["clean"], true, "{:#}", records[1]["selection"]);
    assert_eq!(
        tests(2),
        set(&["PriceTest"]),
        "{:#}",
        records[2]["selection"]
    );
    assert_eq!(records[3]["selection"]["mode"], "NONE");
    assert_eq!(
        records[0]["planted"]["detected"], true,
        "{:#}",
        records[0]["planted"]
    );
    assert_eq!(report["summary"]["missed_failures"], 0);
    let (code, report) = walk(true);
    assert_eq!(code, 1, "{:#}", report["summary"]);
}

#[test]
#[ignore = "requires a Java 24+ JDK and Maven with the fixture's dependencies"]
fn records_cover_shared_state_annotations_and_context_structure() {
    let p = Project::new();
    p.expect("first run", ALL);
    // Static state read by two tests, initialized by whichever ran first.
    p.edit(
        "src/main/java/example/Defaults.java",
        "List.of(\"a\", \"b\")",
        "List.of(\"a\", \"b\", \"c\")",
    );
    p.expect("static field", &["DefaultsFirstTest", "DefaultsSecondTest"]);
    // A composed annotation configures the tests that carry it.
    let annotation = "src/test/java/example/AppTest.java";
    p.edit(annotation, "prefix=Hello", "prefix=Hi");
    let (code, selection, _) = p.run(&[]);
    assert_eq!(code, 1, "{selection:#}");
    assert_eq!(names(&selection["tests"]), set(&["ComposedGreetingTest"]));
    p.edit(annotation, "prefix=Hi", "prefix=Hello");
    p.expect("annotation restored", &["ComposedGreetingTest"]);
    // The parent of a context hierarchy starts for the test too.
    p.edit(
        "src/test/java/hierarchy/ParentConfig.java",
        "return new StringBuilder(\"parent\").toString();",
        "return String.valueOf(new StringBuilder(\"parent\"));",
    );
    p.expect("hierarchy parent", &["HierarchyTest"]);
    // Test property files count for every test sharing the context.
    p.edit(
        "src/test/resources/greeting-it.properties",
        "app.greeting.prefix=Hello",
        "app.greeting.prefix=Hello\nextra=1",
    );
    p.expect(
        "test property file",
        &["PropertySourceFirstTest", "PropertySourceSecondTest"],
    );
    // Structural changes: a method annotation, a removed method, and a new supertype.
    p.edit(
        "src/main/java/example/Calculator.java",
        "    public int multiply",
        "    @Deprecated\n    public int multiply",
    );
    p.expect("method annotation", &["AdditionTest", "MultiplicationTest"]);
    p.edit(
        "src/main/java/example/LoudGreeter.java",
        "    public String shout(String name) {\n        return greet(name).toUpperCase();\n    }\n",
        "",
    );
    p.expect("removed method", &["GreeterTest"]);
    p.edit(
        "src/main/java/example/Square.java",
        "implements Shape",
        "implements Shape, java.io.Serializable",
    );
    p.expect("supertype", &["ShapeTest"]);
}

#[test]
#[ignore = "requires a Java 24+ JDK and Maven with the fixture's dependencies"]
fn context_attribution_holds_in_reverse_test_class_order() {
    let p = Project::new();
    let reverse = ["-Dsurefire.runOrder=reversealphabetical"];
    let expect = |label: &str, ran: &[&str]| {
        let (code, selection, starts) = p.run(&reverse);
        assert_eq!((code, starts), (0, 1), "{label}: {selection:#}");
        assert_eq!(
            names(&selection["tests"]),
            set(ran),
            "{label}: {selection:#}"
        );
    };
    expect("first run", ALL);
    p.edit(
        "src/main/java/example/OrderListener.java",
        "log.add(\"placed \" + event.id());",
        "String entry = \"placed \" + event.id();\n        log.add(entry);",
    );
    expect("listener", &["OrderFlowTest"]);
    p.edit(
        "src/main/java/example/GreetingService.java",
        "this.prefix = prefix;",
        "this.prefix = prefix.trim();",
    );
    expect("bean constructor", BOOT);
    p.edit(
        "src/test/resources/greeting-it.properties",
        "app.greeting.prefix=Hello",
        "app.greeting.prefix=Hello\nextra=1",
    );
    expect(
        "test property file",
        &["PropertySourceFirstTest", "PropertySourceSecondTest"],
    );
}

#[test]
#[ignore = "requires a Java 24+ JDK and Gradle 8.14+ on PATH (or IMPACT_GRADLE)"]
fn gradle_records_skip_unchanged_tests_on_one_daemon_build() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    copy(&Path::new(ROOT).join("projects/single-gradle"), &root);
    // Tests run on the JDK that runs Gradle, which local mode needs to be Java 24+.
    let build = fs::read_to_string(root.join("build.gradle")).unwrap();
    let build: String = build
        .lines()
        .filter(|l| !l.contains("toolchain"))
        .map(|l| format!("{l}\n"))
        .collect();
    fs::write(root.join("build.gradle"), build).unwrap();
    let mut config: Value =
        serde_json::from_slice(&fs::read(root.join("impact.json")).unwrap()).unwrap();
    config["records"] = Value::Bool(true);
    fs::write(
        root.join("impact.json"),
        serde_json::to_vec_pretty(&config).unwrap(),
    )
    .unwrap();
    git(root.to_str().unwrap(), &["init", "-q"]);
    commit(&root, "base");
    let gradle = std::env::var("IMPACT_GRADLE").unwrap_or_else(|_| "gradle".into());
    let wrapper = temp.path().join("gradle-counting");
    let starts = temp.path().join("starts");
    executable(
        &wrapper,
        &format!(
            "#!/bin/sh\necho start >> '{}'\nexec '{gradle}' \"$@\"\n",
            starts.display()
        ),
    );
    let count = || {
        fs::read_to_string(&starts)
            .map(|s| s.lines().count())
            .unwrap_or(0)
    };
    let selection = temp.path().join("selection.json");
    let run = |extra: &[&str]| -> (i32, Value, usize) {
        let before = count();
        let mut args = vec![
            "run",
            "--workspace",
            root.to_str().unwrap(),
            "--executable",
            wrapper.to_str().unwrap(),
            "--output",
            selection.to_str().unwrap(),
            "--",
        ];
        if std::env::var_os("IMPACT_OFFLINE").is_some() {
            args.push("--offline");
        }
        args.extend(extra);
        let status = Command::new(BIN)
            .args(&args)
            .env("SIEVE_CACHE_DIR", temp.path().join("cache"))
            .status()
            .unwrap();
        let selection: Value = serde_json::from_slice(&fs::read(&selection).unwrap()).unwrap();
        (status.code().unwrap(), selection, count() - before)
    };
    let all = set(&[
        "CalculatorIT",
        "CalculatorTest",
        "DiscountTest",
        "GreeterTest",
        "LimitsTest",
        "StringUtilsTest",
    ]);

    let (code, selection, started) = run(&[]);
    assert_eq!((code, started), (0, 1), "{selection:#}");
    assert_eq!(
        names(&selection["tests"]),
        all,
        "first run records every test: {selection:#}"
    );
    assert!(root
        .join(".sieve/records/example.CalculatorTest.json")
        .is_file());

    let (code, selection, started) = run(&[]);
    assert_eq!(
        (code, started),
        (0, 0),
        "unchanged: no build: {selection:#}"
    );
    assert_eq!(selection["mode"], "NONE");

    let source = root.join("src/main/java/example/StringUtils.java");
    let text = fs::read_to_string(&source).unwrap();
    fs::write(
        &source,
        text.replacen("toUpperCase()", "toUpperCase(java.util.Locale.ROOT)", 1),
    )
    .unwrap();
    let (code, selection, started) = run(&[]);
    assert_eq!((code, started), (0, 1), "{selection:#}");
    assert_eq!(
        names(&selection["tests"]),
        set(&["StringUtilsTest"]),
        "{selection:#}"
    );
    assert!(
        !selection["reason"].as_str().unwrap().contains("clean"),
        "Gradle never cleans"
    );

    // A bug in a method body fails the tests that executed it, and the failure propagates.
    let calculator = root.join("src/main/java/example/Calculator.java");
    let text = fs::read_to_string(&calculator).unwrap();
    fs::write(&calculator, text.replacen("a + b;", "a + b + 1;", 1)).unwrap();
    let (code, selection, _) = run(&["--continue"]);
    assert_eq!(code, 1, "{selection:#}");
    assert!(
        names(&selection["tests"]).contains("CalculatorTest"),
        "{selection:#}"
    );
    fs::write(&calculator, text).unwrap();
    let (code, selection, _) = run(&[]);
    assert_eq!(code, 0, "failed tests run until they pass: {selection:#}");
    assert!(
        names(&selection["tests"]).contains("CalculatorTest"),
        "{selection:#}"
    );

    // Named tests always run, although their records are unchanged.
    let (code, selection, started) = run(&["--tests", "example.GreeterTest"]);
    assert_eq!((code, started), (0, 1), "{selection:#}");
    assert_eq!(
        names(&selection["tests"]),
        set(&["GreeterTest"]),
        "{selection:#}"
    );
}
