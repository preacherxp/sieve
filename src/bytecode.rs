//! Change-insensitive digests of compiled classes for test records: a hash per method body
//! and one for the class shape. Both resolve constant-pool references to their values and
//! ignore debug attributes, so comment edits and constant-pool reordering change nothing.
use crate::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// FNV-1a, 64 bits: stable across platforms and releases, unlike `DefaultHasher`.
#[derive(Clone)]
pub struct Hasher(u64);

impl Default for Hasher {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Hasher {
    pub fn bytes(&mut self, bytes: &[u8]) -> &mut Self {
        for &b in bytes {
            self.0 = (self.0 ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
        self
    }

    /// Length-prefixed, so consecutive fields cannot run into each other.
    pub fn field(&mut self, bytes: &[u8]) -> &mut Self {
        self.bytes(&(bytes.len() as u64).to_le_bytes()).bytes(bytes)
    }

    pub fn finish(&self) -> String {
        format!("{:016x}", self.0)
    }
}

pub fn hash(bytes: &[u8]) -> String {
    Hasher::default().bytes(bytes).finish()
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Digest {
    /// Internal name, such as `example/Calculator$Inner`.
    pub name: String,
    /// Superclass and interfaces.
    pub supers: Vec<String>,
    /// Access flags, supertypes, fields, member signatures and annotations, class
    /// attributes, and the static initializer, whose effects outlive the test that ran it.
    pub shape: String,
    /// What dependency-injection containers read: class annotations and supertypes, the
    /// constructors, and every annotated member.
    pub wiring: String,
    /// Body hash per method, keyed `name(descriptor)`.
    pub methods: BTreeMap<String, String>,
    /// The shape without private methods that carry no annotation: only code of the class
    /// itself calls them, and that code's hashes cover them.
    pub api: String,
    /// The wiring split into its parts: the class's access, supertypes and annotations; its
    /// constructors; and each annotated member.
    pub declared: String,
    pub injection: String,
    pub wired: BTreeMap<String, Wired>,
    /// Constructors and the static initializer whose code only stores values into the
    /// class's own fields: their effects reach a test only through code of the class.
    pub plain: BTreeSet<String>,
    /// Request-mapping path patterns per handler method, including the class's prefix.
    pub routes: BTreeMap<String, Vec<String>>,
}

/// An annotated member: the hash of its annotations and their types.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wired {
    pub hash: String,
    pub annotations: Vec<String>,
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

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
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

enum Entry {
    Empty,
    Utf8(Vec<u8>),
    /// A numeric constant: its tag and raw bytes.
    Number(u8, Vec<u8>),
    /// A tag with the indices it refers to (class, string, member, name-and-type, ...).
    Refs(u8, Vec<u16>),
    /// Method handle: reference kind and member index.
    Handle(u8, u16),
}

struct Pool {
    entries: Vec<Entry>,
    /// `BootstrapMethods`: each method handle with its static arguments.
    bootstrap: Vec<Vec<u16>>,
}

impl Pool {
    fn read(r: &mut Reader) -> Result<Self> {
        let count = r.u16()? as usize;
        let mut entries = Vec::with_capacity(count);
        entries.push(Entry::Empty);
        while entries.len() < count {
            let tag = r.u8()?;
            let entry = match tag {
                1 => {
                    let len = r.u16()? as usize;
                    Entry::Utf8(r.take(len)?.to_vec())
                }
                3 | 4 => Entry::Number(tag, r.take(4)?.to_vec()),
                5 | 6 => Entry::Number(tag, r.take(8)?.to_vec()),
                7 | 8 | 16 | 19 | 20 => Entry::Refs(tag, vec![r.u16()?]),
                9..=12 | 17 | 18 => Entry::Refs(tag, vec![r.u16()?, r.u16()?]),
                15 => Entry::Handle(r.u8()?, r.u16()?),
                tag => return Err(format!("Unknown constant pool tag {tag}").into()),
            };
            let wide = matches!(entry, Entry::Number(5 | 6, _));
            entries.push(entry);
            if wide {
                entries.push(Entry::Empty);
            }
        }
        Ok(Self {
            entries,
            bootstrap: Vec::new(),
        })
    }

    fn get(&self, index: u16) -> Result<&Entry> {
        match self.entries.get(index as usize) {
            Some(Entry::Empty) | None => Err("Invalid constant pool index".into()),
            Some(entry) => Ok(entry),
        }
    }

    fn text(&self, index: u16) -> Result<String> {
        match self.get(index)? {
            Entry::Utf8(bytes) => Ok(String::from_utf8_lossy(bytes).into_owned()),
            _ => Err("Expected a UTF-8 constant".into()),
        }
    }

    fn class(&self, index: u16) -> Result<String> {
        match self.get(index)? {
            Entry::Refs(7, refs) => self.text(refs[0]),
            _ => Err("Expected a class constant".into()),
        }
    }

    /// Feeds the entry's value, with the values it refers to, into `h`. Dynamic constants
    /// resolve their bootstrap method and its arguments, such as a lambda's target.
    fn resolve(&self, index: u16, h: &mut Hasher, depth: u8) -> Result<()> {
        if depth > 8 {
            return Err("Constant pool cycle".into());
        }
        match self.get(index)? {
            Entry::Empty => unreachable!(),
            Entry::Utf8(bytes) => {
                h.bytes(&[1]).field(bytes);
            }
            Entry::Number(tag, bytes) => {
                h.bytes(&[*tag]).field(bytes);
            }
            Entry::Refs(tag, refs) => {
                h.bytes(&[*tag]);
                let dynamic = matches!(tag, 17 | 18);
                for (i, &r) in refs.iter().enumerate() {
                    if dynamic && i == 0 {
                        let entry = self
                            .bootstrap
                            .get(r as usize)
                            .ok_or("Invalid bootstrap index")?;
                        h.bytes(&(entry.len() as u16).to_be_bytes());
                        for &arg in entry {
                            self.resolve(arg, h, depth + 1)?;
                        }
                    } else {
                        self.resolve(r, h, depth + 1)?;
                    }
                }
            }
            Entry::Handle(kind, member) => {
                h.bytes(&[15, *kind]);
                self.resolve(*member, h, depth + 1)?;
            }
        }
        Ok(())
    }
}

/// Instruction operand layout: `(operand bytes, constant-pool index width)`; `None` marks
/// the variable-length switches and `wide`.
fn operands(opcode: u8) -> Result<Option<(usize, usize)>> {
    Ok(Some(match opcode {
        0x12 => (1, 1),
        0x13 | 0x14 | 0xb2..=0xb8 | 0xbb | 0xbd | 0xc0 | 0xc1 => (2, 2),
        0xb9 | 0xba => (4, 2),
        0xc5 => (3, 2),
        0x10 | 0x15..=0x19 | 0x36..=0x3a | 0xa9 | 0xbc => (1, 0),
        0x11 | 0x84 | 0x99..=0xa8 | 0xc6 | 0xc7 => (2, 0),
        0xc8 | 0xc9 => (4, 0),
        0xaa | 0xab | 0xc4 => return Ok(None),
        0x00..=0x0f | 0x1a..=0x35 | 0x3b..=0x83 | 0x85..=0x98 | 0xac..=0xb1 | 0xbe | 0xbf => (0, 0),
        0xc2 | 0xc3 => (0, 0),
        opcode => return Err(format!("Unknown opcode {opcode:#x}").into()),
    }))
}

/// Hashes a `Code` attribute with constant-pool operands resolved; debug attributes, stack
/// maps, and type annotations are left out.
fn code(pool: &Pool, body: &[u8], h: &mut Hasher) -> Result<()> {
    let mut r = Reader { bytes: body, at: 0 };
    h.bytes(r.take(4)?);
    let len = r.u32()? as usize;
    let code = r.take(len)?;
    let mut c = Reader { bytes: code, at: 0 };
    while c.at < code.len() {
        let pc = c.at;
        let opcode = c.u8()?;
        h.bytes(&[opcode]);
        match operands(opcode)? {
            Some((size, 0)) => {
                h.bytes(c.take(size)?);
            }
            Some((size, width)) => {
                let index = if width == 1 { c.u8()? as u16 } else { c.u16()? };
                pool.resolve(index, h, 0)?;
                h.bytes(c.take(size - width)?);
            }
            None if opcode == 0xc4 => {
                let inner = c.u8()?;
                h.bytes(&[inner])
                    .bytes(c.take(if inner == 0x84 { 4 } else { 2 })?);
            }
            None => {
                c.take((4 - (pc + 1) % 4) % 4)?;
                let default = c.take(4)?;
                h.bytes(default);
                let count = if opcode == 0xaa {
                    let low = c.u32()? as i32;
                    let high = c.u32()? as i32;
                    h.bytes(&low.to_be_bytes()).bytes(&high.to_be_bytes());
                    (high as i64 - low as i64 + 1) * 4
                } else {
                    let pairs = c.u32()?;
                    h.bytes(&pairs.to_be_bytes());
                    pairs as i64 * 8
                };
                let count = usize::try_from(count).map_err(|_| "Invalid switch")?;
                h.bytes(c.take(count)?);
            }
        }
    }
    for _ in 0..r.u16()? {
        h.bytes(r.take(6)?);
        let catch = r.u16()?;
        if catch == 0 {
            h.bytes(&[0]);
        } else {
            pool.resolve(catch, h, 0)?;
        }
    }
    Ok(())
}

fn element_value(pool: &Pool, r: &mut Reader, h: &mut Hasher) -> Result<()> {
    let tag = r.u8()?;
    h.bytes(&[tag]);
    match tag {
        b'B' | b'C' | b'D' | b'F' | b'I' | b'J' | b'S' | b'Z' | b's' | b'c' => {
            pool.resolve(r.u16()?, h, 0)?;
        }
        b'e' => {
            pool.resolve(r.u16()?, h, 0)?;
            pool.resolve(r.u16()?, h, 0)?;
        }
        b'@' => annotation(pool, r, h)?,
        b'[' => {
            let count = r.u16()?;
            h.bytes(&count.to_be_bytes());
            for _ in 0..count {
                element_value(pool, r, h)?;
            }
        }
        tag => return Err(format!("Unknown element value tag {tag}").into()),
    }
    Ok(())
}

fn annotation(pool: &Pool, r: &mut Reader, h: &mut Hasher) -> Result<()> {
    pool.resolve(r.u16()?, h, 0)?;
    let count = r.u16()?;
    h.bytes(&count.to_be_bytes());
    for _ in 0..count {
        pool.resolve(r.u16()?, h, 0)?;
        element_value(pool, r, h)?;
    }
    Ok(())
}

/// Hashes an attribute of a class, field, or method with its constant-pool references
/// resolved. Unknown attributes are hashed as raw bytes, which only errs towards change.
fn attribute(pool: &Pool, name: &str, body: &[u8], h: &mut Hasher) -> Result<()> {
    let mut r = Reader { bytes: body, at: 0 };
    h.field(name.as_bytes());
    match name {
        "RuntimeVisibleAnnotations" | "RuntimeInvisibleAnnotations" => {
            for _ in 0..r.u16()? {
                // Kotlin keeps the source map, which line shifts change, in an annotation.
                let at = r.at;
                let kind = pool.text(Reader { bytes: body, at }.u16()?)?;
                if kind == "Lkotlin/jvm/internal/SourceDebugExtension;" {
                    annotation(pool, &mut r, &mut Hasher::default())?;
                } else {
                    annotation(pool, &mut r, h)?;
                }
            }
        }
        "RuntimeVisibleParameterAnnotations" | "RuntimeInvisibleParameterAnnotations" => {
            for _ in 0..r.u8()? {
                let count = r.u16()?;
                h.bytes(&count.to_be_bytes());
                for _ in 0..count {
                    annotation(pool, &mut r, h)?;
                }
            }
        }
        "RuntimeVisibleTypeAnnotations" | "RuntimeInvisibleTypeAnnotations" => {
            for _ in 0..r.u16()? {
                // The target and type path hold no constant-pool references.
                let target = r.u8()?;
                let info = match target {
                    0x00 | 0x01 | 0x16 => 1,
                    0x10 | 0x11 | 0x12 | 0x17 | 0x42..=0x46 => 2,
                    0x13..=0x15 => 0,
                    0x47..=0x4b => 3,
                    0x40 | 0x41 => {
                        let len = r.u16()?;
                        h.bytes(&len.to_be_bytes());
                        len as usize * 6
                    }
                    target => {
                        return Err(format!("Unknown type annotation target {target:#x}").into())
                    }
                };
                h.bytes(&[target]).bytes(r.take(info)?);
                let path = r.u8()?;
                h.bytes(&[path]).bytes(r.take(path as usize * 2)?);
                annotation(pool, &mut r, h)?;
            }
        }
        "AnnotationDefault" => element_value(pool, &mut r, h)?,
        "ConstantValue" | "Signature" | "NestHost" => pool.resolve(r.u16()?, h, 0)?,
        "Exceptions" | "PermittedSubclasses" => {
            for _ in 0..r.u16()? {
                pool.resolve(r.u16()?, h, 0)?;
            }
        }
        "EnclosingMethod" => {
            pool.resolve(r.u16()?, h, 0)?;
            let method = r.u16()?;
            if method != 0 {
                pool.resolve(method, h, 0)?;
            }
        }
        "MethodParameters" => {
            for _ in 0..r.u8()? {
                let name = r.u16()?;
                if name != 0 {
                    pool.resolve(name, h, 0)?;
                }
                h.bytes(r.take(2)?);
            }
        }
        "Record" => {
            for _ in 0..r.u16()? {
                pool.resolve(r.u16()?, h, 0)?;
                pool.resolve(r.u16()?, h, 0)?;
                let list = read_attributes(pool, &mut r)?;
                attributes(pool, &list, h, None)?;
            }
        }
        _ => {
            h.field(body);
        }
    }
    Ok(())
}

/// Debug information, nesting metadata that an added anonymous class would change, and
/// bootstrap methods, which code hashes resolve where they are used.
const SKIPPED: &[&str] = &[
    "SourceFile",
    "SourceDebugExtension",
    "LineNumberTable",
    "LocalVariableTable",
    "LocalVariableTypeTable",
    "InnerClasses",
    "NestMembers",
    "BootstrapMethods",
    "Deprecated",
];

const ANNOTATIONS: &[&str] = &[
    "RuntimeVisibleAnnotations",
    "RuntimeInvisibleAnnotations",
    "RuntimeVisibleParameterAnnotations",
    "RuntimeInvisibleParameterAnnotations",
    "RuntimeVisibleTypeAnnotations",
    "RuntimeInvisibleTypeAnnotations",
];

const ACC_SYNTHETIC: u16 = 0x1000;
const ACC_PRIVATE: u16 = 0x0002;

type Attributes<'a> = Vec<(String, &'a [u8])>;

fn read_attributes<'a>(pool: &Pool, r: &mut Reader<'a>) -> Result<Attributes<'a>> {
    let mut out = Vec::new();
    for _ in 0..r.u16()? {
        let name = pool.text(r.u16()?)?;
        let len = r.u32()? as usize;
        out.push((name, r.take(len)?));
    }
    Ok(out)
}

/// Hashes the attributes other than `Code` and [`SKIPPED`], or with `only`, just those.
fn attributes(pool: &Pool, list: &Attributes, h: &mut Hasher, only: Option<&[&str]>) -> Result<()> {
    for (name, body) in list {
        let wanted = match only {
            Some(only) => only.contains(&name.as_str()),
            None => name != "Code" && !SKIPPED.contains(&name.as_str()),
        };
        if wanted {
            attribute(pool, name, body, h)?;
        }
    }
    Ok(())
}

struct Member<'a> {
    access: u16,
    name: String,
    descriptor: String,
    attributes: Attributes<'a>,
}

