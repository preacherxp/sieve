//! Timed runs of a build command with a phase breakdown taken from its timestamped output.
use crate::Result;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::Instant,
};

#[derive(Debug, Default, Clone, Serialize)]
pub struct Timed {
    pub exit: Option<i32>,
    pub seconds: f64,
    /// Consecutive phases that add up to `seconds`.
    pub phases: BTreeMap<String, f64>,
    /// Spring application startups and Testcontainers container startups reported in the
    /// output; both fall inside the `tests` phase.
    pub context_seconds: f64,
    pub container_seconds: f64,
    /// Sum of the test suite times in the XML reports.
    pub reported_test_seconds: f64,
    pub cases: usize,
    pub executed: BTreeSet<String>,
    pub failed: BTreeSet<String>,
    /// Reports that could not be read, so that `failed` may be incomplete.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_error: Option<String>,
    #[serde(skip)]
    pub compile_error: bool,
}

impl Timed {
    pub fn inconclusive(&self) -> bool {
        self.exit.is_none()
            || self.report_error.is_some()
            || self.exit != Some(0) && self.failed.is_empty()
    }
}

/// Failures absent from the selected run, plus build/report failures that prevent validation.
pub(crate) fn missed(reference: &Timed, selected: &Timed) -> BTreeSet<String> {
    let mut missed: BTreeSet<String> = reference
        .failed
        .difference(&selected.failed)
        .cloned()
        .collect();
    if reference.exit != Some(0) && selected.exit == Some(0) {
        missed.insert("<the reference build failed; the selected build passed>".into());
    }
    for (run, timed) in [("reference", reference), ("selected", selected)] {
        if let Some(error) = &timed.report_error {
            missed.insert(format!("<unreadable {run} reports: {error}>"));
        }
        if timed.exit.is_none() {
            missed.insert(format!("<inconclusive {run} build: terminated>"));
        } else if timed.exit != Some(0) && timed.failed.is_empty() {
            missed.insert(format!(
                "<inconclusive {run} build: failed without test failures>"
            ));
        }
    }
    missed
}

/// Phase names in order, each ending at the next marker.
pub const PHASES: &[&str] = &[
    "sieve",
    "maven_start",
    "compile",
    "test_jvm_start",
    "tests",
    "after_tests",
];

/// Splits the run into [`PHASES`] from lines timestamped in seconds since the start. The
/// phases always add up to `total`; after a missing marker they are empty.
pub fn phases(lines: &[(f64, String)], total: f64) -> BTreeMap<String, f64> {
    let first = |from: f64, test: &dyn Fn(&str) -> bool| {
        lines
            .iter()
            .find(|(t, l)| *t >= from && test(l))
            .map(|(t, _)| *t)
    };
    let mojo = |l: &str| l.starts_with("[INFO] --- ");
    let scan = first(0.0, &|l| {
        l.contains("Scanning for projects") || l.starts_with("[INFO] ")
    });
    let start_mojo = scan.and_then(|s| first(s, &mojo));
    let test_mojo = start_mojo.and_then(|s| {
        first(s, &|l| {
            mojo(l) && (l.contains("surefire") || l.contains("failsafe"))
        })
    });
    let first_test = test_mojo.and_then(|s| first(s, &|l| l.contains("[INFO] Running ")));
    let tests_end = first_test.and_then(|s| {
        lines
            .iter()
            .rev()
            .find(|(t, l)| *t >= s && l.contains("Tests run:"))
            .map(|(t, _)| *t)
    });
    let mut out = BTreeMap::new();
    let mut at = 0.0f64;
    let marks = [
        scan,
        start_mojo,
        test_mojo,
        first_test,
        tests_end,
        Some(total),
    ];
    for (name, mark) in PHASES.iter().zip(marks) {
        // A missing marker ends the breakdown: its phase takes the rest of the run.
        let end = mark.map_or(total, |m| m.clamp(at, total));
        out.insert((*name).to_owned(), end - at);
        at = end;
    }
    out
}

