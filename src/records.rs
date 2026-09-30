//! Local mode (ADR 0001, ADR 0002): the agent runs inside the test JVM, asks `sieve decide`
//! which test classes to drop at discovery, and hands what each class executed to
//! `sieve record`. A test class is dropped when its last run passed and nothing it executed
//! or read has changed since, or, without a record, when static analysis shows it reaches no
//! change since a green `--base`.
use crate::{bytecode, classes, fingerprint, Config, Result, Selection};
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Shape {
    shape: String,
    wiring: String,
    supers: Vec<String>,
    component: bool,
}

type Snapshot = BTreeMap<String, Shape>;

#[derive(Deserialize)]
struct Raw {
    jdk: String,
    /// When the test JVM started, in epoch milliseconds.
    #[serde(default)]
    started: u64,
    parallel: bool,
    errors: Vec<String>,
    dropped: Vec<String>,
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
    for sub in ["records", "snapshots", "run"] {
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

/// Serializes `record` calls from concurrently forked test JVMs.
struct Lock(PathBuf);

impl Lock {
    fn acquire(dir: &Path) -> Result<Self> {
        let path = dir.join("lock");
        let start = Instant::now();
        loop {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| age > Duration::from_secs(300));
                    if stale {
                        let _ = fs::remove_file(&path);
                    } else if start.elapsed() > Duration::from_secs(600) {
                        return Err(format!("Timed out waiting for {}", path.display()).into());
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn outputs(workspace: &Path) -> [(PathBuf, bool); 2] {
    let target = workspace.join("target");
    [
        (target.join("classes"), false),
        (target.join("test-classes"), true),
    ]
}

/// Content hash of a file a test read; `-` when it does not exist. Spring Boot's build
/// information is hashed without its timestamp, which every native build rewrites.
fn file_hash(path: &Path) -> String {
    if path.is_dir() {
        // A listing counts; class files are left out, because component scanning is covered
        // by the Spring wiring rule and class changes by the method and shape rules.
        let mut names: Vec<String> = fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.ends_with(".class"))
            .collect();
        names.sort();
        return format!("dir:{}", bytecode::hash(names.join("\n").as_bytes()));
    }
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
                let component = self.index.get(name).is_some_and(|&i| self.component[i]);
                let shape = Shape {
                    shape: d.shape.clone(),
                    wiring: d.wiring.clone(),
                    supers: d.supers.clone(),
                    component,
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
    let scratch = ["target/surefire", "target/failsafe"];
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

/// `sieve record --workspace PATH --raw FILE`: turns what the agent saw into test records.
pub fn record(args: Vec<String>) -> Result<u8> {
    let options = options(args, &["--workspace", "--raw"])?;
    let workspace = workspace_option(&options)?;
    let raw_path = PathBuf::from(options.get("--raw").ok_or("--raw is required")?);
    let raw: Raw = serde_json::from_slice(&fs::read(&raw_path)?)?;
    let dir = prepare(&workspace)?;
    let _lock = Lock::acquire(&dir)?;
    let mut summary = Summary {
        ran: raw.tests.iter().map(|t| t.name.clone()).collect(),
        dropped: raw.dropped.iter().cloned().collect(),
        notes: Vec::new(),
    };
    // Without complete evidence only failures are kept; older records stay as they were, and
    // their hashes no longer match whatever changed since.
    let trusted = !raw.parallel && raw.errors.is_empty();
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
    for test in &raw.tests {
        let path = record_path(&dir, &test.name);
        let hash = current.test_hash(&test.name);
        let kept = read_json::<Record>(&path).filter(|r| r.test == hash);
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
        let mut methods: BTreeSet<&str> = test.methods.iter().map(String::as_str).collect();
        let mut files: BTreeSet<String> = test
            .files
            .iter()
            .filter_map(|f| relative(&workspace, f))
            .collect();
        let mut spring = test.spring;
        if let Some(kept) = &kept {
            methods.extend(kept.methods.keys().map(String::as_str));
            files.extend(kept.files.keys().cloned());
            spring |= kept.spring;
        }
        let record = Record {
            test: hash,
            passed: true,
            jdk: raw.jdk.clone(),
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
                    let hash = file_hash(&workspace.join(&f));
                    (f, hash)
                })
                .collect(),
        };
        snapshot_used = true;
        write_json(&path, &record)?;
    }
    if snapshot_used {
        let path = dir.join("snapshots").join(format!("{snapshot_id}.json"));
        if !path.is_file() {
            write_json(&path, &snapshot)?;
        }
    }
    collect_garbage(&dir)?;
    let name = raw_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("raw")
        .replacen("raw", "summary", 1);
    write_json(&dir.join("run").join(format!("{name}.json")), &summary)?;
    Ok(0)
}

/// Removes snapshots that no record refers to any more.
fn collect_garbage(dir: &Path) -> Result<()> {
    let mut used = BTreeSet::new();
    for entry in fs::read_dir(dir.join("records"))? {
        if let Some(record) = read_json::<Record>(&entry?.path()) {
            used.insert(format!("{}.json", record.snapshot));
        }
    }
    for entry in fs::read_dir(dir.join("snapshots"))? {
        let entry = entry?;
        if !used.contains(entry.file_name().to_string_lossy().as_ref()) {
            let _ = fs::remove_file(entry.path());
        }
    }
    Ok(())
}

/// Project supertypes and subtypes of `classes`, transitively, including themselves.
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
                    stack.extend(subtypes.get(name).into_iter().flatten().copied());
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
    };
    let mut changed = BTreeSet::new();
    for name in then.keys().chain(now.keys()) {
        let (a, b) = (then.get(name), now.get(name));
        let component = a.is_some_and(|s| s.component) || b.is_some_and(|s| s.component);
        let differs = a.map(|s| (&s.wiring, s.component)) != b.map(|s| (&s.wiring, s.component));
        if component && differs {
            if b.is_none() {
                wiring.all = Some(format!("Spring component removed: {name}"));
            }
            changed.insert(name.clone());
        }
    }
    changed.retain(|n| now.contains_key(n));
    if changed.is_empty() || wiring.all.is_some() {
        return wiring;
    }
    let label = changed.iter().next().cloned().unwrap_or_default();
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

struct Decider<'a> {
    workspace: &'a Path,
    dir: PathBuf,
    current: Current,
    now: Snapshot,
    snapshots: BTreeMap<String, Option<(Snapshot, Wiring)>>,
}

impl Decider<'_> {
    /// Why the test must run, or `None` when its record shows that nothing it used changed.
    fn check(&mut self, test: &str, record: &Record, jdk: &str) -> Option<String> {
        let current = &self.current;
        if !record.passed {
            return Some("Failed last time".into());
        }
        if record.jdk != jdk {
            return Some(format!("JDK changed from {}", record.jdk));
        }
        if record.test != current.test_hash(test) {
            return Some("Test class changed".into());
        }
        if record.build != current.build {
            return Some("Build input changed".into());
        }
        for (file, hash) in &record.files {
            if file_hash(&self.workspace.join(file)) != *hash {
                return Some(format!("Changed resource: {file}"));
            }
        }
        for (method, hash) in &record.methods {
            if current.method(method) != Some(hash) {
                return Some(format!("Changed method: {method}"));
            }
        }
        if current.context_test(test) && !record.spring {
            return Some("Context test without context evidence".into());
        }
        let snapshots = &mut self.snapshots;
        let (then, wiring) = match snapshots.entry(record.snapshot.clone()).or_insert_with(|| {
            let path = self
                .dir
                .join("snapshots")
                .join(format!("{}.json", record.snapshot));
            let then: Snapshot = read_json(&path)?;
            let wiring = wiring(&then, &self.now, current, self.workspace);
            Some((then, wiring))
        }) {
            Some(entry) => (&entry.0, &entry.1),
            None => return Some("Record snapshot missing".into()),
        };
        let touched: BTreeSet<&str> = record
            .methods
            .keys()
            .filter_map(|m| m.split_once('#').map(|(owner, _)| owner))
            .collect();
        // Classes used without running code of their own: owners of fields read or written,
        // whose static initializers ran elsewhere, and annotation types, transitively.
        let mut shaped: BTreeSet<&str> = touched.clone();
        let mut stack: Vec<&str> = touched.iter().copied().collect();
        while let Some(name) = stack.pop() {
            let Some(&i) = current.index.get(name) else {
                continue;
            };
            let class = &current.classes[i];
            let annotations = class.refs.iter().filter(|r| {
                current
                    .index
                    .get(r.as_str())
                    .is_some_and(|&j| current.classes[j].annotation)
            });
            for used in class.fields.iter().chain(annotations) {
                if let Some((key, _)) = current.index.get_key_value(used.as_str()) {
                    if shaped.insert(key.as_str()) && current.classes[current.index[key]].annotation
                    {
                        stack.push(key.as_str());
                    }
                }
            }
        }
        for name in shaped.difference(&touched) {
            let (a, b) = (then.get(*name), self.now.get(*name));
            if a.map(|s| &s.shape) != b.map(|s| &s.shape) {
                return Some(format!("Structural change: {name}"));
            }
        }
        let related: BTreeSet<&str> = hierarchy(&touched, then)
            .into_iter()
            .chain(hierarchy(&touched, &self.now))
            .collect();
        for name in related {
            let (a, b) = (then.get(name), self.now.get(name));
            if a.map(|s| (&s.shape, &s.supers)) != b.map(|s| (&s.shape, &s.supers)) {
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
        for name in &touched {
            let refs = current.index.get(*name).map(|&i| &current.classes[i].refs);
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
    let options = options(args, &["--workspace", "--jdk", "--out", "--base"])?;
    let workspace = workspace_option(&options)?;
    let jdk = options.get("--jdk").ok_or("--jdk is required")?;
    let out = PathBuf::from(options.get("--out").ok_or("--out is required")?);
    let dir = prepare(&workspace)?;
    let current = Current::load(&workspace)?;
    let now = current.snapshot();
    let tests = current.tests();
    let mut decider = Decider {
        workspace: &workspace,
        dir: dir.clone(),
        current,
        now,
        snapshots: BTreeMap::new(),
    };
    let mut unreached = None;
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
            Some(record) => match decider.check(test, &record, jdk) {
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
    fs::write(
        &out,
        skip.iter().map(|t| format!("{t}\n")).collect::<String>(),
    )?;
    let name = format!("decisions-{}.json", std::process::id());
    write_json(&dir.join("run").join(name), &reasons)?;
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
fn agent_option(workspace: &Path, dir: &Path, mode: &str, base: Option<&str>) -> Result<String> {
    let jar = agent_jar()?;
    let file = dir.join("agent.properties");
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
    ] {
        // Properties files treat backslashes as escapes.
        props += &format!("{key}={}\n", value.replace('\\', "\\\\"));
    }
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

fn single_module_maven(config: &Config) -> Result<()> {
    if config.tool != "maven" || config.modules.keys().ne(["."]) {
        return Err("\"records\" needs a single-module Maven project (modules {\".\": []})".into());
    }
    Ok(())
}

/// `sieve env --workspace PATH [--base REV]`: the `JDK_JAVA_OPTIONS` value that gives plain
/// `mvn test` or `mvn verify` the same selection as `sieve run`.
pub fn env_command(args: Vec<String>) -> Result<u8> {
    let options = options(args, &["--workspace", "--base"])?;
    let workspace = workspace_option(&options)?;
    single_module_maven(&Config::read(&workspace)?)?;
    let dir = prepare(&workspace)?;
    let base = options.get("--base").map(String::as_str);
    println!(
        "{}",
        java_options(&agent_option(&workspace, &dir, "select", base)?)
    );
    Ok(0)
}

/// Files that can affect a build, with size and modification time, relative to the workspace.
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
                format!("{}:{modified}", meta.len())
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
    if last.build != build {
        return Some("Build input changed".into());
    }
    for (path, stamp) in &last.tree {
        if stamp != "-" && tree.get(path).is_none_or(|s| s == "-") {
            return Some(format!("Deleted or renamed: {path}"));
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
            .field(file_hash(&workspace.join(&file)).as_bytes());
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
    let reports = match crate::fixtures::read_reports(workspace, "maven") {
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
    let _lock = Lock::acquire(dir)?;
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

/// Whether the build arguments name goals or phases, which then replace the default `verify`.
fn has_goals(extra: &[String]) -> bool {
    let mut args = extra.iter();
    while let Some(arg) = args.next() {
        if !arg.starts_with('-') {
            return true;
        }
        if VALUED.contains(&arg.as_str()) {
            args.next();
        }
    }
    false
}

fn summarize(dir: &Path, selection: &mut Selection) -> Result<()> {
    let mut ran = BTreeSet::new();
    let mut dropped = BTreeSet::new();
    let mut decided = false;
    let mut notes = Vec::new();
    for entry in fs::read_dir(dir.join("run"))? {
        let path = entry?.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if name.starts_with("decisions-") {
            if let Some(reasons) = read_json::<BTreeMap<String, String>>(&path) {
                decided = true;
                selection.reasons.extend(reasons);
            }
        } else if name.starts_with("summary-") {
            if let Some(summary) = read_json::<Summary>(&path) {
                ran.extend(summary.ran);
                dropped.extend(summary.dropped);
                notes.extend(summary.notes);
            }
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

/// `reuse` and `jgitver` are on unless switched off; `mvnd` is opt-in, because the daemon
/// forks test JVMs with its own environment rather than the client's.
pub const LEVERS: &[&str] = &["reuse", "jgitver", "mvnd"];

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

    fn enabled(&self, name: &str, default: bool) -> bool {
        !self.off.contains(name) && (default || self.on.contains(name))
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
        let reuse = if !self.enabled("reuse", true) {
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
        } else if self.enabled("jgitver", true) {
            // Filtered resources that embed the version change and rerun their readers.
            args.push("-Djgitver.skip=true".into());
            "on".to_owned()
        } else {
            "off".to_owned()
        };
        states.insert("jgitver".into(), jgitver);
        let mvnd = if !self.enabled("mvnd", false) {
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
    } = run;
    let explicit_executable = executable.is_some();
    let mut executable =
        executable.unwrap_or_else(|| crate::setup::default_executable(workspace, &config.tool));
    single_module_maven(config)?;
    let dir = prepare(workspace)?;
    let write = |selection: &Selection| -> Result<()> {
        let json = serde_json::to_string_pretty(selection)? + "\n";
        if let Some(path) = &output {
            fs::write(path, &json)?;
        }
        eprintln!("{json}");
        Ok(())
    };
    let explicit = extra
        .iter()
        .any(|a| a.starts_with("-Dtest=") || a.starts_with("-Dit.test="));
    let goals = has_goals(&extra);
    let tree = tree(config, workspace)?;
    let build = build_inputs(workspace)?;
    let mut key = bytecode::Hasher::default();
    key.field(&serde_json::to_vec(&tree)?)
        .field(build.as_bytes())
        .field(recorded_files(&dir, workspace).as_bytes())
        .field(executable.as_bytes())
        .field(&serde_json::to_vec(&extra)?)
        .field(base.as_deref().unwrap_or_default().as_bytes())
        .field(env!("CARGO_PKG_VERSION").as_bytes());
    for (_, bytes) in JARS {
        key.field(bytes);
    }
    for var in [
        "JAVA_HOME",
        "JDK_JAVA_OPTIONS",
        "MAVEN_ARGS",
        "MAVEN_OPTS",
        "PATH",
    ] {
        key.field(env::var(var).unwrap_or_default().as_bytes());
    }
    key.field(format!("{levers:?}").as_bytes());
    let key = key.finish();
    let state = dir.join("last-run.json");
    let last: Option<LastRun> = read_json(&state);
    let mut selection = config.all("Test records decide inside the test JVM");
    selection.mode = "RECORDS";
    if !full && !explicit && !goals && last.as_ref().is_some_and(|l| l.passed && l.key == key) {
        selection.mode = "NONE";
        selection.reason = "No changes since the last passing run; no build started".into();
        write(&selection)?;
        return Ok(0);
    }
    let clean = clean_reason(config, last.as_ref(), &tree, &build);
    // Until this run finishes, the next one cleans after anything either tree saw deleted.
    let mut seen = last.as_ref().map(|l| l.tree.clone()).unwrap_or_default();
    seen.extend(tree.clone());
    write_json(
        &state,
        &LastRun {
            key: String::new(),
            passed: false,
            build: last.as_ref().map_or_else(String::new, |l| l.build.clone()),
            tree: seen,
        },
    )?;
    let mode = if full { "record" } else { "select" };
    let option = agent_option(workspace, &dir, mode, base.as_deref())?;
    let run_dir = dir.join("run");
    fs::remove_dir_all(&run_dir)?;
    fs::create_dir_all(&run_dir)?;
    let mut args: Vec<String> = vec!["-B".into(), "-ntp".into()];
    if clean.is_some() {
        args.push("clean".into());
    }
    if !goals {
        args.push("verify".into());
    }
    // No test uses the executable archive. Build information stays: applications read it.
    args.push("-Dspring-boot.repackage.skip=true".into());
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
        None => format!("{}; incremental build", selection.reason),
    };
    eprintln!("{}", serde_json::to_string_pretty(&selection)?);
    let mut maven = Command::new(&executable);
    maven.envs(command.get_envs().filter_map(|(k, v)| Some((k, v?))));
    let status = maven
        .current_dir(workspace)
        .args(&args)
        .env("JDK_JAVA_OPTIONS", java_options(&option))
        .status()?;
    summarize(&dir, &mut selection)?;
    // Failures the agent could not record, such as a crashed fork or an ignored test failure.
    let failed = mark_failures(&dir, workspace, &selection.tests, status.success())?;
    if !failed.is_empty() {
        selection.reason = format!("{}; failed: {}", selection.reason, failed.len());
    }
    write(&selection)?;
    let passed = status.success() && failed.is_empty();
    write_json(
        &state,
        &LastRun {
            key,
            passed,
            build,
            tree,
        },
    )?;
    // The exit status stays Maven's, which ignored failures leave at success.
    Ok(if status.success() { 0 } else { 1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(supers: &[&str]) -> Shape {
        Shape {
            shape: "s".into(),
            wiring: "w".into(),
            supers: supers.iter().map(|s| s.to_string()).collect(),
            component: false,
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
        assert!(!has_goals(&args(&["-Pci", "-q", "-Dx=1"])));
        assert!(!has_goals(&args(&["-P", "ci", "-pl", "."])));
        assert!(has_goals(&args(&["test"])));
        assert!(has_goals(&args(&["-P", "ci", "test"])));
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
            build: "b".into(),
            tree: tree(&[
                ("src/A.java", "1:1"),
                ("src/main/resources/api/x.yaml", "1:1"),
            ]),
        };
        assert!(clean_reason(&config, None, &last.tree, "b").is_some());
        assert_eq!(clean_reason(&config, Some(&last), &last.tree, "b"), None);
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
}
