use super::*;

struct Mutation<'a> {
    path: &'a str,
    before: &'a str,
    after: Option<&'a str>,
    mode: &'a str,
    tests: &'a [&'a str],
    failures: &'a [&'a str],
    errors: &'a [&'a str],
    /// What module-level selection runs, when narrower than every test.
    module_tests: Option<&'a [&'a str]>,
}

struct Sample {
    name: &'static str,
    /// Test classes with their invocation counts, as the reports list them.
    baseline: Vec<(&'static str, u64)>,
    graph: Value,
    mutations: Vec<Mutation<'static>>,
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
        (
            "kotlin-invoices",
            "money/src/main/kotlin/example/Sums.kt",
            json!(["invoice", "money"]),
        ),
        (
            "kotlin-shipping",
            "rates/src/main/kotlin/example/Rates.kt",
            json!(["orders", "rates"]),
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
        for sample in samples(tool == "maven") {
            check_sample(tool, &executable, sample);
        }
    }
}

/// This oracle is hand-written from the samples' tests, never from selection JSON.
fn samples(maven: bool) -> Vec<Sample> {
    let mut samples = vec![java_sample(maven)];
    samples.push(if maven {
        kotlin_shipping()
    } else {
        kotlin_invoices()
    });
    samples
}

fn all(baseline: &[(&'static str, u64)]) -> &'static [&'static str] {
    baseline
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .leak()
}

fn java_sample(maven: bool) -> Sample {
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
    let baseline: Vec<(&str, u64)> = if maven {
        vec![(STATUS, 1), (WELCOME, 2)]
    } else {
        vec![(DELIVERY, 1), (INDEPENDENT, 1), (LABEL, 2)]
    };
    let all = all(&baseline);
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
                module_tests: None,
            },
            Mutation {
                path: "provider/src/main/resources/greeting.properties",
                before: "Hello",
                after: Some("Goodbye"),
                mode: "MODULES",
                tests: all,
                failures: &[WELCOME],
                errors: &[],
                module_tests: None,
            },
            Mutation {
                path: "provider/src/main/resources/META-INF/services/example.Greeting",
                before: "",
                after: None,
                mode: "MODULES",
                tests: all,
                failures: &[WELCOME],
                errors: &[WELCOME],
                module_tests: None,
            },
            Mutation {
                path: "app/src/test/java/example/WelcomeIT.java",
                before: "\"Hello \"",
                after: Some("\"Welcome \""),
                mode: "SUBSET",
                tests: &[WELCOME],
                failures: &[WELCOME],
                errors: &[],
                module_tests: None,
            },
            Mutation {
                path: "app/src/main/java/example/Status.java",
                before: "\"ready\"",
                after: Some("\"waiting\""),
                mode: "SUBSET",
                tests: &[STATUS],
                failures: &[STATUS],
                errors: &[],
                module_tests: None,
            },
            Mutation {
                path: "pom.xml",
                before: "</project>",
                after: Some("<!-- build input changed -->\n</project>"),
                mode: "ALL",
                tests: all,
                failures: &[],
                errors: &[],
                module_tests: None,
            },
            Mutation {
                path: "README.md",
                before: "# Runtime plugins",
                after: Some("# Updated runtime plugins"),
                mode: "NONE",
                tests: &[],
                failures: &[],
                errors: &[],
                module_tests: None,
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
                module_tests: None,
            },
            Mutation {
                path: "engine/src/testFixtures/java/example/Examples.java",
                before: "alpha beta",
                after: Some("wrong name"),
                mode: "SUBSET",
                tests: &[DELIVERY],
                failures: &[DELIVERY],
                errors: &[],
                module_tests: None,
            },
            Mutation {
                path: "app/src/integrationTest/resources/expected-label.txt",
                before: "ALPHA-BETA",
                after: Some("WRONG"),
                mode: "MODULES",
                tests: all,
                failures: &[DELIVERY],
                errors: &[],
                module_tests: None,
            },
            Mutation {
                path: "app/src/integrationTest/java/example/DeliveryIT.java",
                before: "Examples.name()",
                after: Some("Examples.name() + \" extra\""),
                mode: "SUBSET",
                tests: &[DELIVERY],
                failures: &[DELIVERY],
                errors: &[],
                module_tests: None,
            },
            Mutation {
                path: "app/src/test/java/example/IndependentTest.java",
                before: "assertEquals(4,",
                after: Some("assertEquals(5,"),
                mode: "SUBSET",
                tests: &[INDEPENDENT],
                failures: &[INDEPENDENT],
                errors: &[],
                module_tests: None,
            },
            Mutation {
                path: "build.gradle",
                before: "subprojects {",
                after: Some("// build input changed\nsubprojects {"),
                mode: "ALL",
                tests: all,
                failures: &[],
                errors: &[],
                module_tests: None,
            },
            Mutation {
                path: "README.md",
                before: "# Separate test suites",
                after: Some("# Updated test suites"),
                mode: "NONE",
                tests: &[],
                failures: &[],
                errors: &[],
                module_tests: None,
            },
        ]
    };
    let graph = if maven {
        json!({"api": [], "provider": ["api"], "app": ["api", "provider"]})
    } else {
        // java-test-fixtures declares a dependency on its own main variant.
        json!({".": [], "engine": ["engine"], "app": ["engine"]})
    };
    Sample {
        name: sample,
        baseline,
        graph,
        mutations,
    }
}

fn check_sample(tool: &str, executable: &str, sample: Sample) {
    let maven = tool == "maven";
    let Sample {
        name: sample,
        baseline,
        graph: expected_graph,
        mutations,
    } = sample;
    let all = all(&baseline);
    let temp = tempfile::tempdir().unwrap();
    // Spaces also exercise paths passed to the compiler and test-filter adapters.
    let root = temp.path().join("sample project");
    copy_sample(&Path::new(ROOT).join("samples").join(sample), &root);
    let workspace = root.to_str().unwrap();
    let selection_path = temp.path().join("selection.json");
    let mut config: Value =
        serde_json::from_slice(&fs::read(root.join("impact.json")).unwrap()).unwrap();
    assert_eq!(config["modules"], expected_graph);
    // Rediscover using the actual build and compare to the committed graph.
    success(&[
        "refresh",
        "--workspace",
        workspace,
        "--executable",
        executable,
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
        Command::new(executable)
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
                executable,
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
            } else if mode == "ALL" {
                all
            } else {
                case.module_tests.unwrap_or(all)
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

fn kotlin_invoices() -> Sample {
    const DISCOUNT: &str = "invoice:unit:example.DiscountTest";
    const INVOICE: &str = "invoice:unit:example.InvoiceTest";
    const FORMAT: &str = "money:unit:example.FormatTest";
    const SUMS: &str = "money:unit:example.SumsTest";
    const INVOICE_MODULE: &[&str] = &[DISCOUNT, INVOICE];
    let baseline = vec![(DISCOUNT, 2), (INVOICE, 2), (FORMAT, 1), (SUMS, 1)];
    let all = all(&baseline);
    let mutations = vec![
        // Invoice carries a copy of the inline body, so its test runs without a reference.
        Mutation {
            path: "money/src/main/kotlin/example/Sums.kt",
            before: "total += amount(item)",
            after: Some("total += amount(item) + Cents(1)"),
            mode: "SUBSET",
            tests: &[INVOICE, SUMS],
            failures: &[INVOICE, SUMS],
            errors: &[],
            module_tests: None,
        },
        Mutation {
            path: "money/src/main/kotlin/example/Format.kt",
            before: "\"%d.%02d\"",
            after: Some("\"%d,%02d\""),
            mode: "SUBSET",
            tests: &[FORMAT],
            failures: &[FORMAT],
            errors: &[],
            module_tests: None,
        },
        // Selected through Invoice.due, which passes: InvoiceTest applies no discount.
        Mutation {
            path: "invoice/src/main/kotlin/example/Discount.kt",
            before: "(100 - percent)",
            after: Some("(100 + percent)"),
            mode: "SUBSET",
            tests: &[DISCOUNT, INVOICE],
            failures: &[DISCOUNT],
            errors: &[],
            module_tests: Some(INVOICE_MODULE),
        },
        Mutation {
            path: "invoice/src/test/kotlin/example/DiscountTest.kt",
            before: "Cents(1800)",
            after: Some("Cents(1700)"),
            mode: "SUBSET",
            tests: &[DISCOUNT],
            failures: &[DISCOUNT],
            errors: &[],
            module_tests: Some(INVOICE_MODULE),
        },
        Mutation {
            path: "build.gradle.kts",
            before: "plugins {",
            after: Some("// build input changed\nplugins {"),
            mode: "ALL",
            tests: all,
            failures: &[],
            errors: &[],
            module_tests: None,
        },
        Mutation {
            path: "README.md",
            before: "# Kotlin invoices",
            after: Some("# Updated Kotlin invoices"),
            mode: "NONE",
            tests: &[],
            failures: &[],
            errors: &[],
            module_tests: None,
        },
    ];
    Sample {
        name: "kotlin-invoices",
        baseline,
        graph: json!({".": [], "invoice": ["money"], "money": []}),
        mutations,
    }
}

fn kotlin_shipping() -> Sample {
    const PARCEL: &str = "orders:unit:example.ParcelTest";
    const QUOTE: &str = "orders:unit:example.QuoteTest";
    const ZONE: &str = "rates:unit:example.ZoneTest";
    let baseline = vec![(PARCEL, 1), (QUOTE, 2), (ZONE, 2)];
    let all = all(&baseline);
    // Failures stay in `orders`, the last module: Maven stops at the first failing module,
    // which would leave the tests of the modules after it unrun.
    let mutations = vec![
        // Quote's bytecode holds a copy of the constant, not a reference to Rates.
        Mutation {
            path: "rates/src/main/kotlin/example/Rates.kt",
            before: "BASE_FEE = 499",
            after: Some("BASE_FEE = 599"),
            mode: "SUBSET",
            tests: &[QUOTE],
            failures: &[QUOTE],
            errors: &[],
            module_tests: None,
        },
        Mutation {
            path: "rates/src/main/kotlin/example/Zone.kt",
            before: "\"DE\", \"FR\"",
            after: Some("\"FR\""),
            mode: "SUBSET",
            tests: &[QUOTE, ZONE],
            failures: &[QUOTE],
            errors: &[],
            module_tests: None,
        },
        // Selected through Quote, which passes: its parcels round to the same kilos.
        Mutation {
            path: "orders/src/main/kotlin/example/Parcel.kt",
            before: "(grams + 999)",
            after: Some("(grams + 500)"),
            mode: "SUBSET",
            tests: &[PARCEL, QUOTE],
            failures: &[PARCEL],
            errors: &[],
            module_tests: Some(&[PARCEL, QUOTE]),
        },
        Mutation {
            path: "orders/src/test/kotlin/example/ParcelTest.kt",
            before: "assertEquals(2,",
            after: Some("assertEquals(3,"),
            mode: "SUBSET",
            tests: &[PARCEL],
            failures: &[PARCEL],
            errors: &[],
            module_tests: Some(&[PARCEL, QUOTE]),
        },
        Mutation {
            path: "pom.xml",
            before: "</project>",
            after: Some("<!-- build input changed -->\n</project>"),
            mode: "ALL",
            tests: all,
            failures: &[],
            errors: &[],
            module_tests: None,
        },
        Mutation {
            path: "README.md",
            before: "# Kotlin shipping",
            after: Some("# Updated Kotlin shipping"),
            mode: "NONE",
            tests: &[],
            failures: &[],
            errors: &[],
            module_tests: None,
        },
    ];
    Sample {
        name: "kotlin-shipping",
        baseline,
        graph: json!({"orders": ["rates"], "rates": []}),
        mutations,
    }
}
