//! Replays recent history: selects tests for each commit against its parent and, with
//! `--run`, compares the selected run with the full suite on the same revision.
use crate::{timing, timing::Timed, Config, Result};
use serde_json::{json, Value};
use std::{env, fs, path::Path, process::Command};

const USAGE: &str = "sieve replay --workspace PATH [--commits N] [--run | --walk [--plant]]\n\
    [--executable PATH] [--output FILE] [-- BUILD_ARGS]\n\n\
    Uses the workspace's impact.json for each of the last N first-parent commits.\n\
    --run executes the full suite and the selection on every commit from clean trees and\n\
    reports missed failures and inconclusive builds (exit 1).\n\
    --walk (local mode, single-module Maven) applies the commits in order to one working\n\
    tree, carrying test records and build output forward, and compares `sieve run` with a\n\
    native full build; --plant also plants a bug in each commit's changed code.";

pub(crate) fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            "user.name=Sieve Replay",
            "-c",
            "user.email=replay@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
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
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

/// Commits the checked-out tree plus the Sieve configuration, as a maintained setup would.
fn configure(clone: &Path, workspace: &Path, config: &Config, message: &str) -> Result<String> {
    if config.tool == "maven" {
        for module in config.modules.keys() {
            let pom = workspace.join(module).join("pom.xml");
            let xml = fs::read_to_string(&pom).map_err(|e| format!("{}: {e}", pom.display()))?;
            fs::write(
                &pom,
                crate::setup::install_maven_adapter(&xml, module, true)?,
            )?;
        }
    }
    let mut config: Config = serde_json::from_value(serde_json::to_value(config)?)?;
    config.build_fingerprint = Some(crate::fingerprint::build_inputs(workspace)?);
    fs::write(
        workspace.join("impact.json"),
        serde_json::to_string_pretty(&config)? + "\n",
    )?;
    git(clone, &["add", "-A"])?;
    git(
        clone,
        &[
            "commit",
            "-q",
            "--no-verify",
            "--allow-empty",
            "-m",
            message,
        ],
    )?;
    git(clone, &["rev-parse", "HEAD"])
}

/// Runs `sieve run` with a clean tree and returns its timing and native test reports.
fn execute(
    clone: &Path,
    workspace: &Path,
    config: &Config,
    selection: &[&str],
    extra: &[String],
    log: &Path,
) -> Result<Timed> {
    git(clone, &["clean", "-fdxq"])?;
    timing::run(
        Command::new(env::current_exe()?)
            .arg("run")
            .arg("--workspace")
            .arg(workspace)
            .args(selection)
            .arg("--")
            .args(extra),
        log,
        workspace,
        &config.tool,
    )
}

