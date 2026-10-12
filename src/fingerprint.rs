use crate::Result;
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

// ponytail: conventional workspace build inputs only; custom external models still
// require graph review. Do not claim detection of undeclared runtime dependencies.
pub fn build_inputs(workspace: &Path) -> Result<String> {
    build_inputs_with_logic(workspace, &[])
}

pub fn build_inputs_with_logic(workspace: &Path, build_logic: &[String]) -> Result<String> {
    fn collect(
        root: &Path,
        dir: &Path,
        all: bool,
        build_logic: &[String],
        inputs: &mut BTreeMap<String, Vec<u8>>,
    ) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let path = entry.path();
            let kind = entry.file_type()?;
            let project_dir = kind.is_dir()
                && ["pom.xml", "build.gradle", "build.gradle.kts"]
                    .iter()
                    .any(|file| path.join(file).is_file());
            if !project_dir
                && matches!(
                    name.as_ref(),
                    ".git" | "target" | "build" | ".gradle" | ".kotlin" | ".idea" | "node_modules"
                )
            {
                continue;
            }
            let build_dir = build_logic.iter().any(|dir| path == root.join(dir))
                || matches!(
                    name.as_ref(),
                    ".mvn" | "gradle" | "buildSrc" | "build-logic"
                );
            if build_dir && kind.is_symlink() {
                return Err(
                    format!("Unsupported linked build directory: {}", path.display()).into(),
                );
            }
            if kind.is_dir()
                && (all
                    || name != "src"
                    || project_dir
                    || build_logic
                        .iter()
                        .any(|dir| root.join(dir).starts_with(&path)))
            {
                collect(root, &path, all || build_dir, build_logic, inputs)?;
            } else if all
                || name.ends_with(".gradle")
                || name.ends_with(".gradle.kts")
                || matches!(
                    name.as_ref(),
                    "pom.xml"
                        | "build.gradle"
                        | "build.gradle.kts"
                        | "settings.gradle"
                        | "settings.gradle.kts"
                        | "gradle.properties"
                        | "gradle.lockfile"
                        | "mvnw"
                        | "mvnw.cmd"
                        | "gradlew"
                        | "gradlew.bat"
                )
            {
                if !kind.is_file() {
                    return Err(format!("Unsupported build input: {}", path.display()).into());
                }
                inputs.insert(
                    path.strip_prefix(root)?
                        .to_string_lossy()
                        .replace('\\', "/"),
                    fs::read(path)?,
                );
            }
        }
        Ok(())
    }
    let mut inputs = BTreeMap::new();
    collect(workspace, workspace, false, build_logic, &mut inputs)?;
    let mut hash = Command::new("git")
        .args(["hash-object", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    hash.stdin
        .take()
        .ok_or("Missing hash stdin")?
        .write_all(&serde_json::to_vec(&inputs)?)?;
    let output = hash.wait_with_output()?;
    if !output.status.success() {
        return Err("Cannot fingerprint build inputs".into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_named_modules_are_not_mistaken_for_generated_outputs() -> Result<()> {
        let temp = tempfile::tempdir()?;
        for name in ["build", "target", "src"] {
            let module = temp.path().join(name);
            fs::create_dir(&module)?;
            fs::write(module.join("pom.xml"), "<project/>")?;
            let before = build_inputs(temp.path())?;
            fs::write(
                module.join("pom.xml"),
                "<project><!-- changed --></project>",
            )?;
            assert_ne!(before, build_inputs(temp.path())?);
        }
        Ok(())
    }

    #[test]
    fn discovered_plugin_sources_and_resources_affect_the_fingerprint() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path();
        let logic = vec!["tools/conventions".into()];
        let source = root.join("tools/conventions/src/main/java/Convention.java");
        fs::create_dir_all(source.parent().unwrap())?;
        fs::write(&source, "class Convention {}")?;
        let before = build_inputs_with_logic(root, &logic)?;
        fs::write(&source, "class Convention { int changed; }")?;
        assert_ne!(before, build_inputs_with_logic(root, &logic)?);
        let before = build_inputs_with_logic(root, &logic)?;
        let resource = root.join("tools/conventions/src/main/resources/policy.md");
        fs::create_dir_all(resource.parent().unwrap())?;
        fs::write(resource, "new plugin input")?;
        assert_ne!(before, build_inputs_with_logic(root, &logic)?);
        let before = build_inputs_with_logic(root, &logic)?;
        fs::create_dir_all(root.join("tools/conventions/build"))?;
        fs::write(root.join("tools/conventions/build/output"), "ignored")?;
        assert_eq!(before, build_inputs_with_logic(root, &logic)?);
        Ok(())
    }
}
