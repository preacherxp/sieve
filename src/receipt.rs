//! Conservative execution receipts for a later run of the remaining tests.
use crate::{observation::top_level, reports::Reports, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::{ChildStdin, Command, Stdio},
};

pub(crate) type Inventory = BTreeMap<String, String>;
pub(crate) type Exclusions = BTreeMap<String, BTreeSet<String>>;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct Inputs {
    fingerprint: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Receipt {
    version: u8,
    inputs: Inputs,
    inventory: Inventory,
    completed: BTreeSet<String>,
}

fn digest(bytes: &[u8]) -> Result<String> {
    hash_stream(|input| {
        input.write_all(bytes)?;
        Ok(())
    })
}

fn hash_stream(write: impl FnOnce(&mut ChildStdin) -> Result<()>) -> Result<String> {
    let mut child = Command::new("git")
        .args(["hash-object", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut input = child.stdin.take().ok_or("Missing fingerprint input")?;
    let written = write(&mut input);
    drop(input);
    let result = child.wait_with_output()?;
    written?;
    if !result.status.success() {
        return Err("Cannot fingerprint receipt inputs".into());
    }
    Ok(String::from_utf8(result.stdout)?.trim().to_owned())
}

fn file_digest(files: &BTreeMap<String, PathBuf>) -> Result<String> {
    hash_stream(|input| {
        for (name, path) in files {
            let mut file = fs::File::open(path)?;
            let length = file.metadata()?.len();
            input.write_all(&(name.len() as u64).to_be_bytes())?;
            input.write_all(name.as_bytes())?;
            input.write_all(&length.to_be_bytes())?;
            if std::io::copy(&mut file, input)? != length {
                return Err("Receipt input changed while fingerprinting".into());
            }
        }
        Ok(())
    })
}

fn version(workspace: &Path, executable: &str) -> Result<Vec<u8>> {
    let output = Command::new(executable)
        .arg("--version")
        .current_dir(workspace)
        .output()?;
    if !output.status.success() {
        return Err(format!("Cannot verify toolchain: {executable} --version").into());
    }
    let mut bytes = output.stdout;
    bytes.extend(output.stderr);
    Ok(bytes)
}

/// Bind the receipt to this checkout, build invocation, toolchain and environment.
/// Artifact paths are excluded only when Git does not track them.
pub(crate) fn capture(
    workspace: &Path,
    executable: &str,
    arguments: &[String],
    artifacts: &[PathBuf],
) -> Result<Inputs> {
    let tracked = crate::git(workspace, &["ls-files", "-z"])?;
    let mut paths = tracked.clone();
    paths.extend(crate::git(
        workspace,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )?);
    let tracked: BTreeSet<&[u8]> = tracked
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .collect();
    let mut files = BTreeMap::new();
    let mut missing = BTreeSet::new();
    for path in paths.split(|b| *b == 0).filter(|p| !p.is_empty()) {
        let name = std::str::from_utf8(path)?;
        let file = workspace.join(name);
        if !tracked.contains(path) && artifacts.iter().any(|p| p == &file) {
            continue;
        }
        match fs::symlink_metadata(&file) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!("Cannot verify linked receipt input: {name}").into());
            }
            Ok(meta) if meta.is_dir() => {
                return Err(format!("Cannot verify submodule receipt input: {name}").into())
            }
            Ok(_) => {
                files.insert(name.to_owned(), file);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.insert(name.to_owned());
            }
            Err(error) => return Err(error.into()),
        }
    }
    let environment: BTreeMap<String, String> = env::vars()
        .filter(|(key, _)| !matches!(key.as_str(), "_" | "SHLVL" | "OLDPWD"))
        .collect();
    for variable in [
        "JAVA_TOOL_OPTIONS",
        "JDK_JAVA_OPTIONS",
        "MAVEN_OPTS",
        "GRADLE_OPTS",
    ] {
        for option in environment
            .get(variable)
            .into_iter()
            .flat_map(|value| value.split_whitespace())
        {
            let option = option.trim_matches(['\'', '"']);
            if let Some(agent) = option.strip_prefix("-javaagent:") {
                let path = workspace.join(agent.split('=').next().unwrap_or(agent));
                files.insert(format!("@{variable}:{agent}"), path);
            }
        }
    }
    let current_exe = env::current_exe()?;
    let build_exe = if executable.contains(std::path::MAIN_SEPARATOR) {
        workspace.join(executable)
    } else {
        env::split_paths(&env::var_os("PATH").unwrap_or_default())
            .map(|dir| dir.join(executable))
            .find(|path| path.is_file())
            .ok_or("Cannot resolve the receipt's build executable")?
    }
    .canonicalize()?;
    let payload = serde_json::to_vec(&(
        workspace.canonicalize()?,
        crate::git(workspace, &["rev-parse", "HEAD"])?,
        file_digest(&files)?,
        missing,
        crate::fingerprint::build_inputs(workspace)?,
        arguments,
        executable,
        version(workspace, executable)?,
        version(workspace, "java")?,
        environment,
        file_digest(&BTreeMap::from([
            ("sieve".into(), current_exe),
            ("build".into(), build_exe),
        ]))?,
    ))?;
    Ok(Inputs {
        fingerprint: digest(&payload)?,
    })
}

