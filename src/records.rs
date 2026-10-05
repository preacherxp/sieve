//! Local mode (ADR 0001, ADR 0002): the agent runs inside the test JVM, asks `sieve decide`
//! which test classes to drop at discovery, and hands what each class executed to
//! `sieve record`. A test class is dropped when its last run passed and nothing it executed
//! or read has changed since, or, without a record, when static analysis shows it reaches no
//! change since a green `--base`.
use crate::{bytecode, classes, fingerprint, settings, Config, Result, Selection};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    io::Write,
    path::{Component, Path, PathBuf},
    process::Command,
    time::{Duration, Instant, SystemTime},
};

const DIR: &str = ".sieve";

/// What one test class did across its passing runs since the class itself last changed.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Record {
    /// Digest of the test class and its nested classes.
    test: String,
    passed: bool,
    jdk: String,
    /// Invocation, test-JVM properties and declared environment inputs.
    #[serde(default)]
    context: String,
    /// Ran against a Spring test context, whose startup work is included.
    #[serde(default)]
    spring: bool,
    build: String,
    /// The set of resource files on the test classpath.
    resources: String,
    /// Class shapes when the record was last extended.
    snapshot: String,
    /// `owner#name(descriptor)` to body hash.
    methods: BTreeMap<String, String>,
    /// Workspace-relative path to content hash; `-` for a file that did not exist.
    files: BTreeMap<String, String>,
    /// Digest of the test class path's jars when the record was last extended.
    #[serde(default)]
    classpath: String,
    /// Content hash to slot of each class-path jar whose code ran or whose entries were read.
    #[serde(default)]
    jars: BTreeMap<String, String>,
}

/// What a class looked like when a record was last extended. Fields missing from older
/// snapshots compare as changed.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
struct Shape {
    shape: String,
    wiring: String,
    supers: Vec<String>,
    component: bool,
    #[serde(default)]
    api: String,
    #[serde(default)]
    declared: String,
    #[serde(default)]
    injection: String,
    #[serde(default)]
    wired: BTreeMap<String, bytecode::Wired>,
    #[serde(default)]
    plain: BTreeSet<String>,
    #[serde(default)]
    routes: BTreeMap<String, Vec<String>>,
    /// Compiled from test sources.
    #[serde(default)]
    test: bool,
}

type Snapshot = BTreeMap<String, Shape>;

#[derive(Deserialize)]
struct Raw {
    jdk: String,
    #[serde(default)]
    context: String,
    #[serde(default)]
    session: String,
    /// When the test JVM started, in epoch milliseconds.
    #[serde(default)]
    started: u64,
    parallel: bool,
    errors: Vec<String>,
    dropped: Vec<String>,
    /// Jars of the test class path outside the workspace.
    #[serde(default)]
    classpath: Vec<String>,
    tests: Vec<RawTest>,
}

#[derive(Deserialize)]
struct RawTest {
    name: String,
    passed: bool,
    spring: bool,
    methods: Vec<String>,
    files: Vec<String>,
}

/// What ran in one test JVM, for `sieve run --output`.
#[derive(Default, Serialize, Deserialize)]
struct Summary {
    ran: BTreeSet<String>,
    dropped: BTreeSet<String>,
    notes: Vec<String>,
}

fn prepare(workspace: &Path) -> Result<PathBuf> {
    let dir = workspace.join(DIR);
    for sub in ["records", "snapshots", "settings", "classpaths", "run"] {
        fs::create_dir_all(dir.join(sub))?;
    }
    let ignore = dir.join(".gitignore");
    if !ignore.is_file() {
        fs::write(ignore, "*\n")?;
    }
    Ok(dir)
}

fn options(args: Vec<String>, names: &[&str]) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if !names.contains(&arg.as_str()) {
            return Err(format!("Unknown option: {arg}").into());
        }
        let value = args
            .next()
            .ok_or_else(|| format!("Missing value for {arg}"))?;
        out.insert(arg, value);
    }
    Ok(out)
}

fn workspace_option(options: &BTreeMap<String, String>) -> Result<PathBuf> {
    Ok(PathBuf::from(
        options
            .get("--workspace")
            .ok_or("--workspace is required")?,
    )
    .canonicalize()?)
}

/// Native locks are released on process exit, including crashes.
struct Lock(fs::File);

impl Lock {
    fn open(dir: &Path, name: &str) -> Result<Self> {
        Ok(Self(
            fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(dir.join(name))?,
        ))
    }

