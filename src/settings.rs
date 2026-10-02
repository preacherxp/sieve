//! Spring Boot configuration files as flat keys, and the project classes that read them.
//! Local mode records whole files; when one changes, only the keys whose values changed
//! matter, and a key that some project class names reaches tests through that class.
use crate::{bytecode, classes::Class};
use std::collections::{BTreeMap, BTreeSet};

/// Whether a workspace-relative path is a Spring Boot configuration file.
pub fn is_config(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return false;
    };
    (stem.starts_with("application") || stem.starts_with("bootstrap"))
        && matches!(extension, "yml" | "yaml" | "properties")
}

/// Key to value hash, with keys of later documents prefixed by `#<n>:`. `None` for syntax
/// this reader does not follow, which then counts as a change to every key.
pub fn flatten(path: &str, text: &str) -> Option<BTreeMap<String, String>> {
    let raw = if path.ends_with(".properties") {
        properties(text)?
    } else {
        yaml(text)?
    };
    Some(
        raw.into_iter()
            .map(|(k, v)| (k, bytecode::hash(v.as_bytes())))
            .collect(),
    )
}

fn document(n: usize, key: &str) -> String {
    if n == 0 {
        key.to_owned()
    } else {
        format!("#{n}:{key}")
    }
}

fn properties(text: &str) -> Option<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    let mut n = 0;
    for line in text.lines() {
        let line = line.trim_start();
        if matches!(line.trim_end(), "#---" | "!---") {
            n += 1;
            continue;
        }
        if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
            continue;
        }
        if line.ends_with('\\') || line.contains("\\=") || line.contains("\\:") {
            return None;
        }
        let at = line.find(['=', ':', ' ', '\t']).unwrap_or(line.len());
        let key = line[..at].trim();
        let value = line[at..]
            .trim_start_matches([' ', '\t'])
            .trim_start_matches(['=', ':'])
            .trim();
        out.insert(document(n, key), value.to_owned());
    }
    Some(out)
}

/// The line without a trailing comment, or `None` when quotes make that unclear.
fn uncomment(line: &str) -> Option<&str> {
    let mut quote = None;
    let mut previous = ' ';
    for (i, c) in line.char_indices() {
        match (quote, c) {
            (None, '#') if previous == ' ' || i == 0 => return Some(&line[..i]),
            (None, '"' | '\'') if matches!(previous, ' ' | ':' | '[' | '{' | ',') => {
                quote = Some(c)
            }
            (Some(q), c) if c == q => quote = None,
            _ => {}
        }
        previous = c;
    }
    if quote.is_some() {
        return None;
    }
    Some(line)
}

/// A key and its value: the text after the first `: ` (or a final `:`) outside quotes.
fn key_value(content: &str) -> Option<(String, &str)> {
    let mut quote = None;
    let bytes = content.as_bytes();
    for (i, c) in content.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') if i == 0 => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, ':') if i + 1 == bytes.len() || bytes[i + 1] == b' ' => {
                let key = content[..i].trim();
                let key = key
                    .strip_prefix(['"', '\''])
                    .and_then(|k| k.strip_suffix(['"', '\'']))
                    .unwrap_or(key);
                return Some((key.to_owned(), content[i + 1..].trim()));
            }
            _ => {}
        }
    }
    None
}

fn yaml(text: &str) -> Option<BTreeMap<String, String>> {
    struct Open {
        indent: usize,
        path: String,
    }
    /// Lines kept verbatim as the value of `path`: a sequence or a block scalar.
    struct Capture {
        path: String,
        indent: usize,
        dash: Option<usize>,
    }
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    let mut stack: Vec<Open> = Vec::new();
    let mut capture: Option<Capture> = None;
    let mut n = 0;
    for line in text.lines() {
        if line.trim_end() == "---" || line.starts_with("--- ") {
            n += 1;
            stack.clear();
            capture = None;
            continue;
        }
        if line.trim_end() == "..." || line.starts_with('%') {
            continue;
        }
        let stripped = uncomment(line)?.trim_end();
        let content = stripped.trim_start();
        if content.is_empty() {
            continue;
        }
        let lead = &stripped[..stripped.len() - content.len()];
        if lead.contains('\t') {
            return None;
        }
        let indent = lead.len();
        if let Some(c) = &capture {
            let item = content.starts_with('-') && c.dash == Some(indent);
            if indent > c.indent || item {
                let value = out.entry(document(n, &c.path)).or_default();
                value.push('\n');
                value.push_str(&stripped[c.indent.min(indent)..]);
                continue;
            }
            capture = None;
        }
        if content == "-" || content.starts_with("- ") {
            while stack.last().is_some_and(|o| o.indent > indent) {
                stack.pop();
            }
            let owner = stack.last()?;
            let path = owner.path.clone();
            let value = out.entry(document(n, &path)).or_default();
            value.push('\n');
            value.push_str(content);
            capture = Some(Capture {
                path,
                indent: owner.indent,
                dash: Some(indent),
            });
            continue;
        }
        while stack.last().is_some_and(|o| o.indent >= indent) {
            stack.pop();
        }
        let (key, value) = key_value(content)?;
        if key.is_empty() || key == "<<" || key.starts_with(['?', '&', '*', '!', '{', '[']) {
            return None;
        }
        let path = match stack.last() {
            Some(parent) => format!("{}.{key}", parent.path),
            None => key,
        };
        if value.is_empty() {
            stack.push(Open { indent, path });
            continue;
        }
        if value.starts_with(['&', '*', '!']) {
            return None;
        }
        if value.starts_with(['[', '{']) && !value.ends_with([']', '}']) {
            return None;
        }
        if value.starts_with(['"', '\'']) && !value.ends_with(&value[..1]) {
            return None;
        }
        out.insert(document(n, &path), value.to_owned());
        if value.starts_with(['|', '>']) {
            capture = Some(Capture {
                path,
                indent,
                dash: None,
            });
        }
    }
    Some(out)
}

