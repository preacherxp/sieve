use crate::{Config, Result};
use quick_xml::{events::Event, Reader};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

pub fn default_executable(workspace: &Path, tool: &str) -> String {
    let (wrapper, installed) = if tool == "maven" {
        (if cfg!(windows) { "mvnw.cmd" } else { "mvnw" }, "mvn")
    } else {
        (
            if cfg!(windows) {
                "gradlew.bat"
            } else {
                "gradlew"
            },
            "gradle",
        )
    };
    if workspace.join(wrapper).is_file() {
        workspace.join(wrapper).to_string_lossy().into_owned()
    } else {
        installed.into()
    }
}

pub fn gradle_script() -> Result<tempfile::NamedTempFile> {
    let mut file = tempfile::Builder::new()
        .prefix("impact-")
        .suffix(".gradle")
        .tempfile()?;
    file.write_all(include_bytes!("gradle.init.gradle"))?;
    Ok(file)
}

fn xml_values(xml: &str) -> Result<BTreeMap<String, Vec<String>>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut stack = Vec::new();
    let mut values: BTreeMap<String, Vec<String>> = BTreeMap::new();
    loop {
        match reader.read_event()? {
            Event::Start(tag) => stack.push(String::from_utf8(tag.local_name().as_ref().to_vec())?),
            Event::End(_) => {
                stack.pop();
            }
            Event::Text(text) => {
                let decoded = text.xml_content()?;
                values
                    .entry(stack.join("/"))
                    .or_default()
                    .push(quick_xml::escape::unescape(&decoded)?.into_owned());
            }
            Event::Eof => {
                if !stack.is_empty() {
                    return Err("Unclosed XML element".into());
                }
                break;
            }
            _ => {}
        }
    }
    Ok(values)
}

fn effective_pom(
    workspace: &Path,
    module: &str,
    executable: &str,
) -> Result<BTreeMap<String, Vec<String>>> {
    let file = tempfile::NamedTempFile::new()?;
    let status = Command::new(executable)
        .current_dir(workspace)
        .args(["-B", "-ntp", "-N", "-f"])
        .arg(workspace.join(module).join("pom.xml"))
        .arg("org.apache.maven.plugins:maven-help-plugin:3.5.1:effective-pom")
        .arg(format!("-Doutput={}", file.path().display()))
        .status()?;
    if !status.success() {
        return Err(format!("Cannot read effective Maven model for {module}").into());
    }
    xml_values(&fs::read_to_string(file.path())?)
}

fn coordinate(model: &BTreeMap<String, Vec<String>>) -> Result<String> {
    let group = model
        .get("project/groupId")
        .and_then(|v| v.first())
        .ok_or("Maven model missing groupId")?;
    let artifact = model
        .get("project/artifactId")
        .and_then(|v| v.first())
        .ok_or("Maven model missing artifactId")?;
    Ok(format!("{group}:{artifact}"))
}

