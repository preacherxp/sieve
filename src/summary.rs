//! The end-of-run summary: how many tests ran out of the suite, and the time the selection
//! saved, estimated from what each skipped test class cost when it last ran.
use crate::{
    classes,
    reports::{self, Cost},
    Result,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

/// Durations from earlier runs, below `.sieve/`.
const STORE: &str = "timings.json";

#[derive(Default, Serialize, Deserialize)]
struct Store {
    /// What the last build that ran tests spent outside them: build-tool start, compilation,
    /// test-JVM start. A run that starts no build saves it too.
    #[serde(default)]
    overhead_seconds: f64,
    /// Per top-level test class, its cost when it last ran.
    #[serde(default)]
    classes: BTreeMap<String, Cost>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Summary {
    pub classes_run: usize,
    pub classes_total: usize,
    pub cases_run: usize,
    pub cases_total: usize,
    /// Skipped test classes that never reported a run, so their cases and seconds are unknown.
    #[serde(skip_serializing_if = "is_zero")]
    pub unmeasured: usize,
    /// This run, from start to the end of the build.
    pub seconds: f64,
    /// This run plus what the skipped test classes took when they last ran.
    pub estimated_full_seconds: f64,
    pub saved_seconds: f64,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// Combines the classes that ran (`ran`, from this build's reports) with the skipped ones:
/// those the selection dropped, plus, without a known `suite`, every class that ran before.
/// `concrete` tells whether a class without a recorded run is a runnable test. Without a
/// build, the full run's build `overhead` counts as saved as well.
#[allow(clippy::too_many_arguments)]
fn compute(
    ran: &BTreeMap<String, Cost>,
    store: &BTreeMap<String, Cost>,
    skipped: &BTreeSet<String>,
    suite_known: bool,
    seconds: f64,
    built: bool,
    overhead: f64,
    concrete: impl Fn(&str) -> bool,
) -> Summary {
    let mut summary = Summary {
        classes_run: ran.len(),
        cases_run: ran.values().map(|c| c.cases).sum(),
        seconds,
        ..Summary::default()
    };
    let mut saved = 0.0;
    let mut measured = 0;
    let candidates = skipped
        .iter()
        .chain(store.keys().filter(|_| !suite_known))
        .filter(|class| !ran.contains_key(*class))
        .collect::<BTreeSet<_>>();
    for class in candidates {
        match store.get(class) {
            Some(cost) => {
                measured += 1;
                summary.cases_total += cost.cases;
                saved += cost.seconds;
            }
            None if concrete(class) => summary.unmeasured += 1,
            None => {}
        }
    }
    summary.classes_total = summary.classes_run + measured + summary.unmeasured;
    summary.cases_total += summary.cases_run;
    if !built && measured > 0 {
        saved += overhead;
    }
    summary.saved_seconds = saved;
    summary.estimated_full_seconds = seconds + saved;
    summary
}

/// Records this build's costs in `dir` when `persist` is set, and summarizes the run. `built`
/// is false when no build ran, so that reports of an earlier build do not count. `suite`, when
/// the selection knows every test class, also drops stored costs of deleted classes.
#[allow(clippy::too_many_arguments)]
pub fn finish(
    persist: bool,
    dir: &Path,
    workspace: &Path,
    tool: &str,
    built: bool,
    skipped: &BTreeSet<String>,
    suite: Option<&BTreeSet<String>>,
    test_dirs: &[PathBuf],
    seconds: f64,
) -> Result<Summary> {
    let ran = if built {
        reports::class_costs(workspace, tool)?
    } else {
        BTreeMap::new()
    };
    let path = dir.join(STORE);
    let mut store: Store = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let overhead = store.overhead_seconds;
    if !ran.is_empty() {
        let tests: f64 = ran.values().map(|c| c.seconds).sum();
        store.overhead_seconds = (seconds - tests).max(0.0);
    }
    store
        .classes
        .extend(ran.iter().map(|(k, v)| (k.clone(), *v)));
    if let Some(suite) = suite {
        store
            .classes
            .retain(|class, _| suite.contains(class) || ran.contains_key(class));
    }
    if persist {
        fs::create_dir_all(dir)?;
        let ignore = dir.join(".gitignore");
        if !ignore.is_file() {
            fs::write(ignore, "*\n")?;
        }
        fs::write(&path, serde_json::to_vec_pretty(&store)?)?;
    }
    let concrete = |class: &str| {
        let file = format!("{}.class", class.replace('.', "/"));
        test_dirs
            .iter()
            .map(|dir| dir.join(&file))
            .find(|path| path.is_file())
            .and_then(|path| fs::read(path).ok())
            .and_then(|bytes| classes::parse(&bytes).ok())
            // A class file that cannot be found or read may still be a test elsewhere.
            .is_none_or(|class| !class.abstract_ && !class.annotation)
    };
    Ok(compute(
        &ran,
        &store.classes,
        skipped,
        suite.is_some(),
        seconds,
        built,
        overhead,
        concrete,
    ))
}

/// Compiled test-class directories of the modules, for telling fixtures from tests.
pub fn test_dirs(workspace: &Path, modules: &BTreeSet<String>, tool: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for module in modules {
        let root = workspace.join(module);
        if tool == "maven" {
            dirs.push(root.join("target/test-classes"));
        } else {
            for language in ["java", "kotlin", "groovy", "scala"] {
                let classes = root.join("build/classes").join(language);
                for set in fs::read_dir(&classes).into_iter().flatten().flatten() {
                    if set.file_name() != "main" {
                        dirs.push(set.path());
                    }
                }
            }
        }
    }
    dirs
}

fn plural(n: usize, word: &str) -> String {
    let suffix = match (n, word.ends_with('s')) {
        (1, _) => "",
        (_, true) => "es",
        _ => "s",
    };
    format!("{n} {word}{suffix}")
}

impl Summary {
    /// The summary as Markdown, for a CI job page.
    pub fn markdown(&self) -> String {
        let cases = match self.unmeasured {
            0 => self.cases_total.to_string(),
            _ => format!("{}+", self.cases_total),
        };
        let mut text = format!(
            "### Sieve test selection\n\n| | Ran | Total |\n| --- | ---: | ---: |\n\
             | Test classes | {} | {} |\n| Test cases | {} | {cases} |\n\n",
            self.classes_run, self.classes_total, self.cases_run
        );
        for line in self.lines() {
            text += line.strip_prefix("sieve: ").unwrap_or(&line);
            text += "  \n";
        }
        text
    }

    /// Appends the Markdown summary to GitHub Actions' job summary, when running there.
    pub fn publish(&self) {
        let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY") else {
            return;
        };
        let appended = fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .and_then(|mut file| std::io::Write::write_all(&mut file, self.markdown().as_bytes()));
        if let Err(error) = appended {
            eprintln!("sieve: cannot write the job summary: {error}");
        }
    }

    /// Human-readable lines for the end of the build output.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.classes_total == 0 {
            lines.push(format!("sieve: ran no tests in {:.1} s", self.seconds));
        } else if self.classes_run == self.classes_total {
            lines.push(format!(
                "sieve: ran all {} ({}) in {:.1} s",
                plural(self.classes_total, "test class"),
                plural(self.cases_total, "test case"),
                self.seconds
            ));
        } else {
            let cases = match self.unmeasured {
                0 => format!("{} of {} test cases", self.cases_run, self.cases_total),
                _ => plural(self.cases_run, "test case"),
            };
            lines.push(format!(
                "sieve: ran {} of {} test classes ({cases}) in {:.1} s",
                self.classes_run, self.classes_total, self.seconds
            ));
            let measured = self.classes_total - self.classes_run - self.unmeasured;
            let unknown = format!(
                "{} {}",
                plural(self.unmeasured, "skipped test class"),
                if self.unmeasured == 1 { "has" } else { "have" }
            );
            lines.push(if measured == 0 {
                format!("sieve: time saved unknown: {unknown} no recorded duration yet")
            } else if self.unmeasured == 0 {
                let percent = 100.0 * self.saved_seconds / self.estimated_full_seconds;
                // Factors for runs that started no build say little.
                let speedup = match self.seconds >= 1.0 {
                    true => format!(", {:.1}x faster", self.estimated_full_seconds / self.seconds),
                    false => String::new(),
                };
                format!(
                    "sieve: saved ~{:.1} s of an estimated {:.1} s full run ({percent:.0}% less{speedup})",
                    self.saved_seconds, self.estimated_full_seconds
                )
            } else {
                format!(
                    "sieve: saved at least ~{:.1} s of an estimated {:.1}+ s full run; {unknown} no recorded duration yet",
                    self.saved_seconds, self.estimated_full_seconds
                )
            });
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn costs(entries: &[(&str, usize, f64)]) -> BTreeMap<String, Cost> {
        entries
            .iter()
            .map(|(name, cases, seconds)| {
                (
                    name.to_string(),
                    Cost {
                        cases: *cases,
                        seconds: *seconds,
                    },
                )
            })
            .collect()
    }

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn skipped_classes_count_with_their_last_cost() {
        let ran = costs(&[("a.ServiceTest", 9, 0.5)]);
        let mut store = costs(&[("a.ListenerTest", 26, 5.5), ("a.RepositoryTest", 13, 3.0)]);
        store.extend(ran.clone());
        let summary = compute(
            &ran,
            &store,
            &set(&["a.ListenerTest", "a.RepositoryTest", "a.Base", "a.NewTest"]),
            true,
            6.0,
            true,
            5.0,
            |class| class != "a.Base",
        );
        assert_eq!(
            summary,
            Summary {
                classes_run: 1,
                classes_total: 4,
                cases_run: 9,
                cases_total: 48,
                unmeasured: 1,
                seconds: 6.0,
                estimated_full_seconds: 14.5,
                saved_seconds: 8.5,
            }
        );
        assert_eq!(
            summary.lines(),
            [
                "sieve: ran 1 of 4 test classes (9 test cases) in 6.0 s",
                "sieve: saved at least ~8.5 s of an estimated 14.5+ s full run; 1 skipped test class has no recorded duration yet",
            ]
        );
        let measured = compute(
            &ran,
            &store,
            &set(&["a.ListenerTest", "a.RepositoryTest", "a.Base"]),
            true,
            6.0,
            true,
            5.0,
            |class| class != "a.Base",
        );
        assert_eq!(
            measured.lines(),
            [
                "sieve: ran 1 of 3 test classes (9 of 48 test cases) in 6.0 s",
                "sieve: saved ~8.5 s of an estimated 14.5 s full run (59% less, 2.4x faster)",
            ]
        );
        // Static selection without local mode keeps no durations.
        let unknown = compute(
            &ran,
            &ran,
            &set(&["a.ListenerTest"]),
            true,
            6.0,
            true,
            0.0,
            |_| true,
        );
        assert_eq!(
            unknown.lines(),
            [
                "sieve: ran 1 of 2 test classes (9 test cases) in 6.0 s",
                "sieve: time saved unknown: 1 skipped test class has no recorded duration yet",
            ]
        );
    }

    #[test]
    fn without_a_known_suite_every_recorded_class_counts() {
        let ran = costs(&[("a.ATest", 2, 1.0)]);
        let store = costs(&[("a.ATest", 2, 1.0), ("b.BTest", 3, 2.0)]);
        let summary = compute(
            &ran,
            &store,
            &BTreeSet::new(),
            false,
            4.0,
            true,
            0.0,
            |_| true,
        );
        assert_eq!((summary.classes_total, summary.cases_total), (2, 5));
        // A run that starts no build also saves the build's own overhead.
        let none = compute(
            &BTreeMap::new(),
            &store,
            &BTreeSet::new(),
            false,
            0.2,
            false,
            6.0,
            |_| true,
        );
        assert_eq!(
            none.lines(),
            [
                "sieve: ran 0 of 2 test classes (0 of 5 test cases) in 0.2 s",
                "sieve: saved ~9.0 s of an estimated 9.2 s full run (98% less)",
            ]
        );
        // A full run reports everything as run.
        let all = compute(
            &store,
            &store,
            &BTreeSet::new(),
            false,
            9.0,
            true,
            0.0,
            |_| true,
        );
        assert_eq!(
            all.lines(),
            ["sieve: ran all 2 test classes (5 test cases) in 9.0 s"]
        );
    }

    #[test]
    fn markdown_tables_the_counts() {
        let summary = Summary {
            classes_run: 1,
            classes_total: 10,
            cases_run: 3,
            cases_total: 117,
            unmeasured: 0,
            seconds: 5.2,
            estimated_full_seconds: 16.7,
            saved_seconds: 11.5,
        };
        let text = summary.markdown();
        assert!(text.contains("| Test classes | 1 | 10 |"), "{text}");
        assert!(text.contains("| Test cases | 3 | 117 |"), "{text}");
        assert!(
            text.contains("saved ~11.5 s of an estimated 16.7 s full run (69% less, 3.2x faster)"),
            "{text}"
        );
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("summary.md");
        std::env::set_var("GITHUB_STEP_SUMMARY", &path);
        summary.publish();
        summary.publish();
        std::env::remove_var("GITHUB_STEP_SUMMARY");
        assert_eq!(fs::read_to_string(path).unwrap(), text.repeat(2));
    }

    #[test]
    fn report_roots_give_costs_per_top_level_class() {
        let temp = tempfile::tempdir().unwrap();
        let reports = temp.path().join("target/surefire-reports");
        fs::create_dir_all(&reports).unwrap();
        // Surefire's totals leave out `@Nested` classes, which report under display names.
        fs::write(
            reports.join("TEST-a.ListenerTest.xml"),
            r#"<testsuite name="a.ListenerTest" time="1,005.5" tests="1" skipped="0"><testcase name="x" classname="a.ListenerTest"/><testcase name="y" classname="Display name"></testcase><testcase name="z" classname="Display name"><skipped/></testcase></testsuite>"#,
        )
        .unwrap();
        // Gradle writes nested classes to their own reports.
        let gradle = temp.path().join("build/test-results/test");
        fs::create_dir_all(&gradle).unwrap();
        fs::write(
            gradle.join("TEST-a.Outer$Inner.xml"),
            r#"<testsuite name="a.Outer$Inner" time="0.5" tests="2" skipped="0"><testcase name="a" classname="a.Outer$Inner"/><testcase name="b" classname="a.Outer$Inner"/></testsuite>"#,
        )
        .unwrap();
        fs::write(
            gradle.join("TEST-a.Outer.xml"),
            r#"<testsuite name="a.Outer" time="0.25" tests="1" skipped="0"><testcase name="c" classname="a.Outer"/></testsuite>"#,
        )
        .unwrap();
        let maven = reports::class_costs(temp.path(), "maven").unwrap();
        assert_eq!(
            maven["a.ListenerTest"],
            Cost {
                cases: 2,
                seconds: 1005.5
            }
        );
        let gradle = reports::class_costs(temp.path(), "gradle").unwrap();
        assert_eq!(
            gradle["a.Outer"],
            Cost {
                cases: 3,
                seconds: 0.75
            }
        );
    }
}
