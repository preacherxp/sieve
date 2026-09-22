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
    extra: &[String],
) -> Result<BTreeMap<String, Vec<String>>> {
    let file = tempfile::NamedTempFile::new()?;
    let status = Command::new(executable)
        .current_dir(workspace)
        .args(["-B", "-ntp", "-N", "-f"])
        .arg(workspace.join(module).join("pom.xml"))
        .arg("org.apache.maven.plugins:maven-help-plugin:3.5.1:effective-pom")
        .arg(format!("-Doutput={}", file.path().display()))
        .args(extra)
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

// Byte ranges let us change only Sieve-owned values and preserve unrelated XML.
fn element_ranges(xml: &str, path: &[&str]) -> Result<Vec<(usize, usize)>> {
    let mut reader = Reader::from_str(xml);
    let mut stack: Vec<(String, usize)> = Vec::new();
    let mut ranges = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event()? {
            Event::Start(tag) => stack.push((
                String::from_utf8(tag.local_name().as_ref().to_vec())?,
                start,
            )),
            Event::Empty(tag) => {
                let name = String::from_utf8(tag.local_name().as_ref().to_vec())?;
                if stack
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .chain(std::iter::once(name.as_str()))
                    .eq(path.iter().copied())
                {
                    ranges.push((start, reader.buffer_position() as usize));
                }
            }
            Event::End(_) => {
                if stack
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .eq(path.iter().copied())
                {
                    ranges.push((
                        stack.last().ok_or("Unbalanced XML")?.1,
                        reader.buffer_position() as usize,
                    ));
                }
                stack.pop().ok_or("Unbalanced XML")?;
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !stack.is_empty() {
        return Err("Unclosed XML element".into());
    }
    Ok(ranges)
}

fn append_xml(xml: &str, path: &[&str], child: &str) -> Result<String> {
    let ranges = element_ranges(xml, path)?;
    if ranges.len() > 1 {
        return Err("Ambiguous Maven XML container".into());
    }
    let Some(&(start, end)) = ranges.first() else {
        let (name, parent) = path.split_last().ok_or("Missing XML root")?;
        return append_xml(xml, parent, &format!("<{name}>{child}</{name}>"));
    };
    let node = &xml[start..end];
    let mut result = xml.to_owned();
    if node.ends_with("/>") {
        let name = node[1..]
            .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .next()
            .ok_or("Missing XML name")?;
        result.replace_range(end - 2..end, &format!(">{child}</{name}>"));
    } else {
        let closing = start + node.rfind("</").ok_or("Missing XML closing tag")?;
        result.insert_str(closing, child);
    }
    Ok(result)
}

fn set_xml(xml: &str, path: &[&str], value: &str) -> Result<String> {
    let (name, parent) = path.split_last().ok_or("Missing XML path")?;
    let node = format!("<{name}>{value}</{name}>");
    let ranges = element_ranges(xml, path)?;
    match ranges.as_slice() {
        [] => append_xml(xml, parent, &node),
        [(start, end)] => {
            let mut result = xml.to_owned();
            result.replace_range(*start..*end, &node);
            Ok(result)
        }
        _ => Err("Duplicate Maven configuration element".into()),
    }
}

fn check_execution_skips(xml: &str) -> Result<()> {
    // Effective models copy plugin settings into executions. Inspect declarations
    // instead, so ordinary inherited plugin configuration remains supported.
    if xml_values(xml)?.keys().any(|key| {
        key.starts_with("project/build/plugins/plugin/executions/execution/configuration/")
            && matches!(
                key.rsplit('/').next(),
                Some("skipTests" | "skipITs" | "skip")
            )
    }) {
        return Err("Execution-specific test skipping requires manual review; move it to plugin configuration before setup".into());
    }
    Ok(())
}

fn install_maven_adapter(xml: &str, module: &str, refresh: bool) -> Result<String> {
    check_execution_skips(xml)?;
    let module = if module == "." { "root" } else { module };
    let property = format!("impact.skip.{module}");
    let mut result = xml.to_owned();
    // Migrate only the old, generated profile. Refuse custom additions instead of deleting them.
    for (start, end) in element_ranges(xml, &["project", "profiles", "profile"])?
        .into_iter()
        .rev()
    {
        let values = xml_values(&xml[start..end])?;
        if values
            .get("profile/id")
            .is_some_and(|ids| ids.iter().any(|id| id == "java-test-impact"))
        {
            if !refresh {
                return Err("A java-test-impact profile already exists; restore impact.json and use refresh".into());
            }
            if values.get("profile/activation/property/name") != Some(&vec![property.clone()])
                || values.get("profile/activation/property/value") != Some(&vec!["true".into()])
                || values.iter().any(|(key, value)| {
                    key.starts_with("profile/properties/") && value != &["true"]
                })
            {
                return Err("Review custom java-test-impact profile before migrating it".into());
            }
            if values.keys().any(|key| {
                !matches!(
                    key.as_str(),
                    "profile/id"
                        | "profile/activation/property/name"
                        | "profile/activation/property/value"
                        | "profile/properties/maven.test.skip"
                        | "profile/properties/skipTests"
                )
            }) {
                return Err("Review custom java-test-impact profile before migrating it".into());
            }
            result.replace_range(start..end, "");
        }
    }
    if !refresh && xml_values(&result)?.contains_key(&format!("project/properties/{property}")) {
        return Err("A Sieve adapter already exists; restore impact.json and use refresh".into());
    }
    result = set_xml(&result, &["project", "properties", &property], "false")?;
    for artifact in ["maven-surefire-plugin", "maven-failsafe-plugin"] {
        let mut found = false;
        for (start, end) in element_ranges(&result, &["project", "build", "plugins", "plugin"])?
            .into_iter()
            .rev()
        {
            let values = xml_values(&result[start..end])?;
            if values
                .get("plugin/artifactId")
                .is_some_and(|v| v == &[artifact])
            {
                if found {
                    return Err("Duplicate Maven test plugin".into());
                }
                found = true;
                let plugin = set_xml(
                    &result[start..end],
                    &["plugin", "configuration", "skipTests"],
                    &format!("${{{property}}}"),
                )?;
                result.replace_range(start..end, &plugin);
            }
        }
        if !found {
            result = append_xml(&result, &["project", "build", "plugins"], &format!("<plugin><groupId>org.apache.maven.plugins</groupId><artifactId>{artifact}</artifactId><configuration><skipTests>${{{property}}}</skipTests></configuration></plugin>"))?;
        }
    }
    Ok(result)
}

fn maven_config(
    workspace: &Path,
    executable: &str,
    refresh: bool,
    extra: &[String],
) -> Result<(Config, Vec<(PathBuf, String)>)> {
    check_execution_skips(&fs::read_to_string(workspace.join("pom.xml"))?)?;
    let root = effective_pom(workspace, ".", executable, extra)?;
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
            effective_pom(workspace, &module, executable, extra)?
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
        build_fingerprint: None,
        ignore: None,
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
        let xml = fs::read_to_string(&pom)?;
        edits.push((pom, install_maven_adapter(&xml, &module, refresh)?));
    }
    Ok((config, edits))
}

pub fn init(args: Vec<String>, refresh: bool) -> Result<u8> {
    let mut workspace = PathBuf::from(".");
    let mut tool = None;
    let mut executable = None;
    let mut extra = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--" {
            extra.extend(args);
            break;
        }
        if matches!(arg.as_str(), "--help" | "-h") {
            println!(
                "sieve init|refresh [--workspace PATH] [--tool maven|gradle] [--executable PATH] [-- BUILD_FLAGS]\n\
                Discovers JVM modules, writes impact.json, and installs Maven test plugin properties.\n\
                init refuses existing configuration; refresh rediscovers the graph and preserves additional declared edges."
            );
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
    if !refresh && fs::symlink_metadata(&config_path).is_ok() {
        return Err("impact.json already exists; setup refuses to overwrite it".into());
    }
    let previous: Option<Config> = if refresh {
        Some(serde_json::from_slice(&fs::read(&config_path)?)?)
    } else {
        None
    };
    if tool.is_none() {
        tool = previous.as_ref().map(|c| c.tool.clone());
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
    let (mut config, edits) = if tool == "maven" {
        maven_config(&workspace, &executable, refresh, &extra)?
    } else {
        let script = gradle_script()?;
        let output = tempfile::NamedTempFile::new()?;
        let status = Command::new(executable)
            .current_dir(&workspace)
            .args(["--no-daemon", "--console=plain", "--init-script"])
            .arg(script.path())
            .arg(format!("-Pimpact.output={}", output.path().display()))
            .arg("impactInit")
            .args(&extra)
            .status()?;
        if !status.success() {
            return Err("Gradle module discovery failed".into());
        }
        (
            serde_json::from_slice(&fs::read(output.path())?)?,
            Vec::new(),
        )
    };
    if let Some(previous) = previous {
        if previous.tool != config.tool {
            return Err("refresh cannot change the build tool".into());
        }
        config.ignore = previous.ignore;
        let known: std::collections::BTreeSet<_> = config.modules.keys().cloned().collect();
        for (module, dependencies) in &mut config.modules {
            dependencies.extend(
                previous
                    .modules
                    .get(module)
                    .into_iter()
                    .flatten()
                    .filter(|d| known.contains(*d))
                    .cloned(),
            );
            dependencies.sort();
            dependencies.dedup();
        }
    }
    // Build discovery and every planned POM edit must succeed before changing project files.
    config.validate(&workspace)?;
    for (path, text) in edits {
        fs::write(path, text)?;
    }
    config.build_fingerprint = Some(crate::fingerprint::build_inputs(&workspace)?);
    fs::write(config_path, serde_json::to_string_pretty(&config)? + "\n")?;
    println!("Configured {tool} test selection in {}. Commit impact.json and any POM changes.\nRun: sieve run --workspace {} --base origin/main", workspace.display(), workspace.display());
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maven_adapter_preserves_profiles_and_existing_configuration() -> Result<()> {
        for xml in [
            "<project><artifactId>x</artifactId></project>",
            "<project><profiles/></project>",
            "<project><profiles><profile><id>existing</id></profile></profiles></project>",
        ] {
            let result = install_maven_adapter(xml, "pricing", false)?;
            let values = xml_values(&result)?;
            assert_eq!(values["project/properties/impact.skip.pricing"], ["false"]);
            assert_eq!(
                values["project/build/plugins/plugin/configuration/skipTests"],
                ["${impact.skip.pricing}", "${impact.skip.pricing}"]
            );
            assert!(install_maven_adapter(&result, "pricing", false).is_err());
            assert_eq!(install_maven_adapter(&result, "pricing", true)?, result);
            if xml.contains("existing") {
                assert!(result.contains("<id>existing</id>"));
            }
        }
        let legacy = "<project><profiles><profile><id>java-test-impact</id><activation><property><name>impact.skip.pricing</name><value>true</value></property></activation><properties><maven.test.skip>true</maven.test.skip></properties></profile></profiles></project>";
        assert!(install_maven_adapter(legacy, "pricing", false).is_err());
        let migrated = install_maven_adapter(legacy, "pricing", true)?;
        assert!(!migrated.contains("<maven.test.skip>"));
        assert!(!migrated.contains("java-test-impact"));
        assert!(install_maven_adapter(
            &legacy.replace("impact.skip.pricing", "custom"),
            "pricing",
            true
        )
        .is_err());
        assert!(install_maven_adapter("<project><build><plugins><plugin><artifactId>maven-surefire-plugin</artifactId><executions><execution><configuration><skipTests>false</skipTests></configuration></execution></executions></plugin></plugins></build></project>", ".", false).is_err());
        assert!(install_maven_adapter("<project>", ".", false).is_err());
        Ok(())
    }
}
