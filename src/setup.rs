use crate::{Config, Result};
use quick_xml::{events::Event, Reader};
use std::{
    collections::{BTreeMap, BTreeSet},
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

pub const GRADLE_SCRIPT: &[u8] = include_bytes!("gradle.init.gradle");

pub fn gradle_script() -> Result<tempfile::NamedTempFile> {
    let mut file = tempfile::Builder::new()
        .prefix("impact-")
        .suffix(".gradle")
        .tempfile()?;
    file.write_all(GRADLE_SCRIPT)?;
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

/// The effective model of `module`, and with `reactor` those of every module below it, which
/// Maven then wraps in `<projects>`.
fn effective_pom(
    workspace: &Path,
    module: &str,
    reactor: bool,
    executable: &str,
    extra: &[String],
) -> Result<String> {
    let file = tempfile::NamedTempFile::new()?;
    let mut command = Command::new(executable);
    command.current_dir(workspace).args(["-B", "-ntp"]);
    if !reactor {
        command.arg("-N");
    }
    let status = command
        .arg("-f")
        .arg(workspace.join(module).join("pom.xml"))
        .arg("org.apache.maven.plugins:maven-help-plugin:3.5.1:effective-pom")
        .arg(format!("-Doutput={}", file.path().display()))
        .args(extra)
        // Build output is a log; `select` keeps standard output for the selection.
        .stdout(std::io::stderr())
        .status()?;
    if !status.success() {
        return Err(format!("Cannot read effective Maven model for {module}").into());
    }
    Ok(fs::read_to_string(file.path())?)
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

pub(crate) fn install_maven_adapter(xml: &str, module: &str, refresh: bool) -> Result<String> {
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

/// Sibling modules that a module's build uses other than as dependencies: as a build plugin,
/// a plugin dependency, an annotation processor path, or an unpacked artifact, and the
/// modules whose directories its build configuration names, such as a shared OpenAPI
/// specification. A change there changes what the module builds.
fn build_uses(
    model: &BTreeMap<String, Vec<String>>,
    coordinates: &BTreeMap<String, String>,
    workspace: &Path,
    module: &str,
) -> BTreeSet<String> {
    let modules: BTreeSet<&str> = coordinates.values().map(String::as_str).collect();
    let mut uses = BTreeSet::new();
    // Plugin management only declares plugins; the modules' own plugin sections use them.
    let build = model.iter().filter(|(key, _)| {
        (key.starts_with("project/build/") || key.starts_with("project/reporting/"))
            && !key.starts_with("project/build/pluginManagement/")
    });
    for (key, values) in build.clone() {
        let Some(element) = key.strip_suffix("/artifactId") else {
            continue;
        };
        let groups = model.get(&format!("{element}/groupId"));
        for (i, artifact) in values.iter().enumerate() {
            // Without a matching group list, any group may be meant.
            let group = groups.filter(|g| g.len() == values.len()).map(|g| &g[i]);
            uses.extend(
                coordinates
                    .iter()
                    .filter(|(coordinate, _)| match group {
                        Some(group) => **coordinate == format!("{group}:{artifact}"),
                        None => coordinate.ends_with(&format!(":{artifact}")),
                    })
                    .map(|(_, module)| module.clone()),
            );
        }
    }
    let base = workspace.join(module);
    let roots = [
        workspace.to_path_buf(),
        workspace.canonicalize().unwrap_or_default(),
    ];
    for value in build.flat_map(|(_, values)| values) {
        for word in value.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';')) {
            if !word.contains('/') {
                continue;
            }
            // Lexically, as the path may not exist yet.
            let mut path = PathBuf::new();
            for part in base.join(word).components() {
                match part {
                    std::path::Component::ParentDir => {
                        path.pop();
                    }
                    std::path::Component::CurDir => {}
                    part => path.push(part),
                }
            }
            let Some(relative) = roots.iter().find_map(|root| path.strip_prefix(root).ok()) else {
                continue;
            };
            let mut parts = relative.iter().filter_map(|part| part.to_str());
            match parts.next() {
                Some(owner) if modules.contains(owner) => {
                    uses.insert(owner.to_owned());
                }
                Some("src") if modules.contains(".") => {
                    uses.insert(".".to_owned());
                }
                _ => {}
            }
        }
    }
    uses
}

/// The module graph from the effective models: one Maven start reports the whole reactor,
/// matched to module directories by the artifact ID each module's POM declares. A module
/// whose model cannot be matched that way is read on its own.
fn maven_graph(workspace: &Path, executable: &str, extra: &[String]) -> Result<Config> {
    let output = effective_pom(workspace, ".", true, executable, extra)?;
    let reactor: Vec<&str> = element_ranges(&output, &["projects", "project"])?
        .into_iter()
        .map(|(start, end)| &output[start..end])
        .collect();
    let artifact = |xml: &str| -> Option<String> {
        xml_values(xml)
            .ok()?
            .get("project/artifactId")?
            .first()
            .cloned()
    };
    let ids: Vec<Option<String>> = reactor.iter().map(|xml| artifact(xml)).collect();
    let model = |module: &str| -> Result<String> {
        let declared = fs::read_to_string(workspace.join(module).join("pom.xml"))
            .ok()
            .and_then(|xml| artifact(&xml))
            .filter(|id| !id.contains('$'));
        let mut found = (0..reactor.len()).filter(|&i| declared.is_some() && ids[i] == declared);
        match (found.next(), found.next()) {
            (Some(i), None) => Ok(reactor[i].to_owned()),
            _ => effective_pom(workspace, module, false, executable, extra),
        }
    };
    let root_xml = if reactor.is_empty() {
        output.clone()
    } else {
        model(".")?
    };
    let root = xml_values(&root_xml)?;
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
        let xml = if module == "." {
            root_xml.clone()
        } else {
            model(&module)?
        };
        let values = xml_values(&xml)?;
        if values.contains_key("project/modules/module") {
            return Err("Nested Maven aggregators need an explicit impact configuration".into());
        }
        models.insert(module, (xml, values));
    }
    let mut coordinates = BTreeMap::new();
    for (module, (_, values)) in &models {
        if coordinates
            .insert(coordinate(values)?, module.clone())
            .is_some()
        {
            return Err("Duplicate Maven coordinates".into());
        }
    }
    let mut graph: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (module, (xml, values)) in &models {
        let mut dependencies = BTreeSet::new();
        for (start, end) in element_ranges(xml, &["project", "dependencies", "dependency"])? {
            let dependency = xml_values(&xml[start..end])?;
            let value = |key: &str| {
                dependency
                    .get(&format!("dependency/{key}"))
                    .and_then(|v| v.first())
                    .map(String::as_str)
            };
            let (Some(group), Some(artifact)) = (value("groupId"), value("artifactId")) else {
                return Err("Incomplete Maven dependency coordinates".into());
            };
            if let Some(sibling) = coordinates.get(&format!("{group}:{artifact}")) {
                dependencies.insert(sibling.clone());
            }
        }
        dependencies.extend(build_uses(values, &coordinates, workspace, module));
        dependencies.remove(module);
        graph.insert(module.clone(), dependencies.into_iter().collect());
    }
    Ok(serde_json::from_value(serde_json::json!({
        "tool": "maven", "modules": graph,
    }))?)
}

fn gradle_graph(workspace: &Path, executable: &str, extra: &[String]) -> Result<Config> {
    let script = gradle_script()?;
    let output = tempfile::NamedTempFile::new()?;
    let status = Command::new(executable)
        .current_dir(workspace)
        // Discovery reads the project model at execution time.
        .args([
            "--no-daemon",
            "--console=plain",
            "--no-configuration-cache",
            "--init-script",
        ])
        .arg(script.path())
        .arg(format!("-Pimpact.output={}", output.path().display()))
        .arg("impactInit")
        .args(extra)
        .stdout(std::io::stderr())
        .status()?;
    if !status.success() {
        return Err("Gradle module discovery failed".into());
    }
    Ok(serde_json::from_slice(&fs::read(output.path())?)?)
}

/// The module graph the build declares now.
fn graph(workspace: &Path, tool: &str, executable: &str, extra: &[String]) -> Result<Config> {
    if tool == "maven" {
        maven_graph(workspace, executable, extra)
    } else {
        gradle_graph(workspace, executable, extra)
    }
}

/// The build arguments that shape the project model: profiles, properties, settings, init
/// scripts, and offline mode. Goals, tasks, and their options would run work.
fn model_args(tool: &str, extra: &[String]) -> Vec<String> {
    let (valued, flags): (&[&str], &[&str]) = if tool == "gradle" {
        (
            &[
                "-P",
                "-D",
                "--project-prop",
                "--system-prop",
                "-I",
                "--init-script",
                "-g",
                "--gradle-user-home",
            ],
            &["--offline"],
        )
    } else {
        (
            &[
                "-P",
                "-D",
                "--activate-profiles",
                "--define",
                "-s",
                "--settings",
                "-gs",
                "--global-settings",
            ],
            &["-o", "--offline"],
        )
    };
    let mut kept = Vec::new();
    let mut args = extra.iter();
    while let Some(arg) = args.next() {
        if valued.contains(&arg.as_str()) {
            kept.push(arg.clone());
            kept.extend(args.next().cloned());
        } else if flags.contains(&arg.as_str())
            || (arg.len() > 2 && (arg.starts_with("-P") || arg.starts_with("-D")))
            || valued
                .iter()
                .any(|v| v.starts_with("--") && arg.starts_with(&format!("{v}=")))
        {
            kept.push(arg.clone());
        }
    }
    kept
}

/// Why `config` no longer describes the build's module graph, or `None` when it still declares
/// every module and dependency edge the build reports. Additional declared edges are kept on
/// purpose, as `refresh` keeps them.
pub fn stale(
    config: &Config,
    workspace: &Path,
    executable: &str,
    extra: &[String],
) -> Result<Option<String>> {
    let found = graph(
        workspace,
        &config.tool,
        executable,
        &model_args(&config.tool, extra),
    )?;
    if found.modules.keys().ne(config.modules.keys()) {
        return Ok(Some("modules were added or removed".into()));
    }
    for (module, dependencies) in &found.modules {
        if let Some(dependency) = dependencies
            .iter()
            .find(|d| !config.modules[module].contains(d))
        {
            return Ok(Some(format!("{module} now depends on {dependency}")));
        }
    }
    Ok(None)
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
    if tool == "maven" {
        check_execution_skips(&fs::read_to_string(workspace.join("pom.xml"))?)?;
    }
    let mut config = graph(&workspace, &tool, &executable, &extra)?;
    let mut edits = Vec::new();
    if tool == "maven" {
        for module in config.modules.keys() {
            let pom = workspace.join(module).join("pom.xml");
            let xml = fs::read_to_string(&pom)?;
            edits.push((pom, install_maven_adapter(&xml, module, refresh)?));
        }
    }
    if let Some(previous) = previous {
        if previous.tool != config.tool {
            return Err("refresh cannot change the build tool".into());
        }
        config.ignore = previous.ignore;
        config.class_level = previous.class_level;
        config.generated = previous.generated;
        config.records = previous.records;
        config.record_env = previous.record_env;
        config.record_ignore_properties = previous.record_ignore_properties;
        config.with = previous.with;
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

    #[test]
    fn discovery_keeps_only_the_arguments_that_shape_the_model() {
        let args = |list: &[&str]| list.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        let maven = args(&[
            "-Pci",
            "-T",
            "4",
            "jacoco:report",
            "-s",
            "settings.xml",
            "-D",
            "a=b",
            "-Dx=y",
            "--offline",
            "-pl",
            "core",
            "--settings=other.xml",
        ]);
        assert_eq!(
            model_args("maven", &maven),
            args(&[
                "-Pci",
                "-s",
                "settings.xml",
                "-D",
                "a=b",
                "-Dx=y",
                "--offline",
                "--settings=other.xml"
            ])
        );
        let gradle = args(&[
            "test",
            "--tests",
            "Foo",
            "-Pci",
            "-x",
            "lint",
            "--offline",
            "-I",
            "init.gradle",
            "--init-script=more.gradle",
            "--continue",
        ]);
        assert_eq!(
            model_args("gradle", &gradle),
            args(&[
                "-Pci",
                "--offline",
                "-I",
                "init.gradle",
                "--init-script=more.gradle"
            ])
        );
    }
}
