//! Syntax highlighting: language detection and line-at-a-time lexers.
//!
//! Each lexer turns one line into colored spans and returns the state the
//! next line starts in (inside a block comment, a multi-line string, a YAML
//! block scalar, nested brackets...). The editor caches the state at every
//! line start, so coloring the screen only lexes the lines on it.

use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Plain,
    Json,
    Yaml,
    Toml,
    Ini,
    Markdown,
    Xml,
    Html,
    Css,
    JavaScript,
    TypeScript,
    Rust,
    Python,
    C,
    Cpp,
    CSharp,
    Java,
    Go,
    Shell,
    PowerShell,
    Batch,
    Sql,
    Lua,
}

/// Every language, in the order the language picker lists them.
pub const LANGS: &[Lang] = &[
    Lang::Plain,
    Lang::Json,
    Lang::Yaml,
    Lang::Toml,
    Lang::Ini,
    Lang::Markdown,
    Lang::Xml,
    Lang::Html,
    Lang::Css,
    Lang::JavaScript,
    Lang::TypeScript,
    Lang::Rust,
    Lang::Python,
    Lang::C,
    Lang::Cpp,
    Lang::CSharp,
    Lang::Java,
    Lang::Go,
    Lang::Shell,
    Lang::PowerShell,
    Lang::Batch,
    Lang::Sql,
    Lang::Lua,
];

impl Lang {
    pub fn name(self) -> &'static str {
        match self {
            Lang::Plain => "Plain Text",
            Lang::Json => "JSON",
            Lang::Yaml => "YAML",
            Lang::Toml => "TOML",
            Lang::Ini => "INI",
            Lang::Markdown => "Markdown",
            Lang::Xml => "XML",
            Lang::Html => "HTML",
            Lang::Css => "CSS",
            Lang::JavaScript => "JavaScript",
            Lang::TypeScript => "TypeScript",
            Lang::Rust => "Rust",
            Lang::Python => "Python",
            Lang::C => "C",
            Lang::Cpp => "C++",
            Lang::CSharp => "C#",
            Lang::Java => "Java",
            Lang::Go => "Go",
            Lang::Shell => "Shell",
            Lang::PowerShell => "PowerShell",
            Lang::Batch => "Batch",
            Lang::Sql => "SQL",
            Lang::Lua => "Lua",
        }
    }

    /// Language from a file name or extension.
    pub fn from_path(path: &Path) -> Lang {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        match name.as_str() {
            "dockerfile" | "makefile" | ".bashrc" | ".zshrc" | ".profile" | ".bash_profile" => {
                return Lang::Shell;
            }
            ".gitconfig" | ".editorconfig" | ".npmrc" | ".env" => return Lang::Ini,
            _ => {}
        }
        let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
        match ext {
            "json" | "jsonc" | "json5" | "geojson" | "jsonl" | "ndjson" | "webmanifest" | "har" | "ipynb"
            | "babelrc" | "eslintrc" | "prettierrc" => Lang::Json,
            "yml" | "yaml" => Lang::Yaml,
            "toml" => Lang::Toml,
            "ini" | "cfg" | "conf" | "properties" | "inf" | "reg" | "env" | "desktop" | "service" => {
                Lang::Ini
            }
            "md" | "markdown" | "mdown" | "mkd" => Lang::Markdown,
            "xml" | "xaml" | "svg" | "csproj" | "vcxproj" | "props" | "targets" | "plist" | "xsd"
            | "xsl" | "xslt" | "config" | "resx" | "nuspec" | "manifest" | "rss" | "atom" => Lang::Xml,
            "html" | "htm" | "xhtml" | "vue" | "svelte" => Lang::Html,
            "css" | "scss" | "sass" | "less" => Lang::Css,
            "js" | "mjs" | "cjs" | "jsx" => Lang::JavaScript,
            "ts" | "tsx" | "mts" | "cts" => Lang::TypeScript,
            "rs" => Lang::Rust,
            "py" | "pyw" | "pyi" => Lang::Python,
            "c" | "h" => Lang::C,
            "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" | "inl" | "ino" => Lang::Cpp,
            "cs" | "csx" => Lang::CSharp,
            "java" | "kt" | "kts" | "scala" | "groovy" | "gradle" => Lang::Java,
            "go" => Lang::Go,
            "sh" | "bash" | "zsh" | "fish" | "ksh" => Lang::Shell,
            "ps1" | "psm1" | "psd1" => Lang::PowerShell,
            "bat" | "cmd" => Lang::Batch,
            "sql" => Lang::Sql,
            "lua" => Lang::Lua,
            _ => Lang::Plain,
        }
    }

    /// Best guess from the start of a document with no telling extension.
    pub fn sniff(head: &[u16]) -> Lang {
        let s: String = String::from_utf16_lossy(&head[..head.len().min(2048)]);
        let t = s.trim_start_matches('\u{feff}').trim_start();
        let first = t.lines().next().unwrap_or("");
        if first.starts_with("#!") {
            return if first.contains("python") {
                Lang::Python
            } else if first.contains("node") || first.contains("deno") {
                Lang::JavaScript
            } else if first.contains("pwsh") || first.contains("powershell") {
                Lang::PowerShell
            } else {
                Lang::Shell
            };
        }
        if t.starts_with('{') || t.starts_with('[') {
            return Lang::Json;
        }
        if t.starts_with("<?xml") {
            return Lang::Xml;
        }
        if t.len() > 9 && t[..9].eq_ignore_ascii_case("<!doctype") || t.starts_with("<html") {
            return Lang::Html;
        }
        if first.trim_end() == "---" || t.starts_with("%YAML") {
            return Lang::Yaml;
        }
        Lang::Plain
    }

    pub fn line_comment(self) -> Option<&'static str> {
        Some(match self {
            Lang::Rust | Lang::C | Lang::Cpp | Lang::CSharp | Lang::Java | Lang::Go | Lang::JavaScript
            | Lang::TypeScript | Lang::Json => "//",
            Lang::Python | Lang::Yaml | Lang::Toml | Lang::Shell | Lang::PowerShell => "#",
            Lang::Ini => ";",
            Lang::Sql | Lang::Lua => "--",
            Lang::Batch => "REM",
            _ => return None,
        })
    }

    pub fn block_comment(self) -> Option<(&'static str, &'static str)> {
        Some(match self {
            Lang::Rust | Lang::C | Lang::Cpp | Lang::CSharp | Lang::Java | Lang::Go | Lang::JavaScript
            | Lang::TypeScript | Lang::Css | Lang::Sql | Lang::Json => ("/*", "*/"),
            Lang::Xml | Lang::Html | Lang::Markdown => ("<!--", "-->"),
            Lang::PowerShell => ("<#", "#>"),
            Lang::Lua => ("--[[", "]]"),
            _ => return None,
        })
    }

    /// Languages where a line ending in ':' opens a nested block.
    pub fn colon_indents(self) -> bool {
        matches!(self, Lang::Yaml | Lang::Python)
    }

    /// Bracket / quote auto-closing makes sense here (not in prose).
    pub fn is_code(self) -> bool {
        !matches!(self, Lang::Plain | Lang::Markdown)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Comment,
    String,
    Escape,
    Number,
    Keyword,
    Literal,
    Type,
    Function,
    Key,
    Bracket(u8),
    Tag,
    Attr,
    Heading,
    Emphasis,
    Code,
    Link,
    Directive,
    Variable,
    Anchor,
    Punct,
    Error,
}

/// A colored stretch of one line, in UTF-16 offsets from the line start.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
}

/// Lexer state at a line start. Small and Copy so a document can cache one
/// per line.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct State {
    pub mode: u8,
    pub aux: u16,
    pub depth: u16,
}

const NORMAL: u8 = 0;
const BLOCK_COMMENT: u8 = 1; // aux: nesting depth (Rust nests)
const STRING: u8 = 2; // aux: the closing quote (multi-line strings)
const RAW_STRING: u8 = 3; // aux: Rust r#"..."# hash count
const TRIPLE: u8 = 4; // aux: quote char of """ / '''
const BLOCK_SCALAR: u8 = 5; // aux: YAML parent indent
const FENCE: u8 = 6; // aux: Markdown fence char << 8 | length
const MARKUP_COMMENT: u8 = 7;
const MARKUP_TAG: u8 = 8;
const CDATA: u8 = 9;
const VERBATIM: u8 = 10; // C# @"..."
const LUA_LONG: u8 = 11; // aux: level; high bit set = comment

/// Lexes one line. With `out` = None only the next state is computed (the
/// fast path for catching up the per-line state cache).
pub fn lex_line(lang: Lang, st: State, line: &[u16], out: Option<&mut Vec<Span>>) -> State {
    lex_line_within(lang, st, line, out, 0, usize::MAX)
}

/// Like lex_line, but only spans touching [lo, hi) are recorded — for
/// coloring part of a very long line without collecting all of its spans.
pub fn lex_line_within(
    lang: Lang,
    st: State,
    line: &[u16],
    out: Option<&mut Vec<Span>>,
    lo: usize,
    hi: usize,
) -> State {
    let mut lx = Lx { s: line, out, lo, hi };
    match lang {
        Lang::Plain => st,
        Lang::Json => json(&mut lx, st),
        Lang::Yaml => yaml(&mut lx, st),
        Lang::Toml => ini(&mut lx, st, true),
        Lang::Ini => ini(&mut lx, st, false),
        Lang::Markdown => markdown(&mut lx, st),
        Lang::Xml | Lang::Html => markup(&mut lx, st),
        Lang::Css => css(&mut lx, st),
        _ => clike(&mut lx, st, config(lang)),
    }
}

// ---- shared scanning ---------------------------------------------------------

struct Lx<'a> {
    s: &'a [u16],
    out: Option<&'a mut Vec<Span>>,
    /// Spans outside [lo, hi) are dropped.
    lo: usize,
    hi: usize,
}

