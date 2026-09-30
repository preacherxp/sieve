//! Sources that the build generates from declared inputs, such as OpenAPI specifications,
//! compared with those it generates at the comparison base.
use crate::{Config, Result};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

/// Maven's generated source roots, below each module. The compiler's annotation-processor
/// output is left out: it follows sources that class-level selection already maps.
const ROOTS: &[&str] = &["target/generated-sources", "target/generated-test-sources"];
const PROCESSOR_OUTPUT: &[&str] = &[
    "target/generated-sources/annotations",
    "target/generated-test-sources/test-annotations",
];

/// Source generation running in a temporary worktree of the comparison base.
pub struct Base {
    repository: PathBuf,
    worktree: PathBuf,
    workspace: PathBuf,
    child: Option<Child>,
    _temp: tempfile::TempDir,
}

impl Base {
    /// Starts generating the sources of `modules` at `revision`. `prefix` is the workspace
    /// path below the repository root.
    pub fn start(
        config: &Config,
        modules: &BTreeSet<String>,
        workspace: &Path,
        prefix: &str,
        revision: &str,
        executable: &str,
        extra: &[String],
    ) -> Result<Self> {
        let repository = PathBuf::from(
            String::from_utf8(crate::git(workspace, &["rev-parse", "--show-toplevel"])?)?.trim(),
        );
        let temp = tempfile::tempdir()?;
        let worktree = temp.path().join("base");
        let path = worktree.to_str().ok_or("Non-UTF-8 temporary path")?;
        crate::git(
            &repository,
            &["worktree", "add", "--detach", "--quiet", path, revision],
        )?;
        let mut base = Self {
            workspace: worktree.join(prefix),
            repository,
            worktree,
            child: None,
            _temp: temp,
        };
        base.child = Some(
            Command::new(executable)
                .current_dir(&base.workspace)
                .args(["-B", "-ntp", "-q"])
                .args(crate::scope(config, modules, "generate-test-sources"))
                .args(extra)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?,
        );
        Ok(base)
    }

    /// Waits for generation and returns the workspace-relative generated files that differ,
    /// including those that exist on only one side.
    pub fn changes(mut self, modules: &BTreeSet<String>, head: &Path) -> Result<BTreeSet<String>> {
        let status = self.child.take().ok_or("Generation not started")?.wait()?;
        if !status.success() {
            return Err("source generation failed at the comparison base".into());
        }
        let mut changed = BTreeSet::new();
        for module in modules {
            for root in ROOTS {
                let relative = Path::new(module).join(root);
                let base = files(&self.workspace.join(&relative))?;
                let current = files(&head.join(&relative))?;
                for path in base.keys().chain(current.keys()) {
                    if base.get(path) != current.get(path) {
                        let path = relative.join(path);
                        let path = path.strip_prefix(".").unwrap_or(&path);
                        let path = path.to_str().ok_or("Non-UTF-8 path")?.replace('\\', "/");
                        if !PROCESSOR_OUTPUT.iter().any(|p| path.starts_with(p)) {
                            changed.insert(path);
                        }
                    }
                }
            }
        }
        Ok(changed)
    }
}

impl Drop for Base {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(path) = self.worktree.to_str() {
            let _ = crate::git(&self.repository, &["worktree", "remove", "--force", path]);
        }
    }
}

/// Java and Kotlin sources below `dir` by relative path, with generation timestamps
/// removed. Generator metadata, such as `.openapi-generator/FILES`, is not compiled.
fn files(dir: &Path) -> Result<BTreeMap<PathBuf, String>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, String>) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                walk(root, &path, out)?;
            } else if path.extension().is_some_and(|e| e == "java" || e == "kt") {
                let text = String::from_utf8_lossy(&fs::read(&path)?).into_owned();
                out.insert(path.strip_prefix(root)?.to_owned(), normalize(&text));
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    if dir.is_dir() {
        walk(dir, dir, &mut out)?;
    }
    Ok(out)
}

/// Drops `@Generated(...)` annotation lines, which carry the generation date.
fn normalize(text: &str) -> String {
    text.lines()
        .filter(|line| {
            let line = line.trim_start();
            !(line.starts_with('@') && line.contains("Generated("))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_dates_do_not_count_as_changes() {
        let at = |date: &str| {
            format!("package a;\n@jakarta.annotation.Generated(value = \"x\", date = \"{date}\")\nclass A {{}}\n")
        };
        assert_eq!(normalize(&at("2026-01-01")), normalize(&at("2026-02-02")));
        assert_ne!(normalize("class A {}"), normalize("class B {}"));
    }

    #[test]
    fn only_compiled_sources_are_compared() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".openapi-generator")).unwrap();
        fs::create_dir_all(dir.path().join("src/a")).unwrap();
        fs::write(dir.path().join(".openapi-generator/FILES"), "src/a/A.java").unwrap();
        fs::write(dir.path().join("src/a/A.java"), "class A {}").unwrap();
        fs::write(dir.path().join("src/a/B.kt"), "class B").unwrap();
        let found: Vec<_> = files(dir.path()).unwrap().into_keys().collect();
        assert_eq!(
            found,
            [PathBuf::from("src/a/A.java"), PathBuf::from("src/a/B.kt")]
        );
    }
}
