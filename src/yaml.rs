//! YAML: a block-structure parser for the YAML real files use — mappings,
//! sequences, flow collections, plain / quoted / block scalars, anchors,
//! aliases, merge keys, several documents — that reports mistakes with
//! exact positions and keeps going, so one typo doesn't hide the next.

use std::collections::HashMap;

use crate::data::{Key, Node, Parsed, Problem, Value};

pub fn parse(src: &[u16]) -> Parsed {
    let mut y = Y::new(src);
    let docs = y.run();
    Parsed { docs, problems: y.problems }
}

#[derive(Clone, Copy)]
struct Line {
    start: usize,
    end: usize,
    /// Columns of leading whitespace (a tab counts as one, and is reported).
    indent: usize,
    /// First non-space character (== end on a blank line).
    text: usize,
}

struct Y<'a> {
    s: &'a [u16],
    lines: Vec<Line>,
    li: usize,
    problems: Vec<Problem>,
    anchors: HashMap<String, Node>,
}

const MAX_PROBLEMS: usize = 200;

#[inline]
fn sp(c: u16) -> bool {
    c == 32 || c == 9
}

impl<'a> Y<'a> {
    fn new(s: &'a [u16]) -> Self {
        let mut lines = Vec::new();
        let mut problems = Vec::new();
        let mut start = 0;
        let mut k = 0;
        while k <= s.len() {
            if k == s.len() || s[k] == 13 || s[k] == 10 {
                if k == s.len() && start == k && k > 0 {
                    break; // nothing after the final line break
                }
                let mut t = start;
                while t < k && sp(s[t]) {
                    t += 1;
                }
                if t < k {
                    if let Some(tab) = (start..t).find(|&q| s[q] == 9) {
                        problems.push(Problem::error(tab, tab + 1, "Tabs can't be used for indentation in YAML; use spaces"));
                    }
                }
                lines.push(Line { start, end: k, indent: t - start, text: t });
                if k < s.len() && s[k] == 13 && s.get(k + 1) == Some(&10) {
                    k += 1;
                }
                start = k + 1;
            }
            k += 1;
        }
        Self { s, lines, li: 0, problems, anchors: HashMap::new() }
    }

    fn at(&self, k: usize) -> u16 {
        self.s.get(k).copied().unwrap_or(0)
    }

    fn err(&mut self, start: usize, end: usize, msg: impl Into<String>) {
        if self.problems.len() < MAX_PROBLEMS {
            self.problems.push(Problem::error(start, end, msg));
        }
    }

    fn blank(&self, li: usize) -> bool {
        let l = self.lines[li];
        l.text >= l.end
    }

    fn comment_line(&self, li: usize) -> bool {
        let l = self.lines[li];
        l.text < l.end && self.s[l.text] == b'#' as u16
    }

    /// "---" or "..." at column 0.
    fn marker(&self, li: usize) -> Option<u16> {
        let l = self.lines[li];
        if l.text != l.start {
            return None;
        }
        let c = self.at(l.start);
        let is = (c == b'-' as u16 || c == b'.' as u16)
            && self.at(l.start + 1) == c
            && self.at(l.start + 2) == c
            && (l.start + 3 >= l.end || sp(self.at(l.start + 3)));
        is.then_some(c)
    }

    fn is_dash(&self, li: usize, k: usize) -> bool {
        self.at(k) == b'-' as u16 && (k + 1 >= self.lines[li].end || sp(self.at(k + 1)))
    }

    /// Next line with content (skips blanks and comments); None at the end
    /// of the document.
    fn next_content(&mut self) -> Option<usize> {
        while self.li < self.lines.len() {
            if self.blank(self.li) || self.comment_line(self.li) {
                self.li += 1;
                continue;
            }
            if self.marker(self.li).is_some() {
                return None;
            }
            return Some(self.li);
        }
        None
    }

    /// End of the content on line `li` from `from`: before a comment and
    /// trailing spaces. Quotes that open a scalar hide '#'.
    fn content_end(&self, li: usize, from: usize) -> usize {
        let l = self.lines[li];
        let mut k = from;
        let mut quote = 0u16;
        while k < l.end {
            let c = self.s[k];
            if quote != 0 {
                if c == b'\\' as u16 && quote == b'"' as u16 {
                    k += 2;
                    continue;
                }
                if c == quote {
                    quote = 0;
                }
            } else if (c == b'"' as u16 || c == b'\'' as u16) && (k == from || matches!(self.at(k - 1), 32 | 9 | 0x5B | 0x7B | 0x2C | 0x3A)) {
                quote = c;
            } else if c == b'#' as u16 && (k == from || sp(self.s[k - 1])) {
                break;
            }
            k += 1;
        }
        while k > from && sp(self.s[k - 1]) {
            k -= 1;
        }
        k
    }

    fn line_of(&self, abs: usize) -> usize {
        self.lines.partition_point(|l| l.start <= abs).saturating_sub(1)
    }

    /// Skips line `li` and every more-indented line under it.
    fn skip_tree(&mut self, li: usize) {
        let ind = self.lines[li].indent;
        self.li = li + 1;
        while self.li < self.lines.len() && (self.blank(self.li) || self.lines[self.li].indent > ind) && self.marker(self.li).is_none() {
            self.li += 1;
        }
    }