impl Lx<'_> {
    #[inline]
    fn at(&self, i: usize) -> u16 {
        self.s.get(i).copied().unwrap_or(0)
    }
    #[inline]
    fn emit(&self) -> bool {
        self.out.is_some()
    }
    fn push(&mut self, start: usize, end: usize, kind: Kind) {
        if let Some(o) = self.out.as_mut() {
            if end > start && end > self.lo && start < self.hi {
                o.push(Span { start, end, kind });
            }
        }
    }
    /// ASCII pattern match at `i`.
    fn starts(&self, i: usize, pat: &str) -> bool {
        pat.bytes().enumerate().all(|(k, b)| self.at(i + k) == b as u16)
    }
    fn starts_ci(&self, i: usize, pat: &str) -> bool {
        pat.bytes()
            .enumerate()
            .all(|(k, b)| lower(self.at(i + k)) == b.to_ascii_lowercase() as u16)
    }
    fn find(&self, from: usize, pat: &str) -> Option<usize> {
        (from..self.s.len()).find(|&i| self.starts(i, pat))
    }
    fn skip_ws(&self, mut i: usize) -> usize {
        while i < self.s.len() && is_space(self.s[i]) {
            i += 1;
        }
        i
    }
}

#[inline]
fn lower(c: u16) -> u16 {
    if (b'A' as u16..=b'Z' as u16).contains(&c) { c + 32 } else { c }
}
#[inline]
fn is_space(c: u16) -> bool {
    c == b' ' as u16 || c == b'\t' as u16 || c == 0x0B || c == 0x0C || c == 0xA0
}
#[inline]
fn is_digit(c: u16) -> bool {
    (b'0' as u16..=b'9' as u16).contains(&c)
}
#[inline]
fn is_ident_start(c: u16) -> bool {
    (b'a' as u16..=b'z' as u16).contains(&c)
        || (b'A' as u16..=b'Z' as u16).contains(&c)
        || c == b'_' as u16
        || c >= 0x80 && c != 0xA0
}
#[inline]
fn is_ident(c: u16) -> bool {
    is_ident_start(c) || is_digit(c)
}
#[inline]
fn ch(c: char) -> u16 {
    c as u16
}

fn scan_ident(s: &[u16], mut i: usize) -> usize {
    while i < s.len() && is_ident(s[i]) {
        i += 1;
    }
    i
}

/// Numbers in the usual C-family shapes: 0x1F, 0b1010, 1_000, 3.14e-2, 10u32.
fn scan_number(s: &[u16], mut i: usize) -> usize {
    let at = |k: usize| s.get(k).copied().unwrap_or(0);
    if at(i) == ch('0') && matches!(lower(at(i + 1)), 0x78 | 0x62 | 0x6F) {
        i += 2;
        while i < s.len() && (s[i].is_ascii_hexdigit_u16() || s[i] == ch('_')) {
            i += 1;
        }
    } else {
        while i < s.len() && (is_digit(s[i]) || s[i] == ch('_')) {
            i += 1;
        }
        if at(i) == ch('.') && is_digit(at(i + 1)) {
            i += 1;
            while i < s.len() && (is_digit(s[i]) || s[i] == ch('_')) {
                i += 1;
            }
        }
        if lower(at(i)) == ch('e') && (is_digit(at(i + 1)) || ((at(i + 1) == ch('-') || at(i + 1) == ch('+')) && is_digit(at(i + 2)))) {
            i += 2;
            while i < s.len() && is_digit(s[i]) {
                i += 1;
            }
        }
    }
    // Type suffixes (u8, f64, L, n, m...).
    while i < s.len() && (s[i].is_ascii_alnum_u16()) {
        i += 1;
    }
    i
}

trait U16Ext {
    fn is_ascii_hexdigit_u16(self) -> bool;
    fn is_ascii_alnum_u16(self) -> bool;
}
impl U16Ext for u16 {
    fn is_ascii_hexdigit_u16(self) -> bool {
        is_digit(self) || (b'a' as u16..=b'f' as u16).contains(&lower(self))
    }
    fn is_ascii_alnum_u16(self) -> bool {
        is_digit(self) || (b'a' as u16..=b'z' as u16).contains(&lower(self))
    }
}

/// Compares an identifier against an ASCII word.
fn word_is(w: &[u16], word: &str, ci: bool) -> bool {
    w.len() == word.len()
        && w.iter().zip(word.bytes()).all(|(&a, b)| {
            if ci { lower(a) == b.to_ascii_lowercase() as u16 } else { a == b as u16 }
        })
}

fn in_list(w: &[u16], list: &[&str], ci: bool) -> bool {
    w.len() <= 24 && list.iter().any(|k| word_is(w, k, ci))
}

/// A quoted string from the opening quote at `i`; returns (end, closed).
/// Escape spans are emitted when `esc` is the escape character.
fn scan_string(lx: &mut Lx, i: usize, quote: u16, esc: Option<u16>) -> (usize, bool) {
    let mut j = i + 1;
    while j < lx.s.len() {
        let c = lx.s[j];
        if Some(c) == esc && j + 1 < lx.s.len() {
            lx.push(j, j + 2, Kind::Escape);
            j += 2;
            continue;
        }
        if c == quote {
            return (j + 1, true);
        }
        j += 1;
    }
    (j, false)
}

fn bracket(lx: &mut Lx, st: &mut State, i: usize) -> bool {
    let c = lx.at(i);
    if c == ch('(') || c == ch('[') || c == ch('{') {
        lx.push(i, i + 1, Kind::Bracket((st.depth % 3) as u8));
        st.depth = st.depth.saturating_add(1);
        true
    } else if c == ch(')') || c == ch(']') || c == ch('}') {
        st.depth = st.depth.saturating_sub(1);
        lx.push(i, i + 1, Kind::Bracket((st.depth % 3) as u8));
        true
    } else {
        false
    }
}

// ---- C family (and every language shaped roughly like it) ---------------------

struct CLike {
    line: &'static [&'static str],
    block: Option<(&'static str, &'static str)>,
    nest: bool,
    quotes: &'static str,
    /// Quotes whose strings may continue onto the next line.
    multiline: &'static str,
    esc: u16,
    char_lit: bool,
    triple: bool,
    rust_raw: bool,
    verbatim: bool,
    preproc: bool,
    hash_attr: bool,
    at_attr: bool,
    dollar_var: bool,
    percent_var: bool,
    dash_ops: bool,
    hash_needs_space: bool,
    upper_types: bool,
    macro_bang: bool,
    lua_long: bool,
    ci: bool,
    keywords: &'static [&'static str],
    types: &'static [&'static str],
    literals: &'static [&'static str],
}

const BASE: CLike = CLike {
    line: &["//"],
    block: Some(("/*", "*/")),
    nest: false,
    quotes: "\"'",
    multiline: "",
    esc: b'\\' as u16,
    char_lit: false,
    triple: false,
    rust_raw: false,
    verbatim: false,
    preproc: false,
    hash_attr: false,
    at_attr: false,
    dollar_var: false,
    percent_var: false,
    dash_ops: false,
    hash_needs_space: false,
    upper_types: false,
    macro_bang: false,
    lua_long: false,
    ci: false,
    keywords: &[],
    types: &[],
    literals: &["true", "false", "null"],
};

