//! JSON: a strict parser that explains mistakes in plain words, and a
//! formatter that re-indents without touching values or dropping comments.

use crate::data::{Key, Node, Parsed, Problem, Value};

/// What counts as an error depends on the flavor of JSON in the file.
#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    /// JSONC: // and /* */ comments and trailing commas are allowed.
    pub jsonc: bool,
    /// JSON Lines: one value per line.
    pub lines: bool,
}

/// Flavor from the file name: .jsonc / .json5 and well-known config files
/// (tsconfig.json, VS Code settings...) allow comments.
pub fn options_for(file_name: Option<&str>) -> Options {
    let name = file_name.unwrap_or("").to_ascii_lowercase();
    let jsonc_names = [
        "tsconfig", "jsconfig", "settings.json", "launch.json", "tasks.json", "keybindings.json",
        "extensions.json", "devcontainer.json", ".eslintrc", ".babelrc", "deno.json",
    ];
    Options {
        jsonc: name.ends_with(".jsonc") || name.ends_with(".json5") || jsonc_names.iter().any(|n| name.starts_with(n) || name.ends_with(n)),
        lines: name.ends_with(".jsonl") || name.ends_with(".ndjson"),
    }
}

pub fn parse(src: &[u16], opt: Options) -> Parsed {
    let mut out = Parsed::default();
    if opt.lines {
        let mut start = 0;
        for (k, &c) in src.iter().enumerate().chain(std::iter::once((src.len(), &13u16))) {
            if c == 13 || c == 10 {
                let line = &src[start..k];
                if line.iter().any(|&c| !is_ws(c)) {
                    let mut p = P::new(src, opt);
                    p.i = start;
                    p.limit = k;
                    if let Ok(v) = p.document() {
                        out.docs.push(v);
                    }
                    out.problems.extend(p.problems);
                }
                start = k + 1;
            }
        }
        return out;
    }
    let mut p = P::new(src, opt);
    if let Ok(v) = p.document() {
        out.docs.push(v);
    }
    out.problems = p.problems;
    out
}

#[inline]
fn is_ws(c: u16) -> bool {
    c == 32 || c == 9 || c == 10 || c == 13 || c == 0xFEFF
}

fn describe(c: u16) -> String {
    match char::from_u32(c as u32) {
        Some(ch) if !ch.is_control() => format!("'{ch}'"),
        _ => format!("U+{c:04X}"),
    }
}

struct P<'a> {
    s: &'a [u16],
    i: usize,
    limit: usize,
    opt: Options,
    problems: Vec<Problem>,
    depth: usize,
    /// End of the last reported problem: a mistake usually trips the
    /// parser again right after it, and those echoes aren't reported.
    quiet_until: usize,
}

type R<T> = Result<T, ()>;

/// Where resynchronizing after a mistake left a container's loop.
enum Sync {
    /// Past a ',': parse the next member.
    Next,
    /// At (and past) this container's closer.
    Closed,
    /// At a closer that belongs to an outer container, or at the end.
    Out,
}

/// Stop collecting after this many problems (the rest is noise).
const MAX_PROBLEMS: usize = 200;

impl<'a> P<'a> {
    fn new(s: &'a [u16], opt: Options) -> Self {
        Self { s, i: 0, limit: s.len(), opt, problems: Vec::new(), depth: 0, quiet_until: 0 }
    }

    fn at(&self, k: usize) -> u16 {
        if k < self.limit { self.s[k] } else { 0 }
    }

    fn eof(&self) -> bool {
        self.i >= self.limit
    }

    /// Records a mistake and carries on.
    fn note(&mut self, start: usize, end: usize, msg: impl Into<String>) {
        if start < self.quiet_until && !self.problems.is_empty() {
            return;
        }
        self.problems.push(Problem::error(start, end, msg));
        self.quiet_until = end.max(start + 1);
        if self.problems.len() >= MAX_PROBLEMS {
            self.limit = self.i.min(self.limit); // stop here
        }
    }

    /// Records a mistake the current value can't get past.
    fn fail<T>(&mut self, start: usize, end: usize, msg: impl Into<String>) -> R<T> {
        self.note(start, end, msg);
        Err(())
    }

