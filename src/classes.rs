//! Class-level test selection computed from compiled bytecode of the revision under test.
use crate::Result;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

const ACC_PRIVATE: u16 = 0x0002;
const ACC_STATIC: u16 = 0x0008;
const ACC_ABSTRACT: u16 = 0x0400;
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
    "org/jboss/weld/junit",
    "io/helidon/microprofile/testing/",
    "io/helidon/microprofile/tests/",
    "org/springframework/modulith/test/",
];

/// Test support that finds classes by scanning packages, class paths, or directories, so a
/// test using it leaves no reference to the classes it checks or runs: architecture rules,
/// module verification, package suites, Cucumber glue, and class-path scanners.
const SCANNING_MARKERS: &[&str] = &[
    "com/tngtech/archunit/",
    "org/springframework/modulith/core/",
    "org/springframework/modulith/docs/",
    "org/junit/platform/suite/api/Select",
    "org/junit/platform/launcher/",
    "org/junit/extensions/cpsuite/",
    "io/cucumber/",
    "io/github/classgraph/",
    "org/reflections/",
    "com/google/common/reflect/ClassPath",
    "org/springframework/context/annotation/ClassPathScanningCandidateComponentProvider",
    "com/openpojo/",
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
    /// Owners of the fields this class reads or writes.
    pub fields: BTreeSet<String>,
    /// Source paths whose bytecode was inlined here (Kotlin SMAP).
    pub inlined: Vec<String>,
    /// Non-private static constants, which `javac`/`kotlinc` copy into callers without
    /// leaving a reference.
    pub constants: Vec<String>,
    pub annotation: bool,
    /// Abstract classes and interfaces, which JUnit never runs by themselves.
    pub abstract_: bool,
    /// Loaded from a test output directory.
    pub test: bool,
    /// Runtime-visible annotations on the class itself.
    pub annotations: Vec<Annotation>,
}

/// An annotation's type and, for each element present, the classes it lists.
#[derive(Debug, Default, Clone)]
pub struct Annotation {
    pub name: String,
    pub elements: BTreeMap<String, Vec<String>>,
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

/// Adds the internal names that dotted words in a string could denote: each run of
/// identifier characters and dots, and its leading segments. `a.B#m`, `T(a.B).m()`, and
/// `a.B.m` all name `a/B`.
fn dotted(text: &str, out: &mut BTreeSet<String>) {
    let word = |c: char| c.is_alphanumeric() || matches!(c, '_' | '$' | '.');
    for run in text.split(|c| !word(c)).filter(|run| run.contains('.')) {
        let mut name = String::new();
        for segment in run.split('.') {
            if segment.is_empty() {
                break;
            }
            if !name.is_empty() {
                name.push('/');
                out.insert(name.clone() + segment);
            }
            name.push_str(segment);
        }
    }
}

/// Adds every name a constant could denote: its typed segments, the whole string, and the
/// class names it spells in binary form.
fn candidates(text: &str, out: &mut BTreeSet<String>) {
    out.insert(text.to_owned());
    dotted(text, out);
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
    let mut field_owners = Vec::new();
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
            tag @ 9..=11 => {
                let owner = r.u16()?;
                member_refs.push((owner, r.u16()?));
                if tag == 9 {
                    field_owners.push(owner);
                }
            }
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
    let access = r.u16()?;
    class.annotation = access & ACC_ANNOTATION != 0;
    for &owner in &field_owners {
        class.fields.insert(class_name(owner)?.to_owned());
    }
    class.abstract_ = access & ACC_ABSTRACT != 0;
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
            "RuntimeVisibleAnnotations" => {
                let mut a = Reader { bytes: body, at: 0 };
                for _ in 0..a.u16()? {
                    class.annotations.push(annotation(&mut a, &text)?);
                }
            }
            _ => {}
        }
    }
    for (i, value) in utf8.iter().enumerate() {
        if let Some(value) = value {
            candidates(value, &mut class.refs);
            if signature != Some(i as u16) {
                typed(value, &mut class.uses);
                dotted(value, &mut class.uses);
            }
        }
    }
    Ok(class)
}

/// The internal name in a field descriptor such as `La/B;` or `[La/B;`.
fn descriptor_name(descriptor: &str) -> String {
    let name = descriptor.trim_start_matches('[');
    let name = name.strip_prefix('L').unwrap_or(name);
    name.strip_suffix(';').unwrap_or(name).to_owned()
}

fn annotation<'a>(r: &mut Reader, text: &impl Fn(u16) -> Result<&'a str>) -> Result<Annotation> {
    let name = descriptor_name(text(r.u16()?)?);
    let mut elements = BTreeMap::new();
    for _ in 0..r.u16()? {
        let element = text(r.u16()?)?.to_owned();
        let mut classes = Vec::new();
        element_value(r, text, &mut classes)?;
        elements.insert(element, classes);
    }
    Ok(Annotation { name, elements })
}