/// The version of a Flyway versioned migration file name, such as `7` or `1.2` for
/// `V1_2__name.sql`.
fn migration_version(name: &str) -> Option<Vec<u64>> {
    let rest = name.strip_prefix('V')?;
    let (version, description) = rest.split_once("__")?;
    if !description.ends_with(".sql") {
        return None;
    }
    version
        .split(['.', '_'])
        .map(|part| part.parse().ok())
        .collect()
}

/// Whether a directory listing holds Flyway SQL migrations.
pub fn is_migrations(names: &[String]) -> bool {
    !names.is_empty()
        && names.iter().all(|n| {
            migration_version(n).is_some()
                || n.starts_with("R__") && n.ends_with(".sql")
                || !n.contains('.')
        })
}

/// Whether migrations added to a listing only append versioned migrations whose statements
/// leave existing tables, rows and constraints as they were: new non-unique indexes,
/// sequences, comments, and tables that reference no other table. Code that uses new tables
/// or sequences changed with them, and the next context start applies the migration.
pub fn inert_migrations(
    then: &[String],
    now: &[String],
    read: impl Fn(&str) -> Option<String>,
) -> bool {
    if then.iter().any(|n| !now.contains(n)) {
        return false;
    }
    let newest = then.iter().filter_map(|n| migration_version(n)).max();
    now.iter().filter(|n| !then.contains(n)).all(|name| {
        migration_version(name).is_some_and(|v| newest.as_ref().is_none_or(|newest| v > *newest))
            && read(name).is_some_and(|sql| inert_sql(&sql))
    })
}

fn inert_sql(sql: &str) -> bool {
    if sql.contains("$$") || sql.contains("/*") {
        return false;
    }
    let text: String = sql
        .lines()
        .map(|l| l.split_once("--").map_or(l, |(code, _)| code))
        .collect::<Vec<_>>()
        .join(" ");
    text.split(';').all(|statement| {
        let words: Vec<String> = statement
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        let starts = |prefix: &[&str]| {
            words.len() >= prefix.len() && words.iter().zip(prefix).all(|(w, p)| w == p)
        };
        words.is_empty()
            || starts(&["create", "index"])
            || starts(&["create", "sequence"])
            || starts(&["comment", "on"])
            || starts(&["create", "table"])
                && !words.iter().any(|w| {
                    w.starts_with("references")
                        || matches!(w.as_str(), "inherits" | "partition" | "as" | "like")
                })
    })
}

/// Keys whose values differ between two flattened files, without document prefixes.
pub fn changed(
    then: &BTreeMap<String, String>,
    now: &BTreeMap<String, String>,
) -> BTreeSet<String> {
    then.keys()
        .chain(now.keys())
        .filter(|k| then.get(*k) != now.get(*k))
        .map(|k| match k.strip_prefix('#') {
            Some(rest) => rest.split_once(':').map_or(rest, |(_, k)| k).to_owned(),
            None => k.clone(),
        })
        .collect()
}

/// Spring's relaxed form of a key: lower case, without dashes and underscores.
fn canonical(key: &str) -> String {
    key.chars()
        .filter(|c| !matches!(c, '-' | '_'))
        .flat_map(char::to_lowercase)
        .collect()
}

/// Property names a class constant can denote: placeholders and dotted keys.
fn names(text: &str, out: &mut Vec<String>) {
    let key = |s: &str| {
        !s.is_empty()
            && s.contains('.')
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '[' | ']'))
    };
    let mut rest = text;
    let mut placeholder = false;
    while let Some(start) = rest.find("${") {
        placeholder = true;
        rest = &rest[start + 2..];
        let end = rest.find(['}', ':']).unwrap_or(rest.len());
        out.push(rest[..end].trim().to_owned());
    }
    if !placeholder && key(text) {
        out.push(text.to_owned());
    }
}

/// Whether `prefix` names `key` or a key below it.
fn below(key: &str, prefix: &str) -> bool {
    key == prefix
        || key
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with(['.', '[']))
}

const BINDING: &[&str] = &[
    "org/springframework/boot/context/properties/ConfigurationProperties",
    "org/springframework/boot/context/properties/bind/Binder",
];