    /// End of the current line, for underlining "rest of line" problems.
    fn line_end(&self, from: usize) -> usize {
        (from..self.limit).find(|&k| self.s[k] == 13 || self.s[k] == 10).unwrap_or(self.limit)
    }

    fn ws(&mut self) {
        loop {
            while !self.eof() && is_ws(self.s[self.i]) {
                self.i += 1;
            }
            if self.at(self.i) == b'/' as u16 && (self.at(self.i + 1) == b'/' as u16 || self.at(self.i + 1) == b'*' as u16) {
                let start = self.i;
                let end = if self.at(self.i + 1) == b'/' as u16 {
                    self.line_end(start)
                } else {
                    match (start + 2..self.limit).find(|&k| self.s[k] == b'*' as u16 && self.at(k + 1) == b'/' as u16) {
                        Some(k) => k + 2,
                        None => {
                            self.note(start, start + 2, "This comment is never closed (missing */)");
                            self.limit
                        }
                    }
                };
                if !self.opt.jsonc {
                    self.note(start, end, "Comments aren't allowed in JSON (they are in .jsonc files)");
                }
                self.i = end;
                continue;
            }
            return;
        }
    }

    /// After a mistake inside a container closed by `close`: skip to the
    /// next ',' or closer at this level.
    fn resync(&mut self, close: u16) -> Sync {
        let mut depth = 0usize;
        while self.i < self.limit {
            let c = self.s[self.i];
            match c {
                0x22 => {
                    // Skip a string (to its quote or the line end).
                    let mut k = self.i + 1;
                    while k < self.limit && self.s[k] != 0x22 && self.s[k] != 13 && self.s[k] != 10 {
                        k += if self.s[k] == 0x5C { 2 } else { 1 };
                    }
                    self.i = (k + 1).min(self.limit);
                    continue;
                }
                0x7B | 0x5B => depth += 1,
                0x7D | 0x5D if depth > 0 => depth -= 1,
                0x7D | 0x5D => {
                    if c == close {
                        self.i += 1;
                        return Sync::Closed;
                    }
                    return Sync::Out;
                }
                0x2C if depth == 0 => {
                    self.i += 1;
                    return Sync::Next;
                }
                _ => {}
            }
            self.i += 1;
        }
        Sync::Out
    }

    fn document(&mut self) -> R<Node> {
        self.ws();
        if self.eof() {
            return Err(()); // empty document: nothing to report
        }
        let v = self.value();
        self.ws();
        if !self.eof() {
            let start = self.i;
            let end = self.line_end(start);
            self.note(start, end, "Unexpected text after the end of the JSON value (only one top-level value is allowed)");
        }
        v
    }

    fn value(&mut self) -> R<Node> {
        let start = self.i;
        let c = self.at(start);
        if self.eof() {
            return self.fail(start.saturating_sub(1), start, "Unexpected end of file: expected a value");
        }
        if self.depth > 1000 {
            return self.fail(start, start + 1, "Nested too deeply");
        }
        match c {
            0x7B => self.object(),
            0x5B => self.array(),
            0x22 => {
                let (s, end) = self.string();
                Ok(Node { start, end, value: Value::Str(s) })
            }
            0x27 => {
                let (s, end) = self.single_quoted();
                self.note(start, end, "Strings must use double quotes (\"), not single quotes");
                Ok(Node { start, end, value: Value::Str(s) })
            }
            0x2D | 0x30..=0x39 => self.number(),
            0x2B | 0x2E => {
                let msg = if c == 0x2B { "Numbers can't start with '+'" } else { "Numbers must start with a digit (write 0.5, not .5)" };
                self.note(start, start + 1, msg);
                self.i += 1;
                let digits = self.i;
                while (0x30..=0x39).contains(&self.at(self.i)) || self.at(self.i) == b'.' as u16 {
                    self.i += 1;
                }
                let text = String::from_utf16_lossy(&self.s[digits..self.i]);
                Ok(Node { start, end: self.i, value: Value::Number(text) })
            }
            _ if c == b'_' as u16 || c >= 0x80 || (c as u8).is_ascii_alphabetic() && c < 0x80 => self.word(),
            0x7D | 0x5D => self.fail(start, start + 1, format!("Unexpected {}: expected a value", describe(c))),
            0x2C => self.fail(start, start + 1, "Unexpected ',': expected a value (is there an extra comma?)"),
            _ => self.fail(start, start + 1, format!("Unexpected character {}", describe(c))),
        }
    }