    fn null_at(pos: usize) -> Node {
        Node { start: pos, end: pos, value: Value::Null }
    }

    // ---- documents ----

    fn run(&mut self) -> Vec<Node> {
        let mut docs = Vec::new();
        while self.li < self.lines.len() {
            if self.blank(self.li) || self.comment_line(self.li) || (self.lines[self.li].text == self.lines[self.li].start && self.at(self.lines[self.li].start) == b'%' as u16) {
                self.li += 1;
                continue;
            }
            let mut node = None;
            if self.marker(self.li) == Some(b'.' as u16) {
                self.li += 1;
                continue;
            }
            if self.marker(self.li) == Some(b'-' as u16) {
                let li = self.li;
                let rest = self.skip_sp(self.lines[li].start + 3, self.lines[li].end);
                if rest < self.content_end(li, rest) {
                    self.lines[li].text = rest;
                    node = Some(self.inline_value(li, -1));
                } else {
                    self.li += 1;
                }
            }
            let node = node.or_else(|| self.block(-1)).unwrap_or_else(|| {
                let pos = self.lines.get(self.li).map(|l| l.start).unwrap_or(self.s.len());
                Self::null_at(pos)
            });
            while let Some(li) = self.next_content() {
                let l = self.lines[li];
                self.err(l.text, self.content_end(li, l.text), "Unexpected content after the end of the document's top-level value (check the indentation)");
                self.skip_tree(li);
            }
            docs.push(node);
        }
        docs
    }

    fn skip_sp(&self, mut k: usize, end: usize) -> usize {
        while k < end && sp(self.s[k]) {
            k += 1;
        }
        k
    }

    fn block(&mut self, parent: isize) -> Option<Node> {
        let li = self.next_content()?;
        let l = self.lines[li];
        if l.indent as isize <= parent {
            return None;
        }
        Some(self.node_here(li, l.indent, parent))
    }

    fn node_here(&mut self, li: usize, ind: usize, parent: isize) -> Node {
        let t = self.lines[li].text;
        if self.is_dash(li, t) {
            self.seq(ind)
        } else if self.key_at(li).is_some() || self.missing_space_key(li) {
            // A "key:value" line is meant as a key: let the mapping report it.
            self.map(ind)
        } else {
            self.inline_value(li, parent)
        }
    }

    // ---- collections ----

    fn seq(&mut self, ind: usize) -> Node {
        let start = self.lines[self.li].text;
        let mut items = Vec::new();
        let mut end = start + 1;
        while let Some(li) = self.next_content() {
            let l = self.lines[li];
            if l.indent < ind {
                break;
            }
            if l.indent > ind {
                self.err(l.text, self.content_end(li, l.text), "Bad indentation: this line is indented further than the list items above it");
                self.skip_tree(li);
                continue;
            }
            if !self.is_dash(li, l.text) {
                break; // back to the parent (e.g. the next key at this level)
            }
            let dash = l.text;
            let content = self.skip_sp(dash + 1, l.end);
            let item = if content >= self.content_end(li, content) {
                self.li = li + 1;
                self.block(ind as isize).unwrap_or_else(|| Self::null_at(dash + 1))
            } else {
                let col = content - l.start;
                self.lines[li].indent = col;
                self.lines[li].text = content;
                self.node_here(li, col, ind as isize)
            };
            end = end.max(item.end);
            items.push(item);
        }
        Node { start, end, value: Value::Array(items) }
    }

    /// A `key:` at the start of line `li`: (key start, key end, colon).
    fn key_at(&self, li: usize) -> Option<(usize, usize, usize)> {
        let l = self.lines[li];
        let t = l.text;
        let c = self.at(t);
        let colon_ok = |k: usize| self.at(k) == b':' as u16 && (k + 1 >= l.end || sp(self.at(k + 1)));
        if c == b'"' as u16 || c == b'\'' as u16 {
            let mut k = t + 1;
            while k < l.end {
                if c == b'"' as u16 && self.s[k] == b'\\' as u16 {
                    k += 2;
                    continue;
                }
                if self.s[k] == c {
                    if c == b'\'' as u16 && self.at(k + 1) == c {
                        k += 2;
                        continue;
                    }
                    break;
                }
                k += 1;
            }
            let colon = self.skip_sp(k + 1, l.end);
            return (k < l.end && colon_ok(colon)).then_some((t, k + 1, colon));
        }
        if c == b'?' as u16 && (t + 1 >= l.end || sp(self.at(t + 1))) {
            return None;
        }
        if matches!(c, 0x5B | 0x7B | 0x5D | 0x7D | 0x2C | 0x23 | 0x26 | 0x2A | 0x21 | 0x7C | 0x3E | 0x25 | 0x40 | 0x60) {
            return None;
        }
        let end = self.content_end(li, t);
        let mut k = t;
        while k < end {
            if colon_ok(k) {
                let mut ke = k;
                while ke > t && sp(self.s[ke - 1]) {
                    ke -= 1;
                }
                return (ke > t).then_some((t, ke, k));
            }
            k += 1;
        }
        None
    }

