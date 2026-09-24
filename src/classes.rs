//! Class-level test selection computed from compiled bytecode of the revision under test.
use crate::Result;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

const ACC_PRIVATE: u16 = 0x0002;
const ACC_STATIC: u16 = 0x0008;
const ACC_ANNOTATION: u16 = 0x2000;

/// Types whose annotated classes a dependency-injection container discovers by scanning,
/// without any test referencing them.
const COMPONENT_MARKERS: &[&str] = &[
    "org/springframework/stereotype/",
    "org/springframework/context/annotation/",
    "org/springframework/context/event/",
    "org/springframework/web/bind/annotation/",
    "org/springframework/boot/SpringBootConfiguration",
    "org/springframework/boot/autoconfigure/",
    "org/springframework/boot/context/properties/",
    "org/springframework/data/",
    "jakarta/persistence/",
    "javax/persistence/",
    "jakarta/inject/",
    "javax/inject/",
    "jakarta/enterprise/",
    "javax/enterprise/",
    "jakarta/ws/rs/",
    "javax/ws/rs/",
    "io/micronaut/",
    "io/quarkus/",
];

/// Test support that starts such a container, declaratively or, as system tests booting
/// several applications do, by calling the container's bootstrap API.
const CONTEXT_MARKERS: &[&str] = &[
    "org/springframework/test/context/",
    "org/springframework/boot/test/",
    "org/springframework/boot/SpringApplication",
    "org/springframework/boot/builder/",
    "org/springframework/context/annotation/AnnotationConfigApplicationContext",
    "org/springframework/context/support/",
    "io/micronaut/context/ApplicationContext",
    "io/micronaut/test/",
    "io/quarkus/test/",
    "org/jboss/arquillian/",
];

#[derive(Debug, Default)]
pub struct Class {
    /// Internal name, such as `example/Calculator$Inner`.
    pub name: String,
    /// Package-relative source path from the `SourceFile` attribute.
    pub source: Option<String>,
    /// Superclass and interfaces.
    pub supers: Vec<String>,
    /// Candidate class names from the constant pool: class entries, descriptors,
    /// signatures, annotations, and string constants such as `Class.forName` arguments.
    pub refs: BTreeSet<String>,
    /// The subset of `refs` used other than as a declared supertype: owners of accessed
    /// members, types in descriptors and member signatures, and string constants.
    pub uses: BTreeSet<String>,
    /// Source paths whose bytecode was inlined here (Kotlin SMAP).
    pub inlined: Vec<String>,
    /// Non-private static constants, which `javac`/`kotlinc` copy into callers without
    /// leaving a reference.
    pub constants: Vec<String>,
    pub annotation: bool,
    /// Loaded from a test output directory.
    pub test: bool,
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(len).filter(|&e| e <= self.bytes.len());
        let end = end.ok_or("Truncated class file")?;
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
}

/// Adds the `Lpkg/Name;` segments of a descriptor or signature.
fn typed(text: &str, out: &mut BTreeSet<String>) {
    let mut rest = text;
    while let Some(start) = rest.find('L') {
        rest = &rest[start + 1..];
        let end = rest.find([';', '<']).unwrap_or(rest.len());
        out.insert(rest[..end].to_owned());
    }
}

/// Adds every name a constant could denote: its typed segments, plus the whole string in
/// internal and binary form.
fn candidates(text: &str, out: &mut BTreeSet<String>) {
    out.insert(text.to_owned());
    if text.contains('.') && !text.contains('/') {
        out.insert(text.replace('.', "/"));
    }
    typed(text, out);
}