    fn word_end(&self, start: usize) -> usize {
        let mut e = start;
        while e < self.limit && (self.s[e] == b'_' as u16 || self.s[e] >= 0x80 || (self.s[e] as u8).is_ascii_alphanumeric() && self.s[e] < 0x80) {
            e += 1;
        }
        e
    }

    fn word(&mut self) -> R<Node> {
        let start = self.i;
        let e = self.word_end(start);
        let w = String::from_utf16_lossy(&self.s[start..e]);
        let lower = w.to_ascii_lowercase();
        let value = match w.as_str() {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            "null" => Value::Null,
            _ if self.at(self.skip_ws_from(e)) == b':' as u16 => {
                // A key where a value should be: the value before it is
                // probably missing. Let the container resynchronize.
                return self.fail(start, e, format!("Unexpected '{w}': expected a value"));
            }
            _ => {
                let (msg, value) = if ["true", "false", "null"].contains(&lower.as_str()) {
                    let v = match lower.as_str() {
                        "true" => Value::Bool(true),
                        "false" => Value::Bool(false),
                        _ => Value::Null,
                    };
                    (format!("JSON literals are lowercase: write {lower}, not {w}"), v)
                } else if w == "None" {
                    ("Use null, not None".to_string(), Value::Null)
                } else if w == "undefined" {
                    ("undefined isn't valid JSON; use null".to_string(), Value::Null)
                } else if w == "NaN" || w == "Infinity" {
                    (format!("{w} isn't a valid JSON number"), Value::Number(w.clone()))
                } else {
                    (format!("Unexpected '{w}': text values must be in double quotes"), Value::Str(w.clone()))
                };
                self.note(start, e, msg);
                value
            }
        };
        self.i = e;
        Ok(Node { start, end: e, value })
    }

    fn skip_ws_from(&self, mut k: usize) -> usize {
        while k < self.limit && (self.s[k] == 32 || self.s[k] == 9) {
            k += 1;
        }
        k
    }

    fn number(&mut self) -> R<Node> {
        let start = self.i;
        let mut k = start;
        let digit = |c: u16| (0x30..=0x39).contains(&c);
        if self.at(k) == b'-' as u16 {
            k += 1;
        }
        if !digit(self.at(k)) {
            return self.fail(start, k + 1, "Expected a digit after '-'");
        }
        if self.at(k) == 0x30 && digit(self.at(k + 1)) {
            let mut e = k;
            while digit(self.at(e)) {
                e += 1;
            }
            self.note(start, e, "Numbers can't have leading zeros");
        }
        while digit(self.at(k)) {
            k += 1;
        }
        if self.at(k) == b'.' as u16 {
            k += 1;
            if !digit(self.at(k)) {
                self.note(start, k, "Expected digits after the decimal point");
            }
            while digit(self.at(k)) {
                k += 1;
            }
        }
        if self.at(k) == b'e' as u16 || self.at(k) == b'E' as u16 {
            k += 1;
            if self.at(k) == b'+' as u16 || self.at(k) == b'-' as u16 {
                k += 1;
            }
            if !digit(self.at(k)) {
                self.note(start, k, "Expected digits in the exponent");
            }
            while digit(self.at(k)) {
                k += 1;
            }
        }
        let next = self.at(k);
        if next != 0 && ((next as u8).is_ascii_alphanumeric() && next < 0x80 || next == b'_' as u16) {
            let e = self.word_end(k);
            self.note(start, e, "Invalid number (values with units or letters need quotes)");
            k = e;
        }
        self.i = k;
        Ok(Node { start, end: k, value: Value::Number(String::from_utf16_lossy(&self.s[start..k])) })
    }