    /// `name:value` — a key with the space after ':' forgotten.
    fn missing_space_key(&self, li: usize) -> bool {
        let l = self.lines[li];
        let mut k = l.text;
        while k < l.end && (self.s[k] < 0x80 && ((self.s[k] as u8).is_ascii_alphanumeric() || matches!(self.s[k], 0x5F | 0x2E | 0x2D))) {
            k += 1;
        }
        k > l.text && self.at(k) == b':' as u16 && k + 1 < l.end && !sp(self.at(k + 1)) && self.at(k + 1) != b'/' as u16 && self.at(k + 1) != b':' as u16
    }

    fn decode_key(&mut self, ks: usize, ke: usize) -> String {
        let c = self.at(ks);
        if c == b'"' as u16 || c == b'\'' as u16 {
            self.unquote(ks, ke)
        } else {
            String::from_utf16_lossy(&self.s[ks..ke])
        }
    }

    fn map(&mut self, ind: usize) -> Node {
        let start = self.lines[self.li].text;
        let mut entries: Vec<(Key, Node)> = Vec::new();
        let mut merges: Vec<Node> = Vec::new();
        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut end = start + 1;
        while let Some(li) = self.next_content() {
            let l = self.lines[li];
            if l.indent < ind {
                break;
            }
            if l.indent > ind {
                self.err(l.text, self.content_end(li, l.text), "Bad indentation: this line is indented more than the keys above it");
                self.skip_tree(li);
                continue;
            }
            if self.is_dash(li, l.text) {
                self.err(l.text, l.text + 1, "A list item can't be at the same level as the 'key:' entries around it");
                self.skip_tree(li);
                continue;
            }
            let Some((ks, ke, colon)) = self.key_at(li) else {
                let ce = self.content_end(li, l.text);
                let missing_space = (l.text..ce).find(|&k| self.s[k] == b':' as u16 && k + 1 < ce && !sp(self.s[k + 1]));
                match missing_space {
                    Some(k) if !self.s[l.text..k].iter().any(|&c| sp(c)) => {
                        self.err(k, k + 1, "Missing space after ':' (write 'key: value')");
                    }
                    _ => self.err(l.text, ce, "Expected 'key: value' here (or this line is indented wrong)"),
                }
                self.skip_tree(li);
                continue;
            };
            let name = self.decode_key(ks, ke);
            if let Some(&first) = seen.get(&name) {
                self.err(ks, ke, format!("Duplicate key '{name}' (first defined on line {})", first + 1));
            }
            seen.insert(name.clone(), li);
            let key = Key { name, start: ks, end: ke };
            let vs = self.skip_sp(colon + 1, l.end);
            let value = if vs >= self.content_end(li, vs) {
                self.li = li + 1;
                self.value_below(ind as isize, colon + 1)
            } else {
                self.lines[li].text = vs;
                self.inline_value(li, ind as isize)
            };
            end = end.max(value.end).max(ke);
            if key.name == "<<" {
                merges.push(value);
            } else {
                entries.push((key, value));
            }
        }
        for m in merges {
            self.merge_into(&mut entries, m);
        }
        Node { start, end, value: Value::Object(entries) }
    }

    /// The value of a key whose line ends after the colon.
    fn value_below(&mut self, ind: isize, pos: usize) -> Node {
        match self.next_content() {
            Some(li) if self.lines[li].indent as isize > ind => {
                let l = self.lines[li];
                self.node_here(li, l.indent, ind)
            }
            // YAML allows a list to sit at the key's own indentation.
            Some(li) if self.lines[li].indent as isize == ind && self.is_dash(li, self.lines[li].text) => self.seq(ind as usize),
            _ => Self::null_at(pos),
        }
    }

    fn merge_into(&mut self, entries: &mut Vec<(Key, Node)>, m: Node) {
        let sources = match m.value {
            Value::Object(_) => vec![m],
            Value::Array(items) => items,
            _ => {
                self.err(m.start, m.end, "'<<' needs a mapping (usually an alias like *defaults)");
                return;
            }
        };
        for src in sources {
            let Value::Object(src_entries) = src.value else {
                self.err(src.start, src.end, "'<<' can only merge mappings");
                continue;
            };
            for (k, v) in src_entries {
                if !entries.iter().any(|(e, _)| e.name == k.name) {
                    entries.push((Key { start: usize::MAX, end: usize::MAX, ..k }, detach(v)));
                }
            }
        }
    }

    // ---- scalars and flow ----

