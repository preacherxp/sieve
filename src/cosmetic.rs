//! Changed Java and Kotlin sources whose code did not change: only their comments, Javadoc,
//! indentation or line breaks differ from the comparison base. Their bytecode is the same
//! apart from line numbers, so class-level selection does not count them as changes.
use crate::Result;
use std::{collections::BTreeSet, path::Path, process::Command};

/// The changed `sources` whose code equals the version at `base`. Anything this cannot decide,
/// such as a file added since the base or an unreadable version, counts as changed.
pub fn unchanged(workspace: &Path, base: &str, sources: &BTreeSet<String>) -> BTreeSet<String> {
    sources
        .iter()
        .filter(|path| {
            let kotlin = path.ends_with(".kt");
            if !(kotlin || path.ends_with(".java")) {
                return false;
            }
            let (Ok(after), Ok(before)) = (
                std::fs::read_to_string(workspace.join(path)),
                at_base(workspace, base, path),
            ) else {
                return false;
            };
            same_code(&before, &after, kotlin)
        })
        .cloned()
        .collect()
}

fn at_base(workspace: &Path, base: &str, path: &str) -> Result<String> {
    // `REV:./PATH` resolves the path from the workspace, wherever the repository root is.
    let output = Command::new("git")
        .current_dir(workspace)
        .args(["show", "--end-of-options", &format!("{base}:./{path}")])
        .output()?;
    if !output.status.success() {
        return Err(format!("{path} is not in {base}").into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

/// Whether two versions of a source differ only in comments and whitespace outside literals.
pub fn same_code(before: &str, after: &str, kotlin: bool) -> bool {
    matches!((code(before, kotlin), code(after, kotlin)), (Some(a), Some(b)) if a == b)
}

/// The source without comments, each whitespace run outside literals collapsed to one space.
/// `None` when the lexer cannot be sure of that reading: Unicode escapes, which Java decodes
/// before it finds comments, Kotlin string templates, and unterminated comments or literals.
fn code(text: &str, kotlin: bool) -> Option<String> {
    if text.contains("\\u") {
        return None;
    }
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut space = false;
    let mut i = 0;
    while i < b.len() {
        let rest = &b[i..];
        if rest.starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            space = true;
            continue;
        }
        if rest.starts_with(b"/*") {
            // Kotlin block comments nest; Java's end at the first `*/`.
            let mut depth = 0;
            loop {
                if i >= b.len() {
                    return None;
                }
                if b[i..].starts_with(b"/*") && (kotlin || depth == 0) {
                    depth += 1;
                    i += 2;
                } else if b[i..].starts_with(b"*/") {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            space = true;
            continue;
        }
        if b[i].is_ascii_whitespace() {
            space = true;
            i += 1;
            continue;
        }
        if space && !out.is_empty() {
            out.push(b' ');
        }
        space = false;
        let end = if rest.starts_with(b"\"\"\"") {
            let close = find(b, i + 3, b"\"\"\"")?;
            // A raw string may end in more quotes, which belong to its content.
            if kotlin && b.get(close + 3) == Some(&b'"') {
                return None;
            }
            close + 3
        } else if matches!(b[i], b'"' | b'\'') || (kotlin && b[i] == b'`') {
            literal(b, i)?
        } else {
            i + 1
        };
        if kotlin && b[i] == b'"' && b[i..end].windows(2).any(|w| w == b"${") {
            return None;
        }
        out.extend_from_slice(&b[i..end]);
        i = end;
    }
    String::from_utf8(out).ok()
}

/// The index of `needle` at or after `from`, outside backslash escapes.
fn find(b: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    let mut i = from;
    while i + needle.len() <= b.len() {
        if b[i] == b'\\' {
            i += 2;
            continue;
        }
        if b[i..].starts_with(needle) {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// The end of a one-line string, character or backtick literal starting at `start`.
fn literal(b: &[u8], start: usize) -> Option<usize> {
    let quote = b[start];
    let mut i = start + 1;
    while i < b.len() {
        match b[i] {
            b'\\' if quote != b'`' => i += 2,
            b'\n' => return None,
            c if c == quote => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_and_layout_are_not_code() {
        let java = "package a;\n\nclass A {\n    int f(int x) {\n        return x + 1;\n    }\n}\n";
        for same in [
            "package a;\n\n/** Docs. */\nclass A {\n    // note\n    int f(int x) {\n        return x + 1; /* why */\n    }\n}\n",
            "package a;\nclass A {\n  int f(int x) {\n    return x\n        + 1;\n  }\n}\n",
            "package a; class A { int f(int x) { return x + 1; } }",
        ] {
            assert!(same_code(java, same, false), "{same}");
        }
        for different in [
            "package a;\nclass A {\n    int f(int x) {\n        return x + 2;\n    }\n}\n",
            // Spacing that joins or splits tokens is a change, as is anything in a literal.
            "package a;\nclass A {\n    int f(int x) {\n        return x+1;\n    }\n}\n",
        ] {
            assert!(!same_code(java, different, false), "{different}");
        }
        let strings = |body: &str| format!("class S {{ String s = {body}; }}");
        for (a, b) in [
            ("\"a  b\"", "\"a b\""),
            ("\"// not a comment\"", "\"// other\""),
            ("\"/* x */\"", "\"/* y */\""),
            ("\"\\\" // quoted\"", "\"\\\" // other\""),
            ("'\"'+\"// x\"", "'\"'+\"// y\""),
            ("\"\"\"\n  a  // b\n  \"\"\"", "\"\"\"\n  a  // c\n  \"\"\""),
        ] {
            assert!(!same_code(&strings(a), &strings(b), false), "{a} vs {b}");
        }
    }

    #[test]
    fn uncertain_sources_count_as_changed() {
        // Java decodes `\u000a` before comments: this line comment ends early.
        let escaped = "class A { // \\u000a int x; \n}";
        assert!(!same_code(escaped, escaped, false));
        assert!(!same_code("class A { /* open", "class A { /* open", false));
        // Kotlin templates can hold quotes; raw strings may end in extra quotes.
        let template = "val s = \"${f(\"x\")} // y\"";
        assert!(!same_code(template, template, true));
        let raw = "val s = \"\"\"a\"\"\"\"";
        assert!(!same_code(raw, raw, true));
    }

    #[test]
    fn kotlin_comments_nest_and_backticks_are_names() {
        let base = "fun f() = 1";
        assert!(same_code(
            base,
            "/* outer /* inner */ still */ fun f() = 1",
            true
        ));
        // In Java the first `*/` closes the comment, leaving code behind.
        assert!(!same_code(
            "class A {}",
            "/* outer /* inner */ still */ class A {}",
            false
        ));
        assert!(!same_code("fun `a  b`() = 1", "fun `a b`() = 1", true));
    }
}
