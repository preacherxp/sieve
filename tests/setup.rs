#![cfg(unix)]
mod support;
use serde_json::{json, Value};
use std::{fs, process::Command};
use support::*;

#[test]
fn bad_maven_models_leave_all_project_files_unchanged() {
    // BUILD-02/04/05: real CLI/XML validation with controlled effective models.
    for model in [
        "<project>",
        "<project><groupId>x</groupId></project>",
        "<project><groupId>x</groupId><artifactId>root</artifactId><packaging>jar</packaging><modules><module>a</module></modules></project>",
        "<project><groupId>x</groupId><artifactId>root</artifactId><packaging>pom</packaging><modules><module>nested/a</module></modules></project>",
        "<project><groupId>x</groupId><artifactId>root</artifactId><packaging>pom</packaging><modules><module>../escape</module></modules></project>",
        "<project><groupId>x</groupId><artifactId>root</artifactId><packaging>pom</packaging><modules><module>a</module><module>b</module></modules></project>",
        "<project><groupId>x</groupId><artifactId>root</artifactId><dependencies><dependency><groupId>x</groupId></dependency></dependencies></project>",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        for path in ["pom.xml", "a/pom.xml", "b/pom.xml"] { write(&root, path, "<project><artifactId>preserved</artifactId></project>"); }
        write(temp.path(), "root.xml", model);
        write(temp.path(), "child.xml", "<project><groupId>x</groupId><artifactId>duplicate</artifactId></project>");
        let build = temp.path().join("mvn");
        executable(&build, r#"#!/bin/sh
next=0
for arg do
  if [ "$next" = 1 ]; then pom=$arg; next=0; fi
  case "$arg" in -f) next=1 ;; -Doutput=*) output=${arg#-Doutput=} ;; esac
done
case "$pom" in */a/pom.xml|*/b/pom.xml) cp "$CHILD_MODEL" "$output" ;; *) cp "$ROOT_MODEL" "$output" ;; esac
"#);
        let output = Command::new(BIN).args(["init", "--workspace", root.to_str().unwrap(), "--executable", build.to_str().unwrap()])
            .env("ROOT_MODEL", temp.path().join("root.xml")).env("CHILD_MODEL", temp.path().join("child.xml")).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "model={model}: {}", String::from_utf8_lossy(&output.stdout));
        assert!(!root.join("impact.json").exists());
        for path in ["pom.xml", "a/pom.xml", "b/pom.xml"] { assert_eq!(fs::read_to_string(root.join(path)).unwrap(), "<project><artifactId>preserved</artifactId></project>"); }
    }
}

#[test]
fn setup_detects_ambiguity_preserves_existing_files_and_refreshes_graph() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    write(&root, "pom.xml", "<project/>");
    write(&root, "build.gradle", "plugins { id 'java' }");
    assert_eq!(
        cli(&["init", "--workspace", root.to_str().unwrap()])
            .status
            .code(),
        Some(2)
    );
    fs::remove_file(root.join("pom.xml")).unwrap();
    for module in ["a", "b", "removed"] {
        fs::create_dir(root.join(module)).unwrap();
    }
    let wrapper = root.join("gradlew");
    executable(
        &wrapper,
        r#"#!/bin/sh
for arg do
 case "$arg" in -Pimpact.output=*) output=${arg#-Pimpact.output=} ;; esac
done
cat model.json > "$output"
"#,
    );
    write(
        &root,
        "model.json",
        serde_json::to_vec(&json!({"tool":"gradle","modules":{"a":[],"b":[],"removed":[]}}))
            .unwrap(),
    );
    success(&["init", "--workspace", root.to_str().unwrap()]);
    let original = fs::read(root.join("impact.json")).unwrap();
    assert_eq!(
        cli(&["init", "--workspace", root.to_str().unwrap()])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(fs::read(root.join("impact.json")).unwrap(), original);
    let mut config: Value = serde_json::from_slice(&original).unwrap();
    config["modules"]["b"] = json!(["a", "removed"]); // manually declared runtime edge
    config["ignore"] = json!(["notes/**"]);
    config["class_tests"] = json!({"a/src/main/java/A.java": ["a:unit:example.ATest"]});
    write(&root, "impact.json", serde_json::to_vec(&config).unwrap());
    fs::remove_dir(root.join("removed")).unwrap();
    fs::create_dir(root.join("added")).unwrap();
    write(
        &root,
        "model.json",
        serde_json::to_vec(&json!({"tool":"gradle","modules":{"a":[],"b":[],"added":["b"]}}))
            .unwrap(),
    );
    success(&["refresh", "--workspace", root.to_str().unwrap()]);
    let updated: Value =
        serde_json::from_slice(&fs::read(root.join("impact.json")).unwrap()).unwrap();
    assert_eq!(updated["modules"], json!({"a":[],"b":["a"],"added":["b"]}));
    assert!(updated["build_fingerprint"].is_string());
    assert_eq!(updated["ignore"], json!(["notes/**"]));
    assert_eq!(
        updated["class_tests"],
        json!({"a/src/main/java/A.java": ["a:unit:example.ATest"]})
    );
    // The rejected model must not partially rewrite the previous configuration.
    let saved = fs::read(root.join("impact.json")).unwrap();
    write(&root, "model.json", "{}");
    assert_eq!(
        cli(&["refresh", "--workspace", root.to_str().unwrap()])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(fs::read(root.join("impact.json")).unwrap(), saved);
}

#[test]
fn module_symlink_cannot_escape_the_workspace() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    std::os::unix::fs::symlink(temp.path(), root.join("linked")).unwrap();
    write(
        &root,
        "impact.json",
        r#"{"tool":"maven","modules":{"linked":[]}}"#,
    );
    assert_eq!(
        cli(&["select", "--workspace", root.to_str().unwrap(), "--full"])
            .status
            .code(),
        Some(2)
    );
}

#[test]
fn discovery_forwards_profile_and_property_arguments_intact() {
    for tool in ["maven", "gradle"] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(
            root,
            if tool == "maven" {
                "pom.xml"
            } else {
                "build.gradle"
            },
            if tool == "maven" { "<project/>" } else { "" },
        );
        let build = root.join("builder");
        executable(
            &build,
            r#"#!/bin/sh
printf '%s\000' "$@" > capture
for arg do
 case "$arg" in
 -Doutput=*) printf '<project><groupId>x</groupId><artifactId>root</artifactId></project>' > "${arg#-Doutput=}" ;;
 -Pimpact.output=*) printf '{"tool":"gradle","modules":{".":[]}}' > "${arg#-Pimpact.output=}" ;;
 esac
done
"#,
        );
        success(&[
            "init",
            "--workspace",
            root.to_str().unwrap(),
            "--executable",
            build.to_str().unwrap(),
            "--",
            "-Pci",
            "-Dmessage=hello world",
        ]);
        let capture = fs::read(root.join("capture")).unwrap();
        let args: Vec<_> = capture.split(|b| *b == 0).collect();
        assert!(args.contains(&b"-Pci".as_slice()));
        assert!(args.contains(&b"-Dmessage=hello world".as_slice()));
    }
}