    fn acquire(dir: &Path, name: &str) -> Result<Self> {
        let lock = Self::open(dir, name)?;
        let start = Instant::now();
        loop {
            match lock.0.try_lock() {
                Ok(()) => return Ok(lock),
                Err(fs::TryLockError::WouldBlock) => {
                    if start.elapsed() > Duration::from_secs(600) {
                        return Err(
                            format!("Timed out waiting for {}", dir.join(name).display()).into(),
                        );
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(fs::TryLockError::Error(error)) => return Err(error.into()),
            }
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

/// Wrapper forks share its session. An unrelated `sieve env` invocation fails open while
/// that wrapper owns the workspace; it must neither select nor write its shared run state.
fn execution_guard(dir: &Path, session: &str) -> Result<Option<Lock>> {
    let lock = Lock::open(dir, "execution.lock")?;
    match lock.0.try_lock() {
        Ok(()) if session.is_empty() => Ok(Some(lock)),
        Ok(()) => Err("The managed test JVM outlived its Sieve run; no records kept".into()),
        Err(fs::TryLockError::WouldBlock)
            if !session.is_empty() && fs::read_to_string(dir.join("session"))? == session =>
        {
            Ok(None)
        }
        Err(fs::TryLockError::WouldBlock) => {
            Err("Another Sieve run owns this workspace; running without record selection".into())
        }
        Err(fs::TryLockError::Error(error)) => Err(error.into()),
    }
}

fn run_dir(dir: &Path, session: &str) -> PathBuf {
    dir.join(if session.is_empty() { "env-run" } else { "run" })
}

/// The workspace's build tool: from `impact.json`, or Maven for the configuration-free default.
fn tool(workspace: &Path) -> String {
    read_json::<serde_json::Value>(&workspace.join("impact.json"))
        .and_then(|config| config["tool"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "maven".into())
}

/// Compiled classes and resources, each marked `true` when it holds test output. Gradle keeps
/// one directory per language and source set; sets named like `test` or `integTest` hold tests.
fn outputs(workspace: &Path) -> Vec<(PathBuf, bool)> {
    if tool(workspace) != "gradle" {
        let target = workspace.join("target");
        return vec![
            (target.join("classes"), false),
            (target.join("test-classes"), true),
        ];
    }
    let build = workspace.join("build");
    // The agent receives these before the first compilation, so conventional ones always count.
    let mut dirs: Vec<PathBuf> = ["java", "kotlin", "groovy", "scala"]
        .iter()
        .flat_map(|l| ["main", "test"].map(|s| build.join("classes").join(l).join(s)))
        .chain(["main", "test"].map(|s| build.join("resources").join(s)))
        .collect();
    let languages = fs::read_dir(build.join("classes"))
        .into_iter()
        .flatten()
        .flatten();
    for language in languages.map(|e| e.path()) {
        for set in fs::read_dir(&language).into_iter().flatten().flatten() {
            dirs.push(set.path());
        }
    }
    for set in fs::read_dir(build.join("resources"))
        .into_iter()
        .flatten()
        .flatten()
    {
        dirs.push(set.path());
    }
    dirs.sort();
    dirs.dedup();
    dirs.into_iter()
        .map(|dir| {
            let set = dir.file_name().map(|n| n.to_string_lossy().to_lowercase());
            let test = set.is_some_and(|s| s.contains("test"));
            (dir, test)
        })
        .collect()
}

/// Marks directories whose listing a test read, as the agent reports them.
const LISTED: &str = "ls:";

/// The state of a recorded path: the content hash of a file, `dir` for a directory, or `-`
/// when it does not exist. Spring Boot's build information is hashed without its timestamp,
/// which every native build rewrites.
fn file_hash(path: &Path) -> String {
    if path.is_dir() {
        return "dir".into();
    }
    file_state(path)
}

/// The state of a recorded key: for a listed directory (`ls:` prefix), its listing.
fn recorded_hash(workspace: &Path, key: &str) -> String {
    match key.strip_prefix(LISTED) {
        Some(dir) => listing_hash(&workspace.join(dir)),
        None => file_hash(&workspace.join(key)),
    }
}

fn listing_hash(path: &Path) -> String {
    if path.is_dir() {
        // A listing counts; class files are left out, because component scanning is covered
        // by the Spring wiring rule and class changes by the method and shape rules.
        return format!(
            "dir:{}",
            bytecode::hash(listing(path).join("\n").as_bytes())
        );
    }
    file_state(path)
}

fn file_state(path: &Path) -> String {
    let Ok(bytes) = fs::read(path) else {
        return "-".into();
    };
    if path.ends_with("META-INF/build-info.properties") {
        let text = String::from_utf8_lossy(&bytes);
        let kept: Vec<&str> = text
            .lines()
            .filter(|l| !l.starts_with("build.time="))
            .collect();
        return bytecode::hash(kept.join("\n").as_bytes());
    }
    bytecode::hash(&bytes)
}

/// The build output as it is now.
struct Current {
    digests: BTreeMap<String, bytecode::Digest>,
    classes: Vec<classes::Class>,
    index: BTreeMap<String, usize>,
    component: Vec<bool>,
    context: Vec<bool>,
    build: String,
    resources: String,
    /// Modification time of the newest class file.
    newest: SystemTime,
}

/// Build inputs: the workspace's, and the parent POMs it inherits from outside it.
fn build_inputs(workspace: &Path) -> Result<String> {
    let mut h = bytecode::Hasher::default();
    h.field(fingerprint::build_inputs(workspace)?.as_bytes());
    let mut pom = workspace.join("pom.xml");
    for _ in 0..8 {
        let Ok(xml) = fs::read_to_string(&pom) else {
            break;
        };
        let Some(parent) = xml
            .split_once("<parent>")
            .and_then(|(_, rest)| rest.split_once("</parent>"))
        else {
            break;
        };
        let relative = match parent.0.split_once("<relativePath>") {
            Some((_, rest)) => rest
                .split_once("</relativePath>")
                .map_or("", |(p, _)| p.trim()),
            None if parent.0.contains("<relativePath/>")
                || parent.0.contains("<relativePath />") =>
            {
                ""
            }
            None => "../pom.xml",
        };
        if relative.is_empty() {
            break;
        }
        let dir = pom.parent().ok_or("No parent directory")?;
        let mut next = dir.join(relative);
        if next.is_dir() {
            next = next.join("pom.xml");
        }
        let Ok(bytes) = fs::read(&next) else {
            break;
        };
        if next.starts_with(workspace) {
            break;
        }
        h.field(next.to_string_lossy().as_bytes()).field(&bytes);
        pom = next;
    }
    Ok(h.finish())
}

impl Current {
    fn load(workspace: &Path) -> Result<Self> {
        fn walk(
            root: &Path,
            dir: &Path,
            test: bool,
            current: &mut Current,
            resources: &mut Vec<String>,
        ) -> Result<()> {
            for entry in fs::read_dir(dir)? {
                let path = entry?.path();
                if path.is_dir() {
                    walk(root, &path, test, current, resources)?;
                } else if path.extension().is_some_and(|e| e == "class") {
                    let bytes = fs::read(&path)?;
                    if let Ok(modified) = fs::metadata(&path).and_then(|m| m.modified()) {
                        current.newest = current.newest.max(modified);
                    }
                    let error = |e| format!("{}: {e}", path.display());
                    let mut class = classes::parse(&bytes).map_err(error)?;
                    let digest = bytecode::digest(&bytes).map_err(error)?;
                    if class.name == "module-info" {
                        continue;
                    }
                    class.test = test;
                    current
                        .index
                        .insert(class.name.clone(), current.classes.len());
                    current.digests.insert(digest.name.clone(), digest);
                    current.classes.push(class);
                } else {
                    let relative = path
                        .strip_prefix(root)?
                        .to_string_lossy()
                        .replace('\\', "/");
                    resources.push(relative);
                }
            }
            Ok(())
        }
        let mut current = Current {
            digests: BTreeMap::new(),
            classes: Vec::new(),
            index: BTreeMap::new(),
            component: Vec::new(),
            context: Vec::new(),
            build: build_inputs(workspace)?,
            resources: String::new(),
            newest: SystemTime::UNIX_EPOCH,
        };
        let mut resources = Vec::new();
        for (dir, test) in outputs(workspace) {
            if dir.is_dir() {
                walk(workspace, &dir, test, &mut current, &mut resources)?;
            }
        }
        resources.sort();
        current.resources = bytecode::hash(resources.join("\n").as_bytes());
        (current.component, current.context) = classes::kinds(&current.classes);
        Ok(current)
    }

    fn snapshot(&self) -> Snapshot {
        self.digests
            .iter()
            .map(|(name, d)| {
                // A test class that starts a context is not one of its components.
                let index = self.index.get(name).copied();
                let component = index.is_some_and(|i| self.component[i] && !self.context[i]);
                let shape = Shape {
                    shape: d.shape.clone(),
                    wiring: d.wiring.clone(),
                    supers: d.supers.clone(),
                    component,
                    api: d.api.clone(),
                    declared: d.declared.clone(),
                    injection: d.injection.clone(),
                    wired: d.wired.clone(),
                    plain: d.plain.clone(),
                    routes: d.routes.clone(),
                    test: index.is_some_and(|i| self.classes[i].test),
                };
                (name.clone(), shape)
            })
            .collect()
    }

    /// Digest of a top-level test class (binary name) and its nested classes.
    fn test_hash(&self, test: &str) -> String {
        let internal = test.replace('.', "/");
        let nested = format!("{internal}$");
        let mut h = bytecode::Hasher::default();
        for (name, d) in &self.digests {
            if *name == internal || name.starts_with(&nested) {
                h.field(name.as_bytes()).field(d.shape.as_bytes());
                for (method, hash) in &d.methods {
                    h.field(method.as_bytes()).field(hash.as_bytes());
                }
            }
        }
        h.finish()
    }

    fn method(&self, key: &str) -> Option<&String> {
        let (owner, method) = key.split_once('#')?;
        self.digests.get(owner)?.methods.get(method)
    }

    /// Binary names of the top-level test classes.
    fn tests(&self) -> BTreeSet<String> {
        self.classes
            .iter()
            .filter(|c| c.test && !c.annotation && !c.abstract_ && !c.name.contains('$'))
            .map(|c| c.name.replace('/', "."))
            .collect()
    }

    fn context_test(&self, test: &str) -> bool {
        self.index
            .get(&test.replace('.', "/"))
            .is_some_and(|&i| self.context[i])
    }
}

/// `path` relative to `root`, after resolving `.` and `..` lexically.
fn relative(root: &Path, path: &str) -> Option<String> {
    let mut parts: Vec<Component> = Vec::new();
    for part in Path::new(path).components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    let normalized: PathBuf = parts.iter().collect();
    let relative = normalized.strip_prefix(root).ok()?;
    let relative = relative.to_str()?.replace('\\', "/");
    // Sieve's own state and the build tool's scratch files are not test inputs.
    let scratch = [
        "target/surefire",
        "target/failsafe",
        "build/tmp",
        ".gradle/",
    ];
    let skip = relative.is_empty()
        || relative.starts_with(".sieve/")
        || scratch.iter().any(|s| relative.starts_with(s));
    (!skip).then_some(relative)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let dir = path.parent().ok_or("No parent directory")?;
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    temp.write_all(&serde_json::to_vec_pretty(value)?)?;
    temp.persist(path)?;
    Ok(())
}

fn record_path(dir: &Path, test: &str) -> PathBuf {
    dir.join("records").join(format!("{test}.json"))
}

/// Marks library jars in the method names and paths the agent reports.
const JAR: &str = "jar:";

/// The slot a jar fills on the class path: its file name without the version, so that a bump
/// keeps the slot while a swapped or added artifact changes the set of slots. The version is
/// the name of the repository directory the jar sits in (Maven) or of the one above (Gradle).
// ponytail: file-name slots; two artifacts with one name in different groups share a slot.
fn slot(path: &str) -> String {
    let path = Path::new(path);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = name.strip_suffix(".jar").unwrap_or(&name).to_owned();
    let dirs = path
        .ancestors()
        .skip(1)
        .take(2)
        .filter_map(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()));
    for version in dirs {
        if let Some((artifact, classifier)) = name.split_once(&format!("-{version}")) {
            return format!("{artifact}{classifier}");
        }
    }
    match name.find(|c: char| c.is_ascii_digit()) {
        Some(i) if i > 1 && name.as_bytes()[i - 1] == b'-' => name[..i - 1].to_owned(),
        _ => name,
    }
}

/// Content hashes of jars, cached by size and modification time in `.sieve/jars.json`.
struct Jars {
    path: PathBuf,
    cache: BTreeMap<String, (u64, u128, String)>,
    dirty: bool,
}

impl Jars {
    fn open(dir: &Path) -> Self {
        let path = dir.join("jars.json");
        Self {
            cache: read_json(&path).unwrap_or_default(),
            path,
            dirty: false,
        }
    }

    fn hash(&mut self, jar: &str) -> Option<String> {
        let meta = fs::metadata(jar).ok()?;
        let modified = meta
            .modified()
            .ok()?
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()?
            .as_nanos();
        if let Some((len, stamp, hash)) = self.cache.get(jar) {
            if *len == meta.len() && *stamp == modified {
                return Some(hash.clone());
            }
        }
        let hash = bytecode::hash(&fs::read(jar).ok()?);
        self.cache
            .insert(jar.to_owned(), (meta.len(), modified, hash.clone()));
        self.dirty = true;
        Some(hash)
    }

    /// `(slot, hash)` of every readable jar, sorted.
    fn listing(&mut self, jars: &[String]) -> Vec<(String, String)> {
        let mut listing: Vec<(String, String)> = jars
            .iter()
            .filter_map(|jar| Some((slot(jar), self.hash(jar)?)))
            .collect();
        listing.sort();
        listing
    }

    fn save(&self) -> Result<()> {
        if self.dirty {
            write_json(&self.path, &self.cache)?;
        }
        Ok(())
    }
}

type Listing = Vec<(String, String)>;

fn listing_path(dir: &Path, digest: &str) -> PathBuf {
    dir.join("classpaths").join(format!("{digest}.json"))
}

/// Keeps a listing under its digest, for the records made against it.
fn save_listing(dir: &Path, listing: &Listing) -> Result<String> {
    let digest = bytecode::hash(&serde_json::to_vec(listing)?);
    let path = listing_path(dir, &digest);
    if !path.is_file() {
        write_json(&path, listing)?;
    }
    Ok(digest)
}

/// Why a record's test must run after a build-input change, or `None` when only the contents
/// of jars it never used changed. Anything else that differs between the class paths, such
/// as an added or removed artifact, is not a bump and keeps the full rerun.
fn dependency_change(
    then: Option<&Listing>,
    now: Option<&Listing>,
    record: &Record,
) -> Option<String> {
    let (Some(then), Some(now)) = (then, now) else {
        return Some("Build input changed".into());
    };
    // Listings are sorted, so equal slot sequences mean the same artifacts, each as often.
    let a: Vec<&str> = then.iter().map(|(s, _)| s.as_str()).collect();
    let b: Vec<&str> = now.iter().map(|(s, _)| s.as_str()).collect();
    if a != b {
        let mut diff: Vec<String> = Vec::new();
        for (list, other, sign) in [(&b, &a, '+'), (&a, &b, '-')] {
            let mut seen = other.clone();
            for slot in list {
                match seen.iter().position(|s| s == slot) {
                    Some(i) => {
                        seen.swap_remove(i);
                    }
                    None => diff.push(format!("{sign}{slot}")),
                }
            }
        }
        let more = diff.len().saturating_sub(5);
        diff.truncate(5);
        let more = if more > 0 {
            format!(" and {more} more")
        } else {
            String::new()
        };
        return Some(format!(
            "Build input changed: dependencies added or removed: {}{more}",
            diff.join(", ")
        ));
    }
    let current: BTreeSet<&str> = now.iter().map(|(_, h)| h.as_str()).collect();
    let used = record
        .jars
        .iter()
        .find(|(hash, _)| !current.contains(hash.as_str()) && then.iter().any(|(_, h)| h == *hash));
    used.map(|(_, slot)| format!("Changed dependency: {slot}"))
}

/// `sieve record --workspace PATH --raw FILE`: turns what the agent saw into test records.
pub fn record(args: Vec<String>) -> Result<u8> {
    let options = options(args, &["--workspace", "--raw"])?;
    let workspace = workspace_option(&options)?;
    let raw_path = PathBuf::from(options.get("--raw").ok_or("--raw is required")?);
    let raw: Raw = serde_json::from_slice(&fs::read(&raw_path)?)?;
    let dir = prepare(&workspace)?;
    let _execution = execution_guard(&dir, &raw.session)?;
    let _lock = Lock::acquire(&dir, "lock")?;
    let mut summary = Summary {
        ran: raw.tests.iter().map(|t| t.name.clone()).collect(),
        dropped: raw.dropped.iter().cloned().collect(),
        notes: Vec::new(),
    };
    // Without complete evidence only failures are kept; older records stay as they were, and
    // their hashes no longer match whatever changed since.
    let trusted = !raw.parallel && raw.errors.is_empty() && !raw.context.is_empty();
    if raw.context.is_empty() {
        summary
            .notes
            .push("Agent supplied no invocation context; no records kept".into());
    }
    if raw.parallel {
        summary
            .notes
            .push("Test classes ran in parallel or a probe failed; no records kept".into());
    }
    for error in &raw.errors {
        summary
            .notes
            .push(format!("Agent error, no records kept: {error}"));
    }
    let current = Current::load(&workspace)?;
    // Class files rewritten while the JVM ran, as by an IDE's build, were not what it tested.
    let started = SystemTime::UNIX_EPOCH + Duration::from_millis(raw.started);
    let trusted = trusted && !(raw.started > 0 && current.newest > started);
    if raw.started > 0 && current.newest > started {
        summary
            .notes
            .push("Class files changed while the tests ran; no records kept".into());
    }
    let snapshot = current.snapshot();
    let snapshot_id = bytecode::hash(&serde_json::to_vec(&snapshot)?);
    let mut snapshot_used = false;
    let mut jars = Jars::open(&dir);
    let listing = jars.listing(&raw.classpath);
    let classpath = match listing.is_empty() {
        true => String::new(),
        false => save_listing(&dir, &listing)?,
    };
    let on_classpath: BTreeMap<&str, &str> = listing
        .iter()
        .map(|(s, h)| (h.as_str(), s.as_str()))
        .collect();
    for test in &raw.tests {
        let path = record_path(&dir, &test.name);
        let hash = current.test_hash(&test.name);
        let kept =
            read_json::<Record>(&path).filter(|r| r.test == hash && r.context == raw.context);
        if !test.passed {
            let mut record = kept.unwrap_or_else(|| Record {
                test: hash,
                ..Record::default()
            });
            record.passed = false;
            write_json(&path, &record)?;
            continue;
        }
        if !trusted {
            continue;
        }
        // A passing run that probed none of the test's own methods recorded nothing reliable.
        let own = test.name.replace('.', "/");
        let probed = test.methods.iter().any(|m| {
            m.split_once('#').is_some_and(|(owner, _)| {
                owner == own
                    || owner
                        .strip_prefix(own.as_str())
                        .is_some_and(|r| r.starts_with('$'))
            })
        });
        if !probed {
            summary.notes.push(format!(
                "{}: no method of the test class was probed; no record kept",
                test.name
            ));
            continue;
        }
        let mut methods: BTreeSet<&str> = test
            .methods
            .iter()
            .map(String::as_str)
            .filter(|m| !m.starts_with(JAR))
            .collect();
        let mut files: BTreeSet<String> = test
            .files
            .iter()
            .filter(|f| !f.starts_with(JAR))
            .filter_map(|f| match f.strip_prefix(LISTED) {
                Some(dir) => relative(&workspace, dir).map(|d| format!("{LISTED}{d}")),
                None => relative(&workspace, f),
            })
            .collect();
        // Jars outside the class path, such as other agents, are not the build's to bump.
        let mut used: BTreeMap<String, String> = test
            .methods
            .iter()
            .chain(&test.files)
            .filter_map(|m| m.strip_prefix(JAR))
            .filter_map(|jar| {
                let hash = jars.hash(jar)?;
                let slot = on_classpath.get(hash.as_str())?;
                Some((hash, (*slot).to_owned()))
            })
            .collect();
        let mut spring = test.spring;
        if let Some(kept) = &kept {
            methods.extend(kept.methods.keys().map(String::as_str));
            files.extend(kept.files.keys().cloned());
            used.extend(
                kept.jars
                    .iter()
                    .filter(|(hash, _)| on_classpath.contains_key(hash.as_str()))
                    .map(|(h, s)| (h.clone(), s.clone())),
            );
            spring |= kept.spring;
        }
        let record = Record {
            test: hash,
            passed: true,
            jdk: raw.jdk.clone(),
            context: raw.context.clone(),
            spring,
            build: current.build.clone(),
            resources: current.resources.clone(),
            snapshot: snapshot_id.clone(),
            // A method that no longer exists went with a structural change, which reruns it.
            methods: methods
                .into_iter()
                .filter_map(|m| Some((m.to_owned(), current.method(m)?.clone())))
                .collect(),
            files: files
                .into_iter()
                .map(|f| {
                    let hash = recorded_hash(&workspace, &f);
                    (f, hash)
                })
                .collect(),
            classpath: classpath.clone(),
            jars: used,
        };
        snapshot_used = true;
        save_settings(&dir, &workspace, &record)?;
        write_json(&path, &record)?;
    }
    if snapshot_used {
        let path = dir.join("snapshots").join(format!("{snapshot_id}.json"));
        if !path.is_file() {
            write_json(&path, &snapshot)?;
        }
    }
    jars.save()?;
    collect_garbage(&dir)?;
    let name = raw_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("raw")
        .replacen("raw", "summary", 1);
    let run = run_dir(&dir, &raw.session);
    fs::create_dir_all(&run)?;
    write_json(&run.join(format!("{name}.json")), &summary)?;
    Ok(0)
}

/// Removes snapshots that no record refers to any more.
fn collect_garbage(dir: &Path) -> Result<()> {
    let mut used = BTreeSet::new();
    for entry in fs::read_dir(dir.join("records"))? {
        if let Some(record) = read_json::<Record>(&entry?.path()) {
            used.insert(format!("{}.json", record.snapshot));
            used.insert(format!("{}.json", record.classpath));
            for hash in record.files.values() {
                used.insert(settings_name(hash));
            }
        }
    }
    for sub in ["snapshots", "settings", "classpaths"] {
        for entry in fs::read_dir(dir.join(sub))? {
            let entry = entry?;
            if !used.contains(entry.file_name().to_string_lossy().as_ref()) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    Ok(())
}

/// The file name under `.sieve/settings` for a recorded hash.
fn settings_name(hash: &str) -> String {
    format!("{}.json", hash.replace(':', "-"))
}

/// The names in a directory listing, as [`listing_hash`] counts them.
fn listing(path: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.ends_with(".class"))
        .collect();
    names.sort();
    names
}

/// Keeps the keys of each configuration file and the names of each migration directory a
/// record read, by hash, so that a later edit can be narrowed to what it changed.
fn save_settings(dir: &Path, workspace: &Path, record: &Record) -> Result<()> {
    for (file, hash) in &record.files {
        let path = dir.join("settings").join(settings_name(hash));
        if hash == "-" || path.is_file() || recorded_hash(workspace, file) != *hash {
            continue;
        }
        if let Some(listed) = file.strip_prefix(LISTED) {
            let names = listing(&workspace.join(listed));
            if settings::is_migrations(&names) {
                let names: BTreeMap<String, String> =
                    names.into_iter().map(|n| (n, String::new())).collect();
                write_json(&path, &names)?;
            }
            continue;
        }
        if !settings::is_config(file) {
            continue;
        }
        let Ok(text) = fs::read_to_string(workspace.join(file)) else {
            continue;
        };
        if let Some(keys) = settings::flatten(file, &text) {
            write_json(&path, &keys)?;
        }
    }
    Ok(())
}

/// The keys of a configuration file at a recorded hash: none for a file that did not exist.
fn settings_at(
    dir: &Path,
    workspace: &Path,
    file: &str,
    hash: &str,
) -> Option<BTreeMap<String, String>> {
    if hash == "-" {
        return Some(BTreeMap::new());
    }
    if recorded_hash(workspace, file) == hash {
        let text = fs::read_to_string(workspace.join(file)).ok()?;
        return settings::flatten(file, &text);
    }
    read_json(&dir.join("settings").join(settings_name(hash)))
}

/// Project supertypes and subtypes of `classes`, transitively, including themselves. Test
/// classes extending them are left out: only JUnit instantiates those.
fn hierarchy<'a>(classes: &BTreeSet<&'a str>, shapes: &'a Snapshot) -> BTreeSet<&'a str> {
    let mut subtypes: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (name, shape) in shapes {
        for sup in &shape.supers {
            subtypes.entry(sup).or_default().push(name);
        }
    }
    let mut out = BTreeSet::new();
    for &start in classes {
        let Some((key, _)) = shapes.get_key_value(start) else {
            continue;
        };
        for up in [true, false] {
            let mut seen = BTreeSet::new();
            let mut stack = vec![key.as_str()];
            while let Some(name) = stack.pop() {
                if !seen.insert(name) {
                    continue;
                }
                out.insert(name);
                if up {
                    let supers = shapes.get(name).into_iter().flat_map(|s| s.supers.iter());
                    stack.extend(
                        supers.filter_map(|s| {
                            shapes.get_key_value(s.as_str()).map(|(k, _)| k.as_str())
                        }),
                    );
                } else {
                    let subs = subtypes.get(name).into_iter().flatten().copied();
                    stack.extend(subs.filter(|s| !shapes.get(*s).is_some_and(|s| s.test)));
                }
            }
        }
    }
    out
}

/// Spring wiring that changed since a snapshot: which tests may load it differently.
struct Wiring {
    /// A removed component, or one static analysis cannot place: every context test runs.
    all: Option<String>,
    selected: BTreeMap<String, String>,
    added: BTreeSet<String>,
    /// Classes whose users run, with the reason: components whose changes stay inside them.
    classes: BTreeMap<String, String>,
    /// Changes that only a context start shows when they break it.
    startup: BTreeSet<String>,
}

/// What a change to a component's annotations can affect.
#[derive(PartialEq, PartialOrd)]
enum Effect {
    /// Calls to the component itself.
    Local,
    /// Requests that the changed handler's path patterns may now match.
    Route,
    /// Anything in the context: listeners, bean definitions, lifecycle callbacks, advice.
    Global,
}

/// Annotations whose effect is limited to calls of the annotated class or member.
const LOCAL_ANNOTATIONS: &[&str] = &[
    "org/springframework/stereotype/Service",
    "org/springframework/stereotype/Component",
    "org/springframework/stereotype/Repository",
    "org/springframework/transaction/annotation/",
    "jakarta/transaction/Transactional",
    "javax/transaction/Transactional",
    "org/springframework/security/access/prepost/",
    "org/springframework/security/access/annotation/Secured",
    "jakarta/annotation/security/",
    "javax/annotation/security/",
    "org/springframework/cache/annotation/Cacheable",
    "org/springframework/cache/annotation/CachePut",
    "org/springframework/cache/annotation/CacheEvict",
    "org/springframework/cache/annotation/Caching",
    "org/springframework/validation/annotation/Validated",
    "jakarta/validation/",
    "javax/validation/",
    "org/springframework/beans/factory/annotation/Value",
    "org/springframework/beans/factory/annotation/Qualifier",
    "org/springframework/beans/factory/annotation/Autowired",
    "org/springframework/web/bind/annotation/PathVariable",
    "org/springframework/web/bind/annotation/RequestParam",
    "org/springframework/web/bind/annotation/RequestBody",
    "org/springframework/web/bind/annotation/RequestHeader",
    "org/springframework/web/bind/annotation/RequestPart",
    "org/springframework/web/bind/annotation/CookieValue",
    "org/springframework/web/bind/annotation/ResponseStatus",
    "org/springframework/web/bind/annotation/ResponseBody",
    "org/springframework/format/annotation/",
    "org/springframework/scheduling/annotation/Async",
    "org/springframework/retry/annotation/",
    "org/springframework/lang/",
    "org/springframework/data/annotation/",
    "org/springframework/data/jpa/repository/Query",
    "org/springframework/data/jpa/repository/Modifying",
    "org/springframework/data/repository/query/Param",
    "jakarta/persistence/",
    "javax/persistence/",
    "org/hibernate/annotations/",
    "com/fasterxml/jackson/annotation/",
    "io/micrometer/",
    "org/jetbrains/annotations/",
    "jakarta/annotation/Nullable",
    "jakarta/annotation/Nonnull",
    "javax/annotation/Nullable",
    "javax/annotation/Nonnull",
    "kotlin/",
    "java/lang/Deprecated",
    "java/lang/SafeVarargs",
    "java/lang/FunctionalInterface",
];

/// Library supertypes that no injection point or framework callback looks for.
const INERT_SUPERTYPES: &[&str] = &[
    "java/lang/Object",
    "java/lang/Record",
    "java/lang/Enum",
    "java/io/Serializable",
    "java/lang/Comparable",
    "java/lang/Cloneable",
    "org/springframework/data/",
    "kotlin/jvm/internal/markers/",
];

fn listed(name: &str, list: &[&str]) -> bool {
    list.iter()
        .any(|p| name == *p || p.ends_with('/') && name.starts_with(p))
}

impl Current {
    fn class(&self, name: &str) -> Option<&classes::Class> {
        self.index.get(name).map(|&i| &self.classes[i])
    }

    /// What an annotation on a component can affect. Project annotations, such as one an
    /// aspect's pointcut names, are local when their own annotations are.
    fn effect(&self, annotation: &str) -> Effect {
        if bytecode::mapping(annotation) {
            return Effect::Route;
        }
        if listed(annotation, LOCAL_ANNOTATIONS) {
            return Effect::Local;
        }
        match self.class(annotation) {
            Some(class) if class.annotation => {
                let meta = class.annotations.iter().all(|a| {
                    a.name.starts_with("java/lang/annotation/")
                        || a.name.starts_with("kotlin/annotation/")
                        || listed(&a.name, LOCAL_ANNOTATIONS)
                });
                if meta {
                    Effect::Local
                } else {
                    Effect::Global
                }
            }
            _ => Effect::Global,
        }
    }

    /// Whether every constructor and the static initializer of `class` is plain.
    fn plain(&self, class: &str, now: &Snapshot) -> bool {
        let (Some(digest), Some(shape)) = (self.digests.get(class), now.get(class)) else {
            return false;
        };
        digest
            .methods
            .keys()
            .filter(|m| construction(m))
            .all(|m| shape.plain.contains(m))
    }
}

/// Whether a method key (`name(descriptor)`) constructs or initializes its class.
fn construction(method: &str) -> bool {
    method.starts_with("<init>(") || method.starts_with("<clinit>(")
}

/// Whether two request-mapping patterns can match the same path.
fn overlap(a: &str, b: &str) -> bool {
    let rest = |s: &str| s.contains("**") || s.starts_with("{*");
    let wild = |s: &str| s.contains(['{', '*', '?']);
    let (x, y): (Vec<&str>, Vec<&str>) = (
        a.split('/').filter(|s| !s.is_empty()).collect(),
        b.split('/').filter(|s| !s.is_empty()).collect(),
    );
    for i in 0..x.len().max(y.len()) {
        match (x.get(i), y.get(i)) {
            (Some(s), Some(t)) => {
                if rest(s) || rest(t) {
                    return true;
                }
                if s != t && !wild(s) && !wild(t) {
                    return false;
                }
            }
            (Some(s), None) | (None, Some(s)) => return rest(s),
            (None, None) => break,
        }
    }
    true
}

fn wiring(then: &Snapshot, now: &Snapshot, current: &Current, workspace: &Path) -> Wiring {
    let mut wiring = Wiring {
        all: None,
        selected: BTreeMap::new(),
        added: now
            .keys()
            .filter(|n| !then.contains_key(*n))
            .cloned()
            .collect(),
        classes: BTreeMap::new(),
        startup: BTreeSet::new(),
    };
    let names: BTreeSet<&String> = then.keys().chain(now.keys()).collect();
    let mut changed = BTreeSet::new();
    let mut patterns = BTreeSet::new();
    for name in names {
        let (a, b) = (then.get(name), now.get(name));
        let component = a.is_some_and(|s| s.component) || b.is_some_and(|s| s.component);
        let differs = a.map(|s| (&s.wiring, s.component)) != b.map(|s| (&s.wiring, s.component));
        // Older snapshots counted context tests as components.
        if !component || !differs || current.context_test(&name.replace('/', ".")) {
            continue;
        }
        let Some(b) = b else {
            wiring.all = Some(format!("Spring component removed: {name}"));
            continue;
        };
        let reason = format!("Spring wiring changed: {name}");
        let mut members = BTreeSet::new();
        let narrowed = match a.filter(|a| a.component) {
            // The same declaration: only constructors or annotated members changed.
            Some(a) if b.component && !a.declared.is_empty() && a.declared == b.declared => {
                let mut effect = Effect::Local;
                for key in a.wired.keys().chain(b.wired.keys()) {
                    let (x, y) = (a.wired.get(key), b.wired.get(key));
                    if x == y {
                        continue;
                    }
                    members.insert(key.clone());
                    for annotation in x.into_iter().chain(y).flat_map(|w| &w.annotations) {
                        let found = current.effect(annotation);
                        if found == Effect::Route {
                            patterns.extend(a.routes.get(key).into_iter().flatten().cloned());
                            patterns.extend(b.routes.get(key).into_iter().flatten().cloned());
                        }
                        if found > effect {
                            effect = found;
                        }
                    }
                }
                effect != Effect::Global
            }
            Some(_) => false,
            None => added(
                name,
                b,
                current,
                then,
                now,
                &reason,
                &mut wiring,
                &mut patterns,
            ),
        };
        if narrowed {
            // Interfaces and abstract classes run no code of their own for the members
            // Spring implements, such as repository queries: callers of those members do.
            if !current.class(name).is_some_and(|c| c.abstract_) {
                members.clear();
            }
            let mut owners = BTreeSet::from([name.as_str()]);
            while let Some(sub) = current.classes.iter().find(|c| {
                !members.is_empty()
                    && !owners.contains(c.name.as_str())
                    && c.supers.iter().any(|s| owners.contains(s.as_str()))
            }) {
                owners.insert(&sub.name);
            }
            for key in &members {
                let calls: Vec<String> = owners.iter().map(|o| format!("{o}#{key}")).collect();
                let calling = current
                    .classes
                    .iter()
                    .filter(|c| calls.iter().any(|call| c.calls.contains(call)));
                for class in calling {
                    wiring
                        .classes
                        .entry(class.name.clone())
                        .or_insert_with(|| format!("{reason}#{key}"));
                }
            }
            wiring.classes.insert(name.clone(), reason.clone());
            wiring.startup.insert(reason);
        } else {
            changed.insert(name.clone());
        }
    }
    // Handlers whose paths may now be matched differently.
    for snapshot in [then, now] {
        for (name, shape) in snapshot {
            let routes = shape.routes.values().flatten();
            if let Some(route) = routes
                .clone()
                .find(|r| patterns.iter().any(|p| overlap(p, r)))
            {
                wiring
                    .classes
                    .entry(name.clone())
                    .or_insert_with(|| format!("Request mapping changed near {route}"));
            }
        }
    }
    if changed.is_empty() || wiring.all.is_some() {
        return wiring;
    }
    let label: Vec<&str> = changed.iter().take(3).map(String::as_str).collect();
    let more = changed.len().saturating_sub(3);
    let more = if more > 0 {
        format!(" and {more} more")
    } else {
        String::new()
    };
    let label = format!("{}{more}", label.join(", "));
    match classes::affected_by(
        &current.classes,
        classes::Changes::Classes(&changed),
        workspace,
    ) {
        Ok(classes::Impact::Tests { selected, .. }) => {
            for test in selected {
                wiring
                    .selected
                    .insert(test, format!("Spring wiring changed: {label}"));
            }
        }
        Ok(classes::Impact::Fallback(reason)) => {
            wiring.all = Some(format!("Spring wiring changed: {reason}"))
        }
        Err(error) => wiring.all = Some(format!("Spring wiring changed: {error}")),
    }
    wiring
}

/// Whether a component added since the snapshot affects only what can reach it: plain
/// stereotypes and local annotations, no library supertype a framework looks for. Its
/// project supertypes' other implementations and their callers are affected, since
/// injection points of those types may now receive it.
#[allow(clippy::too_many_arguments)]
fn added(
    name: &str,
    shape: &Shape,
    current: &Current,
    then: &Snapshot,
    now: &Snapshot,
    reason: &str,
    wiring: &mut Wiring,
    patterns: &mut BTreeSet<String>,
) -> bool {
    let Some(class) = current.class(name) else {
        return false;
    };
    let mut affected = BTreeSet::new();
    let mut stack: Vec<&str> = class.supers.iter().map(String::as_str).collect();
    while let Some(sup) = stack.pop() {
        match current.class(sup) {
            Some(project) => {
                if affected.insert(sup.to_owned()) {
                    stack.extend(project.supers.iter().map(String::as_str));
                }
            }
            None if listed(sup, INERT_SUPERTYPES) => {}
            None => return false,
        }
    }
    let annotations = class.annotations.iter().map(|a| a.name.as_str()).chain(
        shape
            .wired
            .values()
            .flat_map(|w| w.annotations.iter().map(String::as_str)),
    );
    for annotation in annotations {
        let controller = matches!(
            annotation,
            "org/springframework/web/bind/annotation/RestController"
                | "org/springframework/stereotype/Controller"
        );
        match current.effect(annotation) {
            Effect::Global if !controller => return false,
            _ => {}
        }
    }
    patterns.extend(shape.routes.values().flatten().cloned());
    for sup in affected.clone() {
        for snapshot in [then, now] {
            for (other, s) in snapshot {
                if s.supers.contains(&sup) {
                    affected.insert(other.clone());
                }
            }
        }
        for caller in current.classes.iter().filter(|c| c.uses.contains(&sup)) {
            affected.insert(caller.name.clone());
        }
    }
    for other in affected {
        wiring
            .classes
            .entry(other)
            .or_insert_with(|| format!("{reason}, which shares its supertype"));
    }
    true
}

struct Decider<'a> {
    workspace: &'a Path,
    dir: PathBuf,
    current: Current,
    now: Snapshot,
    snapshots: BTreeMap<String, Option<(Snapshot, Wiring)>>,
    /// Changes skipped for some test that only a context start shows when they break it.
    startup: BTreeSet<String>,
    /// Whether the last checked test skipped such a change.
    pending: bool,
    /// Per configuration file and recorded hash: what [`settings_reach`] found.
    settings: BTreeMap<(String, String), std::result::Result<Reach, String>>,
    /// The test class path's jars as the agent sees them now.
    classpath: Option<Listing>,
    /// Listings by digest, as records refer to them.
    listings: BTreeMap<String, Option<Listing>>,
}

/// The project classes that name a changed key, with the key, and whether the change could
/// stop a context from starting.
struct Reach {
    consumers: BTreeMap<String, String>,
    startup: bool,
}

/// What a change to a configuration file or migration directory since `hash` reaches, or
/// why every test reading it runs: an unreadable file, a key no project class names, which
/// the framework or a library may read, or migrations that change existing tables.
fn settings_reach(
    dir: &Path,
    workspace: &Path,
    current: &Current,
    file: &str,
    hash: &str,
) -> std::result::Result<Reach, String> {
    let resource = format!("Changed resource: {file}");
    if let Some(listed) = file.strip_prefix(LISTED) {
        let then: BTreeMap<String, String> =
            read_json(&dir.join("settings").join(settings_name(hash))).ok_or(resource.clone())?;
        let then: Vec<String> = then.into_keys().collect();
        let path = workspace.join(listed);
        let now = listing(&path);
        let read = |name: &str| fs::read_to_string(path.join(name)).ok();
        if settings::is_migrations(&then) && settings::inert_migrations(&then, &now, read) {
            return Ok(Reach {
                consumers: BTreeMap::new(),
                startup: true,
            });
        }
        return Err(resource);
    }
    if !settings::is_config(file) {
        return Err(resource);
    }
    let then = settings_at(dir, workspace, file, hash).ok_or_else(|| resource.clone())?;
    let now = settings_at(dir, workspace, file, &recorded_hash(workspace, file)).ok_or(resource)?;
    let mut consumers = BTreeMap::new();
    let changed = settings::changed(&then, &now);
    for key in &changed {
        let found = settings::consumers(key, &current.classes);
        if found.is_empty() {
            return Err(format!(
                "Changed configuration {key} in {file}, read by the framework"
            ));
        }
        for i in found {
            consumers
                .entry(current.classes[i].name.clone())
                .or_insert_with(|| key.clone());
        }
    }
    Ok(Reach {
        consumers,
        startup: !changed.is_empty(),
    })
}

/// Names the inputs that differ between two test-JVM contexts, from the descriptions the
/// agent writes to `contexts/`: each input's name with a digest of its value.
fn context_change(dir: &Path, old: &str, new: &str) -> String {
    const CHANGED: &str = "Invocation, JVM properties or declared environment changed";
    let read = |digest: &str| -> Option<BTreeMap<String, String>> {
        if digest.is_empty() || !digest.bytes().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let text = fs::read_to_string(dir.join("contexts").join(format!("{digest}.txt"))).ok()?;
        Some(
            text.lines()
                .filter_map(|line| line.split_once(' '))
                .map(|(hash, name)| (name.to_owned(), hash.to_owned()))
                .collect(),
        )
    };
    let (Some(old), Some(new)) = (read(old), read(new)) else {
        return CHANGED.into();
    };
    let names: Vec<String> = old
        .keys()
        .chain(new.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|name| old.get(*name) != new.get(*name))
        .map(|name| match name.as_str() {
            "invocation" => "the sieve invocation (build arguments, speed-ups, Java/Maven \
                             environment)"
                .to_owned(),
            name => name
                .strip_prefix("property:")
                .or_else(|| name.strip_prefix("env:"))
                .unwrap_or(name)
                .to_owned(),
        })
        .collect();
    match names.len() {
        0 => CHANGED.into(),
        1..=5 => format!("{CHANGED}: {}", names.join(", ")),
        n => format!("{CHANGED}: {} and {} more", names[..5].join(", "), n - 5),
    }
}

impl Decider<'_> {
    /// Why the test must run, or `None` when its record shows that nothing it used changed.
    fn check(&mut self, test: &str, record: &Record, jdk: &str, context: &str) -> Option<String> {
        self.pending = false;
        let current = &self.current;
        if !record.passed {
            return Some("Failed last time".into());
        }
        if record.jdk != jdk {
            return Some(format!("JDK changed from {}", record.jdk));
        }
        if context.is_empty() || record.context != context {
            return Some(context_change(&self.dir, &record.context, context));
        }
        if record.test != current.test_hash(test) {
            return Some("Test class changed".into());
        }
        if record.build != current.build {
            // A build-input edit that only bumped versions reruns the users of the bumped jars.
            let then = self
                .listings
                .entry(record.classpath.clone())
                .or_insert_with(|| read_json(&listing_path(&self.dir, &record.classpath)))
                .as_ref();
            if let Some(reason) = dependency_change(then, self.classpath.as_ref(), record) {
                return Some(reason);
            }
        }
        // Classes whose code the test ran beyond constructing them. A context constructs
        // every component it loads; constructing one is not using it.
        let used: BTreeSet<&str> = record
            .methods
            .keys()
            .filter_map(|m| m.split_once('#'))
            .filter(|(_, method)| !construction(method))
            .map(|(owner, _)| owner)
            .collect();
        // With the classes used without running code of their own: owners of fields read
        // or written, whose static initializers ran elsewhere, and annotation types,
        // transitively.
        let mut shaped: BTreeSet<&str> = used.clone();
        let mut stack: Vec<&str> = used.iter().copied().collect();
        while let Some(name) = stack.pop() {
            let Some(class) = current.class(name) else {
                continue;
            };
            let annotations = class
                .refs
                .iter()
                .filter(|r| current.class(r).is_some_and(|c| c.annotation));
            for used in class.fields.iter().chain(annotations) {
                if let Some((key, _)) = current.index.get_key_value(used.as_str()) {
                    if shaped.insert(key.as_str())
                        && current.class(key).is_some_and(|c| c.annotation)
                    {
                        stack.push(key.as_str());
                    }
                }
            }
        }
        // A configuration file changes for the tests that use a class naming a changed key, or
        // that construct one through code other than plain field stores.
        let consumer = |class: &str| {
            let ran = record.methods.keys().filter_map(|m| m.split_once('#'));
            let constructed = ran
                .clone()
                .any(|(owner, m)| owner == class && construction(m));
            ran.clone().any(|(owner, m)| {
                owner == class
                    && !construction(m)
                    && !matches!(m, "hashCode()I" | "equals(Ljava/lang/Object;)Z")
            }) || shaped.contains(class) && !used.contains(class)
                || constructed && !current.plain(class, &self.now)
        };
        for (file, hash) in &record.files {
            if recorded_hash(self.workspace, file) == *hash {
                continue;
            }
            let reach = self
                .settings
                .entry((file.clone(), hash.clone()))
                .or_insert_with(|| settings_reach(&self.dir, self.workspace, current, file, hash));
            let reach = match reach {
                Err(reason) => return Some(reason.clone()),
                Ok(reach) => reach,
            };
            if let Some((class, key)) = reach.consumers.iter().find(|(c, _)| consumer(c)) {
                return Some(format!(
                    "Changed configuration {key} in {file}, read by {class}"
                ));
            }
            if reach.startup {
                self.startup.insert(format!("Changed: {file}"));
                self.pending = true;
            }
        }
        let snapshots = &mut self.snapshots;
        let entry = snapshots.entry(record.snapshot.clone()).or_insert_with(|| {
            let path = self
                .dir
                .join("snapshots")
                .join(format!("{}.json", record.snapshot));
            let then: Snapshot = read_json(&path)?;
            let wiring = wiring(&then, &self.now, current, self.workspace);
            Some((then, wiring))
        });
        let snapshot = entry.as_ref().map(|e| (&e.0, &e.1));
        for (method, hash) in &record.methods {
            if current.method(method) == Some(hash) {
                continue;
            }
            // A plain constructor or static initializer only sets the class's own fields,
            // which reach the test only through code of the class it never ran.
            if let (Some((owner, name)), Some((then, _))) = (method.split_once('#'), snapshot) {
                let confined = construction(name)
                    && !shaped.contains(owner)
                    && then.get(owner).is_some_and(|s| s.plain.contains(name))
                    && current.plain(owner, &self.now);
                if confined {
                    self.startup
                        .insert(format!("Construction changed: {}", owner.replace('/', ".")));
                    self.pending = true;
                    continue;
                }
            }
            return Some(format!("Changed method: {method}"));
        }
        if current.context_test(test) && !record.spring {
            return Some("Context test without context evidence".into());
        }
        let Some((then, wiring)) = snapshot else {
            return Some("Record snapshot missing".into());
        };
        for name in shaped.difference(&used) {
            let (a, b) = (then.get(*name), self.now.get(*name));
            if a.map(|s| &s.api) != b.map(|s| &s.api) {
                return Some(format!("Structural change: {name}"));
            }
        }
        let related: BTreeSet<&str> = hierarchy(&used, then)
            .into_iter()
            .chain(hierarchy(&used, &self.now))
            .collect();
        for name in related {
            let (a, b) = (then.get(name), self.now.get(name));
            if a.map(|s| (&s.api, &s.supers)) != b.map(|s| (&s.api, &s.supers)) {
                return Some(format!("Structural change: {name}"));
            }
        }
        if let Some(reason) = &wiring.all {
            if current.context_test(test) {
                return Some(reason.clone());
            }
        }
        if let Some(reason) = wiring.selected.get(test) {
            return Some(reason.clone());
        }
        if let Some(reason) = shaped.iter().find_map(|c| wiring.classes.get(*c)) {
            return Some(reason.clone());
        }
        if current.context_test(test) && !wiring.startup.is_empty() {
            self.startup.extend(wiring.startup.iter().cloned());
            self.pending = true;
        }
        let touched = record
            .methods
            .keys()
            .filter_map(|m| m.split_once('#').map(|(owner, _)| owner));
        for name in touched {
            // A class only constructed through plain code ran none of the code naming it.
            if !shaped.contains(name) && current.plain(name, &self.now) {
                continue;
            }
            let refs = current.class(name).map(|c| &c.refs);
            if let Some(added) = wiring
                .added
                .iter()
                .find(|a| refs.is_some_and(|r| r.contains(*a)))
            {
                return Some(format!("Added class named in {name}: {added}"));
            }
        }
        None
    }
}

/// Test classes that reach no change since `base` by static analysis, or `None` when that
/// cannot be established.
fn statically_unreached(
    workspace: &Path,
    current: &Current,
    base: &str,
) -> Result<Option<BTreeSet<String>>> {
    // A single module needs no graph fingerprint: build inputs changed since the base select
    // everything anyway.
    let config = Config::read(workspace)?;
    let (changed, prefix, _) = crate::changed_paths(workspace, base)?;
    let selection = config.select(changed, &prefix);
    Ok(match selection.mode {
        "NONE" => Some(current.tests()),
        "MODULES" if !selection.sources.iter().any(|p| config.generated_input(p)) => {
            match classes::affected(&current.classes, &selection.sources, workspace)? {
                classes::Impact::Tests { unselected, .. } => Some(unselected),
                classes::Impact::Fallback(_) => None,
            }
        }
        _ => None,
    })
}

/// `sieve decide --workspace PATH --jdk JDK --out FILE [--base REV]`: writes the top-level
/// test classes that may be dropped, one per line.
pub fn decide(args: Vec<String>) -> Result<u8> {
    let options = options(
        args,
        &[
            "--workspace",
            "--jdk",
            "--out",
            "--base",
            "--context",
            "--session",
            "--classpath",
        ],
    )?;
    let workspace = workspace_option(&options)?;
    let jdk = options.get("--jdk").ok_or("--jdk is required")?;
    let out = PathBuf::from(options.get("--out").ok_or("--out is required")?);
    let dir = prepare(&workspace)?;
    let session = options.get("--session").map_or("", String::as_str);
    let _execution = execution_guard(&dir, session)?;
    let context = options.get("--context").map_or("", String::as_str);
    let classpath = match options.get("--classpath") {
        Some(file) => {
            let jars: Vec<String> = fs::read_to_string(file)?
                .lines()
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect();
            let mut cache = Jars::open(&dir);
            let listing = cache.listing(&jars);
            cache.save()?;
            (!listing.is_empty()).then_some(listing)
        }
        None => None,
    };
    let current = Current::load(&workspace)?;
    let now = current.snapshot();
    let tests = current.tests();
    let mut decider = Decider {
        workspace: &workspace,
        dir: dir.clone(),
        current,
        now,
        snapshots: BTreeMap::new(),
        startup: BTreeSet::new(),
        pending: false,
        settings: BTreeMap::new(),
        classpath,
        listings: BTreeMap::new(),
    };
    let mut unreached = None;
    // Per context test with a record: components it constructed, and methods it ran.
    let mut contexts = BTreeMap::new();
    let mut skip = BTreeSet::new();
    let mut reasons = BTreeMap::new();
    let broken = env::var_os("SIEVE_BROKEN_SELECTOR").is_some();
    for test in &tests {
        if broken {
            // Test hook: a selector that drops everything, which planted bugs must catch.
            skip.insert(test.clone());
            reasons.insert(test.clone(), "SIEVE_BROKEN_SELECTOR".into());
            continue;
        }
        let reason = match read_json::<Record>(&record_path(&dir, test)) {
            Some(record) if decider.current.context_test(test) && record.spring => {
                let components = record
                    .methods
                    .keys()
                    .filter_map(|m| m.split_once('#'))
                    .filter(|(owner, method)| {
                        method.starts_with("<init>(")
                            && decider.now.get(*owner).is_some_and(|s| s.component)
                    })
                    .map(|(owner, _)| owner)
                    .collect::<BTreeSet<_>>()
                    .len();
                let reason = decider.check(test, &record, jdk, context);
                // Its last passing run started the context as it is now.
                let verified = reason.is_none() && !decider.pending;
                contexts.insert(test.clone(), (components, record.methods.len(), verified));
                match reason {
                    Some(reason) => reason,
                    None => {
                        skip.insert(test.clone());
                        "Unchanged test record".into()
                    }
                }
            }
            Some(record) => match decider.check(test, &record, jdk, context) {
                Some(reason) => reason,
                None => {
                    skip.insert(test.clone());
                    "Unchanged test record".into()
                }
            },
            None => {
                let unreached = unreached.get_or_insert_with(|| {
                    let base = options.get("--base")?;
                    statically_unreached(&workspace, &decider.current, base).unwrap_or_else(
                        |error| {
                            eprintln!("Static fallback unavailable: {error}");
                            None
                        },
                    )
                });
                if unreached.as_ref().is_some_and(|u| u.contains(test)) {
                    skip.insert(test.clone());
                    "No record; reaches no change since the base".into()
                } else {
                    "No record".into()
                }
            }
        };
        reasons.insert(test.clone(), reason);
    }
    // Changes narrowed to the tests using them can still stop every context from starting:
    // one test that starts a full context runs.
    if let Some(change) = decider.startup.iter().next() {
        let most = contexts.values().map(|(c, _, _)| *c).max().unwrap_or(0);
        let full: Vec<(&String, usize, bool)> = contexts
            .iter()
            .filter(|(_, (c, _, _))| most > 0 && c * 10 >= most * 9)
            .map(|(t, (_, methods, verified))| (t, *methods, *verified))
            .collect();
        if !full
            .iter()
            .any(|(t, _, verified)| *verified || !skip.contains(*t))
        {
            if let Some((test, _, _)) = full.iter().min_by_key(|(t, methods, _)| (*methods, *t)) {
                skip.remove(*test);
                let more = decider.startup.len() - 1;
                let others = if more > 0 {
                    format!(" and {more} more")
                } else {
                    String::new()
                };
                reasons.insert(
                    (*test).clone(),
                    format!("Startup check for changes it does not use: {change}{others}"),
                );
            }
        }
    }
    fs::write(
        &out,
        skip.iter().map(|t| format!("{t}\n")).collect::<String>(),
    )?;
    // Named after the test JVM (its scratch files are `decide-<pid>-…`), so that `run` can take
    // each test's reason from the JVM that ran or dropped it.
    let name = format!("decisions-{}.json", jvm_pid(&out, "decide"));
    let run = run_dir(&dir, session);
    fs::create_dir_all(&run)?;
    write_json(&run.join(name), &reasons)?;
    Ok(0)
}

fn cache_dir() -> Result<PathBuf> {
    if let Some(dir) = env::var_os("SIEVE_CACHE_DIR") {
        return Ok(PathBuf::from(dir));
    }
    if let Some(dir) = env::var_os("XDG_CACHE_HOME") {
        return Ok(PathBuf::from(dir).join("sieve"));
    }
    if cfg!(windows) {
        if let Some(dir) = env::var_os("LOCALAPPDATA") {
            return Ok(PathBuf::from(dir).join("sieve"));
        }
    }
    let home = PathBuf::from(env::var_os("HOME").ok_or("Set SIEVE_CACHE_DIR or HOME")?);
    Ok(if cfg!(target_os = "macos") {
        home.join("Library/Caches/sieve")
    } else {
        home.join(".cache/sieve")
    })
}

#[cfg(feature = "agent")]
const JARS: &[(&str, &[u8])] = &[
    (
        "sieve-agent.jar",
        include_bytes!(concat!(env!("OUT_DIR"), "/sieve-agent.jar")),
    ),
    (
        "sieve-probe.jar",
        include_bytes!(concat!(env!("OUT_DIR"), "/sieve-probe.jar")),
    ),
];

#[cfg(not(feature = "agent"))]
const JARS: &[(&str, &[u8])] = &[];

/// Extracts the embedded agent to the user cache and returns its path. Both jars are compared
/// with the embedded bytes on every use, so a tampered or truncated copy is replaced.
pub fn agent_jar() -> Result<PathBuf> {
    if JARS.is_empty() {
        return Err("This sieve was built without the agent (--no-default-features)".into());
    }
    let mut h = bytecode::Hasher::default();
    for (_, bytes) in JARS {
        h.field(bytes);
    }
    let dir = cache_dir()?.join(format!("agent-{}", h.finish()));
    fs::create_dir_all(&dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    for (name, bytes) in JARS {
        let path = dir.join(name);
        if fs::read(&path).ok().as_deref() != Some(*bytes) {
            let mut temp = tempfile::NamedTempFile::new_in(&dir)?;
            temp.write_all(bytes)?;
            temp.persist(&path)?;
        }
    }
    Ok(dir.join(JARS[0].0))
}

/// Writes the agent options and returns the `-javaagent` option that loads the agent. It
/// reaches test JVMs through `JDK_JAVA_OPTIONS`, which every `java` launch reads whatever the
/// POM's `argLine` says; the agent ignores the build tool's own JVM.
#[allow(clippy::too_many_arguments)]
fn agent_option(
    workspace: &Path,
    dir: &Path,
    mode: &str,
    base: Option<&str>,
    context: &str,
    session: &str,
    config: &Config,
    portable: bool,
) -> Result<String> {
    let jar = agent_jar()?;
    let exe = env::current_exe()?.canonicalize()?;
    let mut props = String::new();
    let outputs: Vec<String> = outputs(workspace).iter().map(|(d, _)| plain(d)).collect();
    let separator = if cfg!(windows) { ";" } else { ":" };
    for (key, value) in [
        ("workspace", plain(workspace)),
        ("sieve", plain(&exe)),
        ("outputs", outputs.join(separator)),
        ("mode", mode.into()),
        ("base", base.unwrap_or_default().into()),
        ("context", context.into()),
        ("session", session.into()),
        ("record_env", config.record_env.join(",")),
        (
            "ignore_properties",
            config.record_ignore_properties.join(","),
        ),
        ("portable", portable.to_string()),
    ] {
        // Properties files treat backslashes as escapes.
        props += &format!("{key}={}\n", value.replace('\\', "\\\\"));
    }
    // Printed env options remain valid after a wrapper run writes its own options.
    let file = if session.is_empty() {
        dir.join(format!(
            "env-agent-{}.properties",
            bytecode::hash(props.as_bytes())
        ))
    } else {
        dir.join("agent.properties")
    };
    fs::write(&file, props)?;
    let option = format!("-javaagent:{}={}", jar.display(), file.display());
    if option.chars().any(char::is_whitespace) {
        return Err(format!(
            "The agent path contains whitespace, which JDK_JAVA_OPTIONS splits: {option}. \
             Set SIEVE_CACHE_DIR, or move the workspace."
        )
        .into());
    }
    Ok(option)
}

/// `JDK_JAVA_OPTIONS` with the agent in front of what the environment already sets.
fn java_options(option: &str) -> String {
    match env::var("JDK_JAVA_OPTIONS") {
        Ok(existing) if !existing.trim().is_empty() => format!("{option} {existing}"),
        _ => option.to_owned(),
    }
}

fn single_module(config: &Config) -> Result<()> {
    if !matches!(config.tool.as_str(), "maven" | "gradle") || config.modules.keys().ne(["."]) {
        return Err(
            "\"records\" needs a single-module Maven or Gradle project (modules {\".\": []})"
                .into(),
        );
    }
    Ok(())
}

/// `sieve env --workspace PATH [--base REV]`: the `JDK_JAVA_OPTIONS` value that gives plain
/// `mvn test` or `mvn verify` the same selection as `sieve run`.
pub fn env_command(args: Vec<String>) -> Result<u8> {
    let options = options(args, &["--workspace", "--base"])?;
    let workspace =
        PathBuf::from(options.get("--workspace").map_or(".", String::as_str)).canonicalize()?;
    let config = match workspace.join("impact.json").is_file() {
        true => Config::read(&workspace)?,
        false => Config::local_default(&workspace)
            .ok_or("No impact.json, and not a single-module Maven project")?,
    };
    if config.tool != "maven" {
        return Err("sieve env is for plain Maven; on Gradle, use sieve run".into());
    }
    single_module(&config)?;
    let dir = prepare(&workspace)?;
    let _execution = execution_guard(&dir, "")?;
    println!(
        "{}",
        // A plain Maven launch has no previously validated wrapper invocation. Its absent
        // records must run even when the caller supplies a green Git base.
        java_options(&agent_option(
            &workspace, &dir, "select", None, "", "", &config, false
        )?)
    );
    Ok(0)
}

/// Files that can affect a build, with metadata and content, relative to the workspace.
fn tree(config: &Config, workspace: &Path) -> Result<BTreeMap<String, String>> {
    let listed = Command::new("git")
        .current_dir(workspace)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .output();
    let mut paths = Vec::new();
    match listed {
        Ok(output) if output.status.success() => {
            for path in output.stdout.split(|b| *b == 0).filter(|p| !p.is_empty()) {
                paths.push(String::from_utf8(path.to_vec())?);
            }
        }
        _ => {
            fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
                for entry in fs::read_dir(dir)? {
                    let entry = entry?;
                    let name = entry.file_name();
                    if entry.file_type()?.is_dir() {
                        if !matches!(name.to_str(), Some("target" | ".git" | DIR | ".idea")) {
                            walk(root, &entry.path(), out)?;
                        }
                    } else {
                        let path = entry.path();
                        out.push(
                            path.strip_prefix(root)?
                                .to_string_lossy()
                                .replace('\\', "/"),
                        );
                    }
                }
                Ok(())
            }
            walk(workspace, workspace, &mut paths)?;
        }
    }
    let mut tree = BTreeMap::new();
    for path in paths {
        if path.starts_with(".sieve/") || config.ignored(&path, "") {
            continue;
        }
        let stamp = match fs::metadata(workspace.join(&path)) {
            Ok(meta) => {
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos());
                format!(
                    "{}:{modified}:{}",
                    meta.len(),
                    file_hash(&workspace.join(&path))
                )
            }
            Err(_) => "-".into(),
        };
        tree.insert(path, stamp);
    }
    Ok(tree)
}

/// The last `sieve run` in local mode.
#[derive(Serialize, Deserialize)]
struct LastRun {
    key: String,
    passed: bool,
    #[serde(default)]
    context: String,
    build: String,
    tree: BTreeMap<String, String>,
}

/// Why the build must start with `clean`: output from deleted or renamed files, or from
/// build and generated inputs, may otherwise survive.
fn clean_reason(
    config: &Config,
    last: Option<&LastRun>,
    tree: &BTreeMap<String, String>,
    build: &str,
) -> Option<String> {
    let Some(last) = last else {
        return Some("No earlier local run".into());
    };
    if !last.passed {
        // Pending state is written before Maven starts. An interrupted clean must never
        // make preserved-metadata source edits look compiled to the next run.
        return Some("Earlier local run did not pass or finish".into());
    }
    if last.build != build {
        return Some("Build input changed".into());
    }
    for (path, stamp) in &last.tree {
        if stamp != "-" && tree.get(path).is_none_or(|s| s == "-") {
            return Some(format!("Deleted or renamed: {path}"));
        }
        // Maven's incremental compiler can miss a body edit whose size and timestamp were
        // preserved. Such an edit must compile before bytecode records may decide tests.
        if let Some(now) = tree.get(path) {
            if stamp != now
                && stamp.rsplit_once(':').map(|(meta, _)| meta)
                    == now.rsplit_once(':').map(|(meta, _)| meta)
            {
                return Some(format!("Content changed with unchanged metadata: {path}"));
            }
        }
    }
    tree.iter()
        .find(|(path, stamp)| config.generated_input(path) && last.tree.get(*path) != Some(stamp))
        .map(|(path, _)| format!("Generated input changed: {path}"))
}

/// Content hashes of every file that any record lists, so that an edit to one of them ends the
/// fast path even where the tree leaves it out (ignored and Git-ignored paths).
fn recorded_files(dir: &Path, workspace: &Path) -> String {
    let mut files = BTreeSet::new();
    for entry in fs::read_dir(dir.join("records"))
        .into_iter()
        .flatten()
        .flatten()
    {
        if let Some(record) = read_json::<Record>(&entry.path()) {
            files.extend(record.files.into_keys());
        }
    }
    let mut h = bytecode::Hasher::default();
    for file in files {
        h.field(file.as_bytes())
            .field(recorded_hash(workspace, &file).as_bytes());
    }
    h.finish()
}

/// Marks the records of test classes that failed in the build reports, and after a failed
/// build, of those that ran without the agent reporting them, as failed. Returns them.
fn mark_failures(
    dir: &Path,
    workspace: &Path,
    reported: &BTreeSet<String>,
    success: bool,
) -> Result<BTreeSet<String>> {
    let reports = match crate::reports::read_reports(workspace, &tool(workspace)) {
        Ok(reports) => reports,
        Err(error) if success => return Err(error),
        Err(_) => Default::default(),
    };
    let class = |id: &String| {
        let name = id.rsplit(':').next().unwrap_or(id);
        name.split('$').next().unwrap_or(name).to_owned()
    };
    let mut failed: BTreeSet<String> = reports.failed.iter().map(class).collect();
    if !success {
        failed.extend(
            reports
                .executed
                .iter()
                .map(class)
                .filter(|c| !reported.contains(c)),
        );
    }
    if failed.is_empty() {
        return Ok(failed);
    }
    let _lock = Lock::acquire(dir, "lock")?;
    for test in &failed {
        let path = record_path(dir, test);
        if let Some(mut record) = read_json::<Record>(&path) {
            record.passed = false;
            write_json(&path, &record)?;
        }
    }
    Ok(failed)
}

/// A path as the JVM spells it: without Windows' verbatim `\\?\` prefix.
fn plain(path: &Path) -> String {
    let text = path.display().to_string();
    text.strip_prefix(r"\\?\")
        .map(str::to_owned)
        .unwrap_or(text)
}

/// Maven options that take the next argument as their value.
const VALUED: &[&str] = &[
    "-P", "-pl", "-f", "-s", "-gs", "-t", "-T", "-rf", "-b", "-l", "-D",
];

/// Gradle options that take the next argument as their value.
const GRADLE_VALUED: &[&str] = &[
    "--tests",
    "-x",
    "--exclude-task",
    "-p",
    "--project-dir",
    "-I",
    "--init-script",
    "-c",
    "--settings-file",
    "-g",
    "--gradle-user-home",
    "--console",
    "--max-workers",
    "--warning-mode",
    "-D",
    "-P",
];

/// Whether the build arguments name goals, phases, or tasks, which then replace the default.
fn has_goals(tool: &str, extra: &[String]) -> bool {
    let valued = if tool == "gradle" {
        GRADLE_VALUED
    } else {
        VALUED
    };
    let mut args = extra.iter();
    while let Some(arg) = args.next() {
        if !arg.starts_with('-') {
            return true;
        }
        if valued.contains(&arg.as_str()) {
            args.next();
        }
    }
    false
}

/// Tests requested by name always run: Surefire's `-Dtest`, Failsafe's `-Dit.test`, Gradle's
/// `--tests`.
fn explicit_tests(extra: &[String]) -> bool {
    extra.iter().any(|a| {
        a.starts_with("-Dtest=")
            || a.starts_with("-Dit.test=")
            || a == "--tests"
            || a.starts_with("--tests=")
    })
}

/// The init script that attaches the agent to Gradle's `Test` tasks, extracted next to the agent
/// so that its path, and with it Gradle's caches, stay stable between runs.
pub(crate) fn gradle_init_script(jar: &Path) -> Result<PathBuf> {
    let bytes = crate::setup::GRADLE_SCRIPT;
    let path = jar.parent().ok_or("No agent directory")?.join(format!(
        "sieve-{}.init.gradle",
        &bytecode::hash(bytes)[..16]
    ));
    if fs::read(&path).ok().as_deref() != Some(bytes) {
        let mut temp = tempfile::NamedTempFile::new_in(path.parent().ok_or("No directory")?)?;
        temp.write_all(bytes)?;
        temp.persist(&path)?;
    }
    Ok(path)
}

/// The test JVM's process id in a scratch file name such as `decide-<pid>-<random>.tmp`.
fn jvm_pid(path: &Path, prefix: &str) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix(prefix)?.strip_prefix('-')?.split('-').next())
        .filter(|pid| !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()))
        .map_or_else(|| std::process::id().to_string(), str::to_owned)
}

fn summarize(dir: &Path, selection: &mut Selection) -> Result<()> {
    let mut ran = BTreeSet::new();
    let mut dropped = BTreeSet::new();
    let mut decided = false;
    let mut notes = Vec::new();
    // Each test JVM decides about every test class, but only the classes it discovered count:
    // Surefire and Failsafe, or two Gradle `Test` tasks, see the others under another context.
    let mut decisions: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut handled: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for entry in fs::read_dir(dir.join("run"))? {
        let path = entry?.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if let Some(pid) = name
            .strip_prefix("decisions-")
            .and_then(|n| n.strip_suffix(".json"))
        {
            if let Some(reasons) = read_json::<BTreeMap<String, String>>(&path) {
                decided = true;
                decisions.insert(pid.to_owned(), reasons);
            }
        } else if name.starts_with("summary-") {
            if let Some(summary) = read_json::<Summary>(&path) {
                let jvm = handled.entry(jvm_pid(&path, "summary")).or_default();
                jvm.extend(summary.ran.iter().chain(&summary.dropped).cloned());
                ran.extend(summary.ran);
                dropped.extend(summary.dropped);
                notes.extend(summary.notes);
            }
        }
    }
    for (pid, reasons) in &decisions {
        let own = handled.get(pid);
        for (test, reason) in reasons {
            if own.is_some_and(|own| own.contains(test)) {
                selection.reasons.insert(test.clone(), reason.clone());
            }
        }
    }
    // Classes no JVM discovered, such as abstract fixtures, keep any JVM's reason.
    for reasons in decisions.values() {
        for (test, reason) in reasons {
            selection
                .reasons
                .entry(test.clone())
                .or_insert_with(|| reason.clone());
        }
    }
    dropped.retain(|t| !ran.contains(t));
    for test in &dropped {
        selection
            .reasons
            .entry(test.clone())
            .or_insert_with(|| "Unchanged test record".into());
    }
    selection.mode = match (ran.is_empty(), dropped.is_empty()) {
        // Nothing ran and nothing was dropped: the build stopped before the tests, or has none.
        (true, true) if !decided => "NONE",
        (_, true) => "ALL",
        (true, false) => "NONE",
        (false, false) => "SUBSET",
    };
    if !decided && !ran.is_empty() {
        notes.push("No selection was made: tests were requested explicitly, ran in parallel, or the test JVM lacks Java 24+".into());
    }
    if !notes.is_empty() {
        selection.reason = format!("{}; {}", selection.reason, notes.join("; "));
    }
    selection.tests = ran;
    selection.skipped = dropped;
    Ok(())
}

/// Local speed-ups, each switched with `--with`/`--without NAME`.
#[derive(Debug, Default)]
pub struct Levers {
    pub on: BTreeSet<String>,
    pub off: BTreeSet<String>,
}

/// Every speed-up changes native build behavior and must be requested explicitly.
pub const LEVERS: &[&str] = &["reuse", "jgitver", "mvnd", "repackage"];

impl Levers {
    pub fn validate(&self) -> Result<()> {
        match self
            .on
            .iter()
            .chain(&self.off)
            .find(|l| !LEVERS.contains(&l.as_str()))
        {
            Some(lever) => {
                Err(format!("Unknown speed-up {lever:?}; known: {}", LEVERS.join(", ")).into())
            }
            None => Ok(()),
        }
    }

