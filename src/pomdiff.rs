//! Maven build-file edits that only change what modules depend on, such as a dependency or
//! BOM version bump. Each module's effective model at the comparison base and now is split
//! into `<dependencies>` and `<dependencyManagement>`, `<properties>`, and everything else:
//! plugins, build settings, profiles, repositories. A difference in everything else selects
//! every module. Changed dependency sections change that module, as a source edit would, so
//! it and its dependents run their tests. Property values are already interpolated into the
//! other sections; a changed property still changes its module unless it is named as a
//! version (`*.version`) and the module filters no resources, since plugins and filtered
//! resources can read any other property at build time.
use crate::{setup, Result};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Debug, PartialEq)]
pub enum Change {
    /// The modules whose dependencies or properties differ; empty when nothing effective did.
    Modules(BTreeSet<String>),
    /// Why every module is selected.
    All(String),
}

/// Compares the effective models of the workspace with those of `revision`, read in a
/// temporary worktree. `prefix` is the workspace path below the repository root.
pub fn compare(
    workspace: &Path,
    prefix: &str,
    revision: &str,
    executable: &str,
    extra: &[String],
) -> Result<Change> {
    let now = setup::effective_models(workspace, executable, extra)?;
    let worktree = Worktree::add(workspace, revision)?;
    let base = worktree.path.join(prefix).canonicalize()?;
    let then = setup::effective_models(&base, executable, extra)?;
    let relocate = |xml: &str| {
        // Effective models spell out absolute build directories.
        xml.replace(
            &base.display().to_string(),
            &workspace.display().to_string(),
        )
    };
    compare_models(
        &now,
        &then.into_iter().map(|(m, x)| (m, relocate(&x))).collect(),
    )
}

fn compare_models(
    now: &BTreeMap<String, String>,
    then: &BTreeMap<String, String>,
) -> Result<Change> {
    if now.keys().ne(then.keys()) {
        return Ok(Change::All("Maven modules were added or removed".into()));
    }
    let mut changed = BTreeSet::new();
    for (module, xml) in now {
        let now = setup::dependency_sections(xml)?;
        let then = setup::dependency_sections(&then[module])?;
        if now.rest != then.rest {
            return Ok(Change::All(format!(
                "the build configuration of {module} changed beyond its dependencies"
            )));
        }
        let filters = now.rest.contains("<filtering>true</filtering>");
        let properties = now
            .properties
            .keys()
            .chain(then.properties.keys())
            .filter(|name| now.properties.get(*name) != then.properties.get(*name))
            .any(|name| filters || !version_only(name));
        if now.dependencies != then.dependencies || properties {
            changed.insert(module.clone());
        }
    }
    Ok(Change::Modules(changed))
}

/// A property named as a version, which builds use for dependency and plugin versions; its
/// uses are interpolated into the compared sections. Names that plugins and the JVM read
/// themselves (`maven.*`, `project.*`, `java.*`) do not count.
fn version_only(name: &str) -> bool {
    name.ends_with(".version")
        && !["maven.", "project.", "java."]
            .iter()
            .any(|prefix| name.starts_with(prefix))
}

/// A detached worktree of the repository at a revision, removed when dropped.
struct Worktree {
    repository: PathBuf,
    path: PathBuf,
    _temp: tempfile::TempDir,
}

impl Worktree {
    fn add(workspace: &Path, revision: &str) -> Result<Self> {
        let repository = PathBuf::from(
            String::from_utf8(crate::git(workspace, &["rev-parse", "--show-toplevel"])?)?.trim(),
        );
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("base");
        let text = path.to_str().ok_or("Non-UTF-8 temporary path")?;
        crate::git(
            &repository,
            &["worktree", "add", "--detach", "--quiet", text, revision],
        )?;
        Ok(Self {
            repository,
            path,
            _temp: temp,
        })
    }
}

