//! Native Maven and Gradle test-report parsing, shared by execution and verification.
use crate::Result;
use quick_xml::{events::Event, Reader};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Default, Serialize)]
pub(crate) struct Reports {
    pub(crate) executed: BTreeSet<String>,
    pub(crate) failed: BTreeSet<String>,
    pub(crate) errors: BTreeSet<String>,
    pub(crate) skipped: BTreeSet<String>,
    pub(crate) cases: usize,
    #[serde(skip)]
    seen: BTreeSet<(String, String)>,
}

fn parse_report(path: &Path, prefix: &str, reports: &mut Reports) -> Result<()> {
    let mut reader = Reader::from_file(path)?;
    reader.config_mut().expand_empty_elements = true;
    let mut buffer = Vec::new();
    let mut case = None;
    let mut depth = 0usize;
    let (mut failed, mut skipped, mut error) = (false, false, false);
    let mut root_seen = false;
    loop {
        let event = reader.read_event_into(&mut buffer)?;
        match &event {
            Event::Start(tag) => {
                if depth == 0 {
                    if root_seen || !matches!(tag.name().as_ref(), b"testsuite" | b"testsuites") {
                        return Err("Expected a single testsuite/testsuites XML root".into());
                    }
                    root_seen = true;
                }
                depth += 1;
            }
            Event::End(_) => depth = depth.checked_sub(1).ok_or("Unexpected XML closing tag")?,
            Event::Eof if depth != 0 => return Err("Truncated XML report".into()),
            _ => {}
        }
        match event {
            Event::Start(tag) if tag.name().as_ref() == b"testcase" => {
                if case.is_some() {
                    return Err("Nested testcase in XML report".into());
                }
                let class = tag
                    .try_get_attribute(b"classname")?
                    .ok_or("Testcase missing classname")?
                    .decode_and_unescape_value(reader.decoder())?
                    .into_owned();
                let id = format!("{prefix}:{class}");
                let name = tag
                    .try_get_attribute(b"name")?
                    .ok_or("Testcase missing name")?
                    .decode_and_unescape_value(reader.decoder())?
                    .into_owned();
                if class.is_empty() || name.is_empty() || !reports.seen.insert((id.clone(), name)) {
                    return Err("Empty or duplicate test identity in XML reports".into());
                }
                case = Some(id);
                (failed, skipped, error) = (false, false, false);
            }
            Event::Start(tag) if case.is_some() => match tag.name().as_ref() {
                b"failure" => failed = true,
                b"error" => {
                    failed = true;
                    error = true;
                }
                b"skipped" => skipped = true,
                _ => {}
            },
            Event::End(tag) if tag.name().as_ref() == b"testcase" => {
                let id = case.take().ok_or("Unexpected testcase end")?;
                if error {
                    reports.errors.insert(id.clone());
                }
                if skipped {
                    reports.skipped.insert(id);
                } else {
                    reports.cases += 1;
                    reports.executed.insert(id.clone());
                    if failed {
                        reports.failed.insert(id);
                    }
                }
            }
            Event::Eof => {
                if case.is_some() || !root_seen {
                    return Err("Truncated XML testcase".into());
                }
                break;
            }
            _ => {}
        }
        buffer.clear();
    }
    Ok(())
}

pub(crate) fn read_reports(workspace: &Path, tool: &str) -> Result<Reports> {
    let mut reports = Reports::default();
    for (prefix, path) in report_files(workspace, tool)? {
        parse_report(&path, &prefix, &mut reports)?;
    }
    Ok(reports)
}

/// What a test class cost in the reports of the last build: executed test cases and seconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, serde::Deserialize)]
pub(crate) struct Cost {
    pub(crate) cases: usize,
    pub(crate) seconds: f64,
}

