//! What a build had to fetch before it could run: artifacts it added to the Maven or Gradle
//! dependency cache, and container images that were not present before. Two runs compare
//! fairly only when neither fetched what the other already had: a selected run that pulls a
//! database image the full run then reuses looks slower than it is, and the reverse.
//! Maven runs with `-ntp`, so downloads never reach the log; the caches themselves show them.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime},
};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ColdStart {
    /// Artifacts (jars, POMs, Gradle module files) the build added to its dependency cache.
    pub artifacts: usize,
    pub artifact_bytes: u64,
    /// Container images present after the build that were not present before it.
    pub images: usize,
    /// Seconds Testcontainers reported for pulling images, where its log shows them.
    pub image_pull_seconds: f64,
}

impl ColdStart {
    /// Whether the build fetched anything, so its time includes a cold start.
    pub fn fetched(&self) -> bool {
        self.artifacts > 0 || self.images > 0 || self.image_pull_seconds > 0.0
    }

    /// One line for build output, or `None` when nothing was fetched.
    pub fn describe(&self) -> Option<String> {
        if !self.fetched() {
            return None;
        }
        let mut parts = Vec::new();
        if self.artifacts > 0 {
            parts.push(format!(
                "added {} artifacts ({:.1} MB) to the dependency cache",
                self.artifacts,
                self.artifact_bytes as f64 / 1_000_000.0
            ));
        }
        if self.images > 0 {
            parts.push(format!("pulled {} container images", self.images));
        }
        if self.image_pull_seconds > 0.0 {
            parts.push(format!(
                "spent {:.1} s pulling images",
                self.image_pull_seconds
            ));
        }
        Some(format!(
            "this build {}; compare its time only with runs that fetched the same",
            parts.join(", ")
        ))
    }
}

/// Each artifact file below a cache, with its size and when it was last written.
type Contents = BTreeMap<PathBuf, (u64, Option<SystemTime>)>;

/// The caches as a build found them, taken right before it starts.
pub struct Probe {
    repository: Option<(PathBuf, Contents)>,
    images: Option<BTreeSet<String>>,
}

/// Starts watching the caches a `tool` build with `args` uses from `workspace`.
pub fn start(workspace: &Path, tool: &str, args: &[String]) -> Probe {
    Probe {
        repository: repository(workspace, tool, args).map(|dir| {
            let contents = contents(&dir);
            (dir, contents)
        }),
        images: images(),
    }
}

impl Probe {
    /// What the build fetched, with Testcontainers' pull times read from its output lines.
    pub fn finish<'a>(self, lines: impl IntoIterator<Item = &'a str>) -> ColdStart {
        let (artifacts, artifact_bytes) = self
            .repository
            .map(|(dir, before)| added(&before, &contents(&dir)))
            .unwrap_or_default();
        let images = match (self.images, images()) {
            (Some(before), Some(after)) => after.difference(&before).count(),
            _ => 0,
        };
        ColdStart {
            artifacts,
            artifact_bytes,
            images,
            image_pull_seconds: lines
                .into_iter()
                .filter_map(pull_seconds)
                .fold(0.0, |sum, seconds| sum + seconds),
        }
    }
}

/// The dependency cache the build writes: Maven's local repository, or Gradle's module cache.
fn repository(workspace: &Path, tool: &str, args: &[String]) -> Option<PathBuf> {
    let home = || env::var_os("HOME").map(PathBuf::from);
    let value = |names: &[&str]| -> Option<String> {
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            for name in names {
                if arg == name {
                    return args.next().cloned();
                }
                if let Some(value) = arg.strip_prefix(&format!("{name}=")) {
                    return Some(value.to_owned());
                }
            }
        }
        None
    };
    match tool {
        "maven" => {
            let config =
                fs::read_to_string(workspace.join(".mvn/maven.config")).unwrap_or_default();
            let options = env::var("MAVEN_OPTS").unwrap_or_default();
            let words: Vec<String> = config
                .split_whitespace()
                .chain(options.split_whitespace())
                .map(str::to_owned)
                .chain(args.iter().cloned())
                .collect();
            // The last definition wins, as on Maven's command line.
            let local = words
                .iter()
                .rev()
                .find_map(|word| word.strip_prefix("-Dmaven.repo.local=").map(str::to_owned));
            local
                .map(|path| workspace.join(path))
                .or_else(|| Some(home()?.join(".m2/repository")))
        }
        "gradle" => value(&["-g", "--gradle-user-home"])
            .map(|path| workspace.join(path))
            .or_else(|| env::var_os("GRADLE_USER_HOME").map(PathBuf::from))
            .or_else(|| Some(home()?.join(".gradle")))
            .map(|home| home.join("caches/modules-2/files-2.1")),
        _ => None,
    }
}