pub fn main(args: Vec<String>) -> Result<u8> {
    let mut workspace = None;
    let mut commits = 20usize;
    let mut run = false;
    let mut walk_history = false;
    let mut plant = false;
    let mut executable = None;
    let mut output = None;
    let mut extra = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--" => {
                extra.extend(args.by_ref());
                break;
            }
            "--run" => run = true,
            "--walk" => walk_history = true,
            "--plant" => plant = true,
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(0);
            }
            "--workspace" | "--commits" | "--executable" | "--output" => {
                let value = args
                    .next()
                    .ok_or_else(|| format!("Missing value for {arg}"))?;
                match arg.as_str() {
                    "--workspace" => workspace = Some(value),
                    "--commits" => commits = value.parse()?,
                    "--executable" => executable = Some(value),
                    _ => output = Some(value),
                }
            }
            _ => return Err(format!("Unknown option: {arg}").into()),
        }
    }
    let workspace = Path::new(&workspace.ok_or("--workspace is required")?).canonicalize()?;
    let config = Config::read(&workspace)?;
    if walk_history {
        return walk(
            &workspace, &config, commits, plant, executable, output, extra,
        );
    }
    if plant {
        return Err("--plant needs --walk".into());
    }
    let root = Path::new(&git(&workspace, &["rev-parse", "--show-toplevel"])?).canonicalize()?;
    let prefix = workspace.strip_prefix(&root)?.to_owned();
    let history = git(
        &root,
        &[
            "rev-list",
            "--first-parent",
            "-n",
            &commits.to_string(),
            "HEAD",
        ],
    )?;
    let temp = tempfile::Builder::new().prefix("sieve-replay-").tempdir()?;
    let clone = temp.path().join("repo");
    // Logs outlive the temporary clone when a report file is written.
    let logs = match &output {
        Some(path) => Path::new(&format!("{path}.logs")).to_owned(),
        None => temp.path().join("logs"),
    };
    fs::create_dir_all(&logs)?;
    git(
        temp.path(),
        &[
            "clone",
            "-q",
            "--shared",
            "--no-checkout",
            root.to_str().ok_or("Non-UTF-8 path")?,
            "repo",
        ],
    )?;
    let clone_workspace = clone.join(&prefix);
    let mut executable_args = Vec::new();
    if let Some(executable) = &executable {
        executable_args = vec!["--executable", executable.as_str()];
    }
    let mut records = Vec::new();
    for commit in history.lines() {
        let parent = match git(&root, &["rev-parse", &format!("{commit}^")]) {
            Ok(parent) => parent,
            Err(_) => continue,
        };
        let subject = git(&root, &["log", "-1", "--format=%s", commit])?;
        let short = &commit[..commit.len().min(10)];
        eprintln!("replay {short} {subject}");
        let mut record = json!({"commit": commit, "subject": subject});
        // The configured parent, then the commit's tree with the same configuration.
        let prepared = git(&clone, &["checkout", "-q", "--detach", "-f", &parent])
            .and_then(|_| git(&clone, &["clean", "-fdxq"]))
            .and_then(|_| configure(&clone, &clone_workspace, &config, "configured parent"))
            .and_then(|base| {
                git(&clone, &["read-tree", "-u", "--reset", commit])?;
                configure(&clone, &clone_workspace, &config, "configured commit")?;
                Ok(base)
            });
        let base = match prepared {
            Ok(base) => base,
            Err(error) => {
                record["skipped"] = json!(error.to_string());
                records.push(record);
                continue;
            }
        };
        let selection_path = logs.join(format!("{short}-selection.json"));
        let mut selected_args = vec!["--base", base.as_str()];
        selected_args.extend(&executable_args);
        let selection_arg = selection_path.to_str().ok_or("Non-UTF-8 path")?;
        let select = Command::new(env::current_exe()?)
            .arg("select")
            .arg("--workspace")
            .arg(&clone_workspace)
            .args(["--base", &base, "--output", selection_arg])
            .output()?;
        if !select.status.success() {
            record["skipped"] = json!(String::from_utf8_lossy(&select.stderr).trim());
            records.push(record);
            continue;
        }
        record["selection"] = serde_json::from_slice(&fs::read(&selection_path)?)?;
        if run {
            let mut full_args = vec!["--full"];
            full_args.extend(&executable_args);
            let full_log = logs.join(format!("{short}-full.log"));
            let full = execute(
                &clone,
                &clone_workspace,
                &config,
                &full_args,
                &extra,
                &full_log,
            )?;
            selected_args.extend(["--output", selection_arg]);
            let selected_log = logs.join(format!("{short}-selected.log"));
            let selected = execute(
                &clone,
                &clone_workspace,
                &config,
                &selected_args,
                &extra,
                &selected_log,
            )?;
            // Refined by class-level analysis during the run.
            record["selection"] = serde_json::from_slice(&fs::read(&selection_path)?)?;
            record["missed"] = json!(timing::missed(&full, &selected));
            record["inconclusive"] = json!(full.inconclusive() || selected.inconclusive());
            record["full"] = json!(full);
            record["full"]["log"] = json!(full_log);
            record["selected"] = json!(selected);
            record["selected"]["log"] = json!(selected_log);
        }
        records.push(record);
    }
    let replayed: Vec<&Value> = records
        .iter()
        .filter(|r| r.get("skipped").is_none())
        .collect();
    let count = |mode: &str| {
        replayed
            .iter()
            .filter(|r| r["selection"]["mode"] == mode)
            .count()
    };
    let sum = |run: &str, key: &str| -> f64 {
        replayed.iter().filter_map(|r| r[run][key].as_f64()).sum()
    };
    let missed: usize = replayed
        .iter()
        .filter_map(|r| r["missed"].as_array().map(Vec::len))
        .sum();
    let mut summary = json!({
        "commits": records.len(), "replayed": replayed.len(),
        "modes": {"ALL": count("ALL"), "MODULES": count("MODULES"), "SUBSET": count("SUBSET"), "NONE": count("NONE")},
    });
    if run {
        summary["full_seconds"] = json!(sum("full", "seconds"));
        summary["selected_seconds"] = json!(sum("selected", "seconds"));
        summary["full_cases"] = json!(sum("full", "cases"));
        summary["selected_cases"] = json!(sum("selected", "cases"));
        summary["missed_failures"] = json!(missed);
        summary["inconclusive_runs"] = json!(replayed
            .iter()
            .filter(|r| r["inconclusive"] == true)
            .count());
    }
    let report = json!({
        "workspace": workspace, "tool": config.tool, "class_level": config.class_level,
        "extra_build_args": extra, "summary": summary, "records": records,
    });
    let text = serde_json::to_string_pretty(&report)? + "\n";
    match output {
        Some(path) => fs::write(path, &text)?,
        None => print!("{text}"),
    }
    eprintln!("{}", serde_json::to_string_pretty(&report["summary"])?);
    Ok(u8::from(missed > 0))
}