/// Inventory uses native module:suite identities. It hashes all files in test output
/// directories, including resources and nested classes, not just the selected classes.
pub(crate) fn inventory(suites: &BTreeMap<String, Vec<PathBuf>>) -> Result<Inventory> {
    fn collect(
        dir: &Path,
        relative: &str,
        entries: &mut BTreeMap<String, PathBuf>,
        directories: &mut BTreeSet<String>,
    ) -> Result<()> {
        if !dir.exists() {
            if dir
                .extension()
                .is_some_and(|extension| extension == "jar" || extension == "zip")
            {
                return Err(format!("Missing runtime dependency: {}", dir.display()).into());
            }
            return Ok(());
        }
        if dir.is_file() {
            entries.insert(relative.into(), dir.to_owned());
            return Ok(());
        }
        directories.insert(relative.into());
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let name = format!(
                "{relative}/{}",
                entry.file_name().to_str().ok_or("Non-UTF-8 test output")?
            );
            let kind = entry.file_type()?;
            if kind.is_dir() {
                collect(&entry.path(), &name, entries, directories)?;
            } else if kind.is_file() {
                entries.insert(name, entry.path());
            } else {
                return Err("Cannot verify linked test output".into());
            }
        }
        Ok(())
    }
    suites
        .iter()
        .map(|(suite, dirs)| {
            let mut entries = BTreeMap::new();
            let mut directories = BTreeSet::new();
            for (index, dir) in dirs.iter().enumerate() {
                collect(
                    dir,
                    &format!("{index}:{}", dir.display()),
                    &mut entries,
                    &mut directories,
                )?;
            }
            Ok((
                suite.clone(),
                digest(&serde_json::to_vec(&(
                    dirs,
                    directories,
                    file_digest(&entries)?,
                ))?)?,
            ))
        })
        .collect()
}

pub(crate) fn read(path: &Path, current: &Inputs) -> Result<Receipt> {
    let receipt: Receipt = serde_json::from_slice(&fs::read(path)?)?;
    if receipt.version != 1 || receipt.inputs != *current {
        return Err("Receipt inputs changed; run the full suite".into());
    }
    if receipt.completed.is_empty() {
        return Err(
            "Receipt contains no verified successful test classes; run the full suite".into(),
        );
    }
    for class in &receipt.completed {
        let Some((suite, name)) = class.rsplit_once(':') else {
            return Err("Malformed completed test identity".into());
        };
        if name.is_empty()
            || !name.split('.').all(|part| {
                !part.is_empty()
                    && part
                        .chars()
                        .all(|c| c.is_alphanumeric() || matches!(c, '_' | '$'))
            })
            || !receipt.inventory.contains_key(suite)
        {
            return Err("Malformed completed test identity".into());
        }
    }
    Ok(receipt)
}

/// A build failure, changed inputs or incomplete XML never authorizes an exclusion.
pub(crate) fn write(
    path: &Path,
    before: &Inputs,
    after: &Inputs,
    reports: &Reports,
    success: bool,
    inventory: Inventory,
) -> Result<()> {
    if !success || before != after {
        // Replace any older receipt, so a reused output path cannot retain stale authority.
        fs::write(path, b"null\n")?;
        return Ok(());
    }
    let normalize = |id: &String| {
        id.rsplit_once(':')
            .map(|(suite, name)| format!("{suite}:{}", top_level(name)))
    };
    let blocked: BTreeSet<String> = reports
        .failed
        .iter()
        .chain(&reports.skipped)
        .chain(reports.executed.difference(&reports.completed))
        .filter_map(normalize)
        .collect();
    let completed = reports
        .executed
        .intersection(&reports.completed)
        .filter_map(normalize)
        .filter(|id| !blocked.contains(id))
        .filter(|id| {
            id.rsplit_once(':')
                .is_some_and(|(suite, _)| inventory.contains_key(suite))
        })
        .collect();
    let receipt = Receipt {
        version: 1,
        inputs: before.clone(),
        inventory,
        completed,
    };
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temp, &receipt)?;
    temp.write_all(b"\n")?;
    temp.persist(path)?;
    Ok(())
}