    fn enabled(&self, name: &str) -> bool {
        !self.off.contains(name) && self.on.contains(name)
    }

    /// Switches the levers on for `command`, returning each lever's state for `--output`.
    fn apply(
        &self,
        workspace: &Path,
        executable: &mut String,
        explicit: bool,
        command: &mut Command,
        args: &mut Vec<String>,
    ) -> BTreeMap<String, String> {
        let mut states = BTreeMap::new();
        let reuse = if !self.enabled("reuse") {
            "off".to_owned()
        } else if env::var_os("TESTCONTAINERS_REUSE_ENABLE").is_some() {
            "as set in the environment".to_owned()
        } else {
            // Effective for containers that opt in with `withReuse(true)`.
            command.env("TESTCONTAINERS_REUSE_ENABLE", "true");
            "on".to_owned()
        };
        states.insert("reuse".into(), reuse);
        let extensions =
            fs::read_to_string(workspace.join(".mvn/extensions.xml")).unwrap_or_default();
        let jgitver = if !extensions.contains("jgitver") {
            "not used".to_owned()
        } else if self.enabled("jgitver") {
            // Filtered resources that embed the version change and rerun their readers.
            args.push("-Djgitver.skip=true".into());
            "on".to_owned()
        } else {
            "off".to_owned()
        };
        states.insert("jgitver".into(), jgitver);
        let repackage = if self.enabled("repackage") {
            args.push("-Dspring-boot.repackage.skip=true".into());
            "on"
        } else {
            "off"
        };
        states.insert("repackage".into(), repackage.into());
        let mvnd = if !self.enabled("mvnd") {
            "off".to_owned()
        } else if explicit {
            "off: --executable given".to_owned()
        } else if Command::new("mvnd")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
        {
            *executable = "mvnd".into();
            "on".to_owned()
        } else {
            "not installed".to_owned()
        };
        states.insert("mvnd".into(), mvnd);
        states
    }
}

/// Options of `sieve run` in local mode.
pub struct Run {
    pub base: Option<String>,
    pub full: bool,
    pub output: Option<PathBuf>,
    pub executable: Option<String>,
    pub levers: Levers,
    pub extra: Vec<String>,
    /// Records that hold on other machines and checkouts: no absolute paths, tool locations,
    /// or host identity in the invocation and test-JVM context.
    pub ci: bool,
}

/// Environment variables whose values are tool locations, which `--ci` leaves out.
const LOCATIONS: &[&str] = &["JAVA_HOME", "PATH"];

fn invocation(
    config: &Config,
    executable: &str,
    extra: &[String],
    levers: &Levers,
    portable: bool,
) -> Result<String> {
    let mut h = bytecode::Hasher::default();
    let tool = match portable {
        // The same wrapper or build tool, wherever it is installed.
        true => Path::new(executable)
            .file_name()
            .map_or_else(|| executable.into(), |n| n.to_string_lossy()),
        false => executable.into(),
    };
    h.field(portable.to_string().as_bytes())
        .field(tool.as_bytes())
        .field(&serde_json::to_vec(extra)?)
        .field(format!("{levers:?}").as_bytes());
    let mut names: BTreeSet<&str> = [
        "JAVA_HOME",
        "JDK_JAVA_OPTIONS",
        "JAVA_TOOL_OPTIONS",
        "MAVEN_ARGS",
        "MAVEN_OPTS",
        "GRADLE_OPTS",
        "TESTCONTAINERS_REUSE_ENABLE",
        "PATH",
    ]
    .into_iter()
    .collect();
    names.extend(config.record_env.iter().map(String::as_str));
    if portable {
        names.retain(|name| !LOCATIONS.contains(name));
    }
    for name in names {
        h.field(name.as_bytes())
            .field(&serde_json::to_vec(&env::var(name).ok())?);
    }
    let java_home = env::var_os("JAVA_HOME").map(PathBuf::from).or_else(|| {
        env::var_os("PATH").and_then(|path| {
            env::split_paths(&path).find_map(|dir| {
                let java = dir
                    .join(if cfg!(windows) { "java.exe" } else { "java" })
                    .canonicalize()
                    .ok()?;
                Some(java.parent()?.parent()?.to_owned())
            })
        })
    });
    if let Some(home) = java_home {
        h.field(file_hash(&home.join("release")).as_bytes());
    }
    Ok(h.finish())
}

fn output_state(workspace: &Path) -> Result<String> {
    fn collect(dir: &Path, files: &mut BTreeMap<PathBuf, String>) -> Result<()> {
        if !dir.is_dir() {
            return Ok(());
        }
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                collect(&entry.path(), files)?;
            } else {
                files.insert(entry.path(), file_hash(&entry.path()));
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    for (dir, _) in outputs(workspace) {
        collect(&dir, &mut files)?;
    }
    Ok(bytecode::hash(&serde_json::to_vec(&files)?))
}

/// `sieve run` in local mode: one incremental Maven build with the agent.
pub fn run(config: &Config, workspace: &Path, run: Run) -> Result<u8> {
    let Run {
        base,
        full,
        output,
        executable,
        levers,
        extra,
        ci,
    } = run;
    let started = std::time::Instant::now();
    let explicit_executable = executable.is_some();
    let mut executable =
        executable.unwrap_or_else(|| crate::setup::default_executable(workspace, &config.tool));
    single_module(config)?;
    let gradle = config.tool == "gradle";
    if gradle {
        if let Some(lever) = levers.on.iter().find(|l| *l != "reuse") {
            return Err(format!("The {lever} speed-up is for Maven only").into());
        }
    }
    let dir = prepare(workspace)?;
    let _execution = Lock::acquire(&dir, "execution.lock")?;
    let session = format!(
        "{}:{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_nanos()
    );
    // Some native file locks also prohibit non-owner reads. Keep the token outside the lock.
    fs::write(dir.join("session"), &session)?;
    let write = |selection: &Selection| -> Result<()> {
        let json = serde_json::to_string_pretty(selection)? + "\n";
        if let Some(path) = &output {
            fs::write(path, &json)?;
        }
        eprintln!("{json}");
        Ok(())
    };
    let explicit = explicit_tests(&extra);
    let goals = has_goals(&config.tool, &extra);
    let tree = tree(config, workspace)?;
    let build = build_inputs(workspace)?;
    let context = invocation(config, &executable, &extra, &levers, ci)?;
    let mut key = bytecode::Hasher::default();
    key.field(&serde_json::to_vec(&tree)?)
        .field(build.as_bytes())
        .field(executable.as_bytes())
        .field(&serde_json::to_vec(&extra)?)
        .field(base.as_deref().unwrap_or_default().as_bytes())
        .field(context.as_bytes())
        .field(env!("CARGO_PKG_VERSION").as_bytes());
    for (_, bytes) in JARS {
        key.field(bytes);
    }
    // The files that records list, as they are now; stored with what this run recorded.
    let keyed = |key: &bytecode::Hasher| -> Result<String> {
        let mut key = key.clone();
        key.field(recorded_files(&dir, workspace).as_bytes());
        // IDE and plain Maven builds can change bytecode without changing workspace sources.
        key.field(output_state(workspace)?.as_bytes());
        Ok(key.finish())
    };
    let base_key = key;
    let key = keyed(&base_key)?;
    let state = dir.join("last-run.json");
    let last: Option<LastRun> = read_json(&state);
    let mut selection = config.all("Test records decide inside the test JVM");
    selection.mode = "RECORDS";
    if !full && !explicit && !goals && last.as_ref().is_some_and(|l| l.passed && l.key == key) {
        selection.mode = "NONE";
        selection.reason = "No changes since the last passing run; no build started".into();
        summarize_run(
            &dir,
            workspace,
            config,
            false,
            &BTreeSet::new(),
            None,
            started,
            &mut selection,
        );
        write(&selection)?;
        print_summary(&selection);
        return Ok(0);
    }
    // Gradle's incremental compilation removes the output of deleted sources itself.
    let clean = match gradle {
        true => None,
        false => clean_reason(config, last.as_ref(), &tree, &build),
    };
    // Paths added, edited, or removed since the last run, for `--output`.
    if let Some(last) = &last {
        selection.changed = tree
            .iter()
            .filter(|(path, stamp)| last.tree.get(*path) != Some(stamp))
            .map(|(path, _)| path.clone())
            .chain(last.tree.keys().filter(|p| !tree.contains_key(*p)).cloned())
            .collect();
    }
    // Until this run finishes, the next one cleans after anything either tree saw deleted.
    let mut seen = last.as_ref().map(|l| l.tree.clone()).unwrap_or_default();
    seen.extend(tree.clone());
    write_json(
        &state,
        &LastRun {
            key: String::new(),
            passed: false,
            context: context.clone(),
            build: last.as_ref().map_or_else(String::new, |l| l.build.clone()),
            tree: seen,
        },
    )?;
    // A Gradle test JVM cannot see `--tests`, so named tests switch selection off here.
    let mode = if full || (gradle && explicit) {
        "record"
    } else {
        "select"
    };
    // The base can substitute for missing records only under an invocation already shown
    // green. Git alone says nothing about changed -D properties or environment inputs.
    let fallback_base = base.as_deref().filter(|_| {
        last.as_ref()
            .is_some_and(|l| l.passed && l.context == context)
    });
    let option = agent_option(
        workspace,
        &dir,
        mode,
        fallback_base,
        &context,
        &session,
        config,
        ci,
    )?;
    let run_dir = dir.join("run");
    fs::remove_dir_all(&run_dir)?;
    fs::create_dir_all(&run_dir)?;
    let mut args: Vec<String> = if gradle {
        // The daemon stays: it is what keeps a warm Gradle fast. The agent reaches only the
        // `Test` tasks' JVMs, through the init script, never the daemons.
        let script = gradle_init_script(&agent_jar()?)?;
        vec![
            "--console=plain".into(),
            "--init-script".into(),
            plain(&script),
            format!("-Pimpact.agent={option}"),
        ]
    } else {
        vec!["-B".into(), "-ntp".into()]
    };
    if clean.is_some() {
        args.push("clean".into());
    }
    if !goals {
        // Gradle's `--tests` filters the task named before it, which must be a `Test` task.
        let task = if explicit { "test" } else { "impactTests" };
        args.push(if gradle { task } else { "verify" }.into());
    }
    let mut command = Command::new("");
    selection.speedups = levers.apply(
        workspace,
        &mut executable,
        explicit_executable,
        &mut command,
        &mut args,
    );
    args.extend(extra);
    // Reports of dropped tests would otherwise survive from earlier runs.
    crate::timing::clear_reports(workspace)?;
    selection.reason = match &clean {
        Some(reason) => format!("{}; clean: {reason}", selection.reason),
        None if gradle => format!("{}; incremental build, Gradle daemon", selection.reason),
        None => format!("{}; incremental build", selection.reason),
    };
    eprintln!("{}", serde_json::to_string_pretty(&selection)?);
    let mut build_tool = Command::new(&executable);
    build_tool.envs(command.get_envs().filter_map(|(k, v)| Some((k, v?))));
    if !gradle {
        build_tool.env("JDK_JAVA_OPTIONS", java_options(&option));
    }
    let status = build_tool.current_dir(workspace).args(&args).status()?;
    summarize(&dir, &mut selection)?;
    if !status.success() && selection.tests.is_empty() && selection.skipped.is_empty() {
        selection.reason = format!("{}; the build failed before any test ran", selection.reason);
    }
    // Failures the agent could not record, such as a crashed fork or an ignored test failure.
    let failed = mark_failures(&dir, workspace, &selection.tests, status.success())?;
    if !failed.is_empty() {
        selection.reason = format!("{}; failed: {}", selection.reason, failed.len());
    }
    // Every test class the JVMs discovered, unless named tests limited discovery.
    let suite: BTreeSet<String> = selection
        .tests
        .iter()
        .chain(&selection.skipped)
        .cloned()
        .collect();
    let skipped = selection.skipped.clone();
    summarize_run(
        &dir,
        workspace,
        config,
        true,
        &skipped,
        (!explicit).then_some(&suite),
        started,
        &mut selection,
    );
    write(&selection)?;
    print_summary(&selection);
    let passed = status.success() && failed.is_empty();
    write_json(
        &state,
        &LastRun {
            key: keyed(&base_key)?,
            passed,
            context,
            build,
            tree,
        },
    )?;
    // The exit status stays Maven's, which ignored failures leave at success.
    Ok(if status.success() { 0 } else { 1 })
}

/// Adds the run summary to the selection.
#[allow(clippy::too_many_arguments)]
fn summarize_run(
    dir: &Path,
    workspace: &Path,
    config: &Config,
    built: bool,
    skipped: &BTreeSet<String>,
    suite: Option<&BTreeSet<String>>,
    started: std::time::Instant,
    selection: &mut Selection,
) {
    let test_dirs: Vec<PathBuf> = outputs(workspace)
        .into_iter()
        .filter(|(_, test)| *test)
        .map(|(dir, _)| dir)
        .collect();
    match crate::summary::finish(
        true,
        dir,
        workspace,
        &config.tool,
        built,
        skipped,
        suite,
        &test_dirs,
        started.elapsed().as_secs_f64(),
    ) {
        Ok(summary) => selection.summary = Some(summary),
        // The summary is informational: a problem with it never fails the run.
        Err(error) => eprintln!("sieve: no summary: {error}"),
    }
}

/// Prints the summary last, below the build output and the selection.
fn print_summary(selection: &Selection) {
    if let Some(summary) = &selection.summary {
        for line in summary.lines() {
            eprintln!("{line}");
        }
        summary.publish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jar_slots_drop_the_version_and_keep_the_classifier() {
        let maven =
            "/r/com/fasterxml/jackson/core/jackson-databind/2.17.0/jackson-databind-2.17.0.jar";
        assert_eq!(slot(maven), "jackson-databind");
        let gradle =
            "/g/files-2.1/org.testcontainers/postgresql/1.20.1/abc123/postgresql-1.20.1.jar";
        assert_eq!(slot(gradle), "postgresql");
        assert_eq!(slot("/r/example/lib/1.0/lib-1.0-tests.jar"), "lib-tests");
        assert_eq!(slot("/elsewhere/lib/lib-1.0.jar"), "lib");
        assert_eq!(slot("/elsewhere/lib/annotations-13.0.jar"), "annotations");
        assert_eq!(slot("/elsewhere/gradle-worker.jar"), "gradle-worker");
        assert_eq!(slot("/elsewhere/h2.jar"), "h2");
    }

    #[test]
    fn version_bumps_rerun_the_users_of_the_bumped_jar_only() {
        let listing = |jars: &[(&str, &str)]| -> Listing {
            let mut l: Listing = jars
                .iter()
                .map(|(s, h)| (s.to_string(), h.to_string()))
                .collect();
            l.sort();
            l
        };
        let then = listing(&[("a", "a1"), ("b", "b1"), ("junit", "j1")]);
        let bumped = listing(&[("a", "a2"), ("b", "b1"), ("junit", "j1")]);
        let uses_a = Record {
            jars: [("a1", "a"), ("j1", "junit")]
                .into_iter()
                .map(|(h, s)| (h.into(), s.into()))
                .collect(),
            ..Record::default()
        };
        let uses_b = Record {
            jars: [("b1", "b"), ("j1", "junit")]
                .into_iter()
                .map(|(h, s)| (h.into(), s.into()))
                .collect(),
            ..Record::default()
        };
        assert_eq!(
            dependency_change(Some(&then), Some(&bumped), &uses_a).as_deref(),
            Some("Changed dependency: a")
        );
        assert_eq!(dependency_change(Some(&then), Some(&bumped), &uses_b), None);
        // A jar the record used that was never on the recorded class path cannot be a bump.
        let other_agent = Record {
            jars: [("x9", "jacoco")]
                .into_iter()
                .map(|(h, s)| (h.into(), s.into()))
                .collect(),
            ..Record::default()
        };
        assert_eq!(
            dependency_change(Some(&then), Some(&bumped), &other_agent),
            None
        );
        // Added, removed, or swapped artifacts are not bumps.
        let added = listing(&[("a", "a1"), ("b", "b1"), ("c", "c1"), ("junit", "j1")]);
        assert_eq!(
            dependency_change(Some(&then), Some(&added), &uses_b).as_deref(),
            Some("Build input changed: dependencies added or removed: +c")
        );
        let swapped = listing(&[("a", "a1"), ("d", "d1"), ("junit", "j1")]);
        assert_eq!(
            dependency_change(Some(&then), Some(&swapped), &uses_a).as_deref(),
            Some("Build input changed: dependencies added or removed: +d, -b")
        );
        // Without both listings the old rule holds.
        assert_eq!(
            dependency_change(None, Some(&bumped), &uses_b).as_deref(),
            Some("Build input changed")
        );
        assert_eq!(
            dependency_change(Some(&then), None, &uses_b).as_deref(),
            Some("Build input changed")
        );
    }

    fn shape(supers: &[&str]) -> Shape {
        Shape {
            shape: "s".into(),
            wiring: "w".into(),
            supers: supers.iter().map(|s| s.to_string()).collect(),
            ..Shape::default()
        }
    }

    #[test]
    fn hierarchy_follows_project_supertypes_and_subtypes() {
        let shapes: Snapshot = [
            ("a/Api", shape(&["java/lang/Object"])),
            ("a/Impl", shape(&["a/Base", "a/Api"])),
            ("a/Base", shape(&["java/lang/Object"])),
            ("a/Other", shape(&["a/Base"])),
            ("a/Unrelated", shape(&[])),
        ]
        .into_iter()
        .map(|(n, s)| (n.to_owned(), s))
        .collect();
        let related = hierarchy(&BTreeSet::from(["a/Impl"]), &shapes);
        assert_eq!(related, BTreeSet::from(["a/Api", "a/Base", "a/Impl"]));
        let related = hierarchy(&BTreeSet::from(["a/Api"]), &shapes);
        assert_eq!(related, BTreeSet::from(["a/Api", "a/Impl"]));
    }

    #[test]
    fn relative_paths_skip_scratch_and_outside_files() {
        let root = Path::new("/ws");
        assert_eq!(
            relative(root, "/ws/src/test/resources/../resources/a.json").as_deref(),
            Some("src/test/resources/a.json")
        );
        assert_eq!(
            relative(root, "/ws/./target/test-classes/a.json").as_deref(),
            Some("target/test-classes/a.json")
        );
        assert_eq!(relative(root, "/other/a.json"), None);
        assert_eq!(relative(root, "/ws/.sieve/records/x.json"), None);
        assert_eq!(relative(root, "/ws/target/surefire/tmp1"), None);
    }

    #[test]
    fn gradle_outputs_mark_test_source_sets() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::write(
            root.join("impact.json"),
            r#"{"tool":"gradle","modules":{".":[]}}"#,
        )
        .unwrap();
        // Before the first compilation, the conventional directories already count.
        let before = outputs(root);
        assert!(before.contains(&(root.join("build/classes/kotlin/main"), false)));
        assert!(before.contains(&(root.join("build/resources/test"), true)));
        for dir in [
            "build/classes/kotlin/integTest",
            "build/classes/java/generated",
            "build/resources/integTest",
        ] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        let after = outputs(root);
        assert!(after.contains(&(root.join("build/classes/kotlin/integTest"), true)));
        assert!(after.contains(&(root.join("build/classes/java/generated"), false)));
        assert!(after.contains(&(root.join("build/resources/integTest"), true)));
        assert_eq!(after.len(), after.iter().collect::<BTreeSet<_>>().len());
        fs::write(
            root.join("impact.json"),
            r#"{"tool":"maven","modules":{".":[]}}"#,
        )
        .unwrap();
        assert_eq!(
            outputs(root),
            [
                (root.join("target/classes"), false),
                (root.join("target/test-classes"), true)
            ]
        );
    }

    #[test]
    fn reasons_come_from_the_jvm_that_ran_each_test() {
        let temp = tempfile::tempdir().unwrap();
        let run = temp.path().join("run");
        fs::create_dir_all(&run).unwrap();
        assert_eq!(jvm_pid(&run.join("decide-4711-99.tmp"), "decide"), "4711");
        assert_eq!(
            jvm_pid(&run.join("summary-4711-99.json"), "summary"),
            "4711"
        );
        let reasons = |unit: &str, it: &str| serde_json::json!({"a.UnitTest": unit, "a.FlowIT": it, "a.Base": "No record"});
        write_json(
            &run.join("decisions-1.json"),
            &reasons("Unchanged test record", "Invocation changed"),
        )
        .unwrap();
        write_json(
            &run.join("decisions-2.json"),
            &reasons("Invocation changed", "Changed method: a/B#c()V"),
        )
        .unwrap();
        let summary = |ran: &[&str], dropped: &[&str]| Summary {
            ran: ran.iter().map(|s| s.to_string()).collect(),
            dropped: dropped.iter().map(|s| s.to_string()).collect(),
            notes: Vec::new(),
        };
        write_json(
            &run.join("summary-1-5.json"),
            &summary(&[], &["a.UnitTest"]),
        )
        .unwrap();
        write_json(&run.join("summary-2-6.json"), &summary(&["a.FlowIT"], &[])).unwrap();
        let config: Config =
            serde_json::from_value(serde_json::json!({"tool": "maven", "modules": {".": []}}))
                .unwrap();
        let mut selection = config.all("test");
        summarize(temp.path(), &mut selection).unwrap();
        assert_eq!(selection.reasons["a.UnitTest"], "Unchanged test record");
        assert_eq!(selection.reasons["a.FlowIT"], "Changed method: a/B#c()V");
        assert_eq!(selection.reasons["a.Base"], "No record");
        assert_eq!(selection.mode, "SUBSET");
    }

    #[test]
    fn build_info_hashes_without_its_timestamp() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("META-INF/build-info.properties");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(
            &file,
            "build.version=1\nbuild.time=2026-01-01T00\\:00\\:00Z\n",
        )
        .unwrap();
        let before = file_hash(&file);
        fs::write(
            &file,
            "build.version=1\nbuild.time=2026-02-02T00\\:00\\:00Z\n",
        )
        .unwrap();
        assert_eq!(file_hash(&file), before);
        fs::write(
            &file,
            "build.version=2\nbuild.time=2026-02-02T00\\:00\\:00Z\n",
        )
        .unwrap();
        assert_ne!(file_hash(&file), before);
        assert_eq!(file_hash(&temp.path().join("missing")), "-");
    }

    #[test]
    fn goals_replace_verify() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(!has_goals("maven", &args(&["-Pci", "-q", "-Dx=1"])));
        assert!(!has_goals("maven", &args(&["-P", "ci", "-pl", "."])));
        assert!(has_goals("maven", &args(&["test"])));
        assert!(has_goals("maven", &args(&["-P", "ci", "test"])));
        assert!(!has_goals(
            "gradle",
            &args(&["--tests", "a.BTest", "-Pci=1", "--offline"])
        ));
        assert!(!has_goals(
            "gradle",
            &args(&["-x", "detekt", "--console", "plain"])
        ));
        assert!(has_goals("gradle", &args(&["check"])));
        assert!(has_goals(
            "gradle",
            &args(&["--tests", "a.BTest", ":integrationTest"])
        ));
        assert!(explicit_tests(&args(&["--tests", "a.BTest"])));
        assert!(explicit_tests(&args(&["-Dtest=ATest"])));
        assert!(!explicit_tests(&args(&["-Pci", "check"])));
    }