/// Skips an element value, collecting the classes it lists outside nested annotations.
fn element_value<'a>(
    r: &mut Reader,
    text: &impl Fn(u16) -> Result<&'a str>,
    classes: &mut Vec<String>,
) -> Result<()> {
    match r.take(1)?[0] {
        b'B' | b'C' | b'D' | b'F' | b'I' | b'J' | b'S' | b'Z' | b's' => {
            r.take(2)?;
        }
        b'e' => {
            r.take(4)?;
        }
        b'c' => classes.push(descriptor_name(text(r.u16()?)?)),
        b'@' => {
            annotation(r, text)?;
        }
        b'[' => {
            for _ in 0..r.u16()? {
                element_value(r, text, classes)?;
            }
        }
        tag => return Err(format!("Unknown element value tag {tag}").into()),
    }
    Ok(())
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

/// Whether a directory holds build output, unless it is itself a project directory.
fn output_dir(dir: &Path, name: &str) -> bool {
    matches!(
        name,
        ".git" | ".gradle" | ".idea" | ".kotlin" | ".sieve" | "node_modules"
    ) || matches!(name, "target" | "build")
        && !["pom.xml", "build.gradle", "build.gradle.kts"]
            .iter()
            .any(|file| dir.join(file).is_file())
}

/// Files below `workspace` accepted by `keep`, by workspace-relative path, with their text.
/// `outputs` also descends into build output, where generated sources live.
fn text_files(
    workspace: &Path,
    outputs: bool,
    keep: &dyn Fn(&str) -> bool,
) -> Result<Vec<(String, String)>> {
    fn walk(
        root: &Path,
        dir: &Path,
        outputs: bool,
        keep: &dyn Fn(&str) -> bool,
        out: &mut Vec<(String, String)>,
    ) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if entry.file_type()?.is_dir() {
                // Below a source directory, `build` and `target` are ordinary directories.
                let sources = path.strip_prefix(root)?.iter().any(|part| part == "src");
                let skip = if outputs {
                    matches!(
                        name.as_ref(),
                        ".git" | ".gradle" | ".idea" | ".kotlin" | "node_modules"
                    )
                } else {
                    output_dir(&path, &name)
                        && !(sources && matches!(name.as_ref(), "build" | "target"))
                };
                if !skip {
                    walk(root, &path, outputs, keep, out)?;
                }
            } else {
                let relative = path
                    .strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/");
                if keep(&relative) {
                    out.push((relative, String::from_utf8_lossy(&fs::read(&path)?).into()));
                }
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(workspace, workspace, outputs, keep, &mut out)?;
    Ok(out)
}

/// Java and Kotlin sources below `workspace`, relative to it, with their text.
fn source_files(workspace: &Path) -> Result<Vec<(String, String)>> {
    text_files(workspace, true, &|path| {
        path.ends_with(".java") || path.ends_with(".kt")
    })
}

/// The identifiers in a source text.
fn identifiers(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The package a Java or Kotlin source declares, in internal form (`""` for the default
/// package), or `None` when the declaration cannot be read.
fn declared_package(text: &str) -> Option<String> {
    let mut rest = text.trim_start_matches('\u{feff}');
    loop {
        rest = rest.trim_start();
        if let Some(comment) = rest.strip_prefix("//") {
            rest = comment.split_once('\n').map_or("", |(_, after)| after);
        } else if let Some(comment) = rest.strip_prefix("/*") {
            rest = comment.split_once("*/")?.1;
        } else if rest.starts_with("@file:") {
            // Kotlin file annotations, such as `@file:JvmName("Names")`, may take arguments.
            let end = rest.find(['\n', '('])?;
            rest = if rest[end..].starts_with('(') {
                rest[end..].split_once(')')?.1
            } else {
                &rest[end..]
            };
        } else if let Some(declaration) = rest.strip_prefix("package") {
            if !declaration.starts_with(char::is_whitespace) {
                return None;
            }
            let name: String = declaration
                .trim_start()
                .chars()
                .take_while(|&c| c.is_alphanumeric() || matches!(c, '_' | '$' | '.' | '`'))
                .filter(|&c| c != '`')
                .collect();
            return (!name.is_empty()).then(|| name.replace('.', "/"));
        } else {
            return Some(String::new());
        }
    }
}

/// Class names spelled in the text files of source directories other than Java and Kotlin
/// sources: bean definitions, factories, service files, and logging or mapping
/// configuration. Values of `name` attributes, such as logger names, are left out.
fn resource_names(workspace: &Path) -> Result<Vec<(String, BTreeSet<String>)>> {
    const LIMIT: usize = 1 << 20;
    let files = text_files(workspace, false, &|path| {
        path.split('/').any(|segment| segment == "src")
            && ![".java", ".kt", ".class"].iter().any(|e| path.ends_with(e))
    })?;
    let word = |c: char| c.is_alphanumeric() || matches!(c, '_' | '$' | '.');
    let mut out = Vec::new();
    for (path, text) in files {
        if text.len() > LIMIT || text.contains('\0') {
            continue;
        }
        let mut names = BTreeSet::new();
        let mut at = 0;
        for run in text.split(|c| !word(c)) {
            let before = text[..at].trim_end_matches(['"', '\'']).trim_end();
            let attribute = before
                .strip_suffix('=')
                .map(str::trim_end)
                .and_then(|b| b.strip_suffix("name"))
                .is_some_and(|b| !b.ends_with(|c: char| c.is_alphanumeric() || c == '-'));
            if !attribute {
                dotted(run, &mut names);
            }
            at += run.len();
            at += text[at..].chars().next().map_or(0, char::len_utf8);
        }
        if !names.is_empty() {
            out.push((path, names));
        }
    }
    Ok(out)
}

/// Component kinds that Spring Boot test slices load. A slice scans only the kinds it lists,
/// so other components never enter its context.
const GLOBAL: u8 = 1;
const CONTROLLER: u8 = 2;
const WEB: u8 = 4;
const JACKSON: u8 = 8;
const DATA: u8 = 16;

/// Kinds from an annotation on a component. The application class and configuration
/// properties join every slice; so does auto-configuration, which slices import.
fn annotation_kind(name: &str) -> u8 {
    match name {
        "org/springframework/boot/SpringBootConfiguration"
        | "org/springframework/boot/context/properties/ConfigurationProperties" => GLOBAL,
        n if n.starts_with("org/springframework/boot/autoconfigure/") => GLOBAL,
        "org/springframework/stereotype/Controller"
        | "org/springframework/web/bind/annotation/RestController" => CONTROLLER,
        "org/springframework/web/bind/annotation/ControllerAdvice"
        | "org/springframework/web/bind/annotation/RestControllerAdvice" => WEB,
        n if n.starts_with("org/springframework/boot/jackson/")
            || n.starts_with("org/springframework/boot/jackson2/") =>
        {
            JACKSON
        }
        n if [
            "org/springframework/data/",
            "jakarta/persistence/",
            "javax/persistence/",
        ]
        .iter()
        .any(|p| n.starts_with(p)) =>
        {
            DATA
        }
        _ => 0,
    }
}

/// Kinds from a library supertype: web slices also scan filters, converters, interceptors,
/// and configurers, and data slices find every repository.
fn supertype_kind(name: &str) -> u8 {
    const WEB_TYPES: &[&str] = &[
        "org/springframework/web/",
        "org/springframework/http/",
        "org/springframework/core/convert/",
        "org/springframework/format/",
        "org/springframework/validation/",
        "org/springframework/security/",
        "org/springframework/boot/web/",
        "org/springframework/boot/webmvc/",
        "org/springframework/boot/webflux/",
        "org/springframework/boot/servlet/",
        "jakarta/servlet/",
        "javax/servlet/",
        "org/thymeleaf/",
    ];
    const JACKSON_TYPES: &[&str] = &[
        "com/fasterxml/jackson/",
        "tools/jackson/",
        "org/springframework/boot/jackson/",
        "org/springframework/boot/jackson2/",
    ];
    let any = |prefixes: &[&str]| prefixes.iter().any(|p| name.starts_with(p));
    if any(WEB_TYPES) {
        WEB
    } else if any(JACKSON_TYPES) {
        JACKSON
    } else if name.starts_with("org/springframework/data/") {
        DATA
    } else {
        0
    }
}

/// What the context of a test started by a Spring Boot slice annotation loads, or `None`
/// for any other annotation. Slices with custom include filters count as full contexts.
fn slice_loads(annotation: &Annotation) -> Option<u8> {
    let (package, simple) = annotation.name.rsplit_once('/')?;
    if !package.starts_with("org/springframework/boot/")
        || annotation.elements.contains_key("includeFilters")
        || annotation.elements.contains_key("useDefaultFilters")
    {
        return None;
    }
    let listed = |element: &str| {
        annotation
            .elements
            .get(element)
            .is_some_and(|c| !c.is_empty())
    };
    match simple {
        // Listed controllers replace controller scanning; tests reference them anyway.
        "WebMvcTest" | "WebFluxTest" if listed("value") || listed("controllers") => {
            Some(WEB | JACKSON)
        }
        "WebMvcTest" | "WebFluxTest" => Some(WEB | JACKSON | CONTROLLER),
        "JsonTest" | "RestClientTest" => Some(JACKSON),
        "DataMongoTest"
        | "DataJpaTest"
        | "DataJdbcTest"
        | "DataR2dbcTest"
        | "DataRedisTest"
        | "DataCassandraTest"
        | "DataElasticsearchTest"
        | "DataNeo4jTest"
        | "DataCouchbaseTest"
        | "DataLdapTest"
        | "JdbcTest"
        | "JooqTest" => Some(DATA),
        _ => None,
    }
}

/// Which classes are DI components, which tests start a context, and which tests scan for
/// classes, given each class's resolved references (`links`) and project supertypes.
fn flags(
    classes: &[Class],
    links: &[Vec<usize>],
    supers: &[Vec<usize>],
) -> (Vec<bool>, Vec<bool>, Vec<bool>) {
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
    // Spring Boot 4 moved test slices to `org/springframework/boot/<technology>/test/`.
    let boot_test = |c: &Class| {
        c.refs.iter().any(|r| {
            r.strip_prefix("org/springframework/boot/")
                .is_some_and(|rest| rest.contains("/test/"))
        })
    };
    let mut context: Vec<bool> = classes
        .iter()
        .map(|c| c.test && (marked(c, CONTEXT_MARKERS) || boot_test(c)))
        .collect();
    let mut scanning: Vec<bool> = classes
        .iter()
        .map(|c| c.test && marked(c, SCANNING_MARKERS))
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
            if class.test && !scanning[i] && via(&scanning, true) {
                scanning[i] = true;
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    (component, context, scanning)
}

/// Per class: whether it is a DI component, and whether it is a test that starts a context.
pub fn kinds(classes: &[Class]) -> (Vec<bool>, Vec<bool>) {
    let mut index: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, class) in classes.iter().enumerate() {
        index.entry(&class.name).or_default().push(i);
    }
    let lookup = |name: &str| index.get(name).into_iter().flatten().copied();
    let mut links = vec![Vec::new(); classes.len()];
    let mut supers = vec![Vec::new(); classes.len()];
    for (i, class) in classes.iter().enumerate() {
        links[i].extend(
            class
                .refs
                .iter()
                .flat_map(|r| lookup(r))
                .filter(|&j| j != i),
        );
        supers[i].extend(
            class
                .supers
                .iter()
                .flat_map(|s| lookup(s))
                .filter(|&j| j != i),
        );
    }
    let (component, context, _) = flags(classes, &links, &supers);
    (component, context)
}

#[derive(Debug, PartialEq)]
pub enum Impact {
    /// Binary names of the selected and unselected top-level test classes, and for each
    /// selected one, how it reaches a change.
    Tests {
        selected: BTreeSet<String>,
        unselected: BTreeSet<String>,
        reasons: BTreeMap<String, String>,
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

/// Workspace sources, each as the classes compiled from it and the identifiers it names, and
/// per class whether its source was found.
type Sources = (Vec<(Vec<usize>, BTreeSet<String>)>, Vec<bool>);

/// Why a class is affected: the class it is affected through, and how.
#[derive(Clone, Copy)]
enum Cause {
    Changed,
    Calls(usize),
    Extends(usize),
    /// A supertype, through which callers may run the changed subtype.
    ImplementedBy(usize),
    CopiesConstantOf(usize),
    /// A context test whose context loads the component.
    Loads(usize),
    Scans,
}

/// The path from a class to a change, such as `a.ServiceTest → a.Service → a.Tax (changed)`.
fn explain(classes: &[Class], cause: &[Option<Cause>], mut i: usize) -> String {
    let name = |i: usize| classes[i].name.replace('/', ".");
    let mut text = String::new();
    let mut seen = BTreeSet::new();
    loop {
        text += &name(i);
        if !seen.insert(i) {
            return text + " …";
        }
        i = match cause[i] {
            None => return text,
            Some(Cause::Changed) => return text + " (changed)",
            Some(Cause::Scans) => return text + " (scans for classes)",
            Some(Cause::Calls(j)) => {
                text += " → ";
                j
            }
            Some(Cause::Extends(j)) => {
                text += " extends ";
                j
            }
            Some(Cause::ImplementedBy(j)) => {
                text += ", implemented by ";
                j
            }
            Some(Cause::CopiesConstantOf(j)) => {
                text += " copies a constant of ";
                j
            }
            Some(Cause::Loads(j)) => {
                text += " (context) loads ";
                j
            }
        };
    }
}

/// Selects test classes that reach the changed source files (workspace-relative paths).
///
/// A class is fully affected when it changed, references or inlines an affected class, or
/// extends one. When a subtype is affected, callers of its supertypes are affected too,
/// because they may run it through DI or `ServiceLoader`; the supertype's other subtypes
/// are not. A class copying a changed constant is found by its source naming the constant
/// or its owner. A reached DI component affects every test that starts a full container,
/// and the Spring Boot slice tests whose slice scans its kind of component.
pub fn affected(classes: &[Class], changed: &BTreeSet<String>, workspace: &Path) -> Result<Impact> {
    affected_by(classes, Changes::Sources(changed), workspace)
}

/// What changed: workspace-relative source paths, or internal class names.
pub enum Changes<'a> {
    Sources(&'a BTreeSet<String>),
    Classes(&'a BTreeSet<String>),
}

pub fn affected_by(classes: &[Class], changes: Changes, workspace: &Path) -> Result<Impact> {
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
    let (mut component, context, scanning) = flags(classes, &links, &supers);
    // Spring reads its factory and import files only while it starts a context, which then
    // loads the classes they name, whatever the slice: auto-configuration, initializers,
    // listeners, and test context customizers.
    let resources = resource_names(workspace)?;
    let spring = |resource: &str| resource.contains("META-INF/spring");
    let mut bootstrapped = vec![false; n];
    for (_, names) in resources.iter().filter(|(resource, _)| spring(resource)) {
        for j in names.iter().flat_map(|name| lookup(name)) {
            component[j] = true;
            bootstrapped[j] = true;
        }
    }
    // Annotations of a class, including those composed into its project annotations.
    let annotations_of = |i: usize| -> Vec<&Annotation> {
        let mut out = Vec::new();
        let mut seen = BTreeSet::from([i]);
        let mut stack = vec![i];
        while let Some(k) = stack.pop() {
            for a in &classes[k].annotations {
                out.push(a);
                for j in lookup(&a.name).filter(|&j| classes[j].annotation) {
                    if seen.insert(j) {
                        stack.push(j);
                    }
                }
            }
        }
        out
    };
    // The class and its project supertypes, or with `enclosing`, also the classes nesting
    // it, whose test configuration JUnit `@Nested` classes inherit.
    let hierarchy = |i: usize, enclosing: bool| -> BTreeSet<usize> {
        let mut out = BTreeSet::new();
        let mut stack = vec![i];
        while let Some(k) = stack.pop() {
            if out.insert(k) {
                stack.extend(supers[k].iter().copied());
                if let Some((outer, _)) = classes[k].name.rsplit_once('$').filter(|_| enclosing) {
                    stack.extend(lookup(outer));
                }
            }
        }
        out
    };
    let kind: Vec<u8> = (0..n)
        .map(|i| {
            if !component[i] {
                return 0;
            }
            let mut kind = if bootstrapped[i] { GLOBAL } else { 0 };
            for k in hierarchy(i, false) {
                kind |= annotations_of(k)
                    .iter()
                    .fold(0, |kind, a| kind | annotation_kind(&a.name));
                for sup in &classes[k].supers {
                    if lookup(sup).next().is_none() {
                        kind |= supertype_kind(sup);
                    }
                }
            }
            kind
        })
        .collect();
    // An explicit `@ComponentScan` on the application drops the slices' exclude filter.
    let sliced = !(0..n).any(|i| {
        kind[i] & GLOBAL != 0
            && classes[i].annotations.iter().any(|a| {
                a.name == "org/springframework/context/annotation/ComponentScan"
                    || a.name == "org/springframework/context/annotation/ComponentScans"
            })
    });
    // `None` marks a full context, which every reached component can affect.
    let loads: Vec<Option<u8>> = (0..n)
        .map(|i| {
            if !context[i] || !sliced {
                return None;
            }
            let mut loads = None;
            for k in hierarchy(i, true) {
                for a in annotations_of(k) {
                    if a.name == "org/springframework/boot/test/context/SpringBootTest" {
                        return None;
                    }
                    if let Some(slice) = slice_loads(a) {
                        loads = Some(loads.unwrap_or(0) | slice);
                    }
                }
            }
            loads
        })
        .collect();
    // The classes compiled from a source file: by package-relative path, and by the package
    // the file declares, which may differ from its directory.
    let compiled = |path: &str, text: &str| -> Vec<usize> {
        let mut owned = owners(path);
        if let Some(package) = declared_package(text) {
            let file = path.rsplit('/').next().unwrap_or(path);
            let source = match package.as_str() {
                "" => file.to_owned(),
                package => format!("{package}/{file}"),
            };
            owned.extend(
                by_source
                    .get(source.as_str())
                    .into_iter()
                    .flatten()
                    .copied(),
            );
        }
        owned.sort_unstable();
        owned.dedup();
        owned
    };
    // Each item is a class, how it is affected, and whether its constants may have changed.
    let mut pending = Vec::new();
    match changes {
        Changes::Sources(changed) => {
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
                let text = String::from_utf8_lossy(&fs::read(workspace.join(path))?).into_owned();
                let owned = compiled(path, &text);
                if owned.is_empty() {
                    return Ok(Impact::Fallback(format!("{path} has no compiled classes")));
                }
                pending.extend(
                    owned
                        .into_iter()
                        .map(|i| (i, Reach::Full, true, Cause::Changed)),
                );
            }
        }
        Changes::Classes(names) => {
            for name in names {
                let owned: Vec<usize> = lookup(name).collect();
                if owned.is_empty() {
                    return Ok(Impact::Fallback(format!("{name} has no compiled class")));
                }
                pending.extend(
                    owned
                        .into_iter()
                        .map(|i| (i, Reach::Full, true, Cause::Changed)),
                );
            }
        }
    }
    // Scanning tests see every class of the packages they scan, whatever changed.
    if !pending.is_empty() {
        pending.extend(
            (0..n)
                .filter(|&j| scanning[j])
                .map(|j| (j, Reach::Full, false, Cause::Scans)),
        );
    }
    // Workspace sources with the classes compiled from them, read once a constant changes.
    let mut sources: Option<Sources> = None;
    let mut searched = vec![false; n];
    let mut reach = vec![Reach::None; n];
    let mut cause = vec![None; n];
    // Full contexts started, and component kinds whose slices started.
    let mut containers = false;
    let mut started = 0u8;
    while let Some((i, level, recomputed, why)) = pending.pop() {
        // A changed constant may change the constants computed from it, which compilers copy
        // onward in turn, so the search repeats for every class naming it.
        if recomputed && level == Reach::Full && !classes[i].constants.is_empty() && !searched[i] {
            searched[i] = true;
            let class = &classes[i];
            let simple = class.name.rsplit('/').next().unwrap_or(&class.name);
            let words: Vec<&str> = simple
                .split('$')
                .chain(class.constants.iter().map(String::as_str))
                .filter(|w| !w.is_empty())
                .collect();
            let (files, located) = match &mut sources {
                Some(sources) => sources,
                None => {
                    let files: Vec<_> = source_files(workspace)?
                        .into_iter()
                        .map(|(file, text)| (compiled(&file, &text), identifiers(&text)))
                        .collect();
                    let mut located = vec![false; n];
                    for &j in files.iter().flat_map(|(owned, _)| owned) {
                        located[j] = true;
                    }
                    sources.insert((files, located))
                }
            };
            for (owned, names) in files.iter() {
                if words.iter().any(|w| names.contains(*w)) {
                    pending.extend(
                        owned
                            .iter()
                            .map(|&j| (j, Reach::Full, true, Cause::CopiesConstantOf(i))),
                    );
                }
            }
            // Without its source, a class may copy the constant unseen.
            pending.extend(
                (0..n)
                    .filter(|&j| !located[j])
                    .map(|j| (j, Reach::Full, true, Cause::CopiesConstantOf(i))),
            );
        }
        if reach[i] >= level {
            continue;
        }
        reach[i] = level;
        cause[i] = Some(why);
        pending.extend(
            callers[i]
                .iter()
                .map(|&j| (j, Reach::Full, false, Cause::Calls(i))),
        );
        pending.extend(
            supers[i]
                .iter()
                .map(|&j| (j, Reach::Dispatch, false, Cause::ImplementedBy(i))),
        );
        if level == Reach::Full {
            pending.extend(
                subtypes[i]
                    .iter()
                    .map(|&j| (j, Reach::Full, false, Cause::Extends(i))),
            );
            if component[i] {
                let kinds = if kind[i] & GLOBAL != 0 {
                    u8::MAX
                } else {
                    kind[i]
                };
                let fresh = kinds & !started;
                let full = !std::mem::replace(&mut containers, true);
                started |= kinds;
                pending.extend(
                    (0..n)
                        .filter(|&j| match loads[j] {
                            _ if !context[j] => false,
                            None => full,
                            Some(slice) => slice & fresh != 0,
                        })
                        .map(|j| (j, Reach::Full, false, Cause::Loads(i))),
                );
            }
        }
    }
    // A class that another resource names may run wherever the resource is read, like a
    // changed resource. Service files for project types are followed through their supertypes.
    for (resource, names) in resources {
        let service = resource
            .rsplit_once("META-INF/services/")
            .is_some_and(|(_, service)| lookup(&service.replace('.', "/")).next().is_some());
        if service || spring(&resource) {
            continue;
        }
        if let Some(name) = names
            .iter()
            .find(|name| lookup(name).any(|j| reach[j] == Reach::Full))
        {
            return Ok(Impact::Fallback(format!(
                "{resource} names {}, which reaches the change",
                name.replace('/', ".")
            )));
        }
    }
    let mut selected = BTreeSet::new();
    let mut unselected = BTreeSet::new();
    let mut reasons = BTreeMap::new();
    for (i, class) in classes.iter().enumerate() {
        if class.test && !class.annotation && !class.name.contains('$') {
            let name = class.name.replace('/', ".");
            if reach[i] == Reach::Full {
                reasons
                    .entry(name.clone())
                    .or_insert_with(|| explain(classes, &cause, i));
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
        reasons,
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
            without_reasons(affected(&self.classes, &changed, self.temp.path()).unwrap())
        }
    }

    fn without_reasons(mut impact: Impact) -> Impact {
        if let Impact::Tests { reasons, .. } = &mut impact {
            reasons.clear();
        }
        impact
    }

    fn tests(selected: &[&str], unselected: &[&str]) -> Impact {
        Impact::Tests {
            selected: selected.iter().map(|s| s.to_string()).collect(),
            unselected: unselected.iter().map(|s| s.to_string()).collect(),
            reasons: BTreeMap::new(),
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

    fn annotated(mut class: Class, name: &str, elements: &[(&str, &[&str])]) -> Class {
        class.refs.insert(name.into());
        class.annotations.push(Annotation {
            name: name.into(),
            elements: elements
                .iter()
                .map(|(e, c)| (e.to_string(), c.iter().map(|c| c.to_string()).collect()))
                .collect(),
        });
        class
    }

    #[test]
    fn reached_components_reach_only_slices_scanning_them() {
        let boot_test = "org/springframework/boot/test/context/SpringBootTest";
        let web_test = "org/springframework/boot/webmvc/test/autoconfigure/WebMvcTest";
        let mongo_test = "org/springframework/boot/data/mongodb/test/autoconfigure/DataMongoTest";
        let classes = vec![
            annotated(
                class("a/App", "a/App.java", false, &[], &[]),
                "org/springframework/boot/autoconfigure/SpringBootApplication",
                &[],
            ),
            annotated(
                class("a/Service", "a/Service.java", false, &["a/Repo"], &[]),
                "org/springframework/stereotype/Service",
                &[],
            ),
            class(
                "a/Repo",
                "a/Repo.java",
                false,
                &["org/springframework/data/repository/Repository"],
                &["org/springframework/data/repository/Repository"],
            ),
            annotated(
                class("a/Api", "a/Api.java", false, &["a/Service"], &[]),
                "org/springframework/web/bind/annotation/RestController",
                &[],
            ),
            annotated(
                class("a/Other", "a/Other.java", false, &[], &[]),
                "org/springframework/web/bind/annotation/RestController",
                &[],
            ),
            annotated(
                class("a/Errors", "a/Errors.java", false, &[], &[]),
                "org/springframework/web/bind/annotation/RestControllerAdvice",
                &[],
            ),
            annotated(
                class("a/Setup", "a/Setup.java", false, &[], &[]),
                "org/springframework/context/annotation/Configuration",
                &[],
            ),
            annotated(
                class("a/AppIT", "a/AppIT.java", true, &[], &[]),
                boot_test,
                &[],
            ),
            annotated(
                class("a/RepoTest", "a/RepoTest.java", true, &["a/Repo"], &[]),
                mongo_test,
                &[],
            ),
            annotated(
                class("a/OtherTest", "a/OtherTest.java", true, &["a/Other"], &[]),
                web_test,
                &[("value", &["a/Other"])],
            ),
            annotated(
                class("a/WebTest", "a/WebTest.java", true, &[], &[]),
                web_test,
                &[],
            ),
            // Nested classes inherit the enclosing slice.
            class(
                "a/OtherTest$Case",
                "a/OtherTest.java",
                true,
                &["org/springframework/test/context/bean/override/mockito/MockitoBean"],
                &[],
            ),
            // Custom include filters may scan anything.
            annotated(
                class("a/FilteredTest", "a/FilteredTest.java", true, &[], &[]),
                mongo_test,
                &[("includeFilters", &[])],
            ),
        ];
        let graph = Graph::new(classes);
        let all = [
            "a.AppIT",
            "a.FilteredTest",
            "a.OtherTest",
            "a.RepoTest",
            "a.WebTest",
        ];
        let split = |selected: &[&str]| {
            let unselected: Vec<&str> = all
                .iter()
                .copied()
                .filter(|t| !selected.contains(t))
                .collect();
            tests(selected, &unselected)
        };
        // Api scans in the open web slice only; the other lists its controller.
        assert_eq!(
            graph.impact(&["src/main/java/a/Service.java"]),
            split(&["a.AppIT", "a.FilteredTest", "a.WebTest"])
        );
        assert_eq!(
            graph.impact(&["src/main/java/a/Repo.java"]),
            split(&["a.AppIT", "a.FilteredTest", "a.RepoTest", "a.WebTest"])
        );
        assert_eq!(
            graph.impact(&["src/main/java/a/Errors.java"]),
            split(&["a.AppIT", "a.FilteredTest", "a.OtherTest", "a.WebTest"])
        );
        // Slices skip scanned configuration; the application joins every context.
        assert_eq!(
            graph.impact(&["src/main/java/a/Setup.java"]),
            split(&["a.AppIT", "a.FilteredTest"])
        );
        assert_eq!(graph.impact(&["src/main/java/a/App.java"]), split(&all));
    }

    #[test]
    fn a_component_scan_on_the_application_disables_slices() {
        let classes = vec![
            annotated(
                annotated(
                    class("a/App", "a/App.java", false, &[], &[]),
                    "org/springframework/boot/autoconfigure/SpringBootApplication",
                    &[],
                ),
                "org/springframework/context/annotation/ComponentScan",
                &[],
            ),
            annotated(
                class("a/Setup", "a/Setup.java", false, &[], &[]),
                "org/springframework/context/annotation/Configuration",
                &[],
            ),
            annotated(
                class("a/RepoTest", "a/RepoTest.java", true, &[], &[]),
                "org/springframework/boot/data/mongodb/test/autoconfigure/DataMongoTest",
                &[],
            ),
        ];
        assert_eq!(
            Graph::new(classes).impact(&["src/main/java/a/Setup.java"]),
            tests(&["a.RepoTest"], &[])
        );
    }

    #[test]
    fn annotations_record_listed_classes() {
        // @T(value = {A.class, B.class}, flag = true), built from the class file layout.
        let texts = ["La/T;", "value", "La/A;", "La/B;", "flag"];
        let text = |i: u16| -> Result<&str> { Ok(texts[i as usize - 1]) };
        let body = [
            0, 1, 0, 2, 0, 2, b'[', 0, 2, b'c', 0, 3, b'c', 0, 4, 0, 5, b'Z', 0, 1,
        ];
        let parsed = annotation(
            &mut Reader {
                bytes: &body,
                at: 0,
            },
            &text,
        )
        .unwrap();
        assert_eq!(parsed.name, "a/T");
        assert_eq!(parsed.elements["value"], ["a/A", "a/B"]);
        assert!(parsed.elements["flag"].is_empty());
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
        let names = identifiers("x(Limits.MAX); MAXIMUM; $MIN");
        assert!(names.contains("MAX") && names.contains("Limits"));
        assert!(!names.contains("MIN"));
    }

    #[test]
    fn chained_constants_reach_the_sources_naming_them() {
        // Before JDK 21, javac leaves no reference to a copied constant's owner.
        let mut classes = vec![
            class("a/Limits", "a/Limits.java", false, &[], &[]),
            class("a/Derived", "a/Derived.java", false, &[], &[]),
            class("t/DerivedTest", "t/DerivedTest.java", true, &[], &[]),
            class("t/OtherTest", "t/OtherTest.java", true, &[], &[]),
        ];
        classes[0].constants = vec!["MAX".into()];
        classes[1].constants = vec!["DOUBLE".into()];
        let graph = Graph::new(classes);
        graph.source(
            "src/main/java/a/Limits.java",
            "class Limits { static final int MAX = 3; }",
        );
        graph.source(
            "src/main/java/a/Derived.java",
            "class Derived { static final int DOUBLE = Limits.MAX * 2; }",
        );
        graph.source(
            "src/test/java/t/DerivedTest.java",
            "int x = Derived.DOUBLE;",
        );
        graph.source("src/test/java/t/OtherTest.java", "int y = 1;");
        assert_eq!(
            graph.impact(&["src/main/java/a/Limits.java"]),
            tests(&["t.DerivedTest"], &["t.OtherTest"])
        );
    }

    #[test]
    fn resources_naming_reached_classes_keep_the_module_selection() {
        let graph = Graph::new(vec![
            class("a/Api", "a/Api.java", false, &[], &[]),
            class("a/Impl", "a/Impl.java", false, &[], &["a/Api"]),
            class("t/ApiTest", "t/ApiTest.java", true, &["a/Api"], &[]),
            class("t/Ext", "t/Ext.java", true, &[], &[]),
            class("t/PlainTest", "t/PlainTest.java", true, &[], &[]),
        ]);
        // Service files of project types are followed through the service type.
        graph.source("src/main/resources/META-INF/services/a.Api", "a.Impl\n");
        assert_eq!(
            graph.impact(&["src/main/java/a/Impl.java"]),
            tests(&["t.ApiTest"], &["t.Ext", "t.PlainTest"])
        );
        // A JUnit extension registered for autodetection runs around every test.
        graph.source(
            "src/test/resources/META-INF/services/org.junit.jupiter.api.extension.Extension",
            "t.Ext\n",
        );
        assert!(matches!(
            graph.impact(&["src/test/java/t/Ext.java"]),
            Impact::Fallback(reason) if reason.contains("extension.Extension names t.Ext")
        ));
        // Resource folders may be called `build`.
        fs::remove_dir_all(graph.temp.path().join("src/test/resources")).unwrap();
        graph.source(
            "src/test/resources/build/logback-test.xml",
            "<appender class=\"t.Ext\"/>",
        );
        assert!(matches!(
            graph.impact(&["src/test/java/t/Ext.java"]),
            Impact::Fallback(reason) if reason.contains("logback-test.xml names t.Ext")
        ));
        // Build output is not a resource.
        fs::remove_dir_all(graph.temp.path().join("src/test/resources")).unwrap();
        graph.source("target/classes/copy.txt", "t.Ext");
        assert_eq!(
            graph.impact(&["src/test/java/t/Ext.java"]),
            tests(&["t.Ext"], &["t.ApiTest", "t.PlainTest"])
        );
    }

    #[test]
    fn spring_bootstrap_files_make_the_classes_they_name_global_components() {
        let web_test = "org/springframework/boot/webmvc/test/autoconfigure/WebMvcTest";
        let graph = Graph::new(vec![
            class(
                "a/AutoConfig",
                "a/AutoConfig.java",
                false,
                &["a/Helper"],
                &[],
            ),
            class("a/Helper", "a/Helper.java", false, &[], &[]),
            annotated(
                class("t/AppIT", "t/AppIT.java", true, &[], &[]),
                "org/springframework/boot/test/context/SpringBootTest",
                &[],
            ),
            annotated(
                class("t/WebTest", "t/WebTest.java", true, &[], &[]),
                web_test,
                &[],
            ),
            class("t/PlainTest", "t/PlainTest.java", true, &[], &[]),
        ]);
        graph.source(
            "src/main/resources/META-INF/spring/org.springframework.boot.autoconfigure.AutoConfiguration.imports",
            "a.AutoConfig\n",
        );
        assert_eq!(
            graph.impact(&["src/main/java/a/Helper.java"]),
            tests(&["t.AppIT", "t.WebTest"], &["t.PlainTest"])
        );
    }

    #[test]
    fn declared_packages_are_read_past_comments_and_file_annotations() {
        for (text, expected) in [
            ("package a.b;\nclass A {}", Some("a/b")),
            ("\u{feff}// c\n/* p\n*/ package a.b\n", Some("a/b")),
            (
                "@file:JvmName(\"Names\")\n@file:Suppress(\"x\")\npackage `a`.b",
                Some("a/b"),
            ),
            ("import a.B;\nclass C {}", Some("")),
            ("packaged", None),
            ("/* unterminated", None),
        ] {
            assert_eq!(declared_package(text).as_deref(), expected, "{text}");
        }
    }

    /// Compiles `src/main/java` and `src/test/java` below `workspace` like a build tool would,
    /// or returns `None` without `javac`.
    fn compile(workspace: &Path) -> Option<Vec<Class>> {
        let sources = |dir: &str| {
            text_files(&workspace.join(dir), true, &|p| p.ends_with(".java"))
                .unwrap()
                .into_iter()
                .map(|(path, _)| workspace.join(dir).join(path))
                .collect::<Vec<_>>()
        };
        let classes = workspace.join("target/classes");
        let tests = workspace.join("target/test-classes");
        for (dir, out) in [("src/main/java", &classes), ("src/test/java", &tests)] {
            let status = std::process::Command::new("javac")
                .arg("-d")
                .arg(out)
                .arg("-cp")
                .arg(&classes)
                .args(sources(dir))
                .output()
                .ok()?;
            assert!(status.status.success(), "{status:?}");
        }
        Some(load(&[(classes, false), (tests, true)]).unwrap())
    }

    #[test]
    fn compiled_projects_reveal_indirect_references() {
        let graph = Graph::new(Vec::new());
        for (path, text) in [
            (
                "src/main/java/a/Factory.java",
                "package a; public class Factory { public static int[] cases() { return new int[] {1}; } }",
            ),
            (
                "src/main/java/a/Masking.java",
                "package a; public class Masking { public String mask(String s) { return s; } }",
            ),
            ("src/main/java/a/Quiet.java", "package a; public class Quiet {}"),
            // javac accepts a source whose directory differs from its package.
            (
                "src/main/java/misplaced/Moved.java",
                "/* moved */\npackage a;\npublic class Moved { public int value() { return 1; } }",
            ),
            (
                "src/main/resources/logback.xml",
                "<configuration><conversionRule conversionWord=\"mask\" converterClass=\"a.Masking\"/>\
                 <logger name=\"a.Quiet\" level=\"DEBUG\"/></configuration>",
            ),
            ("src/test/java/t/Src.java", "package t; public @interface Src { String value(); }"),
            (
                "src/test/java/t/ParamTest.java",
                "package t; public class ParamTest { @Src(\"a.Factory#cases\") void cases() {} }",
            ),
            (
                "src/test/java/t/MovedTest.java",
                "package t; public class MovedTest { int v() { return new a.Moved().value(); } }",
            ),
            (
                "src/test/java/t/QuietTest.java",
                "package t; public class QuietTest { Object quiet = new a.Quiet(); }",
            ),
            ("src/test/java/t/PlainTest.java", "package t; public class PlainTest {}"),
            // An architecture test analyzes the packages it names, without references.
            (
                "src/test/java/com/tngtech/archunit/junit/AnalyzeClasses.java",
                "package com.tngtech.archunit.junit; public @interface AnalyzeClasses { String[] packages(); }",
            ),
            (
                "src/test/java/t/ArchTest.java",
                "package t; @com.tngtech.archunit.junit.AnalyzeClasses(packages = \"a\") public class ArchTest {}",
            ),
        ] {
            graph.source(path, text);
        }
        let Some(classes) = compile(graph.temp.path()) else {
            return;
        };
        let all = [
            "t.ArchTest",
            "t.MovedTest",
            "t.ParamTest",
            "t.PlainTest",
            "t.QuietTest",
        ];
        let explained = |path: &str| {
            affected(
                &classes,
                &BTreeSet::from([path.to_owned()]),
                graph.temp.path(),
            )
            .unwrap()
        };
        let impact = |path: &str| without_reasons(explained(path));
        let split = |selected: &[&str]| {
            let unselected: Vec<&str> = all
                .iter()
                .copied()
                .filter(|t| !selected.contains(t))
                .collect();
            tests(selected, &unselected)
        };
        // `Class#member` strings, as in `@MethodSource`, name the class.
        assert_eq!(
            impact("src/main/java/a/Factory.java"),
            split(&["t.ArchTest", "t.ParamTest"])
        );
        assert_eq!(
            impact("src/main/java/misplaced/Moved.java"),
            split(&["t.ArchTest", "t.MovedTest"])
        );
        // A logger name does not load the class; a converter class does.
        assert_eq!(
            impact("src/main/java/a/Quiet.java"),
            split(&["t.ArchTest", "t.QuietTest"])
        );
        assert!(matches!(
            impact("src/main/java/a/Masking.java"),
            Impact::Fallback(reason) if reason.contains("logback.xml names a.Masking")
        ));
        assert_eq!(
            impact("src/test/java/t/PlainTest.java"),
            split(&["t.ArchTest", "t.PlainTest"])
        );
        // Each selected test says how it reaches the change.
        let Impact::Tests { reasons, .. } = explained("src/main/java/a/Factory.java") else {
            panic!("no selection");
        };
        assert_eq!(
            reasons,
            BTreeMap::from([
                (
                    "t.ArchTest".into(),
                    "t.ArchTest → com.tngtech.archunit.junit.AnalyzeClasses (scans for classes)"
                        .into()
                ),
                (
                    "t.ParamTest".into(),
                    "t.ParamTest → a.Factory (changed)".into()
                ),
            ])
        );
    }

    #[test]
    fn reasons_trace_dispatch_contexts_and_constants() {
        let spring = "org/springframework/boot/test/context/SpringBootTest";
        let mut classes = vec![
            class("a/Api", "a/Api.java", false, &[], &[]),
            class("a/Impl", "a/Impl.java", false, &[], &["a/Api"]),
            class(
                "a/Service",
                "a/Service.java",
                false,
                &["La/Api;", "org/springframework/stereotype/Service"],
                &[],
            ),
            class(
                "t/ServiceTest",
                "t/ServiceTest.java",
                true,
                &["a/Service"],
                &[],
            ),
            class("t/AppIT", "t/AppIT.java", true, &[spring], &[]),
            class("a/Limits", "a/Limits.java", false, &[], &[]),
            class("t/LimitsTest", "t/LimitsTest.java", true, &[], &[]),
        ];
        classes[5].constants = vec!["MAX".into()];
        let graph = Graph::new(classes);
        for source in [
            "main/java/a/Api",
            "main/java/a/Service",
            "test/java/t/ServiceTest",
        ] {
            graph.source(&format!("src/{source}.java"), "class Plain {}");
        }
        graph.source("src/test/java/t/AppIT.java", "class Plain {}");
        graph.source("src/test/java/t/LimitsTest.java", "x(Limits.MAX);");
        let changed = BTreeSet::from([
            "src/main/java/a/Impl.java".to_owned(),
            "src/main/java/a/Limits.java".to_owned(),
        ]);
        for path in &changed {
            graph.source(path, "");
        }
        let Impact::Tests { reasons, .. } =
            affected(&graph.classes, &changed, graph.temp.path()).unwrap()
        else {
            panic!("no selection");
        };
        assert_eq!(
            reasons,
            BTreeMap::from([
                (
                    "t.AppIT".into(),
                    "t.AppIT (context) loads a.Service → a.Api, implemented by a.Impl (changed)"
                        .into()
                ),
                (
                    "t.LimitsTest".into(),
                    "t.LimitsTest copies a constant of a.Limits (changed)".into()
                ),
                (
                    "t.ServiceTest".into(),
                    "t.ServiceTest → a.Service → a.Api, implemented by a.Impl (changed)".into()
                ),
            ])
        );
    }
}