const C_KW: &[&str] = &[
    "auto", "break", "case", "const", "continue", "default", "do", "else", "enum", "extern", "for",
    "goto", "if", "inline", "register", "restrict", "return", "sizeof", "static", "struct", "switch",
    "typedef", "union", "volatile", "while", "_Alignas", "_Alignof", "_Atomic", "_Generic",
    "_Noreturn", "_Static_assert", "_Thread_local",
];
const C_TYPES: &[&str] = &[
    "char", "double", "float", "int", "long", "short", "signed", "unsigned", "void", "bool", "_Bool",
    "size_t", "ssize_t", "int8_t", "int16_t", "int32_t", "int64_t", "uint8_t", "uint16_t",
    "uint32_t", "uint64_t", "uintptr_t", "intptr_t", "wchar_t", "FILE",
];
const CPP_KW: &[&str] = &[
    "alignas", "alignof", "and", "asm", "auto", "break", "case", "catch", "class", "co_await",
    "co_return", "co_yield", "concept", "const", "const_cast", "consteval", "constexpr",
    "constinit", "continue", "decltype", "default", "delete", "do", "dynamic_cast", "else", "enum",
    "explicit", "export", "extern", "final", "for", "friend", "goto", "if", "inline", "mutable",
    "namespace", "new", "noexcept", "not", "operator", "or", "override", "private", "protected",
    "public", "register", "reinterpret_cast", "requires", "return", "sizeof", "static",
    "static_assert", "static_cast", "struct", "switch", "template", "this", "thread_local", "throw",
    "try", "typedef", "typeid", "typename", "union", "using", "virtual", "volatile", "while", "xor",
];
const CS_KW: &[&str] = &[
    "abstract", "as", "async", "await", "base", "break", "case", "catch", "checked", "class",
    "const", "continue", "default", "delegate", "do", "else", "enum", "event", "explicit",
    "extern", "finally", "fixed", "for", "foreach", "get", "goto", "if", "implicit", "in", "init",
    "interface", "internal", "is", "lock", "nameof", "namespace", "new", "operator", "out",
    "override", "params", "partial", "private", "protected", "public", "readonly", "record",
    "ref", "required", "return", "sealed", "set", "sizeof", "stackalloc", "static", "struct",
    "switch", "this", "throw", "try", "typeof", "unchecked", "unsafe", "using", "var", "virtual",
    "volatile", "when", "where", "while", "with", "yield",
];
const CS_TYPES: &[&str] = &[
    "bool", "byte", "char", "decimal", "double", "dynamic", "float", "int", "long", "nint",
    "nuint", "object", "sbyte", "short", "string", "uint", "ulong", "ushort", "void",
];
const JAVA_KW: &[&str] = &[
    "abstract", "assert", "break", "case", "catch", "class", "const", "continue", "default", "do",
    "else", "enum", "extends", "final", "finally", "for", "fun", "goto", "if", "implements",
    "import", "instanceof", "interface", "native", "new", "package", "permits", "private",
    "protected", "public", "record", "return", "sealed", "static", "strictfp", "super", "switch",
    "synchronized", "this", "throw", "throws", "transient", "try", "val", "var", "volatile",
    "when", "while", "yield", "object", "override", "data", "companion",
];
const JAVA_TYPES: &[&str] = &["boolean", "byte", "char", "double", "float", "int", "long", "short", "void"];
const GO_KW: &[&str] = &[
    "break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough", "for",
    "func", "go", "goto", "if", "import", "interface", "map", "package", "range", "return",
    "select", "struct", "switch", "type", "var",
];
const GO_TYPES: &[&str] = &[
    "any", "bool", "byte", "complex64", "complex128", "error", "float32", "float64", "int", "int8",
    "int16", "int32", "int64", "rune", "string", "uint", "uint8", "uint16", "uint32", "uint64",
    "uintptr",
];
const JS_KW: &[&str] = &[
    "as", "async", "await", "break", "case", "catch", "class", "const", "continue", "debugger",
    "default", "delete", "do", "else", "export", "extends", "finally", "for", "from", "function",
    "get", "if", "import", "in", "instanceof", "let", "new", "of", "return", "set", "static",
    "super", "switch", "this", "throw", "try", "typeof", "var", "void", "while", "with", "yield",
];
const TS_KW: &[&str] = &[
    "abstract", "as", "asserts", "async", "await", "break", "case", "catch", "class", "const",
    "continue", "debugger", "declare", "default", "delete", "do", "else", "enum", "export",
    "extends", "finally", "for", "from", "function", "get", "if", "implements", "import", "in",
    "infer", "instanceof", "interface", "is", "keyof", "let", "module", "namespace", "new", "of",
    "override", "private", "protected", "public", "readonly", "return", "satisfies", "set",
    "static", "super", "switch", "this", "throw", "try", "type", "typeof", "var", "void", "while",
    "with", "yield",
];
const TS_TYPES: &[&str] = &[
    "any", "bigint", "boolean", "never", "number", "object", "string", "symbol", "unknown", "void",
];
const JS_LIT: &[&str] = &["true", "false", "null", "undefined", "NaN", "Infinity"];
const RUST_KW: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
    "return", "self", "Self", "static", "struct", "super", "trait", "type", "union", "unsafe",
    "use", "where", "while", "yield",
];
const RUST_TYPES: &[&str] = &[
    "bool", "char", "f32", "f64", "i8", "i16", "i32", "i64", "i128", "isize", "str", "u8", "u16",
    "u32", "u64", "u128", "usize",
];
const PY_KW: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "case", "class", "continue", "def", "del",
    "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is",
    "lambda", "match", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with",
    "yield", "self", "cls",
];
const PY_TYPES: &[&str] = &[
    "bool", "bytes", "dict", "float", "frozenset", "int", "list", "object", "set", "str", "tuple",
    "type",
];
const PY_LIT: &[&str] = &["True", "False", "None"];
const LUA_KW: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "for", "function", "goto", "if", "in", "local",
    "not", "or", "repeat", "return", "then", "until", "while",
];
const SH_KW: &[&str] = &[
    "alias", "case", "declare", "do", "done", "elif", "else", "esac", "eval", "exec", "exit",
    "export", "fi", "for", "function", "if", "in", "local", "readonly", "return", "select", "set",
    "shift", "source", "then", "time", "trap", "unset", "until", "while",
];
const PS_KW: &[&str] = &[
    "begin", "break", "catch", "class", "continue", "data", "do", "dynamicparam", "else",
    "elseif", "end", "enum", "exit", "filter", "finally", "for", "foreach", "function", "if",
    "in", "param", "process", "return", "switch", "throw", "trap", "try", "until", "using",
    "while",
];
const BAT_KW: &[&str] = &[
    "call", "cd", "chdir", "copy", "del", "do", "echo", "else", "endlocal", "equ", "errorlevel",
    "exist", "exit", "for", "geq", "goto", "gtr", "if", "in", "leq", "lss", "md", "mkdir", "neq",
    "not", "popd", "pushd", "rd", "ren", "set", "setlocal", "shift", "start", "type",
];
const SQL_KW: &[&str] = &[
    "add", "all", "alter", "and", "as", "asc", "begin", "between", "by", "case", "check",
    "column", "commit", "constraint", "create", "cross", "database", "default", "delete", "desc",
    "distinct", "drop", "else", "end", "exists", "foreign", "from", "full", "group", "having",
    "if", "in", "index", "inner", "insert", "into", "is", "join", "key", "left", "like", "limit",
    "not", "offset", "on", "or", "order", "outer", "primary", "references", "returning",
    "right", "rollback", "select", "set", "table", "then", "transaction", "union", "unique",
    "update", "values", "view", "when", "where", "with",
];
const SQL_TYPES: &[&str] = &[
    "bigint", "bit", "blob", "bool", "boolean", "char", "date", "datetime", "decimal", "double",
    "float", "int", "integer", "json", "money", "nchar", "numeric", "nvarchar", "real",
    "serial", "smallint", "text", "time", "timestamp", "uuid", "varchar",
];

fn config(lang: Lang) -> &'static CLike {
    static RUST: CLike = CLike {
        nest: true,
        quotes: "\"",
        char_lit: true,
        rust_raw: true,
        hash_attr: true,
        upper_types: true,
        macro_bang: true,
        keywords: RUST_KW,
        types: RUST_TYPES,
        literals: &["true", "false"],
        ..BASE
    };
    static C: CLike = CLike { preproc: true, keywords: C_KW, types: C_TYPES, literals: &["true", "false", "NULL"], ..BASE };
    static CPP: CLike = CLike {
        preproc: true,
        keywords: CPP_KW,
        types: C_TYPES,
        literals: &["true", "false", "nullptr", "NULL"],
        ..BASE
    };
    static CS: CLike = CLike {
        preproc: true,
        verbatim: true,
        upper_types: true,
        keywords: CS_KW,
        types: CS_TYPES,
        ..BASE
    };
    static JAVA: CLike = CLike { at_attr: true, upper_types: true, multiline: "`", keywords: JAVA_KW, types: JAVA_TYPES, ..BASE };
    static GO: CLike = CLike {
        quotes: "\"'`",
        multiline: "`",
        keywords: GO_KW,
        types: GO_TYPES,
        literals: &["true", "false", "nil", "iota"],
        ..BASE
    };
    static JS: CLike = CLike {
        quotes: "\"'`",
        multiline: "`",
        at_attr: true,
        upper_types: true,
        keywords: JS_KW,
        literals: JS_LIT,
        ..BASE
    };
    static TS: CLike = CLike {
        quotes: "\"'`",
        multiline: "`",
        at_attr: true,
        upper_types: true,
        keywords: TS_KW,
        types: TS_TYPES,
        literals: JS_LIT,
        ..BASE
    };
    static PY: CLike = CLike {
        line: &["#"],
        block: None,
        triple: true,
        at_attr: true,
        upper_types: true,
        keywords: PY_KW,
        types: PY_TYPES,
        literals: PY_LIT,
        ..BASE
    };
    static LUA: CLike = CLike {
        line: &["--"],
        block: None,
        lua_long: true,
        keywords: LUA_KW,
        literals: &["true", "false", "nil"],
        ..BASE
    };
    static SH: CLike = CLike {
        line: &["#"],
        block: None,
        multiline: "\"'",
        dollar_var: true,
        hash_needs_space: true,
        keywords: SH_KW,
        literals: &["true", "false"],
        ..BASE
    };
    static PS: CLike = CLike {
        line: &["#"],
        block: Some(("<#", "#>")),
        multiline: "\"'",
        esc: b'`' as u16,
        dollar_var: true,
        dash_ops: true,
        ci: true,
        keywords: PS_KW,
        literals: &[],
        ..BASE
    };
    static BAT: CLike = CLike {
        line: &["::", "rem ", "@rem "],
        block: None,
        quotes: "\"",
        esc: 0,
        percent_var: true,
        ci: true,
        keywords: BAT_KW,
        literals: &[],
        ..BASE
    };
    static SQL: CLike = CLike {
        line: &["--"],
        quotes: "'\"`",
        esc: 0,
        ci: true,
        keywords: SQL_KW,
        types: SQL_TYPES,
        ..BASE
    };
    match lang {
        Lang::Rust => &RUST,
        Lang::C => &C,
        Lang::Cpp => &CPP,
        Lang::CSharp => &CS,
        Lang::Java => &JAVA,
        Lang::Go => &GO,
        Lang::JavaScript => &JS,
        Lang::TypeScript => &TS,
        Lang::Python => &PY,
        Lang::Lua => &LUA,
        Lang::Shell => &SH,
        Lang::PowerShell => &PS,
        Lang::Batch => &BAT,
        Lang::Sql => &SQL,
        _ => &BASE,
    }
}