    /// A double-quoted string at self.i; returns (decoded text, end).
    /// Mistakes inside it are noted and skipped.
    fn string(&mut self) -> (String, usize) {
        let start = self.i;
        let mut k = start + 1;
        let mut buf: Vec<u16> = Vec::new();
        loop {
            if k >= self.limit || self.s[k] == 13 || self.s[k] == 10 {
                self.note(start, k, "This string is never closed (missing \")");
                // Carry on as if it ended before the first ',' ']' or '}'
                // on its line: usually only the closing quote is missing,
                // and the brackets after it still close their containers.
                let cut = (start + 1..k).find(|&q| matches!(self.s[q], 0x2C | 0x5D | 0x7D)).unwrap_or(k);
                self.i = cut;
                buf.truncate(cut - start - 1);
                return (String::from_utf16_lossy(&buf), cut);
            }
            let c = self.s[k];
            if c == b'"' as u16 {
                self.i = k + 1;
                return (String::from_utf16_lossy(&buf), k + 1);
            }
            if c == b'\\' as u16 {
                let e = self.at(k + 1);
                let mapped = match e {
                    0x22 => Some(0x22),
                    0x5C => Some(0x5C),
                    0x2F => Some(0x2F),
                    0x62 => Some(8),
                    0x66 => Some(12),
                    0x6E => Some(10),
                    0x72 => Some(13),
                    0x74 => Some(9),
                    _ => None,
                };
                if let Some(m) = mapped {
                    buf.push(m);
                    k += 2;
                    continue;
                }
                if e == b'u' as u16 {
                    let hex: String = (k + 2..k + 6).filter_map(|q| char::from_u32(self.at(q) as u32)).collect();
                    match u16::from_str_radix(&hex, 16) {
                        Ok(v) if hex.len() == 4 => {
                            buf.push(v);
                            k += 6;
                        }
                        _ => {
                            self.note(k, (k + 6).min(self.limit), "\\u must be followed by four hex digits");
                            k += 2;
                        }
                    }
                    continue;
                }
                if e == 0 || e == 13 || e == 10 {
                    k += 1;
                    continue; // the line end reports the unclosed string
                }
                let shown = char::from_u32(e as u32).unwrap_or('?');
                self.note(k, k + 2, format!("Invalid escape '\\{shown}' (write \\\\{shown} for a literal backslash)"));
                buf.push(e);
                k += 2;
                continue;
            }
            if c < 0x20 {
                let what = if c == 9 { "Tab characters" } else { "Control characters" };
                self.note(k, k + 1, format!("{what} must be escaped inside strings"));
            }
            buf.push(c);
            k += 1;
        }
    }

    /// A single-quoted string at self.i (a mistake the caller reports).
    fn single_quoted(&mut self) -> (String, usize) {
        let start = self.i;
        let line_end = self.line_end(start);
        let close = (start + 1..line_end).find(|&k| self.s[k] == 0x27);
        let end = close.map(|k| k + 1).unwrap_or(line_end);
        self.i = end;
        let inner = &self.s[start + 1..close.unwrap_or(line_end)];
        (String::from_utf16_lossy(inner), end)
    }

