//! Structured documents (JSON and YAML): a value tree that remembers where
//! every node sits in the text, the problems found while parsing it, and
//! the tools built on it — the path under the caret, pretty-printing,
//! sorting keys, and converting between the two formats.

/// A parsed value and its span in the source (UTF-16 offsets).
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub start: usize,
    pub end: usize,
    pub value: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    /// Kept as written so formatting never changes a number's spelling.
    Number(String),
    Str(String),
    Array(Vec<Node>),
    Object(Vec<(Key, Node)>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    pub name: String,
    pub start: usize,
    pub end: usize,
}

/// Something wrong (or suspicious) in a document.
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub start: usize,
    pub end: usize,
    pub message: String,
    pub warning: bool,
}

impl Problem {
    pub fn error(start: usize, end: usize, message: impl Into<String>) -> Self {
        Self { start, end: end.max(start + 1), message: message.into(), warning: false }
    }
    pub fn warning(start: usize, end: usize, message: impl Into<String>) -> Self {
        Self { start, end: end.max(start + 1), message: message.into(), warning: true }
    }
}

/// Result of parsing: the documents that could be read (YAML files may hold
/// several) and every problem found.
#[derive(Default, Debug)]
pub struct Parsed {
    pub docs: Vec<Node>,
    pub problems: Vec<Problem>,
}


// ---- the path under the caret -------------------------------------------------

/// JSONPath-style location of `offset`, e.g. `$.spec.containers[0].image`.
pub fn path_at(docs: &[Node], offset: usize) -> Option<String> {
    let (di, doc) = docs.iter().enumerate().find(|(_, d)| d.start <= offset && offset <= d.end)?;
    let mut path = if docs.len() > 1 { format!("$[doc {}]", di + 1) } else { "$".to_string() };
    let mut node = doc;
    'descend: loop {
        match &node.value {
            Value::Object(entries) => {
                for (k, v) in entries {
                    if (k.start <= offset && offset <= k.end) || (v.start <= offset && offset <= v.end && v.end > v.start) {
                        push_key(&mut path, &k.name);
                        if k.start <= offset && offset <= k.end {
                            break 'descend;
                        }
                        node = v;
                        continue 'descend;
                    }
                }
                break;
            }
            Value::Array(items) => {
                for (i, v) in items.iter().enumerate() {
                    if v.start <= offset && offset <= v.end {
                        path.push_str(&format!("[{i}]"));
                        node = v;
                        continue 'descend;
                    }
                }
                break;
            }
            _ => break,
        }
    }
    Some(path)
}

fn push_key(path: &mut String, key: &str) {
    let plain = !key.is_empty()
        && key.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
        && key.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '$');
    if plain {
        path.push('.');
        path.push_str(key);
    } else {
        path.push_str("[\"");
        path.push_str(&key.replace('\\', "\\\\").replace('"', "\\\""));
        path.push_str("\"]");
    }
}

// ---- writing values back out ---------------------------------------------------

/// JSON text for a value. `indent` None writes it compact on one line.
pub fn to_json(node: &Node, indent: Option<&str>, sort_keys: bool) -> String {
    let mut out = String::new();
    write_json(node, indent, sort_keys, 0, &mut out);
    if indent.is_some() {
        out.push('\n');
    }
    out
}

fn write_json(node: &Node, indent: Option<&str>, sort: bool, depth: usize, out: &mut String) {
    let nl = |out: &mut String, d: usize| {
        if let Some(unit) = indent {
            out.push('\n');
            for _ in 0..d {
                out.push_str(unit);
            }
        }
    };
    match &node.value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&json_number(n)),
        Value::Str(s) => write_json_string(s, out),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, v) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                nl(out, depth + 1);
                write_json(v, indent, sort, depth + 1, out);
            }
            nl(out, depth);
            out.push(']');
        }
        Value::Object(entries) => {
            if entries.is_empty() {
                out.push_str("{}");
                return;
            }
            let mut order: Vec<&(Key, Node)> = entries.iter().collect();
            if sort {
                order.sort_by(|a, b| a.0.name.cmp(&b.0.name));
            }
            out.push('{');
            for (i, (k, v)) in order.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                nl(out, depth + 1);
                write_json_string(&k.name, out);
                out.push(':');
                if indent.is_some() {
                    out.push(' ');
                }
                write_json(v, indent, sort, depth + 1, out);
            }
            nl(out, depth);
            out.push('}');
        }
    }
}

