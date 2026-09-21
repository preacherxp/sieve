#![allow(dead_code)]
use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

pub const BIN: &str = env!("CARGO_BIN_EXE_sieve");
pub const ROOT: &str = env!("CARGO_MANIFEST_DIR");

pub fn cli(args: &[&str]) -> Output {
    Command::new(BIN)
        .current_dir(ROOT)
        .args(args)
        .output()
        .unwrap()
}

pub fn success(args: &[&str]) -> Value {
    let output = cli(args);
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or(Value::Null)
}

pub fn git(workspace: &str, args: &[&str]) -> String {
    let output = Command::new("git")
        .args([
            "-C",
            workspace,
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

pub fn select(workspace: &str, base: &str) -> Value {
    success(&["select", "--workspace", workspace, "--base", base])
}

pub fn write(root: &Path, path: &str, contents: impl AsRef<[u8]>) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

pub fn fixture(tool: &str) -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    success(&[
        "fixtures",
        "prepare",
        "--tool",
        tool,
        "--dest",
        temp.path().join("project").to_str().unwrap(),
        "--git",
    ]);
    temp
}

pub fn commit(root: &Path, message: &str) -> String {
    let root = root.to_str().unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", message]);
    git(root, &["rev-parse", "HEAD"])
}

#[cfg(unix)]
pub fn executable(path: &Path, contents: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}