pub fn parse(bytes: &[u8]) -> Result<Class> {
    let mut r = Reader { bytes, at: 0 };
    if r.u32()? != 0xCAFE_BABE {
        return Err("Not a class file".into());
    }
    r.take(4)?;
    let count = r.u16()? as usize;
    let mut utf8: Vec<Option<String>> = vec![None; count];
    let mut class_refs = vec![0u16; count];
    let mut member_refs = Vec::new();
    let mut member_names = vec![0u16; count];
    let mut index = 1;
    while index < count {
        match r.take(1)?[0] {
            1 => {
                let len = r.u16()? as usize;
                // Modified UTF-8 differs only for NUL and supplementary characters.
                utf8[index] = Some(String::from_utf8_lossy(r.take(len)?).into_owned());
            }
            7 => class_refs[index] = r.u16()?,
            9..=11 => member_refs.push((r.u16()?, r.u16()?)),
            12 => {
                member_names[index] = r.u16()?;
                r.take(2)?;
            }
            8 | 16 | 19 | 20 => {
                r.take(2)?;
            }
            15 => {
                r.take(3)?;
            }
            3 | 4 | 17 | 18 => {
                r.take(4)?;
            }
            5 | 6 => {
                r.take(8)?;
                index += 1;
            }
            tag => return Err(format!("Unknown constant pool tag {tag}").into()),
        }
        index += 1;
    }
    let text = |i: u16| -> Result<&str> {
        utf8.get(i as usize)
            .and_then(Option::as_deref)
            .ok_or_else(|| "Invalid constant pool index".into())
    };
    let class_name = |i: u16| -> Result<&str> {
        let name = *class_refs.get(i as usize).ok_or("Invalid class index")?;
        text(name)
    };
    let mut class = Class::default();
    // Constructor calls, including every subclass's `super()`, never dispatch to a subtype.
    for &(owner, member) in &member_refs {
        let name = member_names
            .get(member as usize)
            .ok_or("Invalid member index")?;
        if text(*name)? != "<init>" {
            class.uses.insert(class_name(owner)?.to_owned());
        }
    }
    class.annotation = r.u16()? & ACC_ANNOTATION != 0;
    class.name = class_name(r.u16()?)?.to_owned();
    let superclass = r.u16()?;
    if superclass != 0 {
        class.supers.push(class_name(superclass)?.to_owned());
    }
    for _ in 0..r.u16()? {
        class.supers.push(class_name(r.u16()?)?.to_owned());
    }
    for member in 0..2 {
        for _ in 0..r.u16()? {
            let access = r.u16()?;
            let name = text(r.u16()?)?;
            r.take(2)?;
            for _ in 0..r.u16()? {
                let attribute = text(r.u16()?)?;
                let len = r.u32()? as usize;
                r.take(len)?;
                if member == 0
                    && attribute == "ConstantValue"
                    && access & ACC_STATIC != 0
                    && access & ACC_PRIVATE == 0
                {
                    class.constants.push(name.to_owned());
                }
            }
        }
    }
    let package = class.name.rsplit_once('/').map_or("", |(p, _)| p);
    // The class signature names generic supertypes, which are not uses.
    let mut signature = None;
    for _ in 0..r.u16()? {
        let name = text(r.u16()?)?;
        let len = r.u32()? as usize;
        let body = r.take(len)?;
        match name {
            "SourceFile" if len == 2 => {
                let file = text(u16::from_be_bytes([body[0], body[1]]))?;
                class.source = Some(if package.is_empty() {
                    file.to_owned()
                } else {
                    format!("{package}/{file}")
                });
            }
            "Signature" if len == 2 => signature = Some(u16::from_be_bytes([body[0], body[1]])),
            "SourceDebugExtension" => class.inlined = smap_files(&String::from_utf8_lossy(body)),
            _ => {}
        }
    }
    for (i, value) in utf8.iter().enumerate() {
        if let Some(value) = value {
            candidates(value, &mut class.refs);
            if signature != Some(i as u16) {
                typed(value, &mut class.uses);
                if value.contains('.') && !value.contains('/') {
                    class.uses.insert(value.replace('.', "/"));
                }
            }
        }
    }
    Ok(class)
}