    /// A value starting mid-line at lines[li].text. `parent` is the
    /// indentation continuation lines must exceed.
    fn inline_value(&mut self, li: usize, parent: isize) -> Node {
        let line_end = self.lines[li].end;
        let mut p = self.lines[li].text;
        let mut anchor = None;
        let mut force_str = false;
        loop {
            let c = self.at(p);
            if (c == b'&' as u16 || c == b'!' as u16) && p < line_end {
                let mut e = p + 1;
                while e < line_end && !sp(self.s[e]) && !matches!(self.s[e], 0x2C | 0x5B | 0x5D | 0x7B | 0x7D) {
                    e += 1;
                }
                if c == b'&' as u16 {
                    anchor = Some(String::from_utf16_lossy(&self.s[p + 1..e]));
                } else if String::from_utf16_lossy(&self.s[p..e]) == "!!str" {
                    force_str = true;
                }
                p = self.skip_sp(e, line_end);
            } else {
                break;
            }
        }
        let ce = self.content_end(li, p);
        let mut node = if p >= ce {
            // Only an anchor/tag on this line: the value is below.
            self.li = li + 1;
            self.value_below(parent, p)
        } else {
            let c = self.at(p);
            match c {
                0x2A => self.alias(li, p, ce),
                0x7C | 0x3E => self.block_scalar(li, p, parent),
                0x5B | 0x7B => self.flow_top(p),
                0x22 | 0x27 => self.quoted_top(p),
                0x2D if p + 1 >= line_end || sp(self.at(p + 1)) => {
                    self.err(p, p + 1, "A list can't start on the same line as its key; put each '- item' on its own line below");
                    self.skip_tree(li);
                    Self::null_at(p)
                }
                0x40 | 0x60 => {
                    self.err(p, p + 1, "'@' and '`' are reserved in YAML: put this value in quotes");
                    self.plain(li, p, parent)
                }
                _ => self.plain(li, p, parent),
            }
        };
        if force_str {
            if !matches!(node.value, Value::Array(_) | Value::Object(_)) {
                node.value = Value::Str(String::from_utf16_lossy(&self.s[node.start..node.end]));
            }
        }
        if let Some(name) = anchor {
            self.anchors.insert(name, node.clone());
        }
        node
    }

    fn alias(&mut self, li: usize, p: usize, ce: usize) -> Node {
        let mut e = p + 1;
        while e < ce && !sp(self.s[e]) {
            e += 1;
        }
        let name = String::from_utf16_lossy(&self.s[p + 1..e]);
        if self.skip_sp(e, ce) < ce {
            self.err(e, ce, "Unexpected text after the alias");
        }
        self.li = li + 1;
        match self.anchors.get(&name) {
            Some(n) => Node { start: p, end: e, value: n.value.clone() },
            None => {
                self.err(p, e, format!("Unknown alias '*{name}' (the &{name} anchor must be defined above it)"));
                Node { start: p, end: e, value: Value::Null }
            }
        }
    }

    fn plain(&mut self, li: usize, p: usize, parent: isize) -> Node {
        let ce = self.content_end(li, p);
        if let Some(k) = (p..ce).find(|&k| self.s[k] == b':' as u16 && (k + 1 >= ce || sp(self.s[k + 1]))) {
            self.err(k, k + 1, "This value contains ': ': put it in quotes (or move a nested key to its own line)");
        }
        let mut text = String::from_utf16_lossy(&self.s[p..ce]);
        let mut end = ce;
        self.li = li + 1;
        // Multi-line plain scalars: more-indented lines continue the text.
        let mut blanks = 0;
        let mut k = self.li;
        while k < self.lines.len() {
            if self.blank(k) {
                blanks += 1;
                k += 1;
                continue;
            }
            if self.comment_line(k) || self.marker(k).is_some() || self.lines[k].indent as isize <= parent {
                break;
            }
            let l = self.lines[k];
            if self.key_at(k).is_some() {
                self.err(l.text, self.content_end(k, l.text), "This line is indented under a key that already has a value on its line. Remove the value above, or un-indent this line");
                self.skip_tree(k);
                break;
            }
            if self.is_dash(k, l.text) {
                self.err(l.text, l.text + 1, "This list item is indented under a key that already has a value on its line");
                self.skip_tree(k);
                break;
            }
            let lce = self.content_end(k, l.text);
            if blanks > 0 {
                text.push_str(&"\n".repeat(blanks));
            } else {
                text.push(' ');
            }
            text.push_str(&String::from_utf16_lossy(&self.s[l.text..lce]));
            end = lce;
            blanks = 0;
            k += 1;
            self.li = k;
        }
        Node { start: p, end, value: resolve(&text) }
    }