/// Artifact files below `dir`. Checksums, metadata and download markers are left out.
fn contents(dir: &Path) -> Contents {
    const ARTIFACTS: &[&str] = &["jar", "pom", "module", "aar", "war", "klib"];
    let mut found = Contents::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file()
                && path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| ARTIFACTS.contains(&e))
            {
                if let Ok(meta) = entry.metadata() {
                    found.insert(path, (meta.len(), written(&meta)));
                }
            }
        }
    }
    found
}

/// Artifacts that are new or were written again, such as a refreshed SNAPSHOT, and their bytes.
fn added(before: &Contents, after: &Contents) -> (usize, u64) {
    after
        .iter()
        .filter(|(path, entry)| before.get(*path) != Some(entry))
        .fold((0, 0), |(count, bytes), (_, (len, _))| {
            (count + 1, bytes + len)
        })
}

/// When a file arrived. Resolvers give downloads the server's modification time, so on Unix
/// the inode change time, which every write and rename sets, is the one to trust.
fn written(meta: &fs::Metadata) -> Option<SystemTime> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let seconds = u64::try_from(meta.ctime()).ok()?;
        let nanos = u32::try_from(meta.ctime_nsec()).ok()?;
        Some(SystemTime::UNIX_EPOCH + Duration::new(seconds, nanos))
    }
    #[cfg(not(unix))]
    {
        meta.created().or_else(|_| meta.modified()).ok()
    }
}

/// The ids of the local container images, or `None` without a reachable Docker.
fn images() -> Option<BTreeSet<String>> {
    let output = Command::new("docker")
        .args(["image", "ls", "--all", "--quiet", "--no-trunc"])
        .output()
        .ok()?;
    output.status.success().then(|| {
        String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    })
}

/// Testcontainers' `Pull complete. 14 layers, pulled in 12s (downloaded …)`.
fn pull_seconds(line: &str) -> Option<f64> {
    let rest = line.split_once("pulled in ")?.1;
    let number: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    rest[number.len()..]
        .starts_with('s')
        .then(|| number.parse().ok())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_and_rewritten_artifacts_count_and_metadata_does_not() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("org/old/1.0");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("old-1.0.jar"), "old").unwrap();
        let before = contents(temp.path());
        assert_eq!(added(&before, &contents(temp.path())), (0, 0));
        let new = temp.path().join("org/new/2.0");
        fs::create_dir_all(&new).unwrap();
        for (name, body) in [
            ("new-2.0.jar", "12345"),
            ("new-2.0.pom", "123"),
            ("new-2.0.jar.sha1", "x"),
            ("_remote.repositories", "x"),
            ("new-2.0.pom.lastUpdated", "x"),
        ] {
            fs::write(new.join(name), body).unwrap();
        }
        // A resolver stamps a download with the server's time; it still counts.
        fs::File::options()
            .write(true)
            .open(new.join("new-2.0.jar"))
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000))
            .unwrap();
        assert_eq!(added(&before, &contents(temp.path())), (2, 8));
        // A refreshed SNAPSHOT is written again at the same path.
        let after = contents(temp.path());
        fs::write(old.join("old-1.0.jar"), "newer").unwrap();
        assert_eq!(added(&after, &contents(temp.path())), (1, 5));
    }

    #[test]
    fn caches_follow_the_build_arguments() {
        let workspace = Path::new("/work/app");
        let args = |list: &[&str]| list.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        assert_eq!(
            repository(
                workspace,
                "maven",
                &args(&["-q", "-Dmaven.repo.local=.repo"])
            ),
            Some(PathBuf::from("/work/app/.repo"))
        );
        assert_eq!(
            repository(
                workspace,
                "gradle",
                &args(&["--gradle-user-home", "/cache/g"])
            ),
            Some(PathBuf::from("/cache/g/caches/modules-2/files-2.1"))
        );
        assert_eq!(
            repository(workspace, "gradle", &args(&["--gradle-user-home=/cache/h"])),
            Some(PathBuf::from("/cache/h/caches/modules-2/files-2.1"))
        );
        assert_eq!(repository(workspace, "jest", &[]), None);
    }

    #[test]
    fn testcontainers_pull_times_are_read_from_the_log() {
        assert_eq!(
            pull_seconds("Pull complete. 14 layers, pulled in 12s (downloaded 150 MB at 12 MB/s)"),
            Some(12.0)
        );
        assert_eq!(pull_seconds("pulled in 1.5s"), Some(1.5));
        assert_eq!(pull_seconds("Container started in PT1.2S"), None);
        assert_eq!(pull_seconds("pulled in 3 layers"), None);
        let cold = ColdStart {
            images: 1,
            image_pull_seconds: 12.0,
            ..ColdStart::default()
        };
        assert!(cold.fetched());
        assert!(cold
            .describe()
            .unwrap()
            .contains("pulled 1 container images"));
        assert_eq!(ColdStart::default().describe(), None);
    }
}