fn clike(lx: &mut Lx, mut st: State, c: &CLike) -> State {
    let n = lx.s.len();
    let mut i = 0;
    let esc = if c.esc == 0 { None } else { Some(c.esc) };

    // Constructs carried over from the previous line.
    match st.mode {
        BLOCK_COMMENT => {
            let (open, close) = c.block.unwrap_or(("/*", "*/"));
            let mut depth = st.aux.max(1);
            let mut j = 0;
            while j < n {
                if c.nest && lx.starts(j, open) {
                    depth += 1;
                    j += open.len();
                } else if lx.starts(j, close) {
                    depth -= 1;
                    j += close.len();
                    if depth == 0 {
                        break;
                    }
                } else {
                    j += 1;
                }
            }
            lx.push(0, j, Kind::Comment);
            if depth > 0 {
                return State { mode: BLOCK_COMMENT, aux: depth, ..st };
            }
            st.mode = NORMAL;
            i = j;
        }
        TRIPLE => {
            let q = st.aux;
            match (0..n).find(|&k| lx.at(k) == q && lx.at(k + 1) == q && lx.at(k + 2) == q) {
                Some(k) => {
                    lx.push(0, k + 3, Kind::String);
                    st.mode = NORMAL;
                    i = k + 3;
                }
                None => {
                    lx.push(0, n, Kind::String);
                    return st;
                }
            }
        }
        RAW_STRING => match find_raw_end(lx, 0, st.aux as usize) {
            Some(k) => {
                lx.push(0, k, Kind::String);
                st.mode = NORMAL;
                i = k;
            }
            None => {
                lx.push(0, n, Kind::String);
                return st;
            }
        },
        STRING => {
            // A multi-line string: rescan from a virtual opening quote.
            let (end, closed) = scan_string_from(lx, 0, st.aux, esc);
            lx.push(0, end, Kind::String);
            if !closed {
                return st;
            }
            st.mode = NORMAL;
            i = end;
        }
        VERBATIM => {
            let (end, closed) = scan_verbatim(lx, 0);
            lx.push(0, end, Kind::String);
            if !closed {
                return st;
            }
            st.mode = NORMAL;
            i = end;
        }
        LUA_LONG => {
            let level = (st.aux & 0x7FFF) as usize;
            let kind = if st.aux & 0x8000 != 0 { Kind::Comment } else { Kind::String };
            match find_lua_close(lx, 0, level) {
                Some(k) => {
                    lx.push(0, k, kind);
                    st.mode = NORMAL;
                    i = k;
                }
                None => {
                    lx.push(0, n, kind);
                    return st;
                }
            }
        }
        _ => {}
    }

    let first_code = lx.skip_ws(0);
    while i < n {
        let ch0 = lx.s[i];
        if is_space(ch0) {
            i += 1;
            continue;
        }
        // Line comments.
        if let Some(pat) = c.line.iter().find(|p| {
            if c.ci { lx.starts_ci(i, p) } else { lx.starts(i, p) }
        }) {
            let at_word = !c.hash_needs_space || i == 0 || is_space(lx.at(i - 1));
            let rem_ok = !pat.starts_with("rem") && !pat.starts_with("@rem") || i == first_code;
            if at_word && rem_ok {
                // Lua long comments: --[[ ... ]]
                if c.lua_long && lx.at(i + 2) == ch('[') {
                    if let Some(level) = lua_open_level(lx, i + 2) {
                        let body = i + 2 + level + 2;
                        match find_lua_close(lx, body, level) {
                            Some(k) => {
                                lx.push(i, k, Kind::Comment);
                                i = k;
                                continue;
                            }
                            None => {
                                lx.push(i, n, Kind::Comment);
                                return State { mode: LUA_LONG, aux: 0x8000 | level as u16, ..st };
                            }
                        }
                    }
                }
                lx.push(i, n, Kind::Comment);
                return st;
            }
        }
        // Block comments.
        if let Some((open, close)) = c.block {
            if lx.starts(i, open) {
                let mut depth = 1u16;
                let mut j = i + open.len();
                while j < n {
                    if c.nest && lx.starts(j, open) {
                        depth += 1;
                        j += open.len();
                    } else if lx.starts(j, close) {
                        depth -= 1;
                        j += close.len();
                        if depth == 0 {
                            break;
                        }
                    } else {
                        j += 1;
                    }
                }
                lx.push(i, j, Kind::Comment);
                if depth > 0 {
                    return State { mode: BLOCK_COMMENT, aux: depth, ..st };
                }
                i = j;
                continue;
            }
        }
        // Preprocessor lines and attributes.
        if c.preproc && ch0 == ch('#') && i == first_code {
            let e = scan_ident(lx.s, lx.skip_ws(i + 1));
            lx.push(i, e, Kind::Directive);
            i = e;
            continue;
        }
        if c.hash_attr && ch0 == ch('#') && (lx.at(i + 1) == ch('[') || (lx.at(i + 1) == ch('!') && lx.at(i + 2) == ch('['))) {
            let mut depth = 0;
            let mut j = i;
            while j < n {
                if lx.s[j] == ch('[') {
                    depth += 1;
                } else if lx.s[j] == ch(']') {
                    depth -= 1;
                    if depth == 0 {
                        j += 1;
                        break;
                    }
                }
                j += 1;
            }
            lx.push(i, j, Kind::Directive);
            i = j;
            continue;
        }
        if c.at_attr && ch0 == ch('@') && is_ident_start(lx.at(i + 1)) {
            let e = scan_ident(lx.s, i + 1);
            lx.push(i, e, Kind::Directive);
            i = e;
            continue;
        }
        // Variables.
        if c.dollar_var && ch0 == ch('$') {
            let next = lx.at(i + 1);
            let e = if next == ch('{') {
                lx.find(i, "}").map(|k| k + 1).unwrap_or(n)
            } else if is_ident_start(next) {
                let mut e = scan_ident(lx.s, i + 1);
                // PowerShell scopes: $env:PATH, $script:x
                if c.dash_ops && lx.at(e) == ch(':') && is_ident_start(lx.at(e + 1)) {
                    e = scan_ident(lx.s, e + 1);
                }
                e
            } else if is_digit(next) || b"?#@*!$-".contains(&(next as u8)) && next < 0x80 {
                i + 2
            } else {
                i + 1
            };
            if e > i + 1 {
                lx.push(i, e, Kind::Variable);
                i = e;
                continue;
            }
        }
        if c.percent_var && ch0 == ch('%') {
            let e = if lx.at(i + 1) == ch('%') || lx.at(i + 1) == ch('~') {
                scan_ident(lx.s, i + 2).max(i + 3).min(n)
            } else if is_digit(lx.at(i + 1)) {
                i + 2
            } else {
                lx.find(i + 1, "%").map(|k| k + 1).unwrap_or(i + 1)
            };
            if e > i + 1 {
                lx.push(i, e, Kind::Variable);
                i = e;
                continue;
            }
        }
        // PowerShell operators: -eq, -match, -and...
        if c.dash_ops && ch0 == ch('-') && is_ident_start(lx.at(i + 1)) && (i == 0 || !is_ident(lx.at(i - 1))) {
            let e = scan_ident(lx.s, i + 1);
            lx.push(i, e, Kind::Keyword);
            i = e;
            continue;
        }
        // Strings.
        if c.triple && (ch0 == ch('"') || ch0 == ch('\'')) && lx.at(i + 1) == ch0 && lx.at(i + 2) == ch0 {
            match (i + 3..n).find(|&k| lx.at(k) == ch0 && lx.at(k + 1) == ch0 && lx.at(k + 2) == ch0) {
                Some(k) => {
                    lx.push(i, k + 3, Kind::String);
                    i = k + 3;
                    continue;
                }
                None => {
                    lx.push(i, n, Kind::String);
                    return State { mode: TRIPLE, aux: ch0, ..st };
                }
            }
        }
        if c.verbatim && ch0 == ch('@') && lx.at(i + 1) == ch('"') {
            let (end, closed) = scan_verbatim(lx, i + 2);
            lx.push(i, end, Kind::String);
            if !closed {
                return State { mode: VERBATIM, ..st };
            }
            i = end;
            continue;
        }
        if c.char_lit && ch0 == ch('\'') {
            // Rust: 'a' / '\n' are chars; 'a alone is a lifetime or label.
            let end = if lx.at(i + 1) == ch('\\') {
                lx.find(i + 2, "'").map(|k| k + 1)
            } else if lx.at(i + 2) == ch('\'') {
                Some(i + 3)
            } else {
                None
            };
            match end {
                Some(e) => lx.push(i, e, Kind::String),
                None => {
                    let e = scan_ident(lx.s, i + 1).max(i + 1);
                    lx.push(i, e, Kind::Type);
                    i = e;
                    continue;
                }
            }
            i = end.unwrap();
            continue;
        }
        if c.quotes.encode_utf16().any(|q| q == ch0) {
            let (end, closed) = scan_string(lx, i, ch0, esc);
            lx.push(i, end, Kind::String);
            if !closed && c.multiline.encode_utf16().any(|q| q == ch0) {
                return State { mode: STRING, aux: ch0, ..st };
            }
            i = end;
            continue;
        }
        if c.lua_long && ch0 == ch('[') {
            if let Some(level) = lua_open_level(lx, i) {
                let body = i + level + 2;
                match find_lua_close(lx, body, level) {
                    Some(k) => {
                        lx.push(i, k, Kind::String);
                        i = k;
                        continue;
                    }
                    None => {
                        lx.push(i, n, Kind::String);
                        return State { mode: LUA_LONG, aux: level as u16, ..st };
                    }
                }
            }
        }
        // Numbers.
        if is_digit(ch0) || (ch0 == ch('.') && is_digit(lx.at(i + 1))) {
            if i == 0 || !is_ident(lx.at(i - 1)) {
                let e = scan_number(lx.s, i);
                lx.push(i, e, Kind::Number);
                i = e;
                continue;
            }
        }
        // Identifiers, keywords, calls.
        if is_ident_start(ch0) {
            let e = scan_ident(lx.s, i);
            // Rust raw strings and Python string prefixes.
            if c.rust_raw && (word_is(&lx.s[i..e], "r", false) || word_is(&lx.s[i..e], "br", false))
                && (lx.at(e) == ch('"') || lx.at(e) == ch('#'))
            {
                let mut hashes = 0;
                while lx.at(e + hashes) == ch('#') {
                    hashes += 1;
                }
                if lx.at(e + hashes) == ch('"') {
                    match find_raw_end(lx, e + hashes + 1, hashes) {
                        Some(k) => {
                            lx.push(i, k, Kind::String);
                            i = k;
                            continue;
                        }
                        None => {
                            lx.push(i, n, Kind::String);
                            return State { mode: RAW_STRING, aux: hashes as u16, ..st };
                        }
                    }
                }
            }
            if c.triple && e - i <= 2 && (lx.at(e) == ch('"') || lx.at(e) == ch('\''))
                && lx.s[i..e].iter().all(|&k| b"rRbBfFuU".contains(&(k as u8)) && k < 0x80)
            {
                // Prefix belongs to the string that follows.
                lx.push(i, e, Kind::String);
                i = e;
                continue;
            }
            if lx.emit() {
                let w = &lx.s[i..e];
                let kind = if in_list(w, c.keywords, c.ci) {
                    Some(Kind::Keyword)
                } else if in_list(w, c.literals, c.ci) {
                    Some(Kind::Literal)
                } else if in_list(w, c.types, c.ci) {
                    Some(Kind::Type)
                } else if c.macro_bang && lx.at(e) == ch('!') && lx.at(e + 1) != ch('=') {
                    lx.push(i, e + 1, Kind::Function);
                    i = e + 1;
                    continue;
                } else if lx.at(lx.skip_ws(e)) == ch('(') {
                    Some(Kind::Function)
                } else if c.upper_types && (b'A' as u16..=b'Z' as u16).contains(&w[0])
                    && w.iter().any(|&k| (b'a' as u16..=b'z' as u16).contains(&k))
                {
                    Some(Kind::Type)
                } else {
                    None
                };
                if let Some(k) = kind {
                    lx.push(i, e, k);
                }
            }
            i = e;
            continue;
        }
        if bracket(lx, &mut st, i) {
            i += 1;
            continue;
        }
        i += 1;
    }
    st
}