    fn block_scalar(&mut self, li: usize, p: usize, parent: isize) -> Node {
        let l = self.lines[li];
        let literal = self.at(p) == b'|' as u16;
        let mut chomp = 0i8; // -1 strip, 0 clip, 1 keep
        let mut explicit = None;
        let mut k = p + 1;
        while k < l.end && !sp(self.s[k]) {
            match self.s[k] {
                0x2D => chomp = -1,
                0x2B => chomp = 1,
                d @ 0x31..=0x39 => explicit = Some((d - 0x30) as usize),
                _ => {
                    self.err(k, k + 1, "Unexpected character after the block scalar indicator");
                    break;
                }
            }
            k += 1;
        }
        let rest = self.skip_sp(k, l.end);
        if rest < l.end && self.s[rest] != b'#' as u16 {
            self.err(rest, self.content_end(li, rest), "Text after '|' or '>' must start on the next line");
        }
        self.li = li + 1;
        let base = parent.max(-1) + 1;
        let first = (self.li..self.lines.len()).find(|&q| !self.blank(q));
        let ci = match explicit {
            Some(d) => parent.max(0) as usize + d,
            None => match first {
                Some(q) if self.lines[q].indent as isize >= base && self.marker(q).is_none() => self.lines[q].indent,
                _ => usize::MAX,
            },
        };
        let mut body: Vec<String> = Vec::new();
        let mut end = rest.max(k);
        while self.li < self.lines.len() && ci != usize::MAX {
            let q = self.li;
            let lq = self.lines[q];
            if self.blank(q) {
                body.push(String::new());
            } else if lq.indent >= ci && self.marker(q).is_none() {
                body.push(String::from_utf16_lossy(&self.s[lq.start + ci..lq.end]));
                end = lq.end;
            } else {
                break;
            }
            self.li += 1;
        }
        // Trailing blank lines belong to chomping, not to the next node.
        let mut trailing = 0;
        while body.last().is_some_and(|b| b.is_empty()) {
            body.pop();
            trailing += 1;
        }
        let mut text = if literal {
            body.join("\n")
        } else {
            let mut t = String::new();
            for (n, line) in body.iter().enumerate() {
                if n > 0 {
                    let prev = &body[n - 1];
                    let more = line.starts_with(' ') || prev.starts_with(' ');
                    if line.is_empty() || prev.is_empty() || more {
                        t.push('\n');
                    } else {
                        t.push(' ');
                    }
                }
                t.push_str(line);
            }
            t
        };
        if !body.is_empty() {
            match chomp {
                0 => text.push('\n'),
                1 => text.push_str(&"\n".repeat(trailing + 1)),
                _ => {}
            }
        }
        Node { start: p, end, value: Value::Str(text) }
    }

    /// A quoted scalar starting at `p`, possibly spanning lines.
    fn quoted_top(&mut self, p: usize) -> Node {
        let (node, e) = self.quoted(p);
        self.finish_line(e, "the closing quote");
        node
    }

    /// After a value that ended at `e`: the rest of its line must be empty
    /// or a comment.
    fn finish_line(&mut self, e: usize, what: &str) {
        let li = self.line_of(e.saturating_sub(1));
        let end = self.lines[li].end;
        let rest = self.skip_sp(e.min(end), end);
        let ce = self.content_end(li, rest);
        if rest < ce {
            self.err(rest, ce, format!("Unexpected text after {what}"));
        }
        self.li = li + 1;
    }

    fn quoted(&mut self, p: usize) -> (Node, usize) {
        let q = self.s[p];
        let dq = q == b'"' as u16;
        let mut k = p + 1;
        let mut buf: Vec<u16> = Vec::new();
        loop {
            if k >= self.s.len() {
                let le = self.lines[self.line_of(p)].end;
                self.err(p, le, "This quoted string is never closed");
                return (Node { start: p, end: self.s.len(), value: Value::Str(String::from_utf16_lossy(&buf)) }, self.s.len());
            }
            let c = self.s[k];
            if c == 13 || c == 10 {
                // Line folding inside quotes.
                while buf.last().is_some_and(|&b| sp(b)) {
                    buf.pop();
                }
                let mut breaks = 0;
                while k < self.s.len() && (self.s[k] == 13 || self.s[k] == 10 || sp(self.s[k])) {
                    if self.s[k] == 13 || (self.s[k] == 10 && self.at(k - 1) != 13) {
                        breaks += 1;
                    }
                    k += 1;
                }
                let li = self.line_of(k);
                if self.marker(li).is_some() {
                    self.err(p, self.lines[self.line_of(p)].end, "This quoted string is never closed");
                    return (Node { start: p, end: k, value: Value::Str(String::from_utf16_lossy(&buf)) }, k);
                }
                if breaks > 1 {
                    buf.extend(std::iter::repeat_n(10u16, breaks - 1));
                } else {
                    buf.push(32);
                }
                continue;
            }
            if c == q {
                if !dq && self.at(k + 1) == q {
                    buf.push(q);
                    k += 2;
                    continue;
                }
                let value = Value::Str(String::from_utf16_lossy(&buf));
                return (Node { start: p, end: k + 1, value }, k + 1);
            }
            if dq && c == b'\\' as u16 {
                let e = self.at(k + 1);
                let simple = match e {
                    0x30 => Some(0),
                    0x61 => Some(7),
                    0x62 => Some(8),
                    0x74 | 0x09 => Some(9),
                    0x6E => Some(10),
                    0x76 => Some(11),
                    0x66 => Some(12),
                    0x72 => Some(13),
                    0x65 => Some(27),
                    0x20 => Some(32),
                    0x22 => Some(34),
                    0x2F => Some(47),
                    0x5C => Some(92),
                    0x4E => Some(0x85),
                    0x5F => Some(0xA0),
                    _ => None,
                };
                if let Some(v) = simple {
                    buf.push(v);
                    k += 2;
                    continue;
                }
                let digits = match e {
                    0x78 => 2,
                    0x75 => 4,
                    0x55 => 8,
                    _ => 0,
                };
                if digits > 0 {
                    let hex: String = (k + 2..k + 2 + digits).filter_map(|q| char::from_u32(self.at(q) as u32)).collect();
                    if let Some(ch) = u32::from_str_radix(&hex, 16).ok().filter(|_| hex.len() == digits).and_then(char::from_u32) {
                        let mut tmp = [0u16; 2];
                        buf.extend_from_slice(ch.encode_utf16(&mut tmp));
                        k += 2 + digits;
                        continue;
                    }
                }
                if e == 13 || e == 10 {
                    // Escaped line break: join without a space.
                    k += 1;
                    while k < self.s.len() && (self.s[k] == 13 || self.s[k] == 10 || sp(self.s[k])) {
                        k += 1;
                    }
                    continue;
                }
                let shown = char::from_u32(e as u32).unwrap_or('?');
                self.err(k, k + 2, format!("Invalid escape '\\{shown}' (use single quotes, or \\\\ for a backslash)"));
                buf.push(e);
                k += 2;
                continue;
            }
            buf.push(c);
            k += 1;
        }
    }