/// Seconds of Spring Boot startups (`Started X in 1.2 seconds`) and Testcontainers startups
/// (`Container x started in PT1.2S`).
pub fn startups(lines: &[(f64, String)]) -> (f64, f64) {
    let number = |text: &str| -> Option<f64> {
        let digits: String = text
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        digits.parse().ok()
    };
    let (mut context, mut container) = (0.0, 0.0);
    for (_, line) in lines {
        if let Some((_, rest)) = line
            .split_once(" in ")
            .filter(|_| line.contains("Started "))
        {
            if rest.contains("seconds") {
                context += number(rest).unwrap_or(0.0);
            }
        }
        if line.contains("Container ") {
            if let Some((_, rest)) = line.split_once(" started in PT") {
                container += number(rest).unwrap_or(0.0);
            }
        }
    }
    (context, container)
}

/// Sum of the `time` attributes of the `testsuite` roots below the report directories.
pub fn reported_seconds(workspace: &Path) -> f64 {
    let mut total = 0.0;
    for folder in ["surefire-reports", "failsafe-reports"] {
        let Ok(entries) = fs::read_dir(workspace.join("target").join(folder)) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "xml") {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            total += suite_time(&text).unwrap_or(0.0);
        }
    }
    total
}

fn suite_time(xml: &str) -> Option<f64> {
    let start = xml.find("<testsuite")?;
    let tag = &xml[start..start + xml[start..].find('>')?];
    let at = tag.find(" time=\"")? + " time=\"".len();
    let value = &tag[at..at + tag[at..].find('"')?];
    value.replace(',', "").parse().ok()
}

/// Deletes test reports, so that a run's reports are its own.
pub fn clear_reports(workspace: &Path) -> Result<()> {
    let folders = [
        workspace.join("target/surefire-reports"),
        workspace.join("target/failsafe-reports"),
        // Gradle keeps the results of a skipped up-to-date `Test` task; without them it reruns.
        workspace.join("build/test-results"),
    ];
    for dir in folders {
        if dir.is_dir() {
            fs::remove_dir_all(dir)?;
        }
    }
    Ok(())
}

/// Whether a build output line reports failed compilation: Maven's banner, or a failed
/// Gradle compile task (Java, Kotlin, Groovy, Scala).
fn compile_failure(line: &str) -> bool {
    line.contains("COMPILATION ERROR")
        || line
            .split_once("Execution failed for task '")
            .is_some_and(|(_, task)| {
                task.split(':')
                    .next_back()
                    .is_some_and(|t| t.starts_with("compile"))
            })
}

/// Runs `command`, timestamping its output lines into `log`, and reads the test reports.
pub fn run(command: &mut Command, log: &Path, workspace: &Path, tool: &str) -> Result<Timed> {
    clear_reports(workspace)?;
    let started = Instant::now();
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let lines = Arc::new(Mutex::new(Vec::new()));
    let readers: Vec<_> = [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    ]
    .into_iter()
    .flatten()
    .map(|stream| {
        let lines = Arc::clone(&lines);
        std::thread::spawn(move || {
            for line in BufReader::new(stream).lines().map_while(|l| l.ok()) {
                let at = started.elapsed().as_secs_f64();
                lines.lock().unwrap().push((at, line));
            }
        })
    })
    .collect();
    let status = child.wait()?;
    for reader in readers {
        let _ = reader.join();
    }
    let seconds = started.elapsed().as_secs_f64();
    let mut lines = std::mem::take(&mut *lines.lock().unwrap());
    lines.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut file = fs::File::create(log)?;
    for (at, line) in &lines {
        writeln!(file, "{at:9.3} {line}")?;
    }
    let (context_seconds, container_seconds) = startups(&lines);
    let (reports, report_error) = match crate::reports::read_reports(workspace, tool) {
        Ok(reports) => (reports, None),
        Err(error) => (Default::default(), Some(error.to_string())),
    };
    Ok(Timed {
        exit: status.code(),
        seconds,
        phases: phases(&lines, seconds),
        context_seconds,
        container_seconds,
        reported_test_seconds: reported_seconds(workspace),
        cases: reports.cases,
        executed: reports.executed,
        failed: reports.failed,
        report_error,
        compile_error: lines.iter().any(|(_, l)| compile_failure(l)),
    })
}