    fn object(&mut self) -> R<Node> {
        let start = self.i;
        self.i += 1;
        self.depth += 1;
        let mut entries: Vec<(Key, Node)> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut after_comma: Option<usize> = None;
        loop {
            self.ws();
            let ks = self.i;
            if self.eof() {
                self.note(start, start + 1, "This '{' is never closed (missing })");
                break;
            }
            let c = self.at(ks);
            let key = match c {
                0x7D => {
                    if let Some(comma) = after_comma.filter(|_| !self.opt.jsonc) {
                        self.note(comma, comma + 1, "Trailing comma before '}' isn't allowed in JSON");
                    }
                    self.i += 1;
                    break;
                }
                0x22 => {
                    let (name, ke) = self.string();
                    Key { name, start: ks, end: ke }
                }
                0x27 => {
                    let (name, ke) = self.single_quoted();
                    self.note(ks, ke, "Property names must use double quotes, not single quotes");
                    Key { name, start: ks, end: ke }
                }
                0x2C => {
                    self.note(ks, ks + 1, "Unexpected ',': expected a property name (is there an extra comma?)");
                    self.i += 1;
                    after_comma = Some(ks);
                    continue;
                }
                _ if (c as u8).is_ascii_alphabetic() && c < 0x80 || c == b'_' as u16 || c == b'$' as u16 => {
                    let mut e = ks;
                    while e < self.limit
                        && ((self.s[e] as u8).is_ascii_alphanumeric() && self.s[e] < 0x80
                            || self.s[e] == b'_' as u16
                            || self.s[e] == b'-' as u16
                            || self.s[e] == b'$' as u16)
                    {
                        e += 1;
                    }
                    self.note(ks, e, "Property names must be in double quotes");
                    self.i = e;
                    Key { name: String::from_utf16_lossy(&self.s[ks..e]), start: ks, end: e }
                }
                0x5D => {
                    // Leave it for the enclosing array: a '}' is missing.
                    self.note(ks, ks + 1, "Unexpected ']': this object was opened with '{' (is a '}' missing?)");
                    break;
                }
                _ => {
                    self.note(ks, ks + 1, format!("Expected a property name in double quotes, found {}", describe(c)));
                    match self.resync(0x7D) {
                        Sync::Next => {
                            after_comma = Some(self.i - 1);
                            continue;
                        }
                        Sync::Closed | Sync::Out => break,
                    }
                }
            };
            after_comma = None;
            self.ws();
            if self.at(self.i) == b':' as u16 {
                self.i += 1;
                self.ws();
            } else {
                self.note(key.start, key.end, format!("Expected ':' after \"{}\"", key.name));
            }
            let c = self.at(self.i);
            if c == b',' as u16 || c == b'}' as u16 || self.eof() {
                self.note(key.start, key.end, format!("\"{}\" has no value after ':'", key.name));
                if c == b',' as u16 {
                    after_comma = Some(self.i);
                    self.i += 1;
                }
                continue;
            }
            let v = match self.value() {
                Ok(v) => v,
                Err(()) => match self.resync(0x7D) {
                    Sync::Next => {
                        after_comma = Some(self.i - 1);
                        continue;
                    }
                    Sync::Closed | Sync::Out => break,
                },
            };
            if !seen.insert(key.name.clone()) {
                self.problems.push(Problem::warning(
                    key.start,
                    key.end,
                    format!("Duplicate key \"{}\": the last one wins", key.name),
                ));
            }
            let v_end = v.end;
            entries.push((key, v));
            self.ws();
            let c = self.at(self.i);
            if c == b',' as u16 {
                after_comma = Some(self.i);
                self.i += 1;
            } else if c == b'}' as u16 {
                self.i += 1;
                break;
            } else if self.eof() {
                self.note(start, start + 1, "This '{' is never closed (missing })");
                break;
            } else if c == b'"' as u16 || c == b'\'' as u16 || (c as u8).is_ascii_alphabetic() && c < 0x80 {
                self.note(v_end.saturating_sub(1), v_end, "Missing ',' after this value (before the next property)");
            } else if c == b']' as u16 {
                self.note(self.i, self.i + 1, "Unexpected ']': this object was opened with '{' (is a '}' missing?)");
                break;
            } else {
                let k = self.i;
                self.note(k, k + 1, format!("Expected ',' or '}}' after the value, found {}", describe(c)));
                match self.resync(0x7D) {
                    Sync::Next => after_comma = Some(self.i - 1),
                    Sync::Closed | Sync::Out => break,
                }
            }
        }
        self.depth -= 1;
        Ok(Node { start, end: self.i, value: Value::Object(entries) })
    }

    fn array(&mut self) -> R<Node> {
        let start = self.i;
        self.i += 1;
        self.depth += 1;
        let mut items = Vec::new();
        let mut after_comma: Option<usize> = None;
        loop {
            self.ws();
            if self.eof() {
                self.note(start, start + 1, "This '[' is never closed (missing ])");
                break;
            }
            let c = self.at(self.i);
            if c == b']' as u16 {
                if let Some(comma) = after_comma.filter(|_| !self.opt.jsonc) {
                    self.note(comma, comma + 1, "Trailing comma before ']' isn't allowed in JSON");
                }
                self.i += 1;
                break;
            }
            if c == b',' as u16 {
                self.note(self.i, self.i + 1, "Unexpected ',': expected a value (is there an extra comma?)");
                after_comma = Some(self.i);
                self.i += 1;
                continue;
            }
            after_comma = None;
            let v = match self.value() {
                Ok(v) => v,
                Err(()) => match self.resync(0x5D) {
                    Sync::Next => {
                        after_comma = Some(self.i - 1);
                        continue;
                    }
                    Sync::Closed | Sync::Out => break,
                },
            };
            let v_end = v.end;
            items.push(v);
            self.ws();
            let c = self.at(self.i);
            if c == b',' as u16 {
                after_comma = Some(self.i);
                self.i += 1;
            } else if c == b']' as u16 {
                self.i += 1;
                break;
            } else if self.eof() {
                self.note(start, start + 1, "This '[' is never closed (missing ])");
                break;
            } else if c == b'}' as u16 {
                self.note(self.i, self.i + 1, "Unexpected '}': this array was opened with '[' (is a ']' missing?)");
                break;
            } else if matches!(c, 0x22 | 0x27 | 0x7B | 0x5B | 0x2D | 0x30..=0x39) || (c as u8).is_ascii_alphabetic() && c < 0x80 {
                self.note(v_end.saturating_sub(1), v_end, "Missing ',' after this value (before the next item)");
            } else {
                let k = self.i;
                self.note(k, k + 1, format!("Expected ',' or ']' after the value, found {}", describe(c)));
                match self.resync(0x5D) {
                    Sync::Next => after_comma = Some(self.i - 1),
                    Sync::Closed | Sync::Out => break,
                }
            }
        }
        self.depth -= 1;
        Ok(Node { start, end: self.i, value: Value::Array(items) })
    }
}

