//! Compiles the test-JVM agent (`agent/`) into two jars that the binary embeds: the agent
//! itself, with the vendored ASM (`agent/asm`), and the probe it adds to the bootstrap class
//! path. Needs a JDK 17+ `javac`, found
//! through `SIEVE_JAVA_HOME`, `JAVA_HOME`, `JAVA_HOME_<version>_*` (as set by CI setup
//! actions), or `PATH`. Build with `--no-default-features` to leave the agent out.
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

const MINIMUM: u32 = 17;

fn main() {
    println!("cargo:rerun-if-changed=agent");
    println!("cargo:rerun-if-env-changed=SIEVE_JAVA_HOME");
    println!("cargo:rerun-if-env-changed=JAVA_HOME");
    if env::var_os("CARGO_FEATURE_AGENT").is_none() {
        return;
    }
    let bin = javac().unwrap_or_else(|| {
        panic!(
            "Building the sieve agent needs JDK {MINIMUM}+ javac. Set SIEVE_JAVA_HOME or JAVA_HOME \
             to a JDK {MINIMUM}+, or build without the agent: cargo build --no-default-features"
        )
    });
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let classes = out.join("agent-classes");
    let _ = fs::remove_dir_all(&classes);
    let probe = classes.join("probe");
    let stubs = classes.join("stubs");
    let agent = classes.join("agent");
    compile(&bin, "17", "agent/probe", &probe, &[], None, None);
    compile(&bin, "17", "agent/stubs", &stubs, &[], None, None);
    compile(&bin, "17", "agent/asm", &agent, &[], None, None);
    // The entry point loads on any JVM that `JDK_JAVA_OPTIONS` reaches, back to Java 8.
    let boot = Path::new("agent/src/sieve/agent/Boot.java");
    compile(&bin, "8", "agent/src", &agent, &[], None, Some(boot));
    let classpath: &[&Path] = &[&probe, &stubs, &agent];
    compile(&bin, "17", "agent/src", &agent, classpath, None, None);
    fs::create_dir_all(agent.join("META-INF")).unwrap();
    fs::copy(
        "agent/asm/LICENSE.txt",
        agent.join("META-INF/ASM-LICENSE.txt"),
    )
    .unwrap();
    copy(Path::new("agent/resources"), &agent);
    let manifest = classes.join("agent.mf");
    fs::write(
        &manifest,
        "Premain-Class: sieve.agent.Boot\nCan-Retransform-Classes: true\nBoot-Class-Path: sieve-probe.jar\n",
    )
    .unwrap();
    jar(&bin, &out.join("sieve-agent.jar"), &agent, Some(&manifest));
    jar(&bin, &out.join("sieve-probe.jar"), &probe, None);
}

fn version(javac: &Path) -> Option<u32> {
    let output = Command::new(javac).arg("-version").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    let version = text.lines().find_map(|l| l.strip_prefix("javac "))?;
    version.split(['.', '-', '+']).next()?.trim().parse().ok()
}

fn javac() -> Option<PathBuf> {
    let mut homes: Vec<PathBuf> = ["SIEVE_JAVA_HOME", "JAVA_HOME"]
        .iter()
        .filter_map(env::var_os)
        .map(PathBuf::from)
        .collect();
    let mut versioned: Vec<(String, PathBuf)> = env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, PathBuf::from(v))))
        .filter(|(k, _)| k.starts_with("JAVA_HOME_"))
        .collect();
    versioned.sort();
    homes.extend(versioned.into_iter().rev().map(|(_, v)| v));
    let mut candidates: Vec<PathBuf> = homes.iter().map(|h| h.join("bin").join("javac")).collect();
    candidates.push(PathBuf::from("javac"));
    candidates
        .into_iter()
        .find(|c| version(c).is_some_and(|v| v >= MINIMUM))
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "java") {
            out.push(path);
        }
    }
}

/// Compiles the sources below `dir`: only `only` when given, otherwise all but `exclude`, and
/// with the output on the classpath, only the files not compiled yet.
fn compile(
    javac: &Path,
    release: &str,
    dir: &str,
    out: &Path,
    classpath: &[&Path],
    exclude: Option<&Path>,
    only: Option<&Path>,
) {
    let mut files = Vec::new();
    sources(Path::new(dir), &mut files);
    files.retain(|f| Some(f.as_path()) != exclude && only.is_none_or(|o| f == o));
    if classpath.contains(&out) {
        files.retain(|f| {
            let relative = f.strip_prefix(dir).unwrap().with_extension("class");
            !out.join(relative).is_file()
        });
    }
    let mut command = Command::new(javac);
    command
        .args(["--release", release, "-nowarn", "-encoding", "UTF-8", "-d"])
        .arg(out);
    if !classpath.is_empty() {
        command
            .arg("-classpath")
            .arg(env::join_paths(classpath).unwrap());
    }
    let status = command.args(&files).status().unwrap();
    assert!(status.success(), "javac failed for {dir}");
}

fn copy(from: &Path, to: &Path) {
    for entry in fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            fs::create_dir_all(&target).unwrap();
            copy(&path, &target);
        } else {
            fs::copy(&path, &target).unwrap();
        }
    }
}

fn jar(javac: &Path, file: &Path, dir: &Path, manifest: Option<&Path>) {
    let tool = javac.with_file_name(if cfg!(windows) { "jar.exe" } else { "jar" });
    let tool = if tool.is_file() {
        tool
    } else {
        PathBuf::from("jar")
    };
    let mut command = Command::new(tool);
    // A fixed timestamp keeps the jar, and so its cache directory, reproducible.
    command
        .args(["--create", "--date=2000-01-01T00:00:00Z", "--file"])
        .arg(file);
    if let Some(manifest) = manifest {
        command.arg("--manifest").arg(manifest);
    }
    let status = command.arg("-C").arg(dir).arg(".").status().unwrap();
    assert!(status.success(), "jar failed for {}", file.display());
}
