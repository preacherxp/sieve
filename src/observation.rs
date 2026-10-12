//! Compare a decision frozen before execution with the native full-run reports.
use crate::{reports, Result, Selection};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Proposal {
    pub(crate) mode: String,
    pub(crate) modules: BTreeSet<String>,
    pub(crate) tests: BTreeSet<String>,
    pub(crate) reason: String,
}

impl Proposal {
    pub(crate) fn of(selection: &Selection) -> Self {
        Self {
            mode: selection.mode.into(),
            modules: selection.modules.clone(),
            tests: selection.tests.clone(),
            reason: selection.reason.clone(),
        }
    }

    fn includes(&self, module: &str, class: &str) -> bool {
        self.mode == "ALL"
            || self.modules.contains(module)
                && (self.mode == "MODULES"
                    || self.mode == "SUBSET" && self.tests.contains(top_level(class)))
    }
}

pub(crate) fn top_level(class: &str) -> &str {
    class.split('$').next().unwrap_or(class)
}

#[derive(Debug, Serialize)]
pub(crate) struct Observation {
    pub(crate) proposed: Proposal,
    /// Maven can fall back independently for each module; keep those decisions separate.
    pub(crate) module_proposals: BTreeMap<String, Proposal>,
    pub(crate) full_build_succeeded: bool,
    pub(crate) executed_classes: BTreeSet<String>,
    pub(crate) failed_classes: BTreeSet<String>,
    pub(crate) missed_failed_classes: BTreeSet<String>,
    pub(crate) failure_retention: Option<f64>,
    pub(crate) would_detect_test_failure: Option<bool>,
    pub(crate) evidence: String,
    pub(crate) report_error: Option<String>,
}

pub(crate) fn finish(
    proposed: Proposal,
    mut module_proposals: BTreeMap<String, Proposal>,
    workspace: &Path,
    tool: &str,
    success: bool,
    build_logic: &[String],
) -> Observation {
    let reports = reports::read_reports(workspace, tool);
    if tool == "gradle" {
        if let Ok(reports) = &reports {
            for id in &reports.executed {
                let module = id.split(':').next().unwrap_or_default();
                if build_logic
                    .iter()
                    .map(String::as_str)
                    .chain(["buildSrc"])
                    .any(|root| {
                        module == root
                            || module
                                .strip_prefix(root)
                                .is_some_and(|path| path.starts_with('/'))
                    })
                {
                    module_proposals
                        .entry(module.to_owned())
                        .or_insert_with(|| Proposal {
                            mode: "ALL".into(),
                            modules: BTreeSet::from([module.to_owned()]),
                            tests: BTreeSet::new(),
                            reason: "Included build tests retain native execution".into(),
                        });
                }
            }
        }
    }
    compare(proposed, module_proposals, reports, success)
}

fn compare(
    proposed: Proposal,
    module_proposals: BTreeMap<String, Proposal>,
    reports: Result<reports::Reports>,
    success: bool,
) -> Observation {
    let (reports, report_error) = match reports {
        Ok(reports) => (reports, None),
        Err(error) => (Default::default(), Some(error.to_string())),
    };
    let missed_failed_classes: BTreeSet<String> = reports
        .failed
        .iter()
        .filter(|id| {
            let mut parts = id.splitn(3, ':');
            let module = parts.next().unwrap_or_default();
            let _suite = parts.next();
            let class = parts.next().unwrap_or_default();
            !module_proposals
                .get(module)
                .unwrap_or(&proposed)
                .includes(module, class)
        })
        .cloned()
        .collect();
    let evidence = if report_error.is_some() {
        "inconclusive: native test reports could not be read"
    } else if reports.failed.is_empty() && !success {
        "inconclusive: build failed without a reported test failure"
    } else if reports.failed.is_empty() {
        "no failure-retention evidence: no failing tests were observed"
    } else {
        "observed full-run test failures; subset execution order was not tested"
    }
    .to_owned();
    let failure_retention = (!reports.failed.is_empty()).then(|| {
        (reports.failed.len() - missed_failed_classes.len()) as f64 / reports.failed.len() as f64
    });
    let would_detect_test_failure =
        (!reports.failed.is_empty()).then_some(missed_failed_classes.len() < reports.failed.len());
    Observation {
        proposed,
        module_proposals,
        full_build_succeeded: success,
        executed_classes: reports.executed,
        failed_classes: reports.failed,
        missed_failed_classes,
        failure_retention,
        would_detect_test_failure,
        evidence,
        report_error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proposal(mode: &str, modules: &[&str], tests: &[&str]) -> Proposal {
        Proposal {
            mode: mode.into(),
            modules: modules.iter().map(|s| (*s).into()).collect(),
            tests: tests.iter().map(|s| (*s).into()).collect(),
            reason: "frozen".into(),
        }
    }

    #[test]
    fn broken_selector_exposes_misses_and_no_failure_is_not_evidence() {
        let mut reports = reports::Reports::default();
        reports.failed.insert("a:unit:example.Test".into());
        let observed = compare(
            proposal("NONE", &[], &[]),
            BTreeMap::new(),
            Ok(reports),
            false,
        );
        assert_eq!(observed.failure_retention, Some(0.0));
        assert_eq!(observed.would_detect_test_failure, Some(false));
        assert_eq!(observed.missed_failed_classes.len(), 1);
        let observed = compare(
            proposal("ALL", &["a"], &[]),
            BTreeMap::new(),
            Ok(Default::default()),
            true,
        );
        assert_eq!(observed.failure_retention, None);
        assert!(observed
            .evidence
            .starts_with("no failure-retention evidence"));
    }

    #[test]
    fn module_fallback_does_not_hide_misses_in_another_module_or_nested_class() {
        let mut reports = reports::Reports::default();
        reports.failed = [
            "a:unit:example.ATest$Nested",
            "b:integration:example.BTest",
            "b:unit:example.ATest",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let modules = BTreeMap::from([
            ("a".into(), proposal("MODULES", &["a"], &[])),
            ("b".into(), proposal("SUBSET", &["b"], &["example.BTest"])),
        ]);
        let observed = compare(
            proposal("MODULES", &["a", "b"], &[]),
            modules,
            Ok(reports),
            false,
        );
        assert_eq!(
            observed.missed_failed_classes,
            BTreeSet::from(["b:unit:example.ATest".into()])
        );
        assert_eq!(observed.would_detect_test_failure, Some(true));
    }

    #[test]
    fn selected_outer_class_covers_nested_reports() {
        assert!(proposal("SUBSET", &["a"], &["example.Test"]).includes("a", "example.Test$Nested"));
        assert!(!proposal("SUBSET", &["a"], &["example.Test"]).includes("b", "example.Test"));
    }
}