/// Scans a string body starting at `from` (no opening quote) up to `quote`.
fn scan_string_from(lx: &mut Lx, from: usize, quote: u16, esc: Option<u16>) -> (usize, bool) {
    let mut j = from;
    while j < lx.s.len() {
        let c = lx.s[j];
        if Some(c) == esc && j + 1 < lx.s.len() {
            lx.push(j, j + 2, Kind::Escape);
            j += 2;
            continue;
        }
        if c == quote {
            return (j + 1, true);
        }
        j += 1;
    }
    (j, false)
}

/// C# verbatim string body: "" is an escaped quote.
fn scan_verbatim(lx: &Lx, from: usize) -> (usize, bool) {
    let mut j = from;
    while j < lx.s.len() {
        if lx.s[j] == ch('"') {
            if lx.at(j + 1) == ch('"') {
                j += 2;
                continue;
            }
            return (j + 1, true);
        }
        j += 1;
    }
    (j, false)
}

fn find_raw_end(lx: &Lx, from: usize, hashes: usize) -> Option<usize> {
    (from..lx.s.len()).find(|&k| lx.s[k] == ch('"') && (1..=hashes).all(|h| lx.at(k + h) == ch('#'))).map(|k| k + 1 + hashes)
}

/// Lua long bracket `[==[` at `i`: returns the level (number of '=').
fn lua_open_level(lx: &Lx, i: usize) -> Option<usize> {
    let mut level = 0;
    while lx.at(i + 1 + level) == ch('=') {
        level += 1;
    }
    (lx.at(i) == ch('[') && lx.at(i + 1 + level) == ch('[')).then_some(level)
}

fn find_lua_close(lx: &Lx, from: usize, level: usize) -> Option<usize> {
    (from..lx.s.len())
        .find(|&k| lx.s[k] == ch(']') && (1..=level).all(|h| lx.at(k + h) == ch('=')) && lx.at(k + level + 1) == ch(']'))
        .map(|k| k + level + 2)
}

// ---- JSON ---------------------------------------------------------------------

fn json(lx: &mut Lx, mut st: State) -> State {
    let n = lx.s.len();
    let mut i = 0;
    if st.mode == BLOCK_COMMENT {
        match lx.find(0, "*/") {
            Some(k) => {
                lx.push(0, k + 2, Kind::Comment);
                st.mode = NORMAL;
                i = k + 2;
            }
            None => {
                lx.push(0, n, Kind::Comment);
                return st;
            }
        }
    }
    while i < n {
        let c = lx.s[i];
        if is_space(c) {
            i += 1;
        } else if lx.starts(i, "//") {
            lx.push(i, n, Kind::Comment);
            return st;
        } else if lx.starts(i, "/*") {
            match lx.find(i + 2, "*/") {
                Some(k) => {
                    lx.push(i, k + 2, Kind::Comment);
                    i = k + 2;
                }
                None => {
                    lx.push(i, n, Kind::Comment);
                    return State { mode: BLOCK_COMMENT, ..st };
                }
            }
        } else if c == ch('"') || c == ch('\'') {
            let mut esc = Vec::new();
            let (end, closed) = {
                let mut j = i + 1;
                let mut closed = false;
                while j < n {
                    if lx.s[j] == ch('\\') && j + 1 < n {
                        esc.push(j);
                        j += 2;
                        continue;
                    }
                    if lx.s[j] == c {
                        closed = true;
                        j += 1;
                        break;
                    }
                    j += 1;
                }
                (j, closed)
            };
            let is_key = closed && lx.at(lx.skip_ws(end)) == ch(':');
            let kind = if !closed || c == ch('\'') {
                Kind::Error
            } else if is_key {
                Kind::Key
            } else {
                Kind::String
            };
            lx.push(i, end, kind);
            if kind != Kind::Error {
                for e in esc {
                    lx.push(e, (e + 2).min(end), Kind::Escape);
                }
            }
            i = end;
        } else if is_digit(c) || c == ch('-') && (is_digit(lx.at(i + 1)) || lx.at(i + 1) == ch('.')) {
            let e = scan_number(lx.s, i + (c == ch('-')) as usize);
            lx.push(i, e, Kind::Number);
            i = e;
        } else if is_ident_start(c) {
            let e = scan_ident(lx.s, i);
            let w = &lx.s[i..e];
            let kind = if in_list(w, &["true", "false", "null"], false) {
                Kind::Literal
            } else if in_list(w, &["NaN", "Infinity"], false) {
                Kind::Number
            } else if lx.at(lx.skip_ws(e)) == ch(':') {
                Kind::Key // unquoted key: shown, and flagged by the validator
            } else {
                Kind::Error
            };
            lx.push(i, e, kind);
            i = e;
        } else if bracket(lx, &mut st, i) || c == ch(',') || c == ch(':') {
            i += 1;
        } else {
            lx.push(i, i + 1, Kind::Error);
            i += 1;
        }
    }
    st
}

// ---- YAML ---------------------------------------------------------------------

fn yaml(lx: &mut Lx, mut st: State) -> State {
    let n = lx.s.len();
    let indent = lx.s.iter().take_while(|&&c| c == ch(' ')).count();
    let blank = lx.s[indent..].iter().all(|&c| is_space(c));

    // Block scalar body (| or >): every line indented past the parent.
    if st.mode == BLOCK_SCALAR {
        if blank || indent > st.aux as usize {
            lx.push(0, n, Kind::String);
            return st;
        }
        st.mode = NORMAL;
    }
    // A quoted scalar continuing from the previous line.
    if st.mode == STRING {
        let q = st.aux;
        let mut j = 0;
        let mut closed = false;
        while j < n {
            if q == ch('"') && lx.s[j] == ch('\\') && j + 1 < n {
                j += 2;
                continue;
            }
            if lx.s[j] == q {
                if q == ch('\'') && lx.at(j + 1) == q {
                    j += 2;
                    continue;
                }
                closed = true;
                j += 1;
                break;
            }
            j += 1;
        }
        lx.push(0, j, Kind::String);
        if !closed {
            return st;
        }
        st.mode = NORMAL;
        return yaml_value(lx, st, j, indent);
    }
    // Tabs in indentation are a YAML error.
    let mut i = 0;
    while i < n && (lx.s[i] == ch(' ') || lx.s[i] == ch('\t')) {
        if lx.s[i] == ch('\t') && st.depth == 0 {
            lx.push(i, i + 1, Kind::Error);
        }
        i += 1;
    }
    if i >= n {
        return st;
    }
    if st.depth > 0 {
        // Inside a flow collection that spans lines.
        return yaml_flow(lx, st, i);
    }
    if lx.at(i) == ch('#') {
        lx.push(i, n, Kind::Comment);
        return st;
    }
    if i == 0 && (lx.starts(0, "---") || lx.starts(0, "...")) && (n == 3 || is_space(lx.at(3))) {
        lx.push(0, 3, Kind::Directive);
        return yaml_value(lx, st, 3, 0);
    }
    if i == 0 && lx.at(0) == ch('%') {
        lx.push(0, n, Kind::Directive);
        return st;
    }
    // Sequence dashes and the complex-key marker.
    while (lx.at(i) == ch('-') || lx.at(i) == ch('?')) && (i + 1 >= n || is_space(lx.at(i + 1))) {
        lx.push(i, i + 1, Kind::Punct);
        i = lx.skip_ws(i + 1);
    }
    if i >= n {
        return st;
    }
    // key: value
    let line_indent = indent as u16;
    if let Some((ks, ke, colon)) = yaml_key(lx, i) {
        lx.push(ks, ke, Kind::Key);
        return yaml_value(lx, st, colon + 1, line_indent as usize);
    }
    yaml_value(lx, st, i, line_indent as usize)
}

/// A mapping key starting at `i`: (key start, key end, colon index).
fn yaml_key(lx: &Lx, i: usize) -> Option<(usize, usize, usize)> {
    let n = lx.s.len();
    let c = lx.at(i);
    if c == ch('"') || c == ch('\'') {
        let mut j = i + 1;
        while j < n && lx.s[j] != c {
            if c == ch('"') && lx.s[j] == ch('\\') {
                j += 1;
            }
            j += 1;
        }
        let colon = lx.skip_ws(j + 1);
        if j < n && lx.at(colon) == ch(':') && (colon + 1 >= n || is_space(lx.at(colon + 1))) {
            return Some((i, j + 1, colon));
        }
        return None;
    }
    if b"[]{}&*!|>%@`#,".contains(&(c as u8)) && c < 0x80 {
        return None;
    }
    let mut j = i;
    while j < n {
        if lx.s[j] == ch(':') && (j + 1 >= n || is_space(lx.at(j + 1))) {
            let mut ke = j;
            while ke > i && is_space(lx.s[ke - 1]) {
                ke -= 1;
            }
            return Some((i, ke, j));
        }
        if lx.s[j] == ch('#') && j > i && is_space(lx.s[j - 1]) {
            return None;
        }
        j += 1;
    }
    None
}