// ---- formatting ---------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum Tok {
    Open(u16),
    Close(u16),
    Comma,
    Colon,
    Atom(usize, usize),
    LineComment(usize, usize),
    BlockComment(usize, usize),
}

/// Tokens plus "a line break came before this token" flags. Expects text
/// that parses (lenient mode), so it doesn't report errors itself.
fn tokens(s: &[u16]) -> Vec<(Tok, bool)> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut nl = false;
    while i < s.len() {
        let c = s[i];
        if c == 13 || c == 10 {
            nl = true;
            i += 1;
            continue;
        }
        if is_ws(c) {
            i += 1;
            continue;
        }
        let tok = match c {
            0x7B | 0x5B => {
                i += 1;
                Tok::Open(c)
            }
            0x7D | 0x5D => {
                i += 1;
                Tok::Close(c)
            }
            0x2C => {
                i += 1;
                Tok::Comma
            }
            0x3A => {
                i += 1;
                Tok::Colon
            }
            0x2F if s.get(i + 1) == Some(&0x2F) => {
                let e = (i..s.len()).find(|&k| s[k] == 13 || s[k] == 10).unwrap_or(s.len());
                let t = Tok::LineComment(i, e);
                i = e;
                t
            }
            0x2F if s.get(i + 1) == Some(&0x2A) => {
                let e = (i + 2..s.len()).find(|&k| s[k] == 0x2A && s.get(k + 1) == Some(&0x2F)).map(|k| k + 2).unwrap_or(s.len());
                let t = Tok::BlockComment(i, e);
                i = e;
                t
            }
            0x22 | 0x27 => {
                let mut k = i + 1;
                while k < s.len() && s[k] != c {
                    if s[k] == 0x5C {
                        k += 1;
                    }
                    k += 1;
                }
                let t = Tok::Atom(i, (k + 1).min(s.len()));
                i = k + 1;
                t
            }
            _ => {
                let mut k = i;
                while k < s.len() && !is_ws(s[k]) && !matches!(s[k], 0x7B | 0x5B | 0x7D | 0x5D | 0x2C | 0x3A | 0x22) {
                    k += 1;
                }
                let t = Tok::Atom(i, k.max(i + 1));
                i = k.max(i + 1);
                t
            }
        };
        out.push((tok, nl));
        nl = false;
    }
    out
}

