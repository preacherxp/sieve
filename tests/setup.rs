#![cfg(unix)]
mod support;
use serde_json::{json, Value};
use std::{fs, process::Command};
use support::*;

#[test]
fn single_module_setup_keeps_selection_opt_in_and_refresh_preserves_preferences() {
    for tool in ["maven", "gradle"] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        let model = if tool == "maven" {
            write(&root, "pom.xml", "<project><modelVersion>4.0.0</modelVersion><groupId>x</groupId><artifactId>app</artifactId><version>1</version></project>");
            "<project><groupId>x</groupId><artifactId>app</artifactId><packaging>jar</packaging></project>".to_owned()
        } else {
            write(&root, "build.gradle", "plugins { id 'java' }");
            json!({"tool": "gradle", "modules": {".": []}}).to_string()
        };
        write(&root, "model", model);
        let build = temp.path().join("build-tool");
        executable(
            &build,
            r#"#!/bin/sh
for arg do
  case "$arg" in -Doutput=*|-Pimpact.output=*) cp model "${arg#*=}" ;; esac
done
"#,
        );
        let workspace = root.to_str().unwrap();
        let executable = build.to_str().unwrap();
        success(&["init", "--workspace", workspace, "--executable", executable]);
        let mut config: Value =
            serde_json::from_slice(&fs::read(root.join("impact.json")).unwrap()).unwrap();
        assert_ne!(config["class_level"], true, "{tool}");
        assert_ne!(config["records"], true, "{tool}");
        config["class_level"] = json!(true);
        config["records"] = json!(false);
        config["record_env"] = json!(["SERVICE_MODE"]);
        write(&root, "impact.json", serde_json::to_vec(&config).unwrap());
        success(&[
            "refresh",
            "--workspace",
            workspace,
            "--executable",
            executable,
        ]);
        let config: Value =
            serde_json::from_slice(&fs::read(root.join("impact.json")).unwrap()).unwrap();
        assert_eq!(config["class_level"], true, "{tool}");
        assert_eq!(config["records"], false, "{tool}");
        assert_eq!(config["record_env"], json!(["SERVICE_MODE"]), "{tool}");
    }
}

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
    assert_ne!(config["class_level"], true);
    config["modules"]["b"] = json!(["a", "removed"]); // manually declared runtime edge
    config["ignore"] = json!(["notes/**"]);
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
    // Refresh preserves class-level selection on a multi-module graph.
    let mut class_level = updated.clone();
    class_level["class_level"] = json!(true);
    write(
        &root,
        "impact.json",
        serde_json::to_vec(&class_level).unwrap(),
    );
    success(&["refresh", "--workspace", root.to_str().unwrap()]);
    let refreshed: Value =
        serde_json::from_slice(&fs::read(root.join("impact.json")).unwrap()).unwrap();
    assert_eq!(refreshed["class_level"], json!(true));
    write(&root, "impact.json", serde_json::to_vec(&updated).unwrap());
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