    fn unquote(&mut self, ks: usize, _ke: usize) -> String {
        let (node, _) = self.quoted(ks);
        match node.value {
            Value::Str(s) => s,
            _ => String::new(),
        }
    }

    fn flow_top(&mut self, p: usize) -> Node {
        let (node, e) = self.flow(p);
        self.finish_line(e, "the closing bracket");
        node
    }

    fn flow_ws(&self, mut k: usize) -> usize {
        while k < self.s.len() {
            let c = self.s[k];
            if c == 13 || c == 10 || sp(c) {
                k += 1;
            } else if c == b'#' as u16 && k > 0 && matches!(self.s[k - 1], 32 | 9 | 13 | 10) {
                while k < self.s.len() && self.s[k] != 13 && self.s[k] != 10 {
                    k += 1;
                }
            } else {
                break;
            }
        }
        k
    }

    fn flow(&mut self, p: usize) -> (Node, usize) {
        let k = self.flow_ws(p);
        let c = self.at(k);
        match c {
            0x5B => {
                let mut items = Vec::new();
                let mut k = k + 1;
                loop {
                    k = self.flow_ws(k);
                    if k >= self.s.len() || self.marker(self.line_of(k)).is_some() && self.lines[self.line_of(k)].start == k {
                        self.err(p, p + 1, "This '[' is never closed (missing ])");
                        return (Node { start: p, end: k, value: Value::Array(items) }, k);
                    }
                    if self.s[k] == b']' as u16 {
                        return (Node { start: p, end: k + 1, value: Value::Array(items) }, k + 1);
                    }
                    let (v, e) = self.flow(k);
                    items.push(v);
                    k = self.flow_ws(e);
                    match self.at(k) {
                        0x2C => k += 1,
                        0x5D => {}
                        _ if k >= self.s.len() => {}
                        _ => {
                            self.err(k, k + 1, "Expected ',' or ']' in this list");
                            let le = self.lines[self.line_of(k)].end;
                            return (Node { start: p, end: le, value: Value::Array(items) }, le);
                        }
                    }
                }
            }
            0x7B => {
                let mut entries: Vec<(Key, Node)> = Vec::new();
                let mut k = k + 1;
                loop {
                    k = self.flow_ws(k);
                    if k >= self.s.len() {
                        self.err(p, p + 1, "This '{' is never closed (missing })");
                        return (Node { start: p, end: k, value: Value::Object(entries) }, k);
                    }
                    if self.s[k] == b'}' as u16 {
                        return (Node { start: p, end: k + 1, value: Value::Object(entries) }, k + 1);
                    }
                    let (kn, ke) = self.flow_scalar(k, true);
                    let name = match &kn.value {
                        Value::Str(s) => s.clone(),
                        Value::Number(n) => n.clone(),
                        Value::Bool(b) => b.to_string(),
                        Value::Null => "null".into(),
                        _ => String::new(),
                    };
                    if entries.iter().any(|(e, _)| e.name == name) {
                        self.err(kn.start, kn.end, format!("Duplicate key '{name}'"));
                    }
                    let key = Key { name, start: kn.start, end: kn.end };
                    let mut k2 = self.flow_ws(ke);
                    let value = if self.at(k2) == b':' as u16 {
                        k2 = self.flow_ws(k2 + 1);
                        if matches!(self.at(k2), 0x2C | 0x7D) {
                            Self::null_at(k2)
                        } else {
                            let (v, e) = self.flow(k2);
                            k2 = e;
                            v
                        }
                    } else {
                        Self::null_at(ke)
                    };
                    entries.push((key, value));
                    k = self.flow_ws(k2);
                    match self.at(k) {
                        0x2C => k += 1,
                        0x7D => {}
                        _ if k >= self.s.len() => {}
                        _ => {
                            self.err(k, k + 1, "Expected ',' or '}' in this mapping");
                            let le = self.lines[self.line_of(k)].end;
                            return (Node { start: p, end: le, value: Value::Object(entries) }, le);
                        }
                    }
                }
            }
            0x22 | 0x27 => self.quoted(k),
            0x2A => {
                let mut e = k + 1;
                while e < self.s.len() && !matches!(self.s[e], 0x2C | 0x5B | 0x5D | 0x7B | 0x7D | 32 | 9 | 13 | 10) {
                    e += 1;
                }
                let name = String::from_utf16_lossy(&self.s[k + 1..e]);
                let value = match self.anchors.get(&name) {
                    Some(n) => n.value.clone(),
                    None => {
                        self.err(k, e, format!("Unknown alias '*{name}'"));
                        Value::Null
                    }
                };
                (Node { start: k, end: e, value }, e)
            }
            _ => self.flow_scalar(k, false),
        }
    }