/// New/missing/changed suite inventories run fully. Successful identities from another
/// module or task never exclude the same named class here.
pub(crate) fn excluded(receipt: &Receipt, current: &Inventory) -> Exclusions {
    let mut exclusions = Exclusions::new();
    for id in &receipt.completed {
        if let Some((suite, class)) = id.rsplit_once(':') {
            if current
                .get(suite)
                .is_some_and(|value| receipt.inventory.get(suite) == Some(value))
            {
                exclusions
                    .entry(suite.into())
                    .or_default()
                    .insert(class.into());
            }
        }
    }
    exclusions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(value: &str) -> Inputs {
        Inputs {
            fingerprint: value.into(),
        }
    }
    fn names(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|v| (*v).into()).collect()
    }

    #[test]
    fn receipts_require_success_complete_reports_and_matching_inputs_and_inventory() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("receipt.json");
        let inventory = BTreeMap::from([
            ("a:unit".into(), "same".into()),
            ("a:integration".into(), "same".into()),
        ]);
        let mut reports = Reports::default();
        reports.executed = names(&[
            "a:unit:GoodTest",
            "a:unit:IncompleteTest",
            "a:unit:SkippedTest",
            "a:unit:SkippedTest$Inner",
        ]);
        reports.completed = names(&[
            "a:unit:GoodTest",
            "a:unit:SkippedTest",
            "a:unit:SkippedTest$Inner",
        ]);
        reports.skipped = names(&["a:unit:SkippedTest$Inner"]);
        write(
            &path,
            &inputs("same"),
            &inputs("same"),
            &reports,
            true,
            inventory.clone(),
        )?;
        let receipt = read(&path, &inputs("same"))?;
        assert_eq!(
            excluded(&receipt, &inventory),
            BTreeMap::from([("a:unit".into(), names(&["GoodTest"]))])
        );
        assert!(read(&path, &inputs("changed")).is_err());
        assert!(excluded(
            &receipt,
            &BTreeMap::from([("a:unit".into(), "changed".into())])
        )
        .is_empty());
        write(
            &path,
            &inputs("same"),
            &inputs("same"),
            &reports,
            false,
            inventory.clone(),
        )?;
        assert!(read(&path, &inputs("same")).is_err());
        write(
            &path,
            &inputs("same"),
            &inputs("changed"),
            &reports,
            true,
            inventory,
        )?;
        assert!(read(&path, &inputs("same")).is_err());
        Ok(())
    }

    #[test]
    fn inventory_detects_new_tests_and_changed_nested_classes() -> Result<()> {
        let temp = tempfile::tempdir()?;
        fs::write(temp.path().join("Test.class"), "one")?;
        let suites = BTreeMap::from([("a:unit".into(), vec![temp.path().to_owned()])]);
        let before = inventory(&suites)?;
        fs::write(temp.path().join("Test$Inner.class"), "two")?;
        assert_ne!(before, inventory(&suites)?);
        let dependency = temp.path().join("dependency.jar");
        fs::write(&dependency, "old jar bytes")?;
        let suites = BTreeMap::from([("a:unit".into(), vec![dependency.clone()])]);
        let before = inventory(&suites)?;
        fs::write(&dependency, "new jar bytes")?;
        assert_ne!(before, inventory(&suites)?);
        fs::remove_file(dependency)?;
        assert!(inventory(&suites).is_err());
        Ok(())
    }

    #[test]
    fn reports_with_missing_or_inconsistent_totals_cannot_complete_a_class() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let reports = temp.path().join("target/surefire-reports");
        fs::create_dir_all(&reports)?;
        let report = reports.join("TEST-ExampleTest.xml");
        for (attributes, complete) in [
            ("", false),
            (
                "tests=\"2\" failures=\"0\" errors=\"0\" skipped=\"0\"",
                false,
            ),
            (
                "tests=\"1\" failures=\"0\" errors=\"0\" skipped=\"0\"",
                true,
            ),
        ] {
            fs::write(&report, format!("<testsuite {attributes}><testcase classname=\"ExampleTest\" name=\"works\"/></testsuite>"))?;
            let parsed = crate::reports::read_reports(temp.path(), "maven")?;
            assert_eq!(parsed.completed.contains(".:unit:ExampleTest"), complete);
        }
        fs::write(report, "<testsuite tests=\"1\" failures=\"0\" errors=\"0\" skipped=\"0\"><testcase classname=\"ExampleTest\" name=\"works\"/>")?;
        assert!(crate::reports::read_reports(temp.path(), "maven").is_err());
        Ok(())
    }

    #[test]
    fn one_complete_report_cannot_cover_another_incomplete_report_of_the_same_class() -> Result<()>
    {
        let temp = tempfile::tempdir()?;
        let reports = temp.path().join("target/surefire-reports");
        fs::create_dir_all(&reports)?;
        for (file, tests, case) in [("one", 1, "first"), ("two", 2, "second")] {
            fs::write(reports.join(format!("TEST-{file}.xml")), format!("<testsuite tests=\"{tests}\" failures=\"0\" errors=\"0\" skipped=\"0\"><testcase classname=\"ExampleTest\" name=\"{case}\"/></testsuite>"))?;
        }
        let parsed = crate::reports::read_reports(temp.path(), "maven")?;
        assert!(parsed.executed.contains(".:unit:ExampleTest"));
        assert!(parsed.completed.is_empty());
        Ok(())
    }
}