/// The value part of a YAML line from `i` on.
fn yaml_value(lx: &mut Lx, mut st: State, i: usize, line_indent: usize) -> State {
    let n = lx.s.len();
    let mut i = lx.skip_ws(i);
    // Anchors, aliases and tags before the value.
    loop {
        let c = lx.at(i);
        if (c == ch('&') || c == ch('*')) && i < n {
            let mut e = i + 1;
            while e < n && !is_space(lx.s[e]) && !b",[]{}".contains(&(lx.s[e] as u8)) {
                e += 1;
            }
            lx.push(i, e, Kind::Anchor);
            i = lx.skip_ws(e);
        } else if c == ch('!') && i < n {
            let mut e = i + 1;
            while e < n && !is_space(lx.s[e]) {
                e += 1;
            }
            lx.push(i, e, Kind::Type);
            i = lx.skip_ws(e);
        } else {
            break;
        }
    }
    if i >= n {
        return st;
    }
    let c = lx.at(i);
    if c == ch('#') {
        lx.push(i, n, Kind::Comment);
        return st;
    }
    // Block scalar header: | |- >+ |2 ...
    if c == ch('|') || c == ch('>') {
        let mut e = i + 1;
        while e < n && (lx.s[e] == ch('-') || lx.s[e] == ch('+') || is_digit(lx.s[e])) {
            e += 1;
        }
        let rest = lx.skip_ws(e);
        if rest >= n || lx.at(rest) == ch('#') {
            lx.push(i, e, Kind::Punct);
            if rest < n {
                lx.push(rest, n, Kind::Comment);
            }
            return State { mode: BLOCK_SCALAR, aux: line_indent as u16, ..st };
        }
    }
    if c == ch('[') || c == ch('{') {
        return yaml_flow(lx, st, i);
    }
    if c == ch('"') || c == ch('\'') {
        let mut j = i + 1;
        let mut closed = false;
        while j < n {
            if c == ch('"') && lx.s[j] == ch('\\') && j + 1 < n {
                lx.push(j, j + 2, Kind::Escape);
                j += 2;
                continue;
            }
            if lx.s[j] == c {
                if c == ch('\'') && lx.at(j + 1) == c {
                    j += 2;
                    continue;
                }
                closed = true;
                j += 1;
                break;
            }
            j += 1;
        }
        lx.push(i, j, Kind::String);
        if !closed {
            return State { mode: STRING, aux: c, ..st };
        }
        let rest = lx.skip_ws(j);
        if lx.at(rest) == ch('#') {
            lx.push(rest, n, Kind::Comment);
        }
        return st;
    }
    // Plain scalar up to a comment.
    let mut e = i;
    while e < n && !(lx.s[e] == ch('#') && e > i && is_space(lx.s[e - 1])) {
        e += 1;
    }
    let mut ve = e;
    while ve > i && is_space(lx.s[ve - 1]) {
        ve -= 1;
    }
    lx.push(i, ve, scalar_kind(&lx.s[i..ve]));
    if e < n {
        lx.push(e, n, Kind::Comment);
    }
    st.mode = NORMAL;
    st
}

/// Flow collections: [a, b] and {k: v}, possibly across lines.
fn yaml_flow(lx: &mut Lx, mut st: State, mut i: usize) -> State {
    let n = lx.s.len();
    while i < n {
        let c = lx.s[i];
        if is_space(c) || c == ch(',') {
            i += 1;
        } else if c == ch('#') && (i == 0 || is_space(lx.s[i - 1])) {
            lx.push(i, n, Kind::Comment);
            break;
        } else if bracket(lx, &mut st, i) {
            i += 1;
            if st.depth == 0 {
                let rest = lx.skip_ws(i);
                if lx.at(rest) == ch('#') {
                    lx.push(rest, n, Kind::Comment);
                }
                break;
            }
        } else if c == ch('"') || c == ch('\'') {
            let (end, _) = scan_string(lx, i, c, (c == ch('"')).then_some(ch('\\')));
            let is_key = lx.at(lx.skip_ws(end)) == ch(':');
            lx.push(i, end, if is_key { Kind::Key } else { Kind::String });
            i = end;
        } else if c == ch(':') {
            i += 1;
        } else if c == ch('&') || c == ch('*') {
            let mut e = i + 1;
            while e < n && !is_space(lx.s[e]) && !b",[]{}".contains(&(lx.s[e] as u8)) {
                e += 1;
            }
            lx.push(i, e, Kind::Anchor);
            i = e;
        } else {
            let mut e = i;
            while e < n && !b",[]{}".contains(&(lx.s[e] as u8)) && !(lx.s[e] == ch(':') && (e + 1 >= n || is_space(lx.at(e + 1)) || b",[]{}".contains(&(lx.at(e + 1) as u8)))) {
                e += 1;
            }
            let mut ve = e;
            while ve > i && is_space(lx.s[ve - 1]) {
                ve -= 1;
            }
            let is_key = lx.at(e) == ch(':');
            lx.push(i, ve, if is_key { Kind::Key } else { scalar_kind(&lx.s[i..ve]) });
            i = e.max(i + 1);
        }
    }
    st
}

/// How a plain scalar resolves under the YAML 1.2 core schema (plus the
/// 1.1 yes/no/on/off booleans many tools still honor).
pub fn scalar_kind(w: &[u16]) -> Kind {
    let t: String = String::from_utf16_lossy(w);
    let lowered = t.to_ascii_lowercase();
    match lowered.as_str() {
        "true" | "false" | "null" | "~" | "yes" | "no" | "on" | "off" => return Kind::Literal,
        ".inf" | "-.inf" | "+.inf" | ".nan" => return Kind::Number,
        _ => {}
    }
    let b = t.as_bytes();
    let numeric = !b.is_empty()
        && (t.parse::<f64>().is_ok() && !lowered.contains("inf") && !lowered.contains("nan")
            || (lowered.starts_with("0x") && b.len() > 2 && b[2..].iter().all(|c| c.is_ascii_hexdigit()))
            || (lowered.starts_with("0o") && b.len() > 2 && b[2..].iter().all(|c| (b'0'..=b'7').contains(c))));
    if numeric { Kind::Number } else { Kind::String }
}

// ---- INI / TOML ----------------------------------------------------------------

fn ini(lx: &mut Lx, mut st: State, toml: bool) -> State {
    let n = lx.s.len();
    let mut i = 0;
    if st.mode == TRIPLE {
        let q = st.aux;
        match (0..n).find(|&k| lx.at(k) == q && lx.at(k + 1) == q && lx.at(k + 2) == q) {
            Some(k) => {
                lx.push(0, k + 3, Kind::String);
                st.mode = NORMAL;
                i = k + 3;
            }
            None => {
                lx.push(0, n, Kind::String);
                return st;
            }
        }
    }
    let start = lx.skip_ws(i);
    if start >= n {
        return st;
    }
    let c0 = lx.at(start);
    if c0 == ch('#') || (!toml && c0 == ch(';')) {
        lx.push(start, n, Kind::Comment);
        return st;
    }
    if st.depth == 0 && i == 0 && c0 == ch('[') {
        // [section] / [[array.of.tables]]
        let end = lx.find(start, "]").map(|k| if lx.at(k + 1) == ch(']') { k + 2 } else { k + 1 }).unwrap_or(n);
        lx.push(start, end, Kind::Heading);
        let rest = lx.skip_ws(end);
        if lx.at(rest) == ch('#') || lx.at(rest) == ch(';') {
            lx.push(rest, n, Kind::Comment);
        }
        return st;
    }
    let mut j = start;
    if st.depth == 0 && i == 0 {
        // key = value (INI also allows key: value)
        let mut k = j;
        while k < n && lx.s[k] != ch('=') && !(!toml && lx.s[k] == ch(':')) {
            k += 1;
        }
        if k < n {
            let mut ke = k;
            while ke > j && is_space(lx.s[ke - 1]) {
                ke -= 1;
            }
            lx.push(j, ke, Kind::Key);
            j = k + 1;
        }
    }
    ini_value(lx, st, j, toml)
}

fn ini_value(lx: &mut Lx, mut st: State, mut i: usize, toml: bool) -> State {
    let n = lx.s.len();
    while i < n {
        let c = lx.s[i];
        if is_space(c) || c == ch(',') || c == ch('=') {
            i += 1;
        } else if c == ch('#') || (!toml && c == ch(';')) {
            lx.push(i, n, Kind::Comment);
            break;
        } else if toml && (c == ch('"') || c == ch('\'')) && lx.at(i + 1) == c && lx.at(i + 2) == c {
            match (i + 3..n).find(|&k| lx.at(k) == c && lx.at(k + 1) == c && lx.at(k + 2) == c) {
                Some(k) => {
                    lx.push(i, k + 3, Kind::String);
                    i = k + 3;
                }
                None => {
                    lx.push(i, n, Kind::String);
                    return State { mode: TRIPLE, aux: c, ..st };
                }
            }
        } else if c == ch('"') || c == ch('\'') {
            let (end, _) = scan_string(lx, i, c, (c == ch('"')).then_some(ch('\\')));
            lx.push(i, end, Kind::String);
            i = end;
        } else if toml && bracket(lx, &mut st, i) {
            i += 1;
        } else if is_ident_start(c) || is_digit(c) || c == ch('-') || c == ch('+') || c == ch('.') {
            let mut e = i;
            while e < n && !is_space(lx.s[e]) && !b",[]{}=#".contains(&(lx.s[e] as u8)) {
                e += 1;
            }
            let w = &lx.s[i..e];
            let inline_key = toml && st.depth > 0 && lx.at(lx.skip_ws(e)) == ch('=');
            let kind = if inline_key {
                Kind::Key
            } else if in_list(w, &["true", "false", "yes", "no", "on", "off"], !toml) {
                Kind::Literal
            } else if is_digit(c) || ((c == ch('-') || c == ch('+')) && is_digit(lx.at(i + 1))) || in_list(w, &["inf", "nan", "+inf", "-inf"], false) {
                Kind::Number
            } else if toml {
                Kind::Error
            } else {
                // INI values are free text: color the rest of the line.
                lx.push(i, n, Kind::String);
                break;
            };
            lx.push(i, e, kind);
            i = e;
        } else {
            i += 1;
        }
    }
    st
}

// ---- Markdown ------------------------------------------------------------------

