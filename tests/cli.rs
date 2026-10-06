use serde_json::{json, Value};
use std::{fs, path::Path};

mod support;
use support::*;

#[cfg(unix)]
#[test]
fn local_records_require_explicit_adoption() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    write(&root, "pom.xml", "<project/>");
    let workspace = root.to_str().unwrap();
    assert!(!cli(&["run", "--workspace", workspace]).status.success());
    assert!(!root.join(".sieve").exists());
    write(
        &root,
        "impact.json",
        r#"{"tool":"maven","modules":{".":[]}}"#,
    );
    let build = temp.path().join("build");
    executable(&build, "#!/bin/sh\nprintf '%s\\n' \"$@\" > argv\n");
    let output = temp.path().join("selection.json");
    success(&[
        "run",
        "--workspace",
        workspace,
        "--executable",
        build.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ]);
    let result: Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    assert_eq!(result["mode"], "ALL");
    let args = fs::read_to_string(root.join("argv")).unwrap();
    assert!(args.lines().any(|arg| arg == "clean"), "{args}");
    assert!(args.lines().any(|arg| arg == "verify"), "{args}");
    assert!(!root.join(".sieve").exists());
    assert!(!cli(&["select", "--workspace", workspace, "--records"])
        .status
        .success());
    write(
        &root,
        "impact.json",
        r#"{"tool":"maven","modules":{".":[]},"record_env":["BAD=NAME"]}"#,
    );
    assert!(!cli(&["select", "--workspace", workspace]).status.success());
}

#[cfg(all(unix, feature = "agent"))]
#[test]
fn local_wrapper_runs_serialize_the_build_in_one_workspace() {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    write(&root, "pom.xml", "<project/>");
    write(
        &root,
        "impact.json",
        r#"{"tool":"maven","modules":{".":[]},"records":true}"#,
    );
    let build = temp.path().join("build");
    executable(
        &build,
        r#"#!/bin/sh
if [ -e active ]; then exit 9; fi
touch active
trap 'rm -f active' EXIT
while [ ! -e release ]; do sleep 0.05; done
echo complete >> completed
"#,
    );
    let start = || {
        Command::new(BIN)
            .args([
                "run",
                "--full",
                "--workspace",
                root.to_str().unwrap(),
                "--executable",
                build.to_str().unwrap(),
            ])
            .env("SIEVE_CACHE_DIR", temp.path().join("cache"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    };
    let mut first = start();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !root.join("active").exists() {
        assert!(
            first.try_wait().unwrap().is_none(),
            "first build exited before acquiring the workspace"
        );
        if Instant::now() >= deadline {
            let _ = first.kill();
            panic!("first build did not start");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut second = start();
    std::thread::sleep(Duration::from_millis(100));
    let overlapping_exit = second.try_wait().unwrap();
    write(&root, "release", "");
    assert!(first.wait().unwrap().success());
    assert!(second.wait().unwrap().success());
    assert!(
        overlapping_exit.is_none(),
        "second build overlapped the first"
    );
    assert_eq!(
        fs::read_to_string(root.join("completed")).unwrap(),
        "complete\ncomplete\n"
    );
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
fn ignored_submodules_still_count_as_changes() {
    let temp = tempfile::tempdir().unwrap();
    let library = temp.path().join("library");
    let library = library.to_str().unwrap();
    let workspace = temp.path().join("project");
    let workspace = workspace.to_str().unwrap();
    success(&[
        "fixtures", "prepare", "--tool", "maven", "--dest", workspace, "--git",
    ]);
    git(temp.path().to_str().unwrap(), &["init", "-q", library]);
    git(library, &["commit", "-q", "--allow-empty", "-m", "One"]);
    git(
        workspace,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            library,
            "vendor/library",
        ],
    );
    // A submodule set to `ignore = all` hides its commits from plain `git diff`.
    git(
        workspace,
        &[
            "config",
            "-f",
            ".gitmodules",
            "submodule.vendor/library.ignore",
            "all",
        ],
    );
    git(workspace, &["add", "."]);
    git(workspace, &["commit", "-qm", "Add library"]);
    let base = git(workspace, &["rev-parse", "HEAD"]);
    let module = Path::new(workspace).join("vendor/library");
    git(
        module.to_str().unwrap(),
        &["commit", "-q", "--allow-empty", "-m", "Two"],
    );
    // Newer Git skips `ignore = all` submodules in `git add` unless forced.
    git(workspace, &["add", "-f", "vendor/library"]);
    git(workspace, &["commit", "-qm", "Update library"]);
    let selection = select(workspace, &base);
    assert_eq!(selection["mode"], "ALL", "{selection}");
    assert_eq!(selection["changed"], json!(["vendor/library"]));
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
        assert_eq!(reports["cases"], 12, "{tool}: {reports}");
        assert_eq!(reports["failed"].as_array().unwrap().len(), 7);
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

#[cfg(all(unix, feature = "agent"))]
#[test]
fn maven_class_selection_starts_maven_once_with_the_extension() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("project");
    let workspace = workspace.to_str().unwrap();
    success(&[
        "fixtures",
        "prepare",
        "--tool",
        "maven",
        "--dest",
        workspace,
        "--git",
        "--class-level",
    ]);
    success(&["fixtures", "apply", "unrelated", "--workspace", workspace]);
    let build = temp.path().join("mvn");
    executable(
        &build,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$0.calls\"\necho --- >> \"$0.calls\"\n",
    );
    let output = std::process::Command::new(BIN)
        .args(["run", "--workspace", workspace, "--base", "HEAD"])
        .args(["--executable", build.to_str().unwrap()])
        .env("SIEVE_CACHE_DIR", temp.path().join("cache"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // One Maven start: the extension selects test classes inside it, with no compile step.
    let calls = fs::read_to_string(temp.path().join("mvn.calls")).unwrap();
    assert_eq!(calls.matches("---").count(), 1, "{calls}");
    assert!(!calls.lines().any(|arg| arg == "test-compile"), "{calls}");
    let jar = calls
        .lines()
        .find_map(|arg| arg.strip_prefix("-Dmaven.ext.class.path="))
        .unwrap();
    assert!(
        jar.ends_with("sieve-maven.jar") && Path::new(jar).is_file(),
        "{calls}"
    );
    for property in ["-Dsieve.request=", "-Dsieve.exe="] {
        assert!(
            calls.lines().any(|arg| arg.starts_with(property)),
            "{calls}"
        );
    }
}