/// Project classes that name `key`: in a placeholder, as a whole key, or, for classes that
/// bind configuration properties, as a prefix. A name below the key, such as one entry of a
/// map or list the key holds, counts too.
pub fn consumers(key: &str, classes: &[Class]) -> Vec<usize> {
    let key = canonical(key);
    let mut out = Vec::new();
    for (i, class) in classes.iter().enumerate() {
        let binds = class.refs.iter().any(|r| BINDING.contains(&r.as_str()));
        let mut found = Vec::new();
        for text in &class.refs {
            names(text, &mut found);
        }
        let reads = found
            .iter()
            .map(|n| canonical(n))
            .any(|name| below(&name, &key) || binds && below(&key, &name) && name.contains('.'));
        if reads {
            out.push(i);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appended_index_and_table_migrations_are_inert() {
        let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let then = names(&["V1__a.sql", "V2__b.sql"]);
        assert!(is_migrations(&then));
        let sql = |name: &str| {
            Some(match name {
                "V3__index.sql" => "create index a_idx on a (b); -- why\nCREATE INDEX c ON d (e) where f is null;".into(),
                "V3__table.sql" => "create table t (id bigint primary key, n varchar(3) not null unique);\ncreate sequence s;".into(),
                "V3__unique.sql" => "create unique index u on a (b);".into(),
                "V3__fk.sql" => "create table t (id bigint, a_id bigint references a (id));".into(),
                _ => "alter table a add column x int;".into(),
            })
        };
        let added = |name: &str| names(&["V1__a.sql", "V2__b.sql", name]);
        assert!(inert_migrations(&then, &added("V3__index.sql"), sql));
        assert!(inert_migrations(&then, &added("V3__table.sql"), sql));
        assert!(!inert_migrations(&then, &added("V3__unique.sql"), sql));
        assert!(!inert_migrations(&then, &added("V3__fk.sql"), sql));
        assert!(!inert_migrations(&then, &added("V3__alter.sql"), sql));
        assert!(!inert_migrations(&then, &added("V1_5__index.sql"), sql));
        assert!(!inert_migrations(&then, &names(&["V1__a.sql"]), sql));
    }

    fn keys(map: &BTreeMap<String, String>) -> Vec<&str> {
        map.keys().map(String::as_str).collect()
    }

    #[test]
    fn yaml_flattens_nested_keys_documents_and_sequences() {
        let text = "spring:\n  datasource:\n    url: jdbc:x # a comment\n    username: 'a: b'\n\
                    clinic:\n  reminders:\n    window: PT24H\n  hosts:\n  - a\n  - b\n\
                    \x20 note: |\n    one\n    two\n---\nspring:\n  config:\n    activate:\n      on-profile: dev\n";
        let map = flatten("application.yml", text).unwrap();
        assert_eq!(
            keys(&map),
            [
                "#1:spring.config.activate.on-profile",
                "clinic.hosts",
                "clinic.note",
                "clinic.reminders.window",
                "spring.datasource.url",
                "spring.datasource.username",
            ]
        );
        let edited = flatten(
            "application.yml",
            &text.replace("PT24H", "PT48H").replace("- b", "- c"),
        )
        .unwrap();
        assert_eq!(
            changed(&map, &edited),
            BTreeSet::from(["clinic.reminders.window".into(), "clinic.hosts".into()])
        );
        let comment = flatten("application.yml", &text.replace("a comment", "other")).unwrap();
        assert!(changed(&map, &comment).is_empty());
    }

    #[test]
    fn unsupported_yaml_is_none() {
        assert!(flatten("application.yml", "a: &x 1\nb: *x\n").is_none());
        assert!(flatten("application.yml", "a:\n\tb: 1\n").is_none());
        assert!(flatten("application.yml", "plain\ncontinued: x\n").is_none());
    }

    #[test]
    fn properties_split_on_separators() {
        let map = flatten(
            "application.properties",
            "# c\na.b=1\nc.d: 2\ne.f 3\n#---\ng=4\n",
        )
        .unwrap();
        assert_eq!(keys(&map), ["#1:g", "a.b", "c.d", "e.f"]);
    }

    #[test]
    fn consumers_name_keys_by_placeholder_key_or_bound_prefix() {
        let class = |refs: &[&str]| Class {
            refs: refs.iter().map(|r| r.to_string()).collect(),
            ..Class::default()
        };
        let classes = [
            class(&["${clinic.reminders.window:PT24H}"]),
            class(&["clinic.notifications", BINDING[0]]),
            class(&["clinic.events.topic"]),
            class(&["clinic.notifications"]),
        ];
        assert_eq!(consumers("clinic.reminders.window", &classes), [0]);
        assert_eq!(
            consumers("clinic.notifications.sms-max-length", &classes),
            [1]
        );
        assert_eq!(consumers("clinic.events.topic", &classes), [2]);
        assert_eq!(consumers("clinic.events", &classes), [2]);
        assert!(consumers("spring.datasource.hikari.maximum-pool-size", &classes).is_empty());
    }
}