    #[test]
    fn clean_after_deletions_build_and_generated_inputs() {
        let config: Config = serde_json::from_value(serde_json::json!({
            "tool": "maven", "modules": {".": []}, "generated": ["src/main/resources/api/**"]
        }))
        .unwrap();
        let tree = |entries: &[(&str, &str)]| -> BTreeMap<String, String> {
            entries
                .iter()
                .map(|(p, s)| (p.to_string(), s.to_string()))
                .collect()
        };
        let last = LastRun {
            key: String::new(),
            passed: true,
            context: String::new(),
            build: "b".into(),
            tree: tree(&[
                ("src/A.java", "1:1"),
                ("src/main/resources/api/x.yaml", "1:1"),
            ]),
        };
        assert!(clean_reason(&config, None, &last.tree, "b").is_some());
        assert_eq!(clean_reason(&config, Some(&last), &last.tree, "b"), None);
        let interrupted = LastRun {
            passed: false,
            key: String::new(),
            context: String::new(),
            build: last.build.clone(),
            tree: last.tree.clone(),
        };
        assert!(clean_reason(&config, Some(&interrupted), &last.tree, "b")
            .unwrap()
            .contains("did not pass or finish"));
        assert!(clean_reason(&config, Some(&last), &last.tree, "c").is_some());
        let edited = tree(&[
            ("src/A.java", "2:2"),
            ("src/main/resources/api/x.yaml", "1:1"),
        ]);
        assert_eq!(clean_reason(&config, Some(&last), &edited, "b"), None);
        let deleted = tree(&[("src/main/resources/api/x.yaml", "1:1")]);
        assert!(clean_reason(&config, Some(&last), &deleted, "b")
            .unwrap()
            .contains("src/A.java"));
        let generated = tree(&[
            ("src/A.java", "1:1"),
            ("src/main/resources/api/x.yaml", "3:3"),
        ]);
        assert!(clean_reason(&config, Some(&last), &generated, "b")
            .unwrap()
            .contains("Generated"));
    }