/// True if `n` is already a valid JSON number token.
fn is_json_number(n: &str) -> bool {
    let b = n.strip_prefix('-').unwrap_or(n).as_bytes();
    let int_end = b.iter().position(|c| !c.is_ascii_digit()).unwrap_or(b.len());
    if int_end == 0 || (int_end > 1 && b[0] == b'0') {
        return false;
    }
    let mut i = int_end;
    if b.get(i) == Some(&b'.') {
        let f = b[i + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
        if f == 0 {
            return false;
        }
        i += 1 + f;
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let e = b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
        if e == 0 {
            return false;
        }
        i += e;
    }
    i == b.len()
}

/// YAML numbers that JSON spells differently (hex, octal, .inf, 007...).
fn json_number(n: &str) -> String {
    if is_json_number(n) {
        return n.to_string();
    }
    let l = n.to_ascii_lowercase();
    let parsed = if let Some(h) = l.strip_prefix("0x") {
        i64::from_str_radix(h, 16).ok().map(|v| v.to_string())
    } else if let Some(o) = l.strip_prefix("0o") {
        i64::from_str_radix(o, 8).ok().map(|v| v.to_string())
    } else if l.contains("inf") || l.contains("nan") {
        Some("null".to_string()) // JSON has no infinities
    } else if l.starts_with('+') {
        Some(n[1..].to_string())
    } else if l.starts_with('.') || l.starts_with("-.") {
        Some(n.replacen('.', "0.", 1))
    } else {
        l.parse::<i64>().map(|v| v.to_string()).ok().or_else(|| l.parse::<f64>().ok().map(|v| v.to_string()))
    };
    parsed.unwrap_or_else(|| n.to_string())
}

pub fn write_json_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// YAML text for a value, in block style.
pub fn to_yaml(node: &Node, indent: usize) -> String {
    let mut out = String::new();
    match &node.value {
        Value::Object(e) if !e.is_empty() => write_yaml_map(e, 0, indent, &mut out),
        Value::Array(a) if !a.is_empty() => write_yaml_seq(a, 0, indent, &mut out),
        _ => {
            write_yaml_inline(node, 0, indent, &mut out);
            out.push('\n');
        }
    }
    out
}

fn pad(out: &mut String, n: usize) {
    for _ in 0..n {
        out.push(' ');
    }
}

fn write_yaml_map(entries: &[(Key, Node)], col: usize, unit: usize, out: &mut String) {
    for (i, (k, v)) in entries.iter().enumerate() {
        if i > 0 {
            pad(out, col);
        }
        out.push_str(&yaml_scalar(&k.name, true));
        out.push(':');
        write_yaml_after_key(v, col, unit, out);
    }
}

fn write_yaml_after_key(v: &Node, col: usize, unit: usize, out: &mut String) {
    match &v.value {
        Value::Object(e) if !e.is_empty() => {
            out.push('\n');
            pad(out, col + unit);
            write_yaml_map(e, col + unit, unit, out);
        }
        Value::Array(a) if !a.is_empty() => {
            out.push('\n');
            pad(out, col + unit);
            write_yaml_seq(a, col + unit, unit, out);
        }
        Value::Str(s) if s.contains('\n') => {
            out.push(' ');
            write_block_literal(s, col + unit, out);
        }
        _ => {
            out.push(' ');
            write_yaml_inline(v, col, unit, out);
            out.push('\n');
        }
    }
}

fn write_yaml_seq(items: &[Node], col: usize, unit: usize, out: &mut String) {
    for (i, v) in items.iter().enumerate() {
        if i > 0 {
            pad(out, col);
        }
        out.push_str("- ");
        match &v.value {
            Value::Object(e) if !e.is_empty() => write_yaml_map(e, col + 2, unit, out),
            Value::Array(a) if !a.is_empty() => write_yaml_seq(a, col + 2, unit, out),
            Value::Str(s) if s.contains('\n') => write_block_literal(s, col + 2, out),
            _ => {
                write_yaml_inline(v, col, unit, out);
                out.push('\n');
            }
        }
    }
}

fn write_block_literal(s: &str, col: usize, out: &mut String) {
    let body = s.strip_suffix('\n').unwrap_or(s);
    out.push_str(if s.ends_with('\n') { "|\n" } else { "|-\n" });
    for line in body.split('\n') {
        if !line.is_empty() {
            pad(out, col);
            out.push_str(line);
        }
        out.push('\n');
    }
}

fn write_yaml_inline(v: &Node, _col: usize, _unit: usize, out: &mut String) {
    match &v.value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(n),
        Value::Str(s) => out.push_str(&yaml_scalar(s, false)),
        Value::Array(_) => out.push_str("[]"),
        Value::Object(_) => out.push_str("{}"),
    }
}