/// Re-indents `s` with `unit` per level. Values are copied exactly as
/// written and comments stay where they were.
pub fn format(s: &[u16], unit: &str) -> String {
    let toks = tokens(s);
    let text = |a: usize, b: usize| String::from_utf16_lossy(&s[a..b]);
    let mut out = String::new();
    let mut depth = 0usize;
    let mut owe_nl = false;
    let newline = |out: &mut String, depth: usize| {
        while out.ends_with(' ') {
            out.pop();
        }
        if !out.is_empty() {
            out.push('\n');
        }
        for _ in 0..depth {
            out.push_str(unit);
        }
    };
    let mut k = 0;
    while k < toks.len() {
        let (t, nl_before) = toks[k];
        match t {
            Tok::Open(c) => {
                if owe_nl {
                    newline(&mut out, depth);
                }
                out.push(char::from_u32(c as u32).unwrap_or('{'));
                if let Some((Tok::Close(_), _)) = toks.get(k + 1) {
                    // Empty container stays on one line.
                    out.push(if c == 0x7B { '}' } else { ']' });
                    k += 2;
                    owe_nl = false;
                    continue;
                }
                depth += 1;
                owe_nl = true;
            }
            Tok::Close(c) => {
                depth = depth.saturating_sub(1);
                newline(&mut out, depth);
                out.push(char::from_u32(c as u32).unwrap_or('}'));
                owe_nl = false;
            }
            Tok::Comma => {
                out.push(',');
                owe_nl = true;
            }
            Tok::Colon => {
                out.push_str(": ");
                owe_nl = false;
            }
            Tok::Atom(a, b) => {
                if owe_nl {
                    newline(&mut out, depth);
                }
                out.push_str(&text(a, b));
                owe_nl = false;
            }
            Tok::LineComment(a, b) => {
                if nl_before || out.is_empty() {
                    newline(&mut out, depth);
                } else {
                    // Trailing comment: stays on the line it followed.
                    out.push(' ');
                }
                out.push_str(&text(a, b));
                owe_nl = true;
            }
            Tok::BlockComment(a, b) => {
                if nl_before || out.is_empty() {
                    // Leads the next token on its own line.
                    newline(&mut out, depth);
                    owe_nl = false;
                } else if !out.ends_with(' ') {
                    out.push(' ');
                }
                out.push_str(&text(a, b));
                out.push(' ');
            }
        }
        k += 1;
    }
    out.push('\n');
    out
}

/// One line, no insignificant whitespace, comments dropped.
pub fn minify(s: &[u16]) -> String {
    let mut out = String::new();
    for (t, _) in tokens(s) {
        match t {
            Tok::Open(c) | Tok::Close(c) => out.push(char::from_u32(c as u32).unwrap_or('?')),
            Tok::Comma => out.push(','),
            Tok::Colon => out.push(':'),
            Tok::Atom(a, b) => out.push_str(&String::from_utf16_lossy(&s[a..b])),
            Tok::LineComment(..) | Tok::BlockComment(..) => {}
        }
    }
    out
}