    /// A plain or quoted scalar inside a flow collection.
    fn flow_scalar(&mut self, k: usize, is_key: bool) -> (Node, usize) {
        let c = self.at(k);
        if c == b'"' as u16 || c == b'\'' as u16 {
            return self.quoted(k);
        }
        let mut e = k;
        while e < self.s.len() {
            let c = self.s[e];
            if matches!(c, 0x2C | 0x5B | 0x5D | 0x7B | 0x7D | 13 | 10) {
                break;
            }
            if c == b':' as u16 && (is_key || matches!(self.at(e + 1), 32 | 9 | 0x2C | 0x5D | 0x7D | 13 | 10)) {
                break;
            }
            if c == b'#' as u16 && e > k && sp(self.s[e - 1]) {
                break;
            }
            e += 1;
        }
        let mut ve = e;
        while ve > k && sp(self.s[ve - 1]) {
            ve -= 1;
        }
        let text = String::from_utf16_lossy(&self.s[k..ve]);
        let value = if is_key { Value::Str(text) } else { resolve(&text) };
        (Node { start: k, end: ve, value }, e)
    }
}

/// A copied node whose spans no longer point into the text (merged keys).
fn detach(mut n: Node) -> Node {
    n.start = usize::MAX;
    n.end = usize::MAX;
    match &mut n.value {
        Value::Array(items) => {
            for v in items.iter_mut() {
                *v = detach(std::mem::replace(v, Node { start: 0, end: 0, value: Value::Null }));
            }
        }
        Value::Object(entries) => {
            for (k, v) in entries.iter_mut() {
                k.start = usize::MAX;
                k.end = usize::MAX;
                *v = detach(std::mem::replace(v, Node { start: 0, end: 0, value: Value::Null }));
            }
        }
        _ => {}
    }
    n
}