/// Source paths listed in the `*F` sections of a JSR-45 source map.
fn smap_files(smap: &str) -> Vec<String> {
    let mut files = Vec::new();
    let mut in_files = false;
    let mut lines = smap.lines();
    while let Some(line) = lines.next() {
        if line.starts_with('*') {
            in_files = line.trim() == "*F";
        } else if in_files {
            let (with_path, entry) = match line.strip_prefix("+ ") {
                Some(entry) => (true, entry),
                None => (false, line),
            };
            let name = entry.split_once(' ').map_or("", |(_, n)| n);
            match (with_path, lines.clone().next()) {
                (true, Some(path)) => {
                    lines.next();
                    files.push(path.trim().to_owned());
                }
                _ => files.push(name.trim().to_owned()),
            }
        }
    }
    files.retain(|f| !f.is_empty());
    files
}

/// Reads every `.class` file below the directories; `true` marks test output.
pub fn load(dirs: &[(PathBuf, bool)]) -> Result<Vec<Class>> {
    fn walk(dir: &Path, test: bool, out: &mut Vec<Class>) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                walk(&path, test, out)?;
            } else if path.extension().is_some_and(|e| e == "class") {
                let mut class =
                    parse(&fs::read(&path)?).map_err(|e| format!("{}: {e}", path.display()))?;
                class.test = test;
                if class.name != "module-info" {
                    out.push(class);
                }
            }
        }
        Ok(())
    }
    let mut classes = Vec::new();
    for (dir, test) in dirs {
        if dir.is_dir() {
            walk(dir, *test, &mut classes)?;
        }
    }
    Ok(classes)
}

/// Java and Kotlin sources below `workspace`, relative to it, with their text.
fn source_files(workspace: &Path) -> Result<Vec<(String, String)>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if entry.file_type()?.is_dir() {
                if !matches!(
                    name.as_ref(),
                    ".git" | ".gradle" | ".idea" | ".kotlin" | "node_modules"
                ) {
                    walk(root, &path, out)?;
                }
            } else if name.ends_with(".java") || name.ends_with(".kt") {
                let relative = path
                    .strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((relative, String::from_utf8_lossy(&fs::read(&path)?).into()));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(workspace, workspace, &mut out)?;
    Ok(out)
}

/// Whether `text` contains `word` delimited by non-identifier characters.
fn mentions(text: &str, word: &str) -> bool {
    let ident = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    text.match_indices(word).any(|(at, _)| {
        !text[..at].chars().next_back().is_some_and(ident)
            && !text[at + word.len()..].chars().next().is_some_and(ident)
    })
}

#[derive(Debug, PartialEq)]
pub enum Impact {
    /// Binary names of the selected and unselected top-level test classes.
    Tests {
        selected: BTreeSet<String>,
        unselected: BTreeSet<String>,
    },
    Fallback(String),
}

/// How a class is affected by the changes.
#[derive(Clone, Copy, PartialEq, PartialOrd)]
enum Reach {
    None,
    /// A subtype changed: only code calling through this type may run the changed code.
    Dispatch,
    /// The class changed or depends on changed code.
    Full,
}

