//! Class-level test selection for single-module projects, computed from compiled
//! bytecode of the revision under test.
use crate::Result;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

const ACC_PRIVATE: u16 = 0x0002;
const ACC_STATIC: u16 = 0x0008;

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
    /// Source paths whose bytecode was inlined here (Kotlin SMAP).
    pub inlined: Vec<String>,
    /// Declares a non-private static constant, which `javac`/`kotlinc` copy into
    /// callers without leaving a reference.
    pub constants: bool,
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

/// Adds every name a descriptor or signature could denote: `Lpkg/Name;` segments,
/// plus the whole string in internal and binary form.
fn candidates(text: &str, out: &mut BTreeSet<String>) {
    out.insert(text.to_owned());
    if text.contains('.') && !text.contains('/') {
        out.insert(text.replace('.', "/"));
    }
    let mut rest = text;
    while let Some(start) = rest.find('L') {
        rest = &rest[start + 1..];
        let end = rest.find([';', '<']).unwrap_or(rest.len());
        out.insert(rest[..end].to_owned());
    }
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
    let mut index = 1;
    while index < count {
        match r.take(1)?[0] {
            1 => {
                let len = r.u16()? as usize;
                // Modified UTF-8 differs only for NUL and supplementary characters.
                utf8[index] = Some(String::from_utf8_lossy(r.take(len)?).into_owned());
            }
            7 => class_refs[index] = r.u16()?,
            8 | 16 | 19 | 20 => {
                r.take(2)?;
            }
            15 => {
                r.take(3)?;
            }
            3 | 4 | 9 | 10 | 11 | 12 | 17 | 18 => {
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
    for value in utf8.iter().flatten() {
        candidates(value, &mut class.refs);
    }
    r.take(2)?;
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
            r.take(4)?;
            for _ in 0..r.u16()? {
                let name = text(r.u16()?)?;
                let len = r.u32()? as usize;
                r.take(len)?;
                if member == 0
                    && name == "ConstantValue"
                    && access & ACC_STATIC != 0
                    && access & ACC_PRIVATE == 0
                {
                    class.constants = true;
                }
            }
        }
    }
    let package = class.name.rsplit_once('/').map_or("", |(p, _)| p);
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
            "SourceDebugExtension" => class.inlined = smap_files(&String::from_utf8_lossy(body)),
            _ => {}
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

#[derive(Debug, PartialEq)]
pub enum Impact {
    /// Binary names of the selected and unselected top-level test classes.
    Tests {
        selected: BTreeSet<String>,
        unselected: BTreeSet<String>,
    },
    Fallback(String),
}

/// Selects test classes that reach a changed class. Edges follow references and
/// supertypes, and also lead from a type to its subtypes, because a caller of an
/// interface may run any implementation, including one wired by DI or `ServiceLoader`.
pub fn affected(classes: &[Class], changed: &BTreeSet<String>, workspace: &Path) -> Impact {
    let index: BTreeMap<&str, usize> = classes
        .iter()
        .enumerate()
        .map(|(i, c)| (c.name.as_str(), i))
        .collect();
    let mut by_source: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, class) in classes.iter().enumerate() {
        if let Some(source) = &class.source {
            by_source.entry(source).or_default().push(i);
        }
    }
    // `a/B.java` is a suffix of both `src/main/java/a/B.java` and a bare SMAP `B.java`
    // entry's owner; either way the match is by whole path segments.
    let suffix = |long: &str, short: &str| {
        long == short
            || long
                .strip_suffix(short)
                .is_some_and(|rest| rest.ends_with('/'))
    };
    let sources = |path: &str| -> Vec<usize> {
        by_source
            .iter()
            .filter(|(source, _)| suffix(path, source))
            .flat_map(|(_, owned)| owned.iter().copied())
            .collect()
    };
    let mut users = vec![BTreeSet::new(); classes.len()];
    for (i, class) in classes.iter().enumerate() {
        for name in class.refs.iter().chain(&class.supers) {
            if let Some(&j) = index.get(name.as_str()) {
                users[j].insert(i);
            }
        }
        for sup in &class.supers {
            if let Some(&j) = index.get(sup.as_str()) {
                users[i].insert(j);
            }
        }
        for path in &class.inlined {
            let owners = match by_source.get(path.as_str()) {
                Some(owned) => owned.clone(),
                None => by_source
                    .iter()
                    .filter(|(source, _)| suffix(source, path))
                    .flat_map(|(_, owned)| owned.iter().copied())
                    .collect(),
            };
            for j in owners {
                users[j].insert(i);
            }
        }
    }
    let mut pending = Vec::new();
    for path in changed.iter().filter(|p| p.starts_with("src/")) {
        let file = path.rsplit('/').next().unwrap_or(path);
        if !workspace.join(path).is_file() {
            return Impact::Fallback(format!("{path} was deleted"));
        }
        if !(file.ends_with(".java") || file.ends_with(".kt"))
            || matches!(file, "package-info.java" | "module-info.java")
        {
            return Impact::Fallback(format!("{path} is not a class source"));
        }
        let owned = sources(path);
        if owned.is_empty() {
            return Impact::Fallback(format!("{path} has no compiled classes"));
        }
        if let Some(&i) = owned.iter().find(|&&i| classes[i].constants) {
            return Impact::Fallback(format!(
                "{path} declares inlinable constants in {}",
                classes[i].name
            ));
        }
        pending.extend(owned);
    }
    let mut reached = vec![false; classes.len()];
    while let Some(i) = pending.pop() {
        if !std::mem::replace(&mut reached[i], true) {
            pending.extend(users[i].iter().copied());
        }
    }
    let mut selected = BTreeSet::new();
    let mut unselected = BTreeSet::new();
    for (i, class) in classes.iter().enumerate() {
        if class.test && !class.name.contains('$') {
            let name = class.name.replace('/', ".");
            if reached[i] {
                selected.insert(name);
            } else {
                unselected.insert(name);
            }
        }
    }
    Impact::Tests {
        selected,
        unselected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(name: &str, source: &str, test: bool, refs: &[&str], supers: &[&str]) -> Class {
        Class {
            name: name.into(),
            source: Some(source.into()),
            supers: supers.iter().map(|s| s.to_string()).collect(),
            refs: refs.iter().fold(BTreeSet::new(), |mut out, r| {
                candidates(r, &mut out);
                out
            }),
            test,
            ..Class::default()
        }
    }

    #[test]
    fn selection_follows_references_subtypes_and_inlining() {
        let temp = tempfile::tempdir().unwrap();
        let mut classes = vec![
            class("a/Api", "a/Api.java", false, &[], &[]),
            class("a/Impl", "a/Impl.java", false, &[], &["a/Api"]),
            class("a/Service", "a/Service.java", false, &["()La/Api;"], &[]),
            class("a/Unused", "a/Unused.java", false, &[], &[]),
            class("a/Limits", "a/Limits.java", false, &[], &[]),
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
        classes[4].constants = true;
        classes[9].inlined = vec!["a/Inline.kt".into()];
        let impact = |paths: &[&str]| {
            for path in paths {
                let file = temp.path().join(path);
                fs::create_dir_all(file.parent().unwrap()).unwrap();
                fs::write(file, "").unwrap();
            }
            let changed = paths.iter().map(|p| p.to_string()).collect();
            affected(&classes, &changed, temp.path())
        };
        let tests = |selected: &[&str], unselected: &[&str]| Impact::Tests {
            selected: selected.iter().map(|s| s.to_string()).collect(),
            unselected: unselected.iter().map(|s| s.to_string()).collect(),
        };
        // An implementation reaches tests of callers that only see the interface.
        assert_eq!(
            impact(&["src/main/java/a/Impl.java"]),
            tests(&["a.ServiceTest"], &["a.OtherTest", "a.ReflectTest"])
        );
        // Dotted string constants count, so Class.forName users are selected.
        assert_eq!(
            impact(&["src/main/java/a/Unused.java", "README.md"]),
            tests(&["a.ReflectTest"], &["a.OtherTest", "a.ServiceTest"])
        );
        assert_eq!(
            impact(&["src/main/kotlin/a/Inline.kt"]),
            tests(&["a.OtherTest"], &["a.ReflectTest", "a.ServiceTest"])
        );
        assert_eq!(
            impact(&["src/test/java/a/ServiceTest.java"]),
            tests(&["a.ServiceTest"], &["a.OtherTest", "a.ReflectTest"])
        );
        for (path, reason) in [
            ("src/main/java/a/Limits.java", "inlinable constants"),
            ("src/main/java/a/New.java", "no compiled classes"),
            ("src/main/resources/app.properties", "not a class source"),
            ("src/main/java/a/package-info.java", "not a class source"),
        ] {
            assert!(
                matches!(impact(&[path]), Impact::Fallback(r) if r.contains(reason)),
                "{path}"
            );
        }
        let changed = BTreeSet::from(["src/main/java/a/Gone.java".to_owned()]);
        assert!(matches!(
            affected(&classes, &changed, temp.path()),
            Impact::Fallback(r) if r.contains("deleted")
        ));
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
    }
}