/// True if the text contains comments (which value-level tools would drop).
pub fn has_comments(s: &[u16]) -> bool {
    tokens(s).iter().any(|(t, _)| matches!(t, Tok::LineComment(..) | Tok::BlockComment(..)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::path_at;

    fn w(s: &str) -> Vec<u16> {
        s.replace("\r\n", "\r").replace('\n', "\r").encode_utf16().collect()
    }

    fn first_error(src: &str, opt: Options) -> Option<(String, String)> {
        let u = w(src);
        let p = parse(&u, opt);
        p.problems.iter().find(|p| !p.warning).map(|e| (String::from_utf16_lossy(&u[e.start..e.end.min(u.len())]), e.message.clone()))
    }

    #[test]
    fn valid_documents_parse() {
        let u = w("{\"a\": [1, -2.5e3, true, null, \"x\\n\\u00e9\"], \"b\": {}}");
        let p = parse(&u, Options::default());
        assert!(p.problems.is_empty(), "{:?}", p.problems);
        let Value::Object(e) = &p.docs[0].value else { panic!() };
        let Value::Array(a) = &e[0].1.value else { panic!() };
        assert_eq!(a[4].value, Value::Str("x\n\u{e9}".into()));
        assert_eq!(a[1].value, Value::Number("-2.5e3".into()));
    }

    #[test]
    fn explains_common_mistakes() {
        let d = Options::default();
        let cases = [
            ("{\"a\": 1 \"b\": 2}", "1", "Missing ','"),
            ("{\"a\": 1,}", ",", "Trailing comma"),
            ("{a: 1}", "a", "double quotes"),
            ("{'a': 1}", "'a'", "single quotes"),
            ("{\"a\" 1}", "\"a\"", "Expected ':'"),
            ("{\"a\": tru}", "tru", "double quotes"),
            ("{\"a\": True}", "True", "lowercase"),
            ("[1, 2", "[", "never closed"),
            ("{\"a\": \"x}", "\"x}", "never closed"),
            ("{\"a\": \"\\q\"}", "\\q", "Invalid escape"),
            ("{\"a\": 01}", "01", "leading zeros"),
            ("{\"a\": 1} {}", "{}", "after the end"),
            ("// hi\n{}", "// hi", "Comments"),
            ("[1 2]", "1", "Missing ','"),
            ("{\"a\":}", "\"a\"", "no value"),
            ("{\"a\": \"tab\there\"}", "\t", "Tab"),
        ];
        for (src, at, msg) in cases {
            let (got_at, got_msg) = first_error(src, d).unwrap_or_else(|| panic!("no error for {src}"));
            assert_eq!(got_at, at, "{src}: {got_msg}");
            assert!(got_msg.contains(msg), "{src}: {got_msg}");
        }
    }

    #[test]
    fn keeps_going_after_mistakes() {
        let src = "{\n \"a\": false\n \"b\": [1, 2,],\n 'c': 'x',\n d: 01,\n \"e\": {\"f\": 1 ]\n}";
        let u = w(src);
        let p = parse(&u, Options::default());
        let msgs: Vec<&str> = p.problems.iter().map(|p| p.message.as_str()).collect();
        let has = |m: &str| msgs.iter().any(|x| x.contains(m));
        assert!(has("Missing ','"), "{msgs:?}");
        assert!(has("Trailing comma"), "{msgs:?}");
        assert!(has("single quotes"), "{msgs:?}");
        assert!(has("Property names must be in double quotes"), "{msgs:?}");
        assert!(has("leading zeros"), "{msgs:?}");
        assert!(has("is a '}' missing"), "{msgs:?}");
        // A lost closing quote is one problem, not one per following line.
        let q = parse(&w("{\n \"a\": [1, \"2],\n \"b\": 3,\n \"c\": {\"d\": 4}\n}"), Options::default());
        assert_eq!(q.problems.len(), 1, "{:?}", q.problems);
        // Everything after the mistakes is still in the tree.
        let Value::Object(e) = &p.docs[0].value else { panic!() };
        assert_eq!(e.iter().map(|(k, _)| k.name.as_str()).collect::<Vec<_>>(), ["a", "b", "c", "d", "e"]);
    }

    #[test]
    fn jsonc_allows_comments_and_trailing_commas() {
        let opt = Options { jsonc: true, lines: false };
        let p = parse(&w("{\n // c\n \"a\": 1, /* b */\n}"), opt);
        assert!(p.problems.is_empty(), "{:?}", p.problems);
        assert!(options_for(Some("tsconfig.json")).jsonc);
        assert!(!options_for(Some("package.json")).jsonc);
    }

    #[test]
    fn duplicate_keys_warn() {
        let p = parse(&w("{\"a\": 1, \"a\": 2}"), Options::default());
        assert_eq!(p.problems.len(), 1);
        assert!(p.problems[0].warning);
    }

    #[test]
    fn json_lines() {
        let p = parse(&w("{\"a\":1}\n\n{\"a\":2}\n{bad}"), Options { jsonc: false, lines: true });
        // The broken line still yields a (partial) value for the caret path.
        assert_eq!(p.docs.len(), 3);
        assert_eq!(p.problems.len(), 1);
    }

    #[test]
    fn path_under_caret() {
        let src = "{\"spec\": {\"containers\": [{\"image\": \"nginx\"}], \"my key\": 1}}";
        let u = w(src);
        let p = parse(&u, Options::default());
        let at = |needle: &str| src.find(needle).unwrap() + 1;
        assert_eq!(path_at(&p.docs, at("nginx")).unwrap(), "$.spec.containers[0].image");
        assert_eq!(path_at(&p.docs, at("\"my key\"")).unwrap(), "$.spec[\"my key\"]");
    }

    #[test]
    fn formats_and_keeps_comments() {
        let src = w("{\"a\":1, // one\n\"b\":[],\"c\":[1,{\"d\":null}],\n/* block */ \"e\":\"x\"}");
        assert_eq!(
            format(&src, "  "),
            "{\n  \"a\": 1, // one\n  \"b\": [],\n  \"c\": [\n    1,\n    {\n      \"d\": null\n    }\n  ],\n  /* block */ \"e\": \"x\"\n}\n"
        );
        assert_eq!(minify(&src), "{\"a\":1,\"b\":[],\"c\":[1,{\"d\":null}],\"e\":\"x\"}");
        assert!(has_comments(&src));
    }
}