fn markdown(lx: &mut Lx, mut st: State) -> State {
    let n = lx.s.len();
    let i = lx.skip_ws(0);
    let fence = |lx: &Lx| -> Option<(u16, usize)> {
        let c = lx.at(i);
        if (c == ch('`') || c == ch('~')) && i < 4 {
            let len = lx.s[i..].iter().take_while(|&&k| k == c).count();
            (len >= 3).then_some((c, len))
        } else {
            None
        }
    };
    if st.mode == FENCE {
        let (fc, flen) = ((st.aux >> 8) as u16, (st.aux & 0xFF) as usize);
        if let Some((c, len)) = fence(lx) {
            if c == fc && len >= flen && lx.s[i + len..].iter().all(|&k| is_space(k)) {
                lx.push(0, n, Kind::Code);
                st.mode = NORMAL;
                return st;
            }
        }
        lx.push(0, n, Kind::Code);
        return st;
    }
    if let Some((c, len)) = fence(lx) {
        lx.push(i, n, Kind::Code);
        return State { mode: FENCE, aux: (c << 8) | len.min(255) as u16, ..st };
    }
    if i >= n {
        return st;
    }
    let c = lx.at(i);
    if c == ch('#') {
        let hashes = lx.s[i..].iter().take_while(|&&k| k == ch('#')).count();
        if hashes <= 6 && (i + hashes >= n || is_space(lx.at(i + hashes))) {
            lx.push(i, n, Kind::Heading);
            return st;
        }
    }
    // Horizontal rules: --- *** ___
    let rest: Vec<u16> = lx.s[i..].iter().copied().filter(|&k| !is_space(k)).collect();
    if rest.len() >= 3 && (rest.iter().all(|&k| k == ch('-')) || rest.iter().all(|&k| k == ch('*')) || rest.iter().all(|&k| k == ch('_'))) {
        lx.push(i, n, Kind::Keyword);
        return st;
    }
    let mut j = i;
    if c == ch('>') {
        while lx.at(j) == ch('>') || is_space(lx.at(j)) {
            j += 1;
        }
        lx.push(i, j, Kind::Keyword);
    }
    // List markers: - * + or 1. 1)
    let k = j;
    if (lx.at(k) == ch('-') || lx.at(k) == ch('*') || lx.at(k) == ch('+')) && is_space(lx.at(k + 1)) {
        lx.push(k, k + 1, Kind::Keyword);
        j = k + 2;
        // Task lists: - [ ] / - [x]
        if lx.at(j) == ch('[') && lx.at(j + 2) == ch(']') {
            lx.push(j, j + 3, Kind::Keyword);
            j += 3;
        }
    } else if is_digit(lx.at(k)) {
        let e = (k..n).find(|&q| !is_digit(lx.s[q])).unwrap_or(n);
        if (lx.at(e) == ch('.') || lx.at(e) == ch(')')) && is_space(lx.at(e + 1)) {
            lx.push(k, e + 1, Kind::Keyword);
            j = e + 2;
        }
    }
    md_inline(lx, j);
    st
}

fn md_inline(lx: &mut Lx, mut i: usize) {
    let n = lx.s.len();
    while i < n {
        let c = lx.s[i];
        if c == ch('\\') {
            i += 2;
        } else if c == ch('`') {
            let ticks = lx.s[i..].iter().take_while(|&&k| k == ch('`')).count();
            let close = (i + ticks..n).find(|&q| (0..ticks).all(|t| lx.at(q + t) == ch('`')));
            match close {
                Some(q) => {
                    lx.push(i, q + ticks, Kind::Code);
                    i = q + ticks;
                }
                None => i += ticks,
            }
        } else if (c == ch('*') || c == ch('_')) && i + 1 < n {
            let strong = lx.at(i + 1) == c;
            let w = if strong { 2 } else { 1 };
            let open_ok = !is_space(lx.at(i + w)) && (c == ch('*') || i == 0 || !is_ident(lx.at(i - 1)));
            let close = open_ok
                .then(|| (i + w + 1..n).find(|&q| lx.at(q) == c && (!strong || lx.at(q + 1) == c) && !is_space(lx.at(q - 1))))
                .flatten();
            match close {
                Some(q) => {
                    lx.push(i, q + w, Kind::Emphasis);
                    i = q + w;
                }
                None => i += w,
            }
        } else if c == ch('[') || (c == ch('!') && lx.at(i + 1) == ch('[')) {
            let b = if c == ch('!') { i + 1 } else { i };
            let close = (b + 1..n).find(|&q| lx.at(q) == ch(']'));
            match close {
                Some(q) if lx.at(q + 1) == ch('(') => {
                    let end = (q + 2..n).find(|&r| lx.at(r) == ch(')')).map(|r| r + 1).unwrap_or(n);
                    lx.push(i, q + 1, Kind::Link);
                    lx.push(q + 1, end, Kind::String);
                    i = end;
                }
                Some(q) => {
                    i = q + 1;
                }
                None => i += 1,
            }
        } else if c == ch('<') && (lx.starts(i + 1, "http") || is_ident_start(lx.at(i + 1)) || lx.at(i + 1) == ch('/')) {
            let end = (i + 1..n).find(|&q| lx.at(q) == ch('>')).map(|q| q + 1);
            match end {
                Some(e) => {
                    let kind = if lx.starts(i + 1, "http") { Kind::Link } else { Kind::Tag };
                    lx.push(i, e, kind);
                    i = e;
                }
                None => i += 1,
            }
        } else {
            i += 1;
        }
    }
}

// ---- XML / HTML ------------------------------------------------------------------

fn markup(lx: &mut Lx, mut st: State) -> State {
    let n = lx.s.len();
    let mut i = 0;
    match st.mode {
        MARKUP_COMMENT => match lx.find(0, "-->") {
            Some(k) => {
                lx.push(0, k + 3, Kind::Comment);
                st.mode = NORMAL;
                i = k + 3;
            }
            None => {
                lx.push(0, n, Kind::Comment);
                return st;
            }
        },
        CDATA => match lx.find(0, "]]>") {
            Some(k) => {
                lx.push(0, k + 3, Kind::String);
                st.mode = NORMAL;
                i = k + 3;
            }
            None => {
                lx.push(0, n, Kind::String);
                return st;
            }
        },
        MARKUP_TAG => {
            let (e, done) = markup_attrs(lx, 0);
            i = e;
            if !done {
                return st;
            }
            st.mode = NORMAL;
        }
        _ => {}
    }
    while i < n {
        let c = lx.s[i];
        if c == ch('<') {
            if lx.starts(i, "<!--") {
                match lx.find(i + 4, "-->") {
                    Some(k) => {
                        lx.push(i, k + 3, Kind::Comment);
                        i = k + 3;
                    }
                    None => {
                        lx.push(i, n, Kind::Comment);
                        return State { mode: MARKUP_COMMENT, ..st };
                    }
                }
            } else if lx.starts(i, "<![CDATA[") {
                match lx.find(i + 9, "]]>") {
                    Some(k) => {
                        lx.push(i, k + 3, Kind::String);
                        i = k + 3;
                    }
                    None => {
                        lx.push(i, n, Kind::String);
                        return State { mode: CDATA, ..st };
                    }
                }
            } else if lx.at(i + 1) == ch('!') || lx.at(i + 1) == ch('?') {
                let e = lx.find(i, ">").map(|k| k + 1).unwrap_or(n);
                lx.push(i, e, Kind::Directive);
                i = e;
            } else if is_ident_start(lx.at(i + 1)) || lx.at(i + 1) == ch('/') {
                let name = if lx.at(i + 1) == ch('/') { i + 2 } else { i + 1 };
                let mut e = name;
                while e < n && (is_ident(lx.s[e]) || lx.s[e] == ch('-') || lx.s[e] == ch(':') || lx.s[e] == ch('.')) {
                    e += 1;
                }
                lx.push(i, name, Kind::Punct);
                lx.push(name, e, Kind::Tag);
                let (after, done) = markup_attrs(lx, e);
                i = after;
                if !done {
                    return State { mode: MARKUP_TAG, ..st };
                }
            } else {
                i += 1;
            }
        } else if c == ch('&') {
            let e = (i + 1..n.min(i + 12)).find(|&k| lx.s[k] == ch(';'));
            match e {
                Some(k) => {
                    lx.push(i, k + 1, Kind::Escape);
                    i = k + 1;
                }
                None => i += 1,
            }
        } else {
            i += 1;
        }
    }
    st
}

/// Attributes inside a tag; returns (index after '>', closed).
fn markup_attrs(lx: &mut Lx, mut i: usize) -> (usize, bool) {
    let n = lx.s.len();
    while i < n {
        let c = lx.s[i];
        if c == ch('>') {
            lx.push(i, i + 1, Kind::Punct);
            return (i + 1, true);
        } else if c == ch('/') && lx.at(i + 1) == ch('>') {
            lx.push(i, i + 2, Kind::Punct);
            return (i + 2, true);
        } else if c == ch('"') || c == ch('\'') {
            let (end, _) = scan_string(lx, i, c, None);
            lx.push(i, end, Kind::String);
            i = end;
        } else if is_ident_start(c) {
            let mut e = i;
            while e < n && (is_ident(lx.s[e]) || lx.s[e] == ch('-') || lx.s[e] == ch(':') || lx.s[e] == ch('.')) {
                e += 1;
            }
            lx.push(i, e, Kind::Attr);
            i = e;
        } else {
            i += 1;
        }
    }
    (n, false)
}

// ---- CSS ---------------------------------------------------------------------------

fn css(lx: &mut Lx, mut st: State) -> State {
    let n = lx.s.len();
    let mut i = 0;
    if st.mode == BLOCK_COMMENT {
        match lx.find(0, "*/") {
            Some(k) => {
                lx.push(0, k + 2, Kind::Comment);
                st.mode = NORMAL;
                i = k + 2;
            }
            None => {
                lx.push(0, n, Kind::Comment);
                return st;
            }
        }
    }
    // Inside braces a line is a declaration unless it opens a nested rule.
    let opens_rule = (i..n).any(|k| lx.s[k] == ch('{'));
    while i < n {
        let c = lx.s[i];
        if is_space(c) || c == ch(';') || c == ch(',') {
            i += 1;
        } else if lx.starts(i, "/*") {
            match lx.find(i + 2, "*/") {
                Some(k) => {
                    lx.push(i, k + 2, Kind::Comment);
                    i = k + 2;
                }
                None => {
                    lx.push(i, n, Kind::Comment);
                    return State { mode: BLOCK_COMMENT, ..st };
                }
            }
        } else if lx.starts(i, "//") {
            lx.push(i, n, Kind::Comment);
            break;
        } else if c == ch('{') || c == ch('}') {
            bracket(lx, &mut st, i);
            i += 1;
        } else if c == ch('@') {
            let e = scan_css_ident(lx.s, i + 1);
            lx.push(i, e, Kind::Directive);
            i = e;
        } else if c == ch('"') || c == ch('\'') {
            let (end, _) = scan_string(lx, i, c, Some(ch('\\')));
            lx.push(i, end, Kind::String);
            i = end;
        } else if st.depth > 0 && !opens_rule {
            // property: value
            let e = scan_css_ident(lx.s, i);
            if e > i && lx.at(lx.skip_ws(e)) == ch(':') {
                lx.push(i, e, Kind::Attr);
                i = lx.skip_ws(e) + 1;
                i = css_value(lx, i);
            } else {
                i = css_value(lx, i);
            }
        } else {
            // Selectors.
            let kind = match c {
                0x2E | 0x23 => Some(Kind::Type), // . #
                0x3A => Some(Kind::Keyword),     // :hover ::before
                _ if is_ident_start(c) => Some(Kind::Tag),
                _ => None,
            };
            let start = i;
            let mut e = if c == ch(':') { i + 1 + (lx.at(i + 1) == ch(':')) as usize } else if kind == Some(Kind::Type) { i + 1 } else { i };
            e = scan_css_ident(lx.s, e);
            if e == start {
                i += 1;
                continue;
            }
            if let Some(k) = kind {
                lx.push(start, e, k);
            }
            i = e;
        }
    }
    st
}