/// A string as a YAML scalar: plain when that reads back as the same
/// string, double-quoted otherwise.
pub fn yaml_scalar(s: &str, is_key: bool) -> String {
    let resolves_to_other = {
        let u: Vec<u16> = s.encode_utf16().collect();
        !matches!(crate::syntax::scalar_kind(&u), crate::syntax::Kind::String)
    };
    let first = s.chars().next();
    let needs_quotes = s.is_empty()
        || resolves_to_other
        || s.starts_with(' ')
        || s.ends_with(' ')
        || first.is_some_and(|c| "-?:,[]{}#&*!|>'\"%@`".contains(c))
        || s.contains(": ")
        || s.ends_with(':')
        || s.contains(" #")
        || s.chars().any(|c| (c as u32) < 0x20)
        || (is_key && s.contains(':'));
    if !needs_quotes {
        return s.to_string();
    }
    let mut out = String::new();
    write_json_string(s, &mut out); // JSON escapes are valid YAML double-quoted escapes
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(value: Value) -> Node {
        Node { start: 0, end: 0, value }
    }
    fn k(name: &str) -> Key {
        Key { name: name.into(), start: 0, end: 0 }
    }

    #[test]
    fn json_and_yaml_writers() {
        let doc = n(Value::Object(vec![
            (k("b"), n(Value::Array(vec![n(Value::Number("1".into())), n(Value::Str("two".into()))]))),
            (k("a"), n(Value::Object(vec![(k("x y"), n(Value::Null)), (k("t"), n(Value::Bool(true)))]))),
            (k("s"), n(Value::Str("true".into()))),
            (k("m"), n(Value::Str("line1\nline2\n".into()))),
            (k("e"), n(Value::Array(vec![]))),
        ]));
        assert_eq!(
            to_json(&doc, None, true),
            r#"{"a":{"t":true,"x y":null},"b":[1,"two"],"e":[],"m":"line1\nline2\n","s":"true"}"#
        );
        assert_eq!(
            to_json(&doc, Some("  "), false),
            "{\n  \"b\": [\n    1,\n    \"two\"\n  ],\n  \"a\": {\n    \"x y\": null,\n    \"t\": true\n  },\n  \"s\": \"true\",\n  \"m\": \"line1\\nline2\\n\",\n  \"e\": []\n}\n"
        );
        assert_eq!(
            to_yaml(&doc, 2),
            "b:\n  - 1\n  - two\na:\n  x y: null\n  t: true\ns: \"true\"\nm: |\n  line1\n  line2\ne: []\n"
        );
    }

    #[test]
    fn scalar_quoting() {
        assert_eq!(yaml_scalar("hello world", false), "hello world");
        assert_eq!(yaml_scalar("123", false), "\"123\"");
        assert_eq!(yaml_scalar("null", false), "\"null\"");
        assert_eq!(yaml_scalar("a: b", false), "\"a: b\"");
        assert_eq!(yaml_scalar("- x", false), "\"- x\"");
        assert_eq!(yaml_scalar("", false), "\"\"");
        assert_eq!(yaml_scalar("C:\\path", false), "C:\\path");
    }
}