/// A plain scalar's value under the YAML 1.2 core schema.
pub fn resolve(t: &str) -> Value {
    match t {
        "" | "~" | "null" | "Null" | "NULL" => return Value::Null,
        "true" | "True" | "TRUE" => return Value::Bool(true),
        "false" | "False" | "FALSE" => return Value::Bool(false),
        ".inf" | ".Inf" | ".INF" | "+.inf" | "-.inf" | "-.Inf" | ".nan" | ".NaN" | ".NAN" => {
            return Value::Number(t.to_string());
        }
        _ => {}
    }
    let b = t.as_bytes();
    let digits = |s: &[u8]| !s.is_empty() && s.iter().all(u8::is_ascii_digit);
    let body = t.strip_prefix(['-', '+']).unwrap_or(t).as_bytes();
    let is_int = digits(body)
        || (t.starts_with("0x") && b.len() > 2 && b[2..].iter().all(u8::is_ascii_hexdigit))
        || (t.starts_with("0o") && b.len() > 2 && b[2..].iter().all(|c| (b'0'..=b'7').contains(c)));
    let is_float = {
        // [-+]? ( \. [0-9]+ | [0-9]+ ( \. [0-9]* )? ) ( [eE] [-+]? [0-9]+ )?
        let (mant, exp) = match body.iter().position(|&c| c == b'e' || c == b'E') {
            Some(i) => (&body[..i], Some(&body[i + 1..])),
            None => (body, None),
        };
        let mant_ok = match mant.iter().position(|&c| c == b'.') {
            Some(i) => (digits(&mant[..i]) || i == 0) && (mant[i + 1..].iter().all(u8::is_ascii_digit)) && (i > 0 || mant.len() > 1),
            None => digits(mant),
        };
        let exp_ok = exp.is_none_or(|e| digits(e.strip_prefix(b"-").or_else(|| e.strip_prefix(b"+")).unwrap_or(e)));
        mant_ok && exp_ok && !body.is_empty()
    };
    if is_int || is_float {
        Value::Number(t.to_string())
    } else {
        Value::Str(t.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{path_at, to_json};

    fn w(s: &str) -> Vec<u16> {
        s.replace("\r\n", "\r").replace('\n', "\r").encode_utf16().collect()
    }

    fn json_of(src: &str) -> String {
        let u = w(src);
        let p = parse(&u);
        assert!(p.problems.is_empty(), "{src}\n{:?}", p.problems);
        to_json(&p.docs[0], None, false)
    }

    fn errors(src: &str) -> Vec<(String, String)> {
        let u = w(src);
        parse(&u)
            .problems
            .iter()
            .map(|e| (String::from_utf16_lossy(&u[e.start.min(u.len())..e.end.min(u.len())]), e.message.clone()))
            .collect()
    }

    #[test]
    fn reads_real_world_yaml() {
        let src = "\
# a deployment
apiVersion: apps/v1
kind: Deployment
metadata:
  name: web   # inline comment
  labels: {app: web, tier: \"front end\"}
spec:
  replicas: 3
  ports: [80, 443]
  containers:
    - name: nginx
      image: nginx:1.25
      args:
      - --verbose
      - \"--port=80\"
      env:
        - {name: A, value: '1'}
    - name: sidecar
      command: >
        run this
        folded
  script: |
    echo hi
    echo # not a comment
  empty:
  nulls: [~, null]
  ok: true
  ratio: 1.5e3
  hex: 0x1F
";
        assert_eq!(
            json_of(src),
            r#"{"apiVersion":"apps/v1","kind":"Deployment","metadata":{"name":"web","labels":{"app":"web","tier":"front end"}},"spec":{"replicas":3,"ports":[80,443],"containers":[{"name":"nginx","image":"nginx:1.25","args":["--verbose","--port=80"],"env":[{"name":"A","value":"1"}]},{"name":"sidecar","command":"run this folded\n"}],"script":"echo hi\necho # not a comment\n","empty":null,"nulls":[null,null],"ok":true,"ratio":1.5e3,"hex":31}}"#
        );
    }

    #[test]
    fn anchors_aliases_merges_and_documents() {
        let src = "defaults: &d\n  retries: 3\n  timeout: 10\nprod:\n  <<: *d\n  timeout: 30\nlist: &l [1, 2]\ncopy: *l\n";
        assert_eq!(
            json_of(src),
            r#"{"defaults":{"retries":3,"timeout":10},"prod":{"timeout":30,"retries":3},"list":[1,2],"copy":[1,2]}"#
        );
        let p = parse(&w("---\na: 1\n---\nb: 2\n...\n"));
        assert_eq!(p.docs.len(), 2);
        assert!(p.problems.is_empty());
    }

    #[test]
    fn scalars_resolve_like_the_core_schema() {
        assert_eq!(resolve("1.0.0"), Value::Str("1.0.0".into()));
        assert_eq!(resolve("-12"), Value::Number("-12".into()));
        assert_eq!(resolve(".5"), Value::Number(".5".into()));
        assert_eq!(resolve("yes"), Value::Str("yes".into()));
        assert_eq!(resolve("~"), Value::Null);
        assert_eq!(json_of("q: \"a\\tb \\u00e9\"\ns: 'it''s'\nm: \"one\n  two\"\n"), r#"{"q":"a\tb é","s":"it's","m":"one two"}"#);
    }

    #[test]
    fn explains_common_mistakes() {
        let cases: &[(&str, &str, &str)] = &[
            ("a:\n\tb: 1\n", "\t", "Tabs"),
            ("a:\n  b: 1\n   c: 2\n", "c: 2", "indented under a key"),
            ("a:\n    b: 1\n  c: 2\n", "c: 2", "Bad indentation"),
            ("a: 1\na: 2\n", "a", "Duplicate key 'a'"),
            ("a: b: c\n", ":", "contains ': '"),
            ("a:1\nb: 2\n", ":", "Missing space after ':'"),
            ("a: \"open\nb: 2\n", "\"open", "never closed"),
            ("a: [1, 2\n", "[", "never closed"),
            ("a: *nope\n", "*nope", "Unknown alias"),
            ("a: 1\n- b\n", "-", "list item"),
            ("a: \"x\" y\n", "y", "after the closing quote"),
            ("a: - b\n", "-", "list can't start"),
            ("key: value\n  child: 1\n", "child: 1", "already has a value"),
            ("a: \"bad \\q\"\n", "\\q", "Invalid escape"),
            ("a: @x\n", "@", "reserved"),
            ("env:\n  - name: MODE\n  value: prod\n", "value: prod", ""),
        ];
        for (src, at, msg) in cases {
            let errs = errors(src);
            assert!(
                errs.iter().any(|(t, m)| t == at && m.contains(msg)),
                "{src:?}: expected {at:?} / {msg:?}, got {errs:?}"
            );
        }
    }

    #[test]
    fn valid_edge_cases_have_no_problems() {
        for src in [
            "url: http://example.com:8080/x\n",
            "path: C:\\Users\\x\n",
            "text: it's fine\n",
            "a:\n- 1\n- 2\nb: 3\n",
            "- a\n- - b\n  - c\n- k: v\n  k2: v2\n",
            "long: this value\n  continues here\n",
            "e: ''\nf: \"\"\n",
            "",
            "# only a comment\n",
            "k: |+\n  keep\n\n",
            "\"quoted key\": 1\n'single': 2\n",
        ] {
            let p = parse(&w(src));
            assert!(p.problems.is_empty(), "{src:?}: {:?}", p.problems);
        }
        assert_eq!(json_of("k: |-\n  a\n  b\n"), r#"{"k":"a\nb"}"#);
        assert_eq!(json_of("k: |+\n  keep\n\n"), r#"{"k":"keep\n\n"}"#);
    }

    #[test]
    fn path_under_caret() {
        let src = "spec:\n  containers:\n    - name: web\n      image: nginx\n";
        let u = w(src);
        let p = parse(&u);
        let at = src.find("nginx").unwrap() + 2;
        assert_eq!(path_at(&p.docs, at).unwrap(), "$.spec.containers[0].image");
    }
}