#[test]
fn maven_setup_follows_modules_the_build_itself_uses() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("project")).unwrap();
    // Maven reports the canonical paths that setup passes to it.
    let root = temp.path().join("project").canonicalize().unwrap();
    for path in ["pom.xml", "api/pom.xml", "processor/pom.xml", "app/pom.xml"] {
        write(&root, path, "<project></project>");
    }
    let models = temp.path().join("models");
    write(&models, "root.xml", "<project><groupId>x</groupId><artifactId>root</artifactId><packaging>pom</packaging><modules><module>api</module><module>processor</module><module>app</module></modules></project>");
    for module in ["api", "processor"] {
        write(&models, &format!("{module}.xml"), format!("<project><groupId>x</groupId><artifactId>{module}</artifactId><build><directory>{}/{module}/target</directory></build></project>", root.display()));
    }
    // An annotation processor path and a specification read from a sibling's directory.
    write(&models, "app.xml", format!("<project><groupId>x</groupId><artifactId>app</artifactId><build><directory>{0}/app/target</directory><plugins>\
        <plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-compiler-plugin</artifactId><configuration><annotationProcessorPaths><path><groupId>x</groupId><artifactId>processor</artifactId></path></annotationProcessorPaths></configuration></plugin>\
        <plugin><groupId>org.openapitools</groupId><artifactId>openapi-generator-maven-plugin</artifactId><executions><execution><configuration><inputSpec>{0}/app/../api/src/main/resources/api.yaml</inputSpec></configuration></execution></executions></plugin>\
        </plugins></build></project>", root.display()));
    let build = temp.path().join("mvn");
    executable(
        &build,
        r#"#!/bin/sh
next=0
for arg do
  if [ "$next" = 1 ]; then pom=$arg; next=0; fi
  case "$arg" in -f) next=1 ;; -Doutput=*) output=${arg#-Doutput=} ;; esac
done
module=$(basename "$(dirname "$pom")")
[ -f "$MODELS/$module.xml" ] || module=root
cp "$MODELS/$module.xml" "$output"
"#,
    );
    let output = Command::new(BIN)
        .args(["init", "--workspace", root.to_str().unwrap()])
        .args(["--executable", build.to_str().unwrap()])
        .env("MODELS", &models)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let config: Value =
        serde_json::from_slice(&fs::read(root.join("impact.json")).unwrap()).unwrap();
    assert_eq!(
        config["modules"],
        json!({"api": [], "app": ["api", "processor"], "processor": []})
    );
}

#[test]
fn maven_setup_reads_the_reactor_once() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("project")).unwrap();
    let root = temp.path().join("project").canonicalize().unwrap();
    write(
        &root,
        "pom.xml",
        "<project><artifactId>root</artifactId></project>",
    );
    write(
        &root,
        "core/pom.xml",
        "<project><artifactId>core</artifactId></project>",
    );
    write(
        &root,
        "app/pom.xml",
        "<project><artifactId>app</artifactId></project>",
    );
    // An artifact ID that names a property cannot be matched to the reactor's models.
    write(
        &root,
        "testkit/pom.xml",
        "<project><artifactId>${kit}</artifactId></project>",
    );
    let models = temp.path().join("models");
    let model = |artifact: &str, body: &str| {
        format!("<project><groupId>x</groupId><artifactId>{artifact}</artifactId>{body}</project>")
    };
    let testkit = model("testkit", "");
    let reactor = [
        model("root", "<packaging>pom</packaging><modules><module>core</module><module>app</module><module>testkit</module></modules>"),
        model("core", ""),
        model("app", "<dependencies><dependency><groupId>x</groupId><artifactId>core</artifactId></dependency>\
            <dependency><groupId>x</groupId><artifactId>testkit</artifactId><classifier>tests</classifier><scope>test</scope></dependency></dependencies>"),
        testkit.clone(),
    ];
    write(
        &models,
        "reactor.xml",
        format!(
            "<?xml version=\"1.0\"?><!-- models --><projects>{}</projects>",
            reactor.join("<!-- next -->")
        ),
    );
    write(&models, "testkit.xml", testkit);
    let build = temp.path().join("mvn");
    executable(
        &build,
        r#"#!/bin/sh
echo "$*" >> "$MODELS/calls"
model=reactor next=0
for arg do
  if [ "$next" = 1 ]; then pom=$arg; next=0; fi
  case "$arg" in -f) next=1 ;; -N) model=single ;; -Doutput=*) output=${arg#-Doutput=} ;; esac
done
[ "$model" = single ] && model=$(basename "$(dirname "$pom")")
cp "$MODELS/$model.xml" "$output"
"#,
    );
    let init = |command: &str| {
        let output = Command::new(BIN)
            .args([command, "--workspace", root.to_str().unwrap()])
            .args(["--executable", build.to_str().unwrap()])
            .env("MODELS", &models)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&fs::read(root.join("impact.json")).unwrap()).unwrap()
    };
    let config = init("init");
    assert_eq!(
        config["modules"],
        json!({"app": ["core", "testkit"], "core": [], "testkit": []})
    );
    // One Maven start for the reactor, one for the module it could not match.
    let calls = fs::read_to_string(models.join("calls")).unwrap();
    assert_eq!(calls.lines().count(), 2, "{calls}");
    assert!(calls.lines().last().unwrap().contains("-N"), "{calls}");
}