/// Costs per top-level test class (binary name without nested parts): the executed
/// `testcase` elements, since Surefire's suite totals leave out `@Nested` classes, and the
/// outermost suite's time, which includes class setup such as starting a Spring context.
/// Gradle reports nested classes separately; they add to their outer class.
pub(crate) fn class_costs(workspace: &Path, tool: &str) -> Result<BTreeMap<String, Cost>> {
    let mut costs: BTreeMap<String, Cost> = BTreeMap::new();
    for (_, path) in report_files(workspace, tool)? {
        let mut reader = Reader::from_file(&path)?;
        let mut buffer = Vec::new();
        let mut class = None;
        let (mut cases, mut seconds) = (0, 0.0);
        let (mut in_case, mut skipped) = (false, false);
        loop {
            match reader.read_event_into(&mut buffer)? {
                Event::Start(tag) | Event::Empty(tag)
                    if tag.name().as_ref() == b"testsuite" && class.is_none() =>
                {
                    let attribute = |name: &[u8]| -> Result<String> {
                        Ok(match tag.try_get_attribute(name)? {
                            Some(value) => {
                                value.decode_and_unescape_value(reader.decoder())?.into()
                            }
                            None => String::new(),
                        })
                    };
                    let name = attribute(b"name")?;
                    class = name.split('$').next().map(str::to_owned);
                    seconds = attribute(b"time")?
                        .replace(',', "")
                        .parse::<f64>()
                        .unwrap_or(0.0);
                }
                Event::Start(tag) if tag.name().as_ref() == b"testcase" => {
                    (in_case, skipped) = (true, false);
                }
                Event::Empty(tag) if tag.name().as_ref() == b"testcase" => cases += 1,
                Event::Start(tag) | Event::Empty(tag)
                    if in_case && tag.name().as_ref() == b"skipped" =>
                {
                    skipped = true;
                }
                Event::End(tag) if tag.name().as_ref() == b"testcase" => {
                    in_case = false;
                    if !skipped {
                        cases += 1;
                    }
                }
                Event::Eof => break,
                _ => {}
            }
            buffer.clear();
        }
        if let Some(class) = class.filter(|c| !c.is_empty()) {
            let cost = costs.entry(class).or_default();
            cost.cases += cases;
            cost.seconds += seconds;
        }
    }
    Ok(costs)
}

/// Every native XML report below the workspace, with its `module:suite` prefix.
fn report_files(workspace: &Path, tool: &str) -> Result<Vec<(String, PathBuf)>> {
    let maven = match tool {
        "maven" => true,
        "gradle" => false,
        _ => return Err(format!("Unknown build tool: {tool}").into()),
    };
    let mut files = Vec::new();
    let mut modules = vec![".".to_owned()];
    for entry in fs::read_dir(workspace)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() && !entry.file_name().to_string_lossy().starts_with('.') {
            modules.push(
                entry
                    .file_name()
                    .into_string()
                    .map_err(|_| "Non-UTF-8 module name")?,
            );
        }
    }
    for module in modules {
        let mut folders = Vec::new();
        if maven {
            for (suite, folder) in [
                ("unit", "surefire-reports"),
                ("integration", "failsafe-reports"),
            ] {
                folders.push((
                    suite.to_owned(),
                    workspace.join(&module).join("target").join(folder),
                ));
            }
        } else {
            let base = workspace.join(&module).join("build/test-results");
            if base.is_dir() {
                for entry in fs::read_dir(base)? {
                    let entry = entry?;
                    if entry.file_type()?.is_dir() {
                        let name = entry.file_name().to_string_lossy().into_owned();
                        let suite = match name.as_str() {
                            "test" => "unit",
                            "integrationTest" => "integration",
                            _ => &name,
                        };
                        folders.push((suite.to_owned(), entry.path()));
                    }
                }
            }
        }
        for (suite, folder) in folders {
            if !folder.exists() {
                continue;
            }
            for entry in fs::read_dir(folder)? {
                let entry = entry?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with("TEST-") && name.ends_with(".xml") {
                    files.push((format!("{module}:{suite}"), entry.path()));
                }
            }
        }
    }
    Ok(files)
}
