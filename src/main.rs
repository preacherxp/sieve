use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    tool: String,
    modules: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Serialize)]
struct Selection {
    mode: &'static str,
    modules: BTreeSet<String>,
    changed: BTreeSet<String>,
    reason: String,
}

impl Config {
    fn read(workspace: &Path) -> Result<Self> {
        let config: Self = serde_json::from_slice(&fs::read(workspace.join("impact.json"))?)?;
        if !matches!(config.tool.as_str(), "maven" | "gradle") || config.modules.is_empty() {
            return Err("impact.json requires tool maven/gradle and a nonempty module map".into());
        }
        for (module, dependencies) in &config.modules {
            if module.is_empty()
                || !module
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
                || !workspace.join(module).is_dir()
                || dependencies.iter().any(|d| !config.modules.contains_key(d))
            {
                return Err(format!("Invalid module or dependency: {module}").into());
            }
        }
        Ok(config)
    }

    fn all(&self, reason: impl Into<String>) -> Selection {
        Selection {
            mode: "ALL",
            modules: self.modules.keys().cloned().collect(),
            changed: BTreeSet::new(),
            reason: reason.into(),
        }
    }

    fn select(&self, changed: BTreeSet<String>) -> Selection {
        let mut selected = BTreeSet::new();
        for path in &changed {
            // Build/configuration changes may alter the module graph or test discovery.
            if let Some((module, rest)) = path.split_once('/') {
                if self.modules.contains_key(module) && rest.starts_with("src/") {
                    selected.insert(module.to_owned());
                    continue;
                }
            }
            if path == "README.md" || path == "VALIDATION.md" || path.starts_with("docs/") {
                continue;
            }
            let mut result = self.all(format!("Unclassified or build input changed: {path}"));
            result.changed = changed;
            return result;
        }
        // ponytail: module granularity includes extra tests; use semantic class dependencies
        // only when this conservative policy no longer saves enough CI time.
        loop {
            let before = selected.len();
            for (module, dependencies) in &self.modules {
                if dependencies
                    .iter()
                    .any(|dependency| selected.contains(dependency))
                {
                    selected.insert(module.clone());
                }
            }
            if selected.len() == before {
                break;
            }
        }
        Selection {
            mode: if selected.is_empty() {
                "NONE"
            } else {
                "MODULES"
            },
            modules: selected,
            changed,
            reason: "Changed source/resource modules and their transitive dependents".into(),
        }
    }
}

fn git(workspace: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .current_dir(workspace)
        .args(args)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "git {}: {}",
            args[0],
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(output.stdout)
}

fn changed_paths(workspace: &Path, base: &str) -> Result<BTreeSet<String>> {
    // Resolve revisions separately, so user input cannot become a Git option/pathspec.
    let revision = format!("{base}^{{commit}}");
    let base = String::from_utf8(git(
        workspace,
        &["rev-parse", "--verify", "--end-of-options", &revision],
    )?)?;
    let merge_base = String::from_utf8(git(workspace, &["merge-base", base.trim(), "HEAD"])?)?;
    let root = String::from_utf8(git(workspace, &["rev-parse", "--show-toplevel"])?)?;
    let root = Path::new(root.trim()).canonicalize()?;
    let prefix = workspace.strip_prefix(&root)?;
    // Run from the repository root: sibling files and shared build inputs must not disappear.
    let mut paths = git(
        &root,
        &[
            "diff",
            "--name-only",
            "-z",
            "--no-renames",
            merge_base.trim(),
            "--",
        ],
    )?;
    paths.extend(git(
        &root,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )?);
    paths
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| {
            let path = std::str::from_utf8(p)?;
            let relative = Path::new(path).strip_prefix(prefix);
            Ok(match relative {
                Ok(relative) => relative.to_str().ok_or("Non-UTF-8 path")?.to_owned(),
                // These docs are explicitly outside the build contract; other external changes
                // cause a full run, including changes to the selector and shared CI scripts.
                Err(_)
                    if path == "README.md"
                        || path == "VALIDATION.md"
                        || path.starts_with("docs/") =>
                {
                    path.to_owned()
                }
                Err(_) => format!("@repository/{path}"),
            })
        })
        .collect()
}

fn build_args(config: &Config, selection: &Selection) -> Vec<String> {
    let mut args: Vec<String> = match config.tool.as_str() {
        "maven" => vec!["-B", "-ntp", "clean"],
        _ => vec!["--no-daemon", "--console=plain", "clean"],
    }
    .into_iter()
    .map(str::to_owned)
    .collect();
    if selection.mode != "NONE" {
        args.push(
            if config.tool == "maven" {
                "verify"
            } else {
                "check"
            }
            .into(),
        );
        if selection.mode == "MODULES" {
            if config.tool == "maven" {
                for module in config.modules.keys() {
                    args.push(format!(
                        "-Dimpact.skip.{module}={}",
                        !selection.modules.contains(module)
                    ));
                }
            } else {
                args.push(format!(
                    "-Pimpact.modules={}",
                    selection
                        .modules
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(",")
                ));
            }
        }
    }
    args
}