/// New-side line ranges (0-based, inclusive) of a unified diff's hunks.
fn hunks(diff: &str) -> Vec<(usize, usize)> {
    diff.lines()
        .filter_map(|l| l.strip_prefix("@@ "))
        .filter_map(|l| l.split(' ').find(|p| p.starts_with('+')))
        .filter_map(|range| {
            let mut parts = range[1..].split(',');
            let start: usize = parts.next()?.parse().ok()?;
            let count: usize = parts.next().map_or(Some(1), |c| c.parse().ok())?;
            (count > 0 && start > 0).then(|| (start - 1, start + count - 2))
        })
        .collect()
}

/// The first changed range in a main Java or Kotlin source of `commit`, workspace-relative.
fn changed_code(
    root: &Path,
    prefix: &Path,
    commit: &str,
) -> Result<Option<crate::catalog::Region>> {
    let parent = format!("{commit}^");
    let listing = git(
        root,
        &[
            "diff-tree",
            "-r",
            "--no-commit-id",
            "--name-status",
            &parent,
            commit,
        ],
    )?;
    for line in listing.lines() {
        let (status, path) = line.split_once('\t').unwrap_or(("", ""));
        let relative = match Path::new(path).strip_prefix(prefix) {
            Ok(relative) => relative.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        let source = relative.ends_with(".java") || relative.ends_with(".kt");
        if !matches!(status, "M" | "A") || !source || !relative.contains("src/main/") {
            continue;
        }
        let diff = git(root, &["diff", "-U0", &parent, commit, "--", path])?;
        if let Some((start, end)) = hunks(&diff).into_iter().next() {
            return Ok(Some((relative, start, end)));
        }
    }
    Ok(None)
}

/// `--walk`: one working tree follows the commits, as a developer's checkout would.
fn walk(
    workspace: &Path,
    config: &Config,
    commits: usize,
    plant: bool,
    executable: Option<String>,
    output: Option<String>,
    extra: Vec<String>,
) -> Result<u8> {
    // The walk measures local mode, whether or not the project has switched it on yet.
    let mut config: Config = serde_json::from_value(serde_json::to_value(config)?)?;
    if !matches!(config.tool.as_str(), "maven" | "gradle") || config.modules.keys().ne(["."]) {
        return Err(
            "--walk needs a single-module Maven or Gradle project; use --run otherwise".into(),
        );
    }
    config.records = Some(true);
    let config = &config;
    let root = Path::new(&git(workspace, &["rev-parse", "--show-toplevel"])?).canonicalize()?;
    let prefix = workspace.strip_prefix(&root)?.to_owned();
    let mut history: Vec<String> = git(
        &root,
        &[
            "rev-list",
            "--first-parent",
            "-n",
            &commits.to_string(),
            "HEAD",
        ],
    )?
    .lines()
    .map(str::to_owned)
    .collect();
    history.reverse();
    let first = history.first().ok_or("No commits to walk")?;
    let start = git(&root, &["rev-parse", &format!("{first}^")])?;
    let temp = tempfile::Builder::new().prefix("sieve-walk-").tempdir()?;
    let clone = temp.path().join("repo");
    let logs = match &output {
        Some(path) => Path::new(&format!("{path}.logs")).to_owned(),
        None => temp.path().join("logs"),
    };
    fs::create_dir_all(&logs)?;
    git(
        temp.path(),
        &[
            "clone",
            "-q",
            "--shared",
            "--no-checkout",
            root.to_str().ok_or("Non-UTF-8 path")?,
            "repo",
        ],
    )?;
    fs::write(clone.join(".git/info/exclude"), "impact.json\n")?;
    let clone_workspace = clone.join(&prefix);
    let config_text = serde_json::to_string_pretty(config)? + "\n";
    let checkout = |commit: &str| -> Result<()> {
        git(&clone, &["checkout", "-q", "--detach", "-f", commit])?;
        fs::write(clone_workspace.join("impact.json"), &config_text)?;
        Ok(())
    };
    let mut runner = crate::catalog::Runner {
        workspace: &clone_workspace,
        tool: config.tool.clone(),
        executable,
        extra: extra.clone(),
        logs: logs.clone(),
        count: 0,
    };
    let mut journal = crate::catalog::Journal::default();
    checkout(&start)?;
    eprintln!("walk: warming up at {}", &start[..start.len().min(10)]);
    let warm = runner.sieve("warm-up", &["--full"])?;
    let selection_file = temp.path().join("selection.json");
    let selection_arg = selection_file.to_str().ok_or("Non-UTF-8 path")?.to_owned();
    let mut records = Vec::new();
    for commit in &history {
        let short = &commit[..commit.len().min(10)];
        let subject = git(&root, &["log", "-1", "--format=%s", commit])?;
        eprintln!("walk {short} {subject}");
        let parent = format!("{commit}^");
        checkout(commit)?;
        let selected = runner.sieve(
            &format!("{short}-selected"),
            &["--base", &parent, "--output", &selection_arg],
        )?;
        let selection: Value =
            serde_json::from_slice(&fs::read(&selection_file).unwrap_or_default())
                .unwrap_or(Value::Null);
        let native = runner.native(&format!("{short}-native"))?;
        let missed = timing::missed(&native, &selected);
        let mut record = json!({
            "commit": commit, "subject": subject,
            "selection": {"mode": selection["mode"], "tests": selection["tests"], "reason": selection["reason"]},
            "clean": selection["reason"].as_str().is_some_and(|r| r.contains("clean:")),
            "inconclusive": native.inconclusive() || selected.inconclusive(),
            "selected": selected, "native": native, "missed": missed,
        });
        if plant {
            let planted = match changed_code(&root, &prefix, commit)? {
                Some(region) => {
                    let cell = std::cell::RefCell::new(&mut runner);
                    let mut chosen = |tag: &str| {
                        cell.borrow_mut().sieve(
                            &format!("{short}-planted-selected {tag}"),
                            &["--base", &parent],
                        )
                    };
                    let mut full = |tag: &str| {
                        cell.borrow_mut()
                            .native(&format!("{short}-planted-native {tag}"))
                    };
                    let result = crate::catalog::plant(
                        &clone_workspace,
                        &region,
                        &mut journal,
                        &mut chosen,
                        &mut full,
                    );
                    result?
                }
                None => json!({"mutant": null, "reason": "no changed main source"}),
            };
            runner.sieve(&format!("{short}-settle"), &["--base", &parent])?;
            record["planted"] = planted;
        }
        records.push(record);
    }
    let sum = |key: &str| -> f64 {
        records
            .iter()
            .filter_map(|r| r[key]["seconds"].as_f64())
            .sum()
    };
    let mut phases: std::collections::BTreeMap<String, f64> = Default::default();
    for record in &records {
        for (name, seconds) in record["selected"]["phases"]
            .as_object()
            .into_iter()
            .flatten()
        {
            *phases.entry(name.clone()).or_default() += seconds.as_f64().unwrap_or(0.0);
        }
    }
    let count = |key: &str| -> usize {
        records
            .iter()
            .filter_map(|r| r[key].as_array().map(Vec::len))
            .sum()
    };
    let planted_missed: usize = records
        .iter()
        .filter_map(|r| r["planted"]["missed"].as_array().map(Vec::len))
        .sum();
    let missed = count("missed") + planted_missed;
    let mode = |m: &str| {
        records
            .iter()
            .filter(|r| r["selection"]["mode"] == m)
            .count()
    };
    let summary = json!({
        "commits": records.len(),
        "modes": {"ALL": mode("ALL"), "SUBSET": mode("SUBSET"), "NONE": mode("NONE")},
        "cleans": records.iter().filter(|r| r["clean"] == true).count(),
        "sieve_seconds": sum("selected"),
        "sieve_phases": phases,
        "native_seconds": sum("native"),
        "planted_detected": records.iter().filter(|r| r["planted"]["detected"] == true).count(),
        "missed_failures": missed,
        "inconclusive_runs": records.iter().filter(|r| r["inconclusive"] == true).count(),
    });
    let report = json!({
        "workspace": workspace, "mode": "walk", "extra_build_args": extra,
        "warm_up": warm, "summary": summary, "records": records,
    });
    let text = serde_json::to_string_pretty(&report)? + "\n";
    match output {
        Some(path) => fs::write(path, &text)?,
        None => print!("{text}"),
    }
    eprintln!("{}", serde_json::to_string_pretty(&report["summary"])?);
    Ok(u8::from(missed > 0))
}

#[cfg(test)]
mod tests {
    #[test]
    fn hunks_give_new_side_lines() {
        let diff = "@@ -3,0 +4,2 @@\n+a\n+b\n@@ -10 +12 @@\n-x\n+y\n@@ -20,2 +21,0 @@\n-z\n";
        assert_eq!(super::hunks(diff), [(3, 4), (11, 11)]);
    }
}