impl Drop for Worktree {
    fn drop(&mut self) {
        if let Some(path) = self.path.to_str() {
            let _ = crate::git(&self.repository, &["worktree", "remove", "--force", path]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn models(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(m, x)| (m.to_string(), x.to_string()))
            .collect()
    }

    /// A model whose `lib` dependency is at `version`, with properties and plugin config.
    fn model(version: &str, properties: &str, build: &str) -> String {
        format!(
            "<!-- Generated by maven-help-plugin on {version} -->\n<project>\n  <artifactId>a</artifactId>\n  <properties>{properties}</properties>\n  <dependencyManagement><dependencies><dependency><artifactId>bom</artifactId><version>1</version></dependency></dependencies></dependencyManagement>\n  <dependencies>\n    <dependency><artifactId>lib</artifactId><version>{version}</version></dependency>\n  </dependencies>\n  <build>{build}<plugins><plugin><artifactId>surefire</artifactId><version>3.5</version></plugin></plugins></build>\n</project>\n"
        )
    }

    const LIB: &str = "<lib.version>1</lib.version>";

    #[test]
    fn dependency_edits_change_their_modules_and_build_edits_change_everything() {
        let base = model("1.0", LIB, "");
        let then = models(&[("app", &base), ("core", &base)]);
        let compare = |app: &str, core: &str| {
            compare_models(&models(&[("app", app), ("core", core)]), &then).unwrap()
        };
        let changed =
            |modules: &[&str]| Change::Modules(modules.iter().map(|m| m.to_string()).collect());
        // A dependency bump changes its module; a version property alone changes nothing,
        // its uses being interpolated into the compared sections.
        let bumped_property = model("1.0", "<lib.version>2</lib.version>", "");
        assert_eq!(
            compare(&model("1.1", LIB, ""), &bumped_property),
            changed(&["app"])
        );
        // Any other property may be read by a plugin or a filtered resource.
        let arg_line = model(
            "1.0",
            "<lib.version>1</lib.version><argLine>-Xmx1g</argLine>",
            "",
        );
        assert_eq!(compare(&base, &arg_line), changed(&["core"]));
        assert_eq!(
            compare(&base, &model("1.0", "<java.version>21</java.version>", "")),
            changed(&["core"])
        );
        let filtering = "<resources><resource><filtering>true</filtering></resource></resources>";
        let filtered = models(&[
            ("app", &model("1.0", LIB, filtering)),
            ("core", &model("1.0", LIB, filtering)),
        ]);
        let filtered_now = models(&[
            (
                "app",
                &model("1.0", "<lib.version>2</lib.version>", filtering),
            ),
            ("core", &model("1.0", LIB, filtering)),
        ]);
        assert_eq!(
            compare_models(&filtered_now, &filtered).unwrap(),
            changed(&["app"])
        );
        // Comments, such as the help plugin's date stamp, and layout are no change.
        assert_eq!(
            compare(
                &base.replace("on 1.0", "on 2.0"),
                &base.replace("\n  ", "\n    ")
            ),
            changed(&[])
        );
        // Plugin versions and other build settings are build configuration, and so are
        // added or removed modules.
        assert!(matches!(
            compare(&base.replace("3.5", "3.6"), &base),
            Change::All(_)
        ));
        assert!(matches!(
            compare(&model("1.0", LIB, "<finalName>x</finalName>"), &base),
            Change::All(_)
        ));
        let fewer = models(&[("app", &base)]);
        assert!(matches!(
            compare_models(&fewer, &then).unwrap(),
            Change::All(_)
        ));
    }

    #[test]
    fn plugin_dependencies_are_build_configuration() {
        let with = |version: &str| {
            format!("<project><build><plugins><plugin><artifactId>p</artifactId><dependencies><dependency><artifactId>d</artifactId><version>{version}</version></dependency></dependencies></plugin></plugins></build></project>")
        };
        let then = models(&[(".", &with("1"))]);
        let now = models(&[(".", &with("2"))]);
        assert!(matches!(
            compare_models(&now, &then).unwrap(),
            Change::All(_)
        ));
    }
}
