use super::*;

struct Mutation<'a> {
    path: &'a str,
    before: &'a str,
    after: Option<&'a str>,
    mode: &'a str,
    tests: &'a [&'a str],
    failures: &'a [&'a str],
    errors: &'a [&'a str],
}

fn copy_sample(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if ["target", "build", ".gradle", ".sieve"].contains(&entry.file_name().to_str().unwrap()) {
            continue;
        }
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_sample(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), dest).unwrap();
        }
    }
}

#[test]
fn sample_graphs_select_runtime_and_test_fixture_consumers() {
    for (sample, path, modules) in [
        (
            "runtime-plugins",
            "provider/src/main/resources/greeting.properties",
            json!(["app", "provider"]),
        ),
        (
            "gradle-test-suites",
            "engine/src/testFixtures/java/example/Examples.java",
            json!(["app", "engine"]),
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("sample");
        copy_sample(&Path::new(ROOT).join("samples").join(sample), &root);
        let workspace = root.to_str().unwrap();
        git(workspace, &["init", "-q"]);
        commit(&root, "sample baseline");
        // Also checks that the committed build fingerprint is current.
        assert_eq!(select(workspace, "HEAD")["mode"], "NONE");
        let original = fs::read_to_string(root.join(path)).unwrap();
        write(&root, path, original + "\n");
        let actual = select(workspace, "HEAD");
        assert_eq!(actual["mode"], "MODULES");
        assert_eq!(actual["modules"], modules);
        assert_eq!(select(workspace, "missing-history")["mode"], "ALL");
    }
}

#[test]
#[ignore = "requires Java 17+ and Maven/Gradle; run by the native fixture CI checks"]
fn native_sample_mutations_match_full_execution() {
    for (tool, executable) in tools() {
        let maven = tool == "maven";
        let sample = if maven {
            "runtime-plugins"
        } else {
            "gradle-test-suites"
        };
        const WELCOME: &str = "app:integration:example.WelcomeIT";
        const STATUS: &str = "app:integration:example.StatusIT";
        const DELIVERY: &str = "app:integration:example.DeliveryIT";
        const LABEL: &str = "app:unit:example.LabelTest";
        const INDEPENDENT: &str = "app:unit:example.IndependentTest";
        // This oracle is hand-written from the sample's tests, never from selection JSON.
        let baseline: Vec<(&str, u64)> = if maven {
            vec![(STATUS, 1), (WELCOME, 2)]
        } else {
            vec![(DELIVERY, 1), (INDEPENDENT, 1), (LABEL, 2)]
        };
        let all: Vec<&str> = baseline.iter().map(|(name, _)| *name).collect();
        let mutations = if maven {
            vec![
                Mutation {
                    path: "provider/src/main/java/example/EnglishGreeting.java",
                    before: " + name;",
                    after: Some(" + name + \"!\";"),
                    mode: "SUBSET",
                    tests: &[WELCOME],
                    failures: &[WELCOME],
                    errors: &[],
                },
                Mutation {
                    path: "provider/src/main/resources/greeting.properties",
                    before: "Hello",
                    after: Some("Goodbye"),
                    mode: "MODULES",
                    tests: &all,
                    failures: &[WELCOME],
                    errors: &[],
                },
                Mutation {
                    path: "provider/src/main/resources/META-INF/services/example.Greeting",
                    before: "",
                    after: None,
                    mode: "MODULES",
                    tests: &all,
                    failures: &[WELCOME],
                    errors: &[WELCOME],
                },
                Mutation {
                    path: "app/src/test/java/example/WelcomeIT.java",
                    before: "\"Hello \"",
                    after: Some("\"Welcome \""),
                    mode: "SUBSET",
                    tests: &[WELCOME],
                    failures: &[WELCOME],
                    errors: &[],
                },
                Mutation {
                    path: "app/src/main/java/example/Status.java",
                    before: "\"ready\"",
                    after: Some("\"waiting\""),
                    mode: "SUBSET",
                    tests: &[STATUS],
                    failures: &[STATUS],
                    errors: &[],
                },
                Mutation {
                    path: "pom.xml",
                    before: "</project>",
                    after: Some("<!-- build input changed -->\n</project>"),
                    mode: "ALL",
                    tests: &all,
                    failures: &[],
                    errors: &[],
                },
                Mutation {
                    path: "README.md",
                    before: "# Runtime plugins",
                    after: Some("# Updated runtime plugins"),
                    mode: "NONE",
                    tests: &[],
                    failures: &[],
                    errors: &[],
                },
            ]
        } else {
            vec![
                Mutation {
                    path: "engine/src/main/java/example/Slug.java",
                    before: "toUpperCase",
                    after: Some("toLowerCase"),
                    mode: "SUBSET",
                    tests: &[DELIVERY, LABEL],
                    failures: &[DELIVERY, LABEL],
                    errors: &[],
                },
                Mutation {
                    path: "engine/src/testFixtures/java/example/Examples.java",
                    before: "alpha beta",
                    after: Some("wrong name"),
                    mode: "SUBSET",
                    tests: &[DELIVERY],
                    failures: &[DELIVERY],
                    errors: &[],
                },
                Mutation {
                    path: "app/src/integrationTest/resources/expected-label.txt",
                    before: "ALPHA-BETA",
                    after: Some("WRONG"),
                    mode: "MODULES",
                    tests: &all,
                    failures: &[DELIVERY],
                    errors: &[],
                },
                Mutation {
                    path: "app/src/integrationTest/java/example/DeliveryIT.java",
                    before: "Examples.name()",
                    after: Some("Examples.name() + \" extra\""),
                    mode: "SUBSET",
                    tests: &[DELIVERY],
                    failures: &[DELIVERY],
                    errors: &[],
                },
                Mutation {
                    path: "app/src/test/java/example/IndependentTest.java",
                    before: "assertEquals(4,",
                    after: Some("assertEquals(5,"),
                    mode: "SUBSET",
                    tests: &[INDEPENDENT],
                    failures: &[INDEPENDENT],
                    errors: &[],
                },
                Mutation {
                    path: "build.gradle",
                    before: "subprojects {",
                    after: Some("// build input changed\nsubprojects {"),
                    mode: "ALL",
                    tests: &all,
                    failures: &[],
                    errors: &[],
                },
                Mutation {
                    path: "README.md",
                    before: "# Separate test suites",
                    after: Some("# Updated test suites"),
                    mode: "NONE",
                    tests: &[],
                    failures: &[],
                    errors: &[],
                },
            ]
        };
        let temp = tempfile::tempdir().unwrap();
        // Spaces also exercise paths passed to the compiler and test-filter adapters.
        let root = temp.path().join("sample project");
        copy_sample(&Path::new(ROOT).join("samples").join(sample), &root);
        let workspace = root.to_str().unwrap();
        let selection_path = temp.path().join("selection.json");
        let mut config: Value =
            serde_json::from_slice(&fs::read(root.join("impact.json")).unwrap()).unwrap();
        let expected_graph = if maven {
            json!({"api": [], "provider": ["api"], "app": ["api", "provider"]})
        } else {
            // java-test-fixtures declares a dependency on its own main variant.
            json!({".": [], "engine": ["engine"], "app": ["engine"]})
        };
        assert_eq!(config["modules"], expected_graph);
        // Rediscover using the actual build and compare to the committed graph.
        success(&[
            "refresh",
            "--workspace",
            workspace,
            "--executable",
            &executable,
        ]);
        config = serde_json::from_slice(&fs::read(root.join("impact.json")).unwrap()).unwrap();
        assert_eq!(config["modules"], expected_graph);
        git(workspace, &["init", "-q"]);

        let native_args = if maven {
            vec!["-B", "-ntp", "clean", "verify"]
        } else {
            // Continue independent tasks after a failure, retaining a failing exit code.
            vec!["--console=plain", "--continue", "clean", "check"]
        };
        let native = || {
            Command::new(&executable)
                .current_dir(&root)
                .args(&native_args)
                .output()
                .unwrap()
        };
        checked(native());
        let baseline_reports = reports(&root, tool);
        assert_eq!(baseline_reports["executed"], json!(all));
        assert_eq!(
            baseline_reports["cases"],
            baseline.iter().map(|(_, count)| count).sum::<u64>()
        );
        assert_eq!(baseline_reports["failed"], json!([]));
        assert_eq!(baseline_reports["skipped"], json!([]));

        for class_level in [false, true] {
            config["class_level"] = json!(class_level);
            write(
                &root,
                "impact.json",
                serde_json::to_vec_pretty(&config).unwrap(),
            );
            commit(&root, &format!("class_level={class_level}"));
            for case in &mutations {
                eprintln!("{sample} class_level={class_level}: {}", case.path);
                let original = fs::read_to_string(root.join(case.path)).unwrap();
                if let Some(after) = case.after {
                    assert_eq!(original.matches(case.before).count(), 1, "{}", case.path);
                    write(&root, case.path, original.replacen(case.before, after, 1));
                } else {
                    fs::remove_file(root.join(case.path)).unwrap();
                }
                // One native reference per mutation; both modes must catch the same failures.
                if !class_level {
                    let output = native();
                    assert_eq!(
                        output.status.success(),
                        case.failures.is_empty(),
                        "{}\n{}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    );
                    let actual = reports(&root, tool);
                    assert_eq!(actual["executed"], json!(all));
                    assert_eq!(actual["cases"], baseline_reports["cases"]);
                    assert_eq!(actual["failed"], json!(case.failures));
                    assert_eq!(actual["errors"], json!(case.errors));
                    assert_eq!(actual["skipped"], json!([]));
                }
                let mut args = vec![
                    "run",
                    "--workspace",
                    workspace,
                    "--executable",
                    &executable,
                    "--base",
                    "HEAD",
                    "--output",
                    selection_path.to_str().unwrap(),
                ];
                if !maven {
                    args.extend(["--", "--continue"]);
                }
                let output = cli(&args);
                assert_eq!(
                    output.status.code(),
                    Some(if case.failures.is_empty() { 0 } else { 1 }),
                    "{}\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                let selected: Value =
                    serde_json::from_slice(&fs::read(&selection_path).unwrap()).unwrap();
                let mode = if !class_level && case.mode == "SUBSET" {
                    "MODULES"
                } else {
                    case.mode
                };
                assert_eq!(selected["mode"], mode, "{selected}");
                let expected = if class_level || mode == "NONE" {
                    case.tests
                } else {
                    &all
                };
                let actual = reports(&root, tool);
                assert_eq!(actual["executed"], json!(expected), "{selected}");
                assert_eq!(
                    actual["cases"],
                    baseline
                        .iter()
                        .filter(|(name, _)| expected.contains(name))
                        .map(|(_, count)| count)
                        .sum::<u64>()
                );
                assert_eq!(actual["failed"], json!(case.failures));
                assert_eq!(actual["errors"], json!(case.errors));
                assert_eq!(actual["skipped"], json!([]));
                // Restoring a deleted resource also proves stale build outputs cannot mask it.
                write(&root, case.path, original);
            }
        }
    }
}