fn members<'a>(pool: &Pool, r: &mut Reader<'a>) -> Result<Vec<Member<'a>>> {
    let mut out = Vec::new();
    for _ in 0..r.u16()? {
        let access = r.u16()?;
        let name = pool.text(r.u16()?)?;
        let descriptor = pool.text(r.u16()?)?;
        let attributes = read_attributes(pool, r)?;
        out.push(Member {
            access,
            name,
            descriptor,
            attributes,
        });
    }
    Ok(out)
}

/// An annotation's type and the strings each element lists, such as a mapping's paths.
type Info = (String, BTreeMap<String, Vec<String>>);

fn strings(pool: &Pool, r: &mut Reader, out: &mut Vec<String>) -> Result<()> {
    match r.u8()? {
        b's' => out.push(pool.text(r.u16()?)?),
        b'B' | b'C' | b'D' | b'F' | b'I' | b'J' | b'S' | b'Z' | b'c' => {
            r.u16()?;
        }
        b'e' => {
            r.take(4)?;
        }
        b'@' => {
            info(pool, r)?;
        }
        b'[' => {
            for _ in 0..r.u16()? {
                strings(pool, r, out)?;
            }
        }
        tag => return Err(format!("Unknown element value tag {tag}").into()),
    }
    Ok(())
}