fn add_maven_profile(xml: &str, module: &str) -> Result<String> {
    let values = xml_values(xml)?;
    if values
        .get("project/profiles/profile/id")
        .is_some_and(|ids| ids.iter().any(|id| id == "java-test-impact"))
    {
        return Err("A java-test-impact profile already exists; preserve it and restore impact.json instead".into());
    }
    let module = if module == "." { "root" } else { module };
    let profile = format!("\n    <profile>\n      <id>java-test-impact</id>\n      <activation><property><name>impact.skip.{module}</name><value>true</value></property></activation>\n      <properties><maven.test.skip>true</maven.test.skip></properties>\n    </profile>\n");
    let mut reader = Reader::from_str(xml);
    let mut depth = 0;
    let mut insertion = None;
    let mut project_end = None;
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event()? {
            Event::Start(_) => depth += 1,
            Event::Empty(tag) if depth == 1 && tag.local_name().as_ref() == b"profiles" => {
                insertion = Some((
                    start,
                    reader.buffer_position() as usize,
                    format!("<profiles>{profile}</profiles>"),
                ));
            }
            Event::End(tag) => {
                if depth == 2 && tag.local_name().as_ref() == b"profiles" {
                    insertion = Some((start, start, profile.clone()));
                }
                if depth == 1 && tag.local_name().as_ref() == b"project" {
                    project_end = Some(start);
                }
                depth -= 1;
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let end = project_end.ok_or("Expected a Maven project XML element")?;
    let (start, end, text) =
        insertion.unwrap_or((end, end, format!("  <profiles>{profile}  </profiles>\n")));
    let mut result = xml.to_owned();
    result.replace_range(start..end, &text);
    Ok(result)
}

fn maven_config(workspace: &Path, executable: &str) -> Result<(Config, Vec<(PathBuf, String)>)> {
    let root = effective_pom(workspace, ".", executable)?;
    if root.contains_key("project/modules/module")
        && root
            .get("project/packaging")
            .and_then(|v| v.first())
            .map(String::as_str)
            != Some("pom")
    {
        return Err(
            "Automatic setup requires a pom-packaged Maven aggregator or a single JVM package"
                .into(),
        );
    }
    let modules = root
        .get("project/modules/module")
        .cloned()
        .unwrap_or_else(|| vec![".".into()]);
    let mut models = BTreeMap::new();
    for module in modules {
        if module == ".."
            || module.is_empty()
            || !module
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
        {
            return Err("Automatic setup supports a single JVM project or direct child modules in matching directories".into());
        }
        let model = if module == "." {
            root.clone()
        } else {
            effective_pom(workspace, &module, executable)?
        };
        if model.contains_key("project/modules/module") {
            return Err("Nested Maven aggregators need an explicit impact configuration".into());
        }
        models.insert(module, model);
    }
    let mut coordinates = BTreeMap::new();
    for (module, model) in &models {
        if coordinates
            .insert(coordinate(model)?, module.clone())
            .is_some()
        {
            return Err("Duplicate Maven coordinates".into());
        }
    }
    let mut config = Config {
        tool: "maven".into(),
        modules: BTreeMap::new(),
    };
    let mut edits = Vec::new();
    for (module, model) in models {
        let groups = model
            .get("project/dependencies/dependency/groupId")
            .cloned()
            .unwrap_or_default();
        let artifacts = model
            .get("project/dependencies/dependency/artifactId")
            .cloned()
            .unwrap_or_default();
        if groups.len() != artifacts.len() {
            return Err("Incomplete Maven dependency coordinates".into());
        }
        let dependencies = groups
            .iter()
            .zip(artifacts)
            .filter_map(|(g, a)| coordinates.get(&format!("{g}:{a}")).cloned())
            .collect();
        config.modules.insert(module.clone(), dependencies);
        let pom = workspace.join(&module).join("pom.xml");
        edits.push((
            pom.clone(),
            add_maven_profile(&fs::read_to_string(&pom)?, &module)?,
        ));
    }
    Ok((config, edits))
}

pub fn init(args: Vec<String>) -> Result<u8> {
    let mut workspace = PathBuf::from(".");
    let mut tool = None;
    let mut executable = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if matches!(arg.as_str(), "--help" | "-h") {
            println!("java-test-impact init [--workspace PATH] [--tool maven|gradle] [--executable PATH]\n\
                Discovers JVM modules, writes impact.json, and installs Maven skip profiles.\n\
                Existing impact.json is never overwritten. Gradle uses a bundled init script.");
            return Ok(0);
        }
        let value = args
            .next()
            .ok_or_else(|| format!("Missing value for {arg}"))?;
        match arg.as_str() {
            "--workspace" => workspace = value.into(),
            "--tool" => tool = Some(value),
            "--executable" => executable = Some(value),
            _ => return Err(format!("Unknown option: {arg}").into()),
        }
    }
    let workspace = workspace.canonicalize()?;
    let config_path = workspace.join("impact.json");
    if fs::symlink_metadata(&config_path).is_ok() {
        return Err("impact.json already exists; setup refuses to overwrite it".into());
    }
    let tool =
        match tool {
            Some(tool) if matches!(tool.as_str(), "maven" | "gradle") => tool,
            Some(_) => return Err("--tool must be maven or gradle".into()),
            None => match (
                workspace.join("pom.xml").is_file(),
                [
                    "build.gradle",
                    "build.gradle.kts",
                    "settings.gradle",
                    "settings.gradle.kts",
                ]
                .iter()
                .any(|f| workspace.join(f).is_file()),
            ) {
                (true, false) => "maven".into(),
                (false, true) => "gradle".into(),
                _ => return Err(
                    "Cannot unambiguously detect the build tool; specify --tool maven or gradle"
                        .into(),
                ),
            },
        };
    let executable = executable.unwrap_or_else(|| default_executable(&workspace, &tool));
    let (config, edits) = if tool == "maven" {
        maven_config(&workspace, &executable)?
    } else {
        let script = gradle_script()?;
        let output = tempfile::NamedTempFile::new()?;
        let status = Command::new(executable)
            .current_dir(&workspace)
            .args(["--no-daemon", "--console=plain", "--init-script"])
            .arg(script.path())
            .arg(format!("-Pimpact.output={}", output.path().display()))
            .arg("impactInit")
            .status()?;
        if !status.success() {
            return Err("Gradle module discovery failed".into());
        }
        (
            serde_json::from_slice(&fs::read(output.path())?)?,
            Vec::new(),
        )
    };
    // Build discovery and every planned POM edit must succeed before changing project files.
    config.validate(&workspace)?;
    for (path, text) in edits {
        fs::write(path, text)?;
    }
    fs::write(config_path, serde_json::to_string_pretty(&config)? + "\n")?;
    println!("Configured {tool} test selection in {}. Commit impact.json and any POM changes.\nRun: java-test-impact run --workspace {} --base origin/main", workspace.display(), workspace.display());
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maven_profiles_preserve_existing_configuration() -> Result<()> {
        for xml in [
            "<project><artifactId>x</artifactId></project>",
            "<project><profiles/></project>",
            "<project><profiles><profile><id>existing</id></profile></profiles></project>",
        ] {
            let result = add_maven_profile(xml, "pricing")?;
            let values = xml_values(&result)?;
            assert!(values["project/profiles/profile/id"].contains(&"java-test-impact".into()));
            assert_eq!(
                values["project/profiles/profile/activation/property/name"],
                ["impact.skip.pricing"]
            );
            assert!(add_maven_profile(&result, "pricing").is_err());
            if xml.contains("existing") {
                assert!(result.contains("<id>existing</id>"));
            }
        }
        assert!(add_maven_profile("<project>", ".").is_err());
        Ok(())
    }
}