/// Median and range of samples.
pub fn stats(samples: &[f64]) -> serde_json::Value {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let median = match sorted.len() {
        0 => f64::NAN,
        n if n % 2 == 1 => sorted[n / 2],
        n => (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0,
    };
    serde_json::json!({
        "median": median,
        "min": sorted.first(),
        "max": sorted.last(),
        "samples": samples,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &[(f64, &str)]) -> Vec<(f64, String)> {
        text.iter().map(|(t, l)| (*t, l.to_string())).collect()
    }

    #[test]
    fn maven_output_splits_into_phases_that_add_up() {
        let log = lines(&[
            (0.2, "{ \"mode\": \"RECORDS\" }"),
            (0.5, "[INFO] Scanning for projects..."),
            (1.5, "[INFO] --- resources:3.3.1:resources (default-resources) @ app ---"),
            (2.0, "[INFO] --- compiler:3.13.0:compile (default-compile) @ app ---"),
            (4.0, "[INFO] --- surefire:3.5.2:test (default-test) @ app ---"),
            (5.0, "[INFO] Running example.ATest"),
            (6.0, "  .   ____          _            __ _ _"),
            (7.5, "Started ATest in 1.25 seconds (process running for 2.5)"),
            (8.0, "Container redis:7 started in PT0.75S"),
            (9.0, "[INFO] Tests run: 3, Failures: 0, Errors: 0, Skipped: 0, Time elapsed: 4 s -- in example.ATest"),
            (9.1, "[INFO] Tests run: 3, Failures: 0, Errors: 0, Skipped: 0"),
            (10.0, "[INFO] BUILD SUCCESS"),
        ]);
        let phases = phases(&log, 10.5);
        assert_eq!(phases["sieve"], 0.5);
        assert_eq!(phases["maven_start"], 1.0);
        assert_eq!(phases["compile"], 2.5);
        assert_eq!(phases["test_jvm_start"], 1.0);
        assert!((phases["tests"] - 4.1).abs() < 1e-9);
        assert!((phases.values().sum::<f64>() - 10.5).abs() < 1e-9);
        assert_eq!(startups(&log), (1.25, 0.75));
    }

    #[test]
    fn runs_without_a_build_are_all_sieve() {
        let phases = phases(&lines(&[(0.01, "{ \"mode\": \"NONE\" }")]), 0.02);
        assert_eq!(phases["sieve"], 0.02);
        assert_eq!(phases.values().sum::<f64>(), 0.02);
    }

    #[test]
    fn report_suite_times() {
        let xml = r#"<?xml version="1.0"?><testsuite name="a.B" time="1,234.5" tests="2"><testcase time="1"/></testsuite>"#;
        assert_eq!(suite_time(xml), Some(1234.5));
        assert_eq!(suite_time("<testsuites/>"), None);
        assert_eq!(stats(&[3.0, 1.0, 2.0])["median"], 2.0);
    }

    #[test]
    fn failed_builds_without_test_failures_cannot_validate_a_selection() {
        let failed = Timed {
            exit: Some(1),
            ..Default::default()
        };
        assert!(failed.inconclusive());
        assert!(!missed(&failed, &failed).is_empty());
        let unreadable = Timed {
            exit: Some(0),
            report_error: Some("truncated XML".into()),
            ..Default::default()
        };
        assert!(unreadable.inconclusive());
        assert!(!missed(&unreadable, &unreadable).is_empty());
        let terminated = Timed {
            failed: BTreeSet::from(["example.PartialTest".into()]),
            ..Default::default()
        };
        assert!(terminated.inconclusive());
        assert!(!missed(&terminated, &terminated).is_empty());
    }

    #[test]
    fn compile_failures_of_both_build_tools() {
        assert!(compile_failure("[ERROR] COMPILATION ERROR : "));
        assert!(compile_failure("Execution failed for task ':compileJava'."));
        assert!(compile_failure(
            "Execution failed for task ':app:compileTestKotlin'."
        ));
        assert!(!compile_failure(
            "Execution failed for task ':integrationTest'."
        ));
        assert!(!compile_failure("> Task :compileJava"));
    }
}