fn main_result() -> Result<u8> {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_default();
    if matches!(command.as_str(), "" | "--help" | "-h") {
        println!(
            "java-test-impact <select|run> --workspace PATH [--base REV | --full]\n\
                  [--output FILE] [--executable PATH] [-- BUILD_ARGS...]\n\n\
                  Requires impact.json and the build adapters documented in README.md.\n\
                  No base or unavailable Git history selects ALL. run propagates build failures."
        );
        return Ok(0);
    }
    if command != "select" && command != "run" {
        return Err(format!("Unknown command: {command}").into());
    }
    let mut workspace = None;
    let mut base = None;
    let mut full = false;
    let mut output = None;
    let mut executable = None;
    let mut extra = Vec::new();
    while let Some(arg) = args.next() {
        if arg == "--" {
            extra.extend(args);
            break;
        }
        if arg == "--full" {
            full = true;
            continue;
        }
        let value = args
            .next()
            .ok_or_else(|| format!("Missing value for {arg}"))?;
        match arg.as_str() {
            "--workspace" => workspace = Some(PathBuf::from(value)),
            "--base" => base = Some(value),
            "--output" => output = Some(PathBuf::from(value)),
            "--executable" => executable = Some(value),
            _ => return Err(format!("Unknown option: {arg}").into()),
        }
    }
    if full && base.is_some() {
        return Err("Use either --base or --full".into());
    }
    let workspace = workspace.ok_or("--workspace is required")?.canonicalize()?;
    let config = Config::read(&workspace)?;
    let selection = match base {
        Some(base) => match changed_paths(&workspace, &base) {
            Ok(changed) => config.select(changed),
            Err(error) => config.all(format!(
                "Cannot establish changed inputs; full fallback: {error}"
            )),
        },
        None => config.all("Full run requested or no comparison base supplied"),
    };
    let json = serde_json::to_string_pretty(&selection)? + "\n";
    if let Some(path) = output {
        fs::write(path, &json)?;
    }
    if command == "select" {
        print!("{json}");
        return Ok(0);
    }
    eprintln!("{json}");
    let executable = executable.unwrap_or_else(|| {
        if config.tool == "maven" {
            "mvn"
        } else {
            "gradle"
        }
        .into()
    });
    let status = Command::new(executable)
        .current_dir(workspace)
        .args(build_args(&config, &selection))
        .args(extra)
        .status()?;
    Ok(if status.success() { 0 } else { 1 })
}

fn main() -> ExitCode {
    match main_result() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("Error: {error}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conservative_selection_and_build_filters() {
        let mut config: Config =
            serde_json::from_str(include_str!("../projects/maven/impact.json")).unwrap();
        let select = |path: &str| config.select(BTreeSet::from([path.into()]));
        let tax = select("pricing/src/main/java/example/TaxRules.java");
        assert_eq!(
            tax.modules,
            BTreeSet::from(["checkout".into(), "pricing".into()])
        );
        let args = build_args(&config, &tax);
        assert!(args.contains(&"-Dimpact.skip.runtime=true".into()));
        assert!(args.contains(&"-Dimpact.skip.checkout=false".into()));
        assert_eq!(
            select("runtime/src/main/resources/application.properties").modules,
            BTreeSet::from(["runtime".into()])
        );
        assert_eq!(
            select("runtime/src/main/resources/docs/readme.md").mode,
            "MODULES"
        );
        assert_eq!(select("pom.xml").mode, "ALL");
        assert_eq!(select("impact.json").mode, "ALL");
        assert_eq!(select("removed/src/main/java/Old.java").mode, "ALL");
        let docs = select("README.md");
        assert_eq!(docs.mode, "NONE");
        assert_eq!(build_args(&config, &docs), ["-B", "-ntp", "clean"]);
        assert_eq!(config.select(BTreeSet::new()).mode, "NONE");
        config.tool = "gradle".into();
        assert!(build_args(&config, &tax).contains(&"-Pimpact.modules=checkout,pricing".into()));
        config
            .modules
            .get_mut("pricing")
            .unwrap()
            .push("checkout".into());
        assert_eq!(
            config
                .select(BTreeSet::from([
                    "checkout/src/test/java/NewTest.java".into()
                ]))
                .modules
                .len(),
            2
        );
    }
}