fn info(pool: &Pool, r: &mut Reader) -> Result<Info> {
    let descriptor = pool.text(r.u16()?)?;
    let name = descriptor
        .strip_prefix('L')
        .and_then(|d| d.strip_suffix(';'))
        .unwrap_or(&descriptor)
        .to_owned();
    let mut elements = BTreeMap::new();
    for _ in 0..r.u16()? {
        let element = pool.text(r.u16()?)?;
        let mut values = Vec::new();
        strings(pool, r, &mut values)?;
        elements.insert(element, values);
    }
    Ok((name, elements))
}

/// The annotations on a class or member, including those on its parameters.
fn annotations(pool: &Pool, list: &Attributes) -> Result<Vec<Info>> {
    let mut out = Vec::new();
    for (name, body) in list {
        let mut r = Reader { bytes: body, at: 0 };
        match name.as_str() {
            "RuntimeVisibleAnnotations" | "RuntimeInvisibleAnnotations" => {
                for _ in 0..r.u16()? {
                    out.push(info(pool, &mut r)?);
                }
            }
            "RuntimeVisibleParameterAnnotations" | "RuntimeInvisibleParameterAnnotations" => {
                for _ in 0..r.u8()? {
                    for _ in 0..r.u16()? {
                        out.push(info(pool, &mut r)?);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

const MAPPINGS: &[&str] = &[
    "org/springframework/web/bind/annotation/RequestMapping",
    "org/springframework/web/bind/annotation/GetMapping",
    "org/springframework/web/bind/annotation/PostMapping",
    "org/springframework/web/bind/annotation/PutMapping",
    "org/springframework/web/bind/annotation/DeleteMapping",
    "org/springframework/web/bind/annotation/PatchMapping",
];

/// Whether an annotation maps requests to a handler.
pub fn mapping(annotation: &str) -> bool {
    MAPPINGS.contains(&annotation)
}

/// The paths a mapping annotation lists, or the empty path.
fn paths(annotations: &[Info]) -> Option<Vec<String>> {
    let (_, elements) = annotations.iter().find(|(name, _)| mapping(name))?;
    let mut out: Vec<String> = ["value", "path"]
        .iter()
        .flat_map(|e| elements.get(*e).into_iter().flatten().cloned())
        .collect();
    if out.is_empty() {
        out.push(String::new());
    }
    Some(out)
}

/// `prefix` and `path` joined into one normalized pattern, such as `/a/{id}`.
fn join(prefix: &str, path: &str) -> String {
    let segments: Vec<&str> = prefix
        .split('/')
        .chain(path.split('/'))
        .filter(|s| !s.is_empty())
        .collect();
    format!("/{}", segments.join("/"))
}

/// Immutable library value types, whose methods only compute values.
const VALUE_TYPES: &[&str] = &[
    "java/lang/String",
    "java/lang/StringBuilder",
    "java/lang/Integer",
    "java/lang/Long",
    "java/lang/Short",
    "java/lang/Byte",
    "java/lang/Character",
    "java/lang/Boolean",
    "java/lang/Double",
    "java/lang/Float",
    "java/lang/Math",
    "java/util/regex/Pattern",
    "java/util/UUID",
    "java/time/",
    "java/math/",
];

/// Library types whose static methods and constructors only build objects the caller owns.
/// Their instance methods may change objects the class did not create, such as an injected
/// registry, so only [`VALUE_TYPES`] take instance calls.
const FACTORY_TYPES: &[&str] = &[
    "java/lang/Enum",
    "java/util/Objects",
    "java/util/List",
    "java/util/Set",
    "java/util/Map",
    "java/util/Collections",
    "java/util/Arrays",
    "java/util/ArrayList",
    "java/util/LinkedList",
    "java/util/HashMap",
    "java/util/LinkedHashMap",
    "java/util/TreeMap",
    "java/util/HashSet",
    "java/util/LinkedHashSet",
    "java/util/TreeSet",
    "java/util/ArrayDeque",
    "java/util/EnumMap",
    "java/util/EnumSet",
    "java/util/Optional",
    "java/util/Comparator",
    "java/util/concurrent/ConcurrentHashMap",
    "java/util/concurrent/ConcurrentLinkedQueue",
    "java/util/concurrent/CopyOnWriteArrayList",
    "java/util/concurrent/atomic/",
    "kotlin/jvm/internal/Intrinsics",
    "kotlin/collections/",
    "kotlin/text/",
    "org/slf4j/LoggerFactory",
];

fn listed(owner: &str, list: &[&str]) -> bool {
    list.iter()
        .any(|p| owner == *p || p.ends_with('/') && owner.starts_with(p))
}

fn throwable(owner: &str) -> bool {
    owner.starts_with("java/lang/") && (owner.ends_with("Exception") || owner.ends_with("Error"))
}

/// The owner and name of the member an instruction refers to.
fn member(pool: &Pool, index: u16) -> Result<(String, String)> {
    let Entry::Refs(9..=11, refs) = pool.get(index)? else {
        return Err("Expected a member reference".into());
    };
    let Entry::Refs(12, nat) = pool.get(refs[1])? else {
        return Err("Expected a name and type".into());
    };
    Ok((pool.class(refs[0])?, pool.text(nat[0])?))
}

/// Whether a constructor or static initializer of `class` only stores values into the
/// class's own fields: it loads arguments and constants, calls its superclass's root
/// constructor or another of its own, creates JDK collections, and computes values with
/// [`VALUE_TYPES`]. Exceptions it throws stop the context from starting, which any test
/// loading the class shows.
fn plain(pool: &Pool, body: &[u8], class: &str, initializer: bool) -> Result<bool> {
    let mut r = Reader { bytes: body, at: 0 };
    r.take(4)?;
    let len = r.u32()? as usize;
    let code = r.take(len)?;
    let mut c = Reader { bytes: code, at: 0 };
    while c.at < code.len() {
        let pc = c.at;
        let opcode = c.u8()?;
        let ok = match opcode {
            // Fields of the class itself; any static field may be read.
            0xb2 => {
                c.u16()?;
                true
            }
            0xb3 => {
                let owner = member(pool, c.u16()?)?.0;
                initializer && owner == class
            }
            0xb4 | 0xb5 => member(pool, c.u16()?)?.0 == class,
            0xb6..=0xb9 => {
                let (owner, name) = member(pool, c.u16()?)?;
                if opcode == 0xb9 {
                    c.take(2)?;
                }
                let value = listed(&owner, VALUE_TYPES);
                let factory = value || listed(&owner, FACTORY_TYPES);
                let root = matches!(
                    owner.as_str(),
                    "java/lang/Object" | "java/lang/Record" | "java/lang/Enum"
                );
                match opcode {
                    0xb8 => factory,
                    0xb7 => {
                        name == "<init>" && (root || owner == class || factory || throwable(&owner))
                    }
                    // `getClass()` is how older compilers check for null.
                    _ => value || owner == "java/lang/Object" && name == "getClass",
                }
            }
            // String concatenation and lambdas capture values without running code.
            0xba => {
                c.take(4)?;
                true
            }
            0xbb => {
                let owner = pool.class(c.u16()?)?;
                listed(&owner, VALUE_TYPES) || listed(&owner, FACTORY_TYPES) || throwable(&owner)
            }
            0xc2 | 0xc3 => false,
            _ => {
                match operands(opcode)? {
                    Some((size, _)) => {
                        c.take(size)?;
                    }
                    None if opcode == 0xc4 => {
                        let inner = c.u8()?;
                        c.take(if inner == 0x84 { 4 } else { 2 })?;
                    }
                    None => {
                        c.take((4 - (pc + 1) % 4) % 4)?;
                        c.take(4)?;
                        let count = if opcode == 0xaa {
                            let low = c.u32()? as i32;
                            let high = c.u32()? as i32;
                            (high as i64 - low as i64 + 1) * 4
                        } else {
                            c.u32()? as i64 * 8
                        };
                        c.take(usize::try_from(count).map_err(|_| "Invalid switch")?)?;
                    }
                }
                true
            }
        };
        if !ok {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn digest(bytes: &[u8]) -> Result<Digest> {
    let mut r = Reader { bytes, at: 0 };
    if r.u32()? != 0xCAFE_BABE {
        return Err("Not a class file".into());
    }
    let version = r.take(4)?;
    let mut pool = Pool::read(&mut r)?;
    let access = r.take(2)?;
    let name = pool.class(r.u16()?)?;
    let mut supers = Vec::new();
    let superclass = r.u16()?;
    if superclass != 0 {
        supers.push(pool.class(superclass)?);
    }
    for _ in 0..r.u16()? {
        supers.push(pool.class(r.u16()?)?);
    }
    let fields = members(&pool, &mut r)?;
    let methods = members(&pool, &mut r)?;
    let class_attributes = read_attributes(&pool, &mut r)?;
    if let Some((_, body)) = class_attributes
        .iter()
        .find(|(n, _)| n == "BootstrapMethods")
    {
        let mut b = Reader { bytes: body, at: 0 };
        for _ in 0..b.u16()? {
            let mut entry = vec![b.u16()?];
            for _ in 0..b.u16()? {
                entry.push(b.u16()?);
            }
            pool.bootstrap.push(entry);
        }
    }
    let mut shape = Hasher::default();
    let mut wiring = Hasher::default();
    shape.bytes(version).bytes(access).field(name.as_bytes());
    wiring.bytes(access).field(name.as_bytes());
    for sup in &supers {
        shape.field(sup.as_bytes());
        wiring.field(sup.as_bytes());
    }
    attributes(&pool, &class_attributes, &mut shape, None)?;
    attributes(&pool, &class_attributes, &mut wiring, Some(ANNOTATIONS))?;
    let declared = wiring.finish();
    let mut api = shape.clone();
    let mut injection = Hasher::default();
    let mut wired = BTreeMap::new();
    let mut plains = BTreeSet::new();
    let mut routes = BTreeMap::new();
    let prefixes = paths(&annotations(&pool, &class_attributes)?).unwrap_or(vec![String::new()]);
    let mut hashes = BTreeMap::new();
    for (kind, list) in [(0u8, &fields), (1, &methods)] {
        for member in list.iter() {
            let key = format!("{}{}", member.name, member.descriptor);
            let body = member
                .attributes
                .iter()
                .find(|(n, _)| n == "Code")
                .map(|(_, b)| *b);
            let mut h = Hasher::default();
            if let Some(body) = body {
                code(&pool, body, &mut h)?;
            }
            let hash = h.finish();
            let annotated = member
                .attributes
                .iter()
                .any(|(n, _)| ANNOTATIONS.contains(&n.as_str()));
            // Synthetic members, such as lambda bodies, are reached only through code that
            // names them, and that code's hash covers them.
            if member.access & ACC_SYNTHETIC == 0 {
                let mut member_shape = Hasher::default();
                member_shape
                    .bytes(&[kind])
                    .bytes(&member.access.to_be_bytes())
                    .field(key.as_bytes());
                attributes(&pool, &member.attributes, &mut member_shape, None)?;
                let member_shape = member_shape.finish();
                shape.field(member_shape.as_bytes());
                if kind == 0 || member.access & ACC_PRIVATE == 0 || annotated {
                    api.field(member_shape.as_bytes());
                }
            }
            if annotated || member.name == "<init>" {
                let mut h = Hasher::default();
                h.bytes(&[kind])
                    .bytes(&member.access.to_be_bytes())
                    .field(key.as_bytes());
                attributes(&pool, &member.attributes, &mut h, Some(ANNOTATIONS))?;
                let h = h.finish();
                wiring.field(h.as_bytes());
                if member.name == "<init>" {
                    injection.field(h.as_bytes());
                } else {
                    let found = annotations(&pool, &member.attributes)?;
                    if kind == 1 {
                        if let Some(paths) = paths(&found) {
                            let patterns = prefixes
                                .iter()
                                .flat_map(|p| paths.iter().map(move |q| join(p, q)))
                                .collect();
                            routes.insert(key.clone(), patterns);
                        }
                    }
                    let mut names: Vec<String> = found.into_iter().map(|(n, _)| n).collect();
                    names.sort();
                    names.dedup();
                    wired.insert(
                        format!("{}{key}", if kind == 0 { "field " } else { "" }),
                        Wired {
                            hash: h,
                            annotations: names,
                        },
                    );
                }
            }
            if kind == 1 {
                let initializer = member.name == "<clinit>";
                if initializer {
                    shape.field(hash.as_bytes());
                    api.field(hash.as_bytes());
                }
                if let Some(body) = body.filter(|_| initializer || member.name == "<init>") {
                    if plain(&pool, body, &name, initializer)? {
                        plains.insert(key.clone());
                    }
                }
                hashes.insert(key, hash);
            }
        }
    }
    Ok(Digest {
        name,
        supers,
        shape: shape.finish(),
        wiring: wiring.finish(),
        methods: hashes,
        api: api.finish(),
        declared,
        injection: injection.finish(),
        wired,
        plain: plains,
        routes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, process::Command};

    /// Compiles Java sources into a map from internal name to digest, or `None` without `javac`.
    fn compile(sources: &[(&str, &str)]) -> Option<BTreeMap<String, Digest>> {
        let temp = tempfile::tempdir().unwrap();
        let mut files = Vec::new();
        for (name, text) in sources {
            let path = temp.path().join(name);
            fs::write(&path, text).unwrap();
            files.push(path);
        }
        let out = temp.path().join("out");
        let status = Command::new("javac")
            .arg("-g")
            .arg("-d")
            .arg(&out)
            .args(&files)
            .output()
            .ok()?;
        assert!(status.status.success(), "{status:?}");
        let mut digests = BTreeMap::new();
        let mut dirs = vec![out];
        while let Some(dir) = dirs.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                    continue;
                }
                let digest = digest(&fs::read(path).unwrap()).unwrap();
                digests.insert(digest.name.clone(), digest);
            }
        }
        Some(digests)
    }

    const BASE: &str = "public class A {
        static final String NAME = \"a\";
        public int one(int x) { return x + 1; }
        public String two() { return NAME + \"two\"; }
    }";

    #[test]
    fn comments_and_line_shifts_change_nothing() {
        let Some(before) = compile(&[("A.java", BASE)]) else {
            return;
        };
        let edited = BASE.replace("public int one", "// note\n\n  public int one");
        let after = compile(&[("A.java", &edited)]).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn body_edits_change_one_method_and_signature_edits_the_shape() {
        let Some(before) = compile(&[("A.java", BASE)]) else {
            return;
        };
        let body = compile(&[("A.java", &BASE.replace("x + 1", "x + 2"))]).unwrap();
        let (a, b) = (&before["A"], &body["A"]);
        assert_eq!(a.shape, b.shape);
        assert_ne!(a.methods["one(I)I"], b.methods["one(I)I"]);
        assert_eq!(
            a.methods["two()Ljava/lang/String;"],
            b.methods["two()Ljava/lang/String;"]
        );
        let field =
            compile(&[("A.java", &BASE.replace("NAME = ", "OTHER = \"b\", NAME = "))]).unwrap();
        assert_ne!(a.shape, field["A"].shape);
        let annotated = compile(&[(
            "A.java",
            &BASE.replace("public int one", "@Deprecated public int one"),
        )])
        .unwrap();
        assert_ne!(a.shape, annotated["A"].shape);
    }

    #[test]
    fn constant_pool_reordering_keeps_unchanged_methods() {
        // A new string constant used first renumbers every later constant-pool entry.
        let Some(before) = compile(&[("A.java", BASE)]) else {
            return;
        };
        let reordered = BASE.replace(
            "public int one",
            "public String zero() { return \"zzz\" + System.nanoTime(); }\n public int one",
        );
        let after = compile(&[("A.java", &reordered)]).unwrap();
        assert_ne!(before["A"].shape, after["A"].shape);
        for (method, hash) in &before["A"].methods {
            assert_eq!(&after["A"].methods[method], hash, "{method}");
        }
    }

    #[test]
    fn type_annotations_survive_constant_pool_reordering() {
        let source = "import java.lang.annotation.*;
            public class T {
                @Target(ElementType.TYPE_USE) @Retention(RetentionPolicy.RUNTIME) @interface N {}
                public int other() { return 1; }
                public java.util.List<@N String> names(@N Integer days) { return java.util.List.of(\"a\"); }
            }";
        let Some(before) = compile(&[("T.java", source)]) else {
            return;
        };
        // New constants in the earlier method renumber the ones the annotations use.
        let edited = source.replace(
            "return 1;",
            "int x = \"zz\".length() + Math.abs(-3) + Long.valueOf(7).intValue(); return 1 + x;",
        );
        let after = compile(&[("T.java", &edited)]).unwrap();
        assert_eq!(before["T"].shape, after["T"].shape);
        assert_ne!(
            before["T"].methods["other()I"],
            after["T"].methods["other()I"]
        );
    }

    #[test]
    fn switches_and_wide_instructions_parse() {
        let source = "public class S {
            int f(int k, String s) {
                int a0=0,a1=1,a2=2,a3=3,a4=4;
                long[] big = new long[300]; int v = 0;
                switch (k) { case 1: v = 1; break; case 2: v = 7; break; case 3: v = 9; break; default: v = 3; }
                switch (k) { case 10: v++; break; case 1000: v--; break; }
                switch (s) { case \"x\": v += 2; break; default: }
                return v + a0 + a1 + a2 + a3 + a4 + (int) big[0];
            }
        }";
        let Some(digests) = compile(&[("S.java", source)]) else {
            return;
        };
        assert!(digests["S"].methods.contains_key("f(ILjava/lang/String;)I"));
    }

    #[test]
    fn plain_constructors_only_store_into_their_own_fields() {
        let source = "import java.util.*;
            public class P {
                static final List<String> NAMES = List.of(\"a\", \"b\");
                private final String name; private final Map<String, Integer> counts = new HashMap<>();
                public P(String name) { this.name = Objects.requireNonNull(name).trim(); }
                public P(int n) { this(String.valueOf(n)); }
                public String name() { return name; }
            }
            class Q {
                static int created;
                private final Runnable task;
                Q(Runnable task) { this.task = task; task.run(); }
                Q() { this(() -> {}); created++; }
            }
            class R {
                R(Map<String, Object> registry) { registry.put(\"r\", this); }
            }";
        let Some(digests) = compile(&[("P.java", source)]) else {
            return;
        };
        let p = &digests["P"].plain;
        assert!(p.contains("<init>(Ljava/lang/String;)V"), "{p:?}");
        assert!(p.contains("<init>(I)V"), "{p:?}");
        assert!(p.contains("<clinit>()V"), "{p:?}");
        // Calling into an argument, or storing into a static field, reaches beyond the class.
        assert!(digests["Q"].plain.is_empty(), "{:?}", digests["Q"].plain);
        // So does changing an object it was given.
        assert!(digests["R"].plain.is_empty(), "{:?}", digests["R"].plain);
    }

    #[test]
    fn private_methods_stay_out_of_the_api() {
        let Some(before) = compile(&[("A.java", BASE)]) else {
            return;
        };
        let helper = BASE.replace(
            "public int one(int x) { return x + 1; }",
            "public int one(int x) { return inc(x); }\n private int inc(int x) { return x + 1; }",
        );
        let after = compile(&[("A.java", &helper)]).unwrap();
        assert_ne!(before["A"].shape, after["A"].shape);
        assert_eq!(before["A"].api, after["A"].api);
        let public = compile(&[(
            "A.java",
            &helper.replace("private int inc", "public int inc"),
        )])
        .unwrap();
        assert_ne!(before["A"].api, public["A"].api);
    }

    #[test]
    fn wiring_splits_into_declaration_constructors_and_annotated_members() {
        let mapping = "package org.springframework.web.bind.annotation;
            import java.lang.annotation.*;
            @Retention(RetentionPolicy.RUNTIME) public @interface GetMapping { String[] value() default {}; String[] path() default {}; }";
        let request = "package org.springframework.web.bind.annotation;
            import java.lang.annotation.*;
            @Retention(RetentionPolicy.RUNTIME) public @interface RequestMapping { String[] value() default {}; }";
        let controller = "import org.springframework.web.bind.annotation.*;
            @RequestMapping(\"/patients\")
            public class C {
                private final Object service;
                public C(Object service) { this.service = service; }
                @GetMapping(\"/{id}\") public String one(long id) { return \"\" + id; }
                @GetMapping public String all() { return \"\"; }
            }";
        let Some(before) = compile(&[
            ("GetMapping.java", mapping),
            ("RequestMapping.java", request),
            ("C.java", controller),
        ]) else {
            return;
        };
        let c = &before["C"];
        assert_eq!(c.routes["one(J)Ljava/lang/String;"], ["/patients/{id}"]);
        assert_eq!(c.routes["all()Ljava/lang/String;"], ["/patients"]);
        assert_eq!(
            c.wired["one(J)Ljava/lang/String;"].annotations,
            ["org/springframework/web/bind/annotation/GetMapping"]
        );
        let injected = controller.replace("public C(Object service)", "public C(String service)");
        let after = compile(&[
            ("GetMapping.java", mapping),
            ("RequestMapping.java", request),
            ("C.java", &injected),
        ])
        .unwrap();
        assert_eq!(c.declared, after["C"].declared);
        assert_eq!(c.wired, after["C"].wired);
        assert_ne!(c.injection, after["C"].injection);
    }
}