/// Selects test classes that reach the changed source files (workspace-relative paths).
///
/// A class is fully affected when it changed, references or inlines an affected class, or
/// extends one. When a subtype is affected, callers of its supertypes are affected too,
/// because they may run it through DI or `ServiceLoader`; the supertype's other subtypes
/// are not. A class copying a changed constant is found by its source naming the constant
/// or its owner. A reached DI component affects every test that starts a container.
pub fn affected(classes: &[Class], changed: &BTreeSet<String>, workspace: &Path) -> Result<Impact> {
    let mut index: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    let mut by_source: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, class) in classes.iter().enumerate() {
        index.entry(&class.name).or_default().push(i);
        if let Some(source) = &class.source {
            by_source.entry(source).or_default().push(i);
        }
    }
    let lookup = |name: &str| index.get(name).into_iter().flatten().copied();
    // `a/B.java` is a suffix of both `src/main/java/a/B.java` and a bare SMAP `B.java`
    // entry's owner; either way the match is by whole path segments.
    let suffix = |long: &str, short: &str| {
        long == short
            || long
                .strip_suffix(short)
                .is_some_and(|rest| rest.ends_with('/'))
    };
    let owners = |path: &str| -> Vec<usize> {
        by_source
            .iter()
            .filter(|(source, _)| suffix(path, source))
            .flat_map(|(_, owned)| owned.iter().copied())
            .collect()
    };
    let n = classes.len();
    let mut callers = vec![BTreeSet::new(); n];
    let mut subtypes = vec![Vec::new(); n];
    let mut supers = vec![Vec::new(); n];
    let mut links = vec![Vec::new(); n];
    for (i, class) in classes.iter().enumerate() {
        for name in &class.refs {
            for j in lookup(name).filter(|&j| j != i) {
                links[i].push(j);
                // A subtype naming its supertype only in its declaration does not call it.
                if !class.supers.contains(name) || class.uses.contains(name) {
                    callers[j].insert(i);
                }
            }
        }
        for sup in &class.supers {
            for j in lookup(sup).filter(|&j| j != i) {
                subtypes[j].push(i);
                supers[i].push(j);
            }
        }
        for path in &class.inlined {
            let owned = match by_source.get(path.as_str()) {
                Some(owned) => owned.clone(),
                None => by_source
                    .iter()
                    .filter(|(source, _)| suffix(source, path))
                    .flat_map(|(_, owned)| owned.iter().copied())
                    .collect(),
            };
            for j in owned {
                callers[j].insert(i);
            }
        }
    }
    let marked = |class: &Class, markers: &[&str]| {
        class
            .refs
            .iter()
            .any(|r| markers.iter().any(|m| r.starts_with(m)))
    };
    // Custom stereotypes and test annotations carry their markers by meta-annotation.
    let mut component: Vec<bool> = classes
        .iter()
        .map(|c| marked(c, COMPONENT_MARKERS))
        .collect();
    let mut context: Vec<bool> = classes
        .iter()
        .map(|c| c.test && marked(c, CONTEXT_MARKERS))
        .collect();
    loop {
        let mut grew = false;
        for (i, class) in classes.iter().enumerate() {
            let via = |flags: &[bool], supertypes: bool| {
                links[i].iter().any(|&j| {
                    flags[j] && (classes[j].annotation || supertypes && supers[i].contains(&j))
                })
            };
            if !component[i] && via(&component, false) {
                component[i] = true;
                grew = true;
            }
            if class.test && !context[i] && via(&context, true) {
                context[i] = true;
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    let mut pending = Vec::new();
    let mut sources = None;
    for path in changed {
        let file = path.rsplit('/').next().unwrap_or(path);
        if !workspace.join(path).is_file() {
            return Ok(Impact::Fallback(format!("{path} was deleted")));
        }
        if !(file.ends_with(".java") || file.ends_with(".kt"))
            || matches!(file, "package-info.java" | "module-info.java")
        {
            return Ok(Impact::Fallback(format!("{path} is not a class source")));
        }
        let owned = owners(path);
        if owned.is_empty() {
            return Ok(Impact::Fallback(format!("{path} has no compiled classes")));
        }
        for &i in &owned {
            let class = &classes[i];
            if class.constants.is_empty() {
                continue;
            }
            let simple = class.name.rsplit('/').next().unwrap_or(&class.name);
            let words: Vec<&str> = simple
                .split('$')
                .chain(class.constants.iter().map(String::as_str))
                .filter(|w| !w.is_empty())
                .collect();
            let sources = match &mut sources {
                Some(sources) => sources,
                None => sources.insert(source_files(workspace)?),
            };
            let mut located = vec![false; n];
            for (file, text) in sources.iter() {
                let owned = owners(file);
                for &j in &owned {
                    located[j] = true;
                }
                if words.iter().any(|w| mentions(text, w)) {
                    pending.extend(owned.into_iter().map(|j| (j, Reach::Full)));
                }
            }
            // Without its source, a class may copy the constant unseen.
            pending.extend((0..n).filter(|&j| !located[j]).map(|j| (j, Reach::Full)));
        }
        pending.extend(owned.into_iter().map(|i| (i, Reach::Full)));
    }
    let mut reach = vec![Reach::None; n];
    let mut containers = false;
    while let Some((i, level)) = pending.pop() {
        if reach[i] >= level {
            continue;
        }
        reach[i] = level;
        pending.extend(callers[i].iter().map(|&j| (j, Reach::Full)));
        pending.extend(supers[i].iter().map(|&j| (j, Reach::Dispatch)));
        if level == Reach::Full {
            pending.extend(subtypes[i].iter().map(|&j| (j, Reach::Full)));
            if component[i] && !std::mem::replace(&mut containers, true) {
                pending.extend((0..n).filter(|&j| context[j]).map(|j| (j, Reach::Full)));
            }
        }
    }
    let mut selected = BTreeSet::new();
    let mut unselected = BTreeSet::new();
    for (i, class) in classes.iter().enumerate() {
        if class.test && !class.annotation && !class.name.contains('$') {
            let name = class.name.replace('/', ".");
            if reach[i] == Reach::Full {
                selected.insert(name);
            } else {
                unselected.insert(name);
            }
        }
    }
    // A name may exist in several modules; running it anywhere keeps each copy selected.
    unselected.retain(|name| !selected.contains(name));
    Ok(Impact::Tests {
        selected,
        unselected,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(name: &str, source: &str, test: bool, refs: &[&str], supers: &[&str]) -> Class {
        let mut refs: BTreeSet<String> = refs.iter().fold(BTreeSet::new(), |mut out, r| {
            candidates(r, &mut out);
            out
        });
        let uses = refs.clone();
        refs.extend(supers.iter().map(|s| s.to_string()));
        Class {
            name: name.into(),
            source: Some(source.into()),
            supers: supers.iter().map(|s| s.to_string()).collect(),
            refs,
            uses,
            test,
            ..Class::default()
        }
    }

    struct Graph {
        temp: tempfile::TempDir,
        classes: Vec<Class>,
    }

    impl Graph {
        fn new(classes: Vec<Class>) -> Self {
            Self {
                temp: tempfile::tempdir().unwrap(),
                classes,
            }
        }

        fn source(&self, path: &str, text: &str) {
            let file = self.temp.path().join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, text).unwrap();
        }

        fn impact(&self, paths: &[&str]) -> Impact {
            for path in paths {
                if !self.temp.path().join(path).exists() {
                    self.source(path, "");
                }
            }
            let changed = paths.iter().map(|p| p.to_string()).collect();
            affected(&self.classes, &changed, self.temp.path()).unwrap()
        }
    }

    fn tests(selected: &[&str], unselected: &[&str]) -> Impact {
        Impact::Tests {
            selected: selected.iter().map(|s| s.to_string()).collect(),
            unselected: unselected.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn selection_follows_references_subtypes_and_inlining() {
        let mut classes = vec![
            class("a/Api", "a/Api.java", false, &[], &[]),
            class("a/Impl", "a/Impl.java", false, &[], &["a/Api"]),
            class("a/Service", "a/Service.java", false, &["()La/Api;"], &[]),
            class("a/Unused", "a/Unused.java", false, &[], &[]),
            class("a/InlineKt", "a/Inline.kt", false, &[], &[]),
            class(
                "a/ServiceTest",
                "a/ServiceTest.java",
                true,
                &["a/Service"],
                &[],
            ),
            class("a/ServiceTest$Nested", "a/ServiceTest.java", true, &[], &[]),
            class(
                "a/ReflectTest",
                "a/ReflectTest.java",
                true,
                &["a.Unused"],
                &[],
            ),
            class("a/OtherTest", "a/OtherTest.java", true, &[], &[]),
        ];
        classes[8].inlined = vec!["a/Inline.kt".into()];
        let graph = Graph::new(classes);
        // An implementation reaches tests of callers that only see the interface.
        assert_eq!(
            graph.impact(&["src/main/java/a/Impl.java"]),
            tests(&["a.ServiceTest"], &["a.OtherTest", "a.ReflectTest"])
        );
        // Dotted string constants count, so Class.forName users are selected.
        assert_eq!(
            graph.impact(&["src/main/java/a/Unused.java"]),
            tests(&["a.ReflectTest"], &["a.OtherTest", "a.ServiceTest"])
        );
        assert_eq!(
            graph.impact(&["src/main/kotlin/a/Inline.kt"]),
            tests(&["a.OtherTest"], &["a.ReflectTest", "a.ServiceTest"])
        );
        assert_eq!(
            graph.impact(&["src/test/java/a/ServiceTest.java"]),
            tests(&["a.ServiceTest"], &["a.OtherTest", "a.ReflectTest"])
        );
        // Multi-module paths match by package-relative suffix.
        assert_eq!(
            graph.impact(&["core/src/main/java/a/Unused.java"]),
            tests(&["a.ReflectTest"], &["a.OtherTest", "a.ServiceTest"])
        );
        for (path, reason) in [
            ("src/main/java/a/New.java", "no compiled classes"),
            ("src/main/resources/app.properties", "not a class source"),
            ("src/main/java/a/package-info.java", "not a class source"),
        ] {
            assert!(
                matches!(graph.impact(&[path]), Impact::Fallback(r) if r.contains(reason)),
                "{path}"
            );
        }
        let changed = BTreeSet::from(["src/main/java/a/Gone.java".to_owned()]);
        assert!(matches!(
            affected(&graph.classes, &changed, graph.temp.path()).unwrap(),
            Impact::Fallback(r) if r.contains("deleted")
        ));
    }

    #[test]
    fn a_changed_implementation_does_not_reach_sibling_implementations() {
        let graph = Graph::new(vec![
            class("a/Rule", "a/Rule.java", false, &[], &[]),
            class("a/Bulk", "a/Bulk.java", false, &[], &["a/Rule"]),
            class("a/Member", "a/Member.java", false, &[], &["a/Rule"]),
            // A decorator implements the interface and also calls through it.
            class(
                "a/Logged",
                "a/Logged.java",
                false,
                &["La/Rule;"],
                &["a/Rule"],
            ),
            class(
                "a/Calc",
                "a/Calc.java",
                false,
                &["Ljava/util/List<La/Rule;>;"],
                &[],
            ),
            class("a/BulkTest", "a/BulkTest.java", true, &["a/Bulk"], &[]),
            class(
                "a/MemberTest",
                "a/MemberTest.java",
                true,
                &["a/Member"],
                &[],
            ),
            class(
                "a/LoggedTest",
                "a/LoggedTest.java",
                true,
                &["a/Logged"],
                &[],
            ),
            class("a/CalcTest", "a/CalcTest.java", true, &["a/Calc"], &[]),
        ]);
        assert_eq!(
            graph.impact(&["src/main/java/a/Bulk.java"]),
            tests(
                &["a.BulkTest", "a.CalcTest", "a.LoggedTest"],
                &["a.MemberTest"]
            )
        );
        // A changed interface still reaches every implementation.
        assert_eq!(
            graph.impact(&["src/main/java/a/Rule.java"]),
            tests(
                &["a.BulkTest", "a.CalcTest", "a.LoggedTest", "a.MemberTest"],
                &[]
            )
        );
    }

    #[test]
    fn constants_reach_sources_that_name_them() {
        let mut classes = vec![
            class("a/Limits", "a/Limits.java", false, &[], &[]),
            class("a/Plain", "a/Plain.java", false, &[], &[]),
            class("a/LimitsTest", "a/LimitsTest.java", true, &[], &[]),
            class("a/ImportTest", "a/ImportTest.java", true, &[], &[]),
            class("a/OtherTest", "a/OtherTest.java", true, &[], &[]),
        ];
        classes[0].constants = vec!["MAX".into()];
        let graph = Graph::new(classes);
        graph.source(
            "src/main/java/a/Limits.java",
            "class Limits { static final int MAX = 1; }",
        );
        graph.source(
            "src/test/java/a/LimitsTest.java",
            "assertEquals(1, Limits.MAX);",
        );
        graph.source(
            "src/test/java/a/ImportTest.java",
            "import static a.Limits.*; x(MAX);",
        );
        graph.source(
            "src/test/java/a/OtherTest.java",
            "MAXIMUM; NoLimits; Limits2",
        );
        assert_eq!(
            graph.impact(&["src/main/java/a/Limits.java"]),
            tests(&["a.ImportTest", "a.LimitsTest"], &["a.OtherTest"])
        );
        // A class whose source cannot be found may copy the constant.
        fs::remove_file(graph.temp.path().join("src/test/java/a/OtherTest.java")).unwrap();
        assert_eq!(
            graph.impact(&["src/main/java/a/Limits.java"]),
            tests(&["a.ImportTest", "a.LimitsTest", "a.OtherTest"], &[])
        );
    }

    #[test]
    fn reached_components_reach_container_tests() {
        let spring = "org/springframework/boot/test/context/SpringBootTest";
        let mut classes = vec![
            class(
                "a/App",
                "a/App.java",
                false,
                &["org/springframework/boot/autoconfigure/SpringBootApplication"],
                &[],
            ),
            class(
                "a/Service",
                "a/Service.java",
                false,
                &["org/springframework/stereotype/Service"],
                &[],
            ),
            class("a/Helper", "a/Helper.java", false, &[], &[]),
            class("a/Plain", "a/Plain.java", false, &[], &[]),
            class(
                "a/Caller",
                "a/Caller.java",
                false,
                &["a/Helper", "a/Custom"],
                &[],
            ),
            class(
                "a/Custom",
                "a/Custom.java",
                false,
                &["org/springframework/stereotype/Component"],
                &[],
            ),
            class("a/Uses", "a/Uses.java", false, &["a/Plain"], &[]),
            // A composed test annotation and an inherited context configuration.
            class(
                "a/IntegrationTest",
                "a/IntegrationTest.java",
                true,
                &[spring],
                &[],
            ),
            class("a/AppIT", "a/AppIT.java", true, &["a/IntegrationTest"], &[]),
            class("a/Base", "a/Base.java", true, &[spring], &[]),
            class("a/WebIT", "a/WebIT.java", true, &[], &["a/Base"]),
            class("a/PlainTest", "a/PlainTest.java", true, &["a/Plain"], &[]),
            // Boots applications itself, as system tests starting several services do.
            class(
                "a/SystemIT",
                "a/SystemIT.java",
                true,
                &["org/springframework/boot/builder/SpringApplicationBuilder"],
                &[],
            ),
        ];
        classes[5].annotation = true;
        classes[7].annotation = true;
        let graph = Graph::new(classes);
        // Helper reaches a class carrying a custom stereotype, so containers start it.
        assert_eq!(
            graph.impact(&["src/main/java/a/Helper.java"]),
            tests(
                &["a.AppIT", "a.Base", "a.SystemIT", "a.WebIT"],
                &["a.PlainTest"]
            )
        );
        assert_eq!(
            graph.impact(&["src/main/java/a/Plain.java"]),
            tests(
                &["a.PlainTest"],
                &["a.AppIT", "a.Base", "a.SystemIT", "a.WebIT"]
            )
        );
    }

    #[test]
    fn smap_lists_inlined_sources() {
        let smap = "SMAP\nCaller.kt\nKotlin\n*S Kotlin\n*F\n+ 1 Caller.kt\na/Caller.kt\n\
                    + 2 Inline.kt\nb/Inline.kt\n3 Bare.kt\n*L\n1#1,5:1\n*E\n";
        assert_eq!(smap_files(smap), ["a/Caller.kt", "b/Inline.kt", "Bare.kt"]);
    }

    #[test]
    fn candidates_cover_descriptors_and_binary_names() {
        let mut out = BTreeSet::new();
        candidates("(La/B;Ljava/util/List<La/C;>;)V", &mut out);
        candidates("a.D", &mut out);
        for name in ["a/B", "java/util/List", "a/C", "a/D"] {
            assert!(out.contains(name), "{name}: {out:?}");
        }
        assert!(mentions("x(Limits.MAX)", "MAX"));
        assert!(!mentions("MAXIMUM", "MAX"));
        assert!(!mentions("$MAX", "MAX"));
    }
}