    #[test]
    fn preserved_metadata_edits_change_the_key_and_clean_before_deciding() {
        let temp = tempfile::tempdir().unwrap();
        let config: Config =
            serde_json::from_value(serde_json::json!({"tool": "maven", "modules": {".": []}}))
                .unwrap();
        let source = temp.path().join("src/A.java");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, "return a + b;").unwrap();
        let last = LastRun {
            key: String::new(),
            passed: true,
            context: String::new(),
            build: "b".into(),
            tree: tree(&config, temp.path()).unwrap(),
        };
        let metadata = fs::metadata(&source).unwrap();
        fs::write(&source, "return a - b;").unwrap();
        fs::File::options()
            .write(true)
            .open(&source)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(metadata.modified().unwrap()))
            .unwrap();
        let now = tree(&config, temp.path()).unwrap();
        assert_ne!(last.tree, now);
        assert!(clean_reason(&config, Some(&last), &now, "b")
            .unwrap()
            .contains("unchanged metadata"));
        let output = temp.path().join("target/classes/A.class");
        fs::create_dir_all(output.parent().unwrap()).unwrap();
        fs::write(&output, "old bytecode").unwrap();
        let before = output_state(temp.path()).unwrap();
        fs::write(&output, "new bytecode").unwrap();
        assert_ne!(before, output_state(temp.path()).unwrap());
    }

    #[test]
    fn execution_lock_allows_only_its_session_and_releases_without_deleting() {
        let temp = tempfile::tempdir().unwrap();
        let owner = Lock::acquire(temp.path(), "execution.lock").unwrap();
        fs::write(temp.path().join("session"), "session").unwrap();
        assert!(execution_guard(temp.path(), "session").unwrap().is_none());
        assert!(execution_guard(temp.path(), "").is_err());
        assert!(execution_guard(temp.path(), "old-session").is_err());
        drop(owner);
        assert!(temp.path().join("execution.lock").is_file());
        assert!(execution_guard(temp.path(), "session").is_err());
        assert!(execution_guard(temp.path(), "").unwrap().is_some());
    }

    #[test]
    fn build_behavior_levers_require_explicit_adoption() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join(".mvn")).unwrap();
        fs::write(temp.path().join(".mvn/extensions.xml"), "jgitver").unwrap();
        let apply = |levers: Levers| {
            let mut command = Command::new("mvn");
            let mut executable = "mvn".to_owned();
            let mut args = Vec::new();
            let states = levers.apply(temp.path(), &mut executable, true, &mut command, &mut args);
            (states, args)
        };
        let (states, args) = apply(Levers::default());
        assert!(states.values().all(|s| s == "off"));
        assert!(args.is_empty());
        let (states, args) = apply(Levers {
            on: BTreeSet::from(["jgitver".into(), "repackage".into()]),
            off: BTreeSet::new(),
        });
        assert_eq!(states["jgitver"], "on");
        assert_eq!(states["repackage"], "on");
        assert_eq!(
            args,
            ["-Djgitver.skip=true", "-Dspring-boot.repackage.skip=true"]
        );
    }

    #[test]
    fn context_changes_name_the_inputs_without_values() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("contexts")).unwrap();
        let describe = |digest: &str, lines: &[&str]| {
            fs::write(
                temp.path().join("contexts").join(format!("{digest}.txt")),
                lines.join("\n") + "\n",
            )
            .unwrap();
        };
        describe(
            "aa",
            &["01 invocation", "02 property:user.dir", "03 env:STAGE"],
        );
        describe(
            "bb",
            &["01 invocation", "09 property:user.dir", "04 property:extra"],
        );
        assert_eq!(
            context_change(temp.path(), "aa", "bb"),
            "Invocation, JVM properties or declared environment changed: STAGE, extra, user.dir"
        );
        describe("cc", &["05 invocation"]);
        assert!(context_change(temp.path(), "aa", "cc").contains("the sieve invocation"));
        // Missing or unsafe descriptions keep the generic reason.
        for (old, new) in [("aa", "dd"), ("../aa", "bb"), ("", "bb")] {
            assert_eq!(
                context_change(temp.path(), old, new),
                "Invocation, JVM properties or declared environment changed"
            );
        }
    }

    #[test]
    fn request_patterns_overlap_by_segment() {
        assert!(overlap("/patients/{id}", "/patients/search"));
        assert!(overlap("/patients/{id}", "/patients/{patientId}"));
        assert!(!overlap(
            "/practitioners/{id}/day/{day}",
            "/practitioners/{id}/slots"
        ));
        assert!(!overlap("/patients", "/patients/{id}"));
        assert!(overlap("/files/**", "/files/a/b"));
        assert!(overlap("/a/{*rest}", "/a"));
        assert!(!overlap("/invoices/{id}/pdf", "/consultations/{id}/letter"));
    }
}