fn scan_css_ident(s: &[u16], mut i: usize) -> usize {
    while i < s.len() && (is_ident(s[i]) || s[i] == ch('-')) {
        i += 1;
    }
    i
}

fn css_value(lx: &mut Lx, mut i: usize) -> usize {
    let n = lx.s.len();
    while i < n {
        let c = lx.s[i];
        if c == ch(';') || c == ch('}') || lx.starts(i, "/*") {
            return i;
        } else if c == ch('#') {
            let e = scan_css_ident(lx.s, i + 1);
            lx.push(i, e, Kind::Number);
            i = e;
        } else if is_digit(c) || (c == ch('.') && is_digit(lx.at(i + 1))) || (c == ch('-') && is_digit(lx.at(i + 1))) {
            let mut e = i + 1;
            while e < n && (is_digit(lx.s[e]) || lx.s[e] == ch('.')) {
                e += 1;
            }
            while e < n && (is_ident_start(lx.s[e]) || lx.s[e] == ch('%')) {
                e += 1;
            }
            lx.push(i, e, Kind::Number);
            i = e;
        } else if c == ch('"') || c == ch('\'') {
            let (end, _) = scan_string(lx, i, c, Some(ch('\\')));
            lx.push(i, end, Kind::String);
            i = end;
        } else if c == ch('!') {
            let e = scan_css_ident(lx.s, i + 1);
            lx.push(i, e, Kind::Keyword);
            i = e;
        } else if is_ident_start(c) || c == ch('-') {
            let e = scan_css_ident(lx.s, i);
            if lx.at(e) == ch('(') {
                lx.push(i, e, Kind::Function);
            } else {
                lx.push(i, e, Kind::Literal);
            }
            i = e.max(i + 1);
        } else {
            i += 1;
        }
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    /// Lexes `lines` and returns (text, kind) for each span.
    fn spans(lang: Lang, lines: &[&str]) -> Vec<Vec<(String, Kind)>> {
        let mut st = State::default();
        lines
            .iter()
            .map(|l| {
                let u = w(l);
                let mut out = Vec::new();
                st = lex_line(lang, st, &u, Some(&mut out));
                out.iter().map(|s| (String::from_utf16_lossy(&u[s.start..s.end]), s.kind)).collect()
            })
            .collect()
    }

    fn has(v: &[(String, Kind)], text: &str, kind: Kind) -> bool {
        v.iter().any(|(t, k)| t == text && *k == kind)
    }

    #[test]
    fn json_keys_values_and_rainbow() {
        let s = spans(Lang::Json, &[r#"{"name": "nimble", "n": -1.5e3, "ok": true, "x": null,"#, r#"  "list": [1, [2]]}"#]);
        assert!(has(&s[0], "\"name\"", Kind::Key));
        assert!(has(&s[0], "\"nimble\"", Kind::String));
        assert!(has(&s[0], "-1.5e3", Kind::Number));
        assert!(has(&s[0], "true", Kind::Literal));
        assert!(has(&s[0], "{", Kind::Bracket(0)));
        assert!(has(&s[1], "[", Kind::Bracket(1)));
        assert!(has(&s[1], "[", Kind::Bracket(2)));
        assert!(has(&s[1], "}", Kind::Bracket(0)));
    }

    #[test]
    fn json_marks_mistakes() {
        let s = spans(Lang::Json, &["{'a': 1, b: 2, \"c\": tru}"]);
        assert!(has(&s[0], "'a'", Kind::Error));
        assert!(has(&s[0], "tru", Kind::Error));
        assert!(has(&s[0], "b", Kind::Key));
    }

    #[test]
    fn yaml_structure() {
        let s = spans(Lang::Yaml, &[
            "# config",
            "name: nimble   # trailing",
            "version: 2",
            "enabled: yes",
            "tags: [a, \"b\"]",
            "items:",
            "  - id: 1",
            "    path: \"C:\\\\x\"",
            "base: &base",
            "other: *base",
            "script: |",
            "  echo hi: there",
            "  # not a comment",
            "after: done",
            "\tbad: tab",
        ]);
        assert!(has(&s[0], "# config", Kind::Comment));
        assert!(has(&s[1], "name", Kind::Key));
        assert!(has(&s[1], "nimble", Kind::String));
        assert!(has(&s[1], "# trailing", Kind::Comment));
        assert!(has(&s[2], "2", Kind::Number));
        assert!(has(&s[3], "yes", Kind::Literal));
        assert!(has(&s[4], "a", Kind::String));
        assert!(has(&s[4], "[", Kind::Bracket(0)));
        assert!(has(&s[6], "id", Kind::Key));
        assert!(has(&s[7], "\\\\", Kind::Escape));
        assert!(has(&s[8], "&base", Kind::Anchor));
        assert!(has(&s[9], "*base", Kind::Anchor));
        assert!(has(&s[11], "  echo hi: there", Kind::String));
        assert!(has(&s[12], "  # not a comment", Kind::String));
        assert!(has(&s[13], "after", Kind::Key));
        assert!(has(&s[14], "\t", Kind::Error));
    }

    #[test]
    fn rust_tokens_and_multiline_states() {
        let s = spans(Lang::Rust, &[
            "#[derive(Debug)]",
            "pub fn main() -> Result<(), String> {",
            "    let s = r#\"raw \"text\"\"#; /* open",
            "    still comment */ println!(\"{}\", 'c');",
            "}",
        ]);
        assert!(has(&s[0], "#[derive(Debug)]", Kind::Directive));
        assert!(has(&s[1], "fn", Kind::Keyword));
        assert!(has(&s[1], "main", Kind::Function));
        assert!(has(&s[1], "Result", Kind::Type));
        assert!(has(&s[2], "r#\"raw \"text\"\"#", Kind::String));
        assert!(has(&s[2], "/* open", Kind::Comment));
        assert!(has(&s[3], "    still comment */", Kind::Comment));
        assert!(has(&s[3], "println!", Kind::Function));
        assert!(has(&s[3], "'c'", Kind::String));
    }

    #[test]
    fn python_triple_quotes_span_lines() {
        let s = spans(Lang::Python, &["def f(x):", "    \"\"\"doc", "    more\"\"\" # c", "    return None"]);
        assert!(has(&s[0], "def", Kind::Keyword));
        assert!(has(&s[1], "\"\"\"doc", Kind::String));
        assert!(has(&s[2], "    more\"\"\"", Kind::String));
        assert!(has(&s[2], "# c", Kind::Comment));
        assert!(has(&s[3], "None", Kind::Literal));
    }

    #[test]
    fn markup_and_markdown() {
        let x = spans(Lang::Xml, &["<a href=\"x\"><!-- c", "--> &amp; </a>"]);
        assert!(has(&x[0], "a", Kind::Tag));
        assert!(has(&x[0], "href", Kind::Attr));
        assert!(has(&x[0], "\"x\"", Kind::String));
        assert!(has(&x[1], "-->", Kind::Comment));
        assert!(has(&x[1], "&amp;", Kind::Escape));
        let m = spans(Lang::Markdown, &["# Title", "Some `code` and **bold**", "```", "# not heading", "```"]);
        assert!(has(&m[0], "# Title", Kind::Heading));
        assert!(has(&m[1], "`code`", Kind::Code));
        assert!(has(&m[1], "**bold**", Kind::Emphasis));
        assert!(has(&m[3], "# not heading", Kind::Code));
    }

    #[test]
    fn toml_and_ini() {
        let t = spans(Lang::Toml, &["[package]", "name = \"nimble\"", "edition = 2024", "list = [1, 2] # c"]);
        assert!(has(&t[0], "[package]", Kind::Heading));
        assert!(has(&t[1], "name", Kind::Key));
        assert!(has(&t[2], "2024", Kind::Number));
        assert!(has(&t[3], "# c", Kind::Comment));
        let i = spans(Lang::Ini, &["; c", "[s]", "path=C:\\x y"]);
        assert!(has(&i[0], "; c", Kind::Comment));
        assert!(has(&i[2], "path", Kind::Key));
        assert!(has(&i[2], "C:\\x y", Kind::String));
    }

    #[test]
    fn detection() {
        assert_eq!(Lang::from_path(Path::new("a/b/config.YAML")), Lang::Yaml);
        assert_eq!(Lang::from_path(Path::new("x.jsonc")), Lang::Json);
        assert_eq!(Lang::from_path(Path::new("Dockerfile")), Lang::Shell);
        assert_eq!(Lang::sniff(&w("  {\"a\":1}")), Lang::Json);
        assert_eq!(Lang::sniff(&w("---\nkey: v")), Lang::Yaml);
        assert_eq!(Lang::sniff(&w("#!/usr/bin/env python3\n")), Lang::Python);
        assert_eq!(Lang::sniff(&w("hello")), Lang::Plain);
    }

    #[test]
    fn state_only_pass_matches_full_pass() {
        let lines = ["/* a", "b */ x = \"y", "z\" + `t", "u` /* c */"];
        let mut a = State::default();
        let mut b = State::default();
        for l in lines {
            let u = w(l);
            let mut out = Vec::new();
            a = lex_line(Lang::JavaScript, a, &u, Some(&mut out));
            b = lex_line(Lang::JavaScript, b, &u, None);
            assert_eq!(a, b);
        }
    }
}
