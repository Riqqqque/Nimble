use std::path::PathBuf;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Controls::RichEdit::CHARRANGE;

use crate::data::Node;
use crate::encoding::{Encoding, Eol};
use crate::syntax::{Kind, Lang, State};

/// How a document indents: tabs or spaces, and how wide a level is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Indent {
    pub tabs: bool,
    pub size: u8,
}

/// A problem found by the JSON / YAML checker, in document offsets.
#[derive(Clone, Debug)]
pub struct DocProblem {
    pub start: i32,
    pub end: i32,
    pub line: usize,
    pub message: String,
    pub warning: bool,
}

/// One open document: a file path (or untitled) plus its editor control.
/// Each tab owns a dedicated RichEdit so switching tabs preserves undo
/// history, scroll position, and selection for free.
pub struct Document {
    pub edit: HWND,
    pub path: Option<PathBuf>,
    pub encoding: Encoding,
    pub eol: Eol,
    pub zoom: i32,
    /// Logical line starts, used only while word wrap is on (RichEdit's own
    /// line messages count wrapped rows then).
    pub lines: LineIndex,
    /// Logical line count for the gutter width (0 = not yet counted), and
    /// whether edits have happened since.
    pub line_total: usize,
    pub total_stale: bool,
    /// Cached EM_GETMODIFY, so the title and tab repaint only on change.
    pub dirty: bool,
    /// The last two selections reported by EN_SELCHANGE. An edit always
    /// starts at or after the lowest of these and the current selection,
    /// which is how EN_CHANGE knows how much of the line index survives.
    pub sel_prev: CHARRANGE,
    pub sel_last: CHARRANGE,
    /// (word wrap, font pt, dark theme) this editor last applied. Hidden
    /// tabs catch up when shown instead of re-wrapping in the background.
    pub view: Option<(bool, i32, bool)>,

    // ---- code features ----
    pub lang: Lang,
    /// Detected rather than picked by the user (re-detected on Save As).
    pub lang_auto: bool,
    pub indent: Indent,
    /// Lexer state at the start of each logical line, valid as a prefix.
    pub syn: Vec<State>,
    /// The colored window: [start, start + text.len()) and its spans.
    pub painted: Option<(i32, i32)>,
    pub win_text: Vec<u16>,
    pub win_spans: Vec<(i32, i32, Kind)>,
    /// Caret-driven highlights currently shown: (start, end, background).
    pub overlay: Vec<(i32, i32, u8)>,
    pub problems: Vec<DocProblem>,
    /// Parsed JSON / YAML, for the path under the caret.
    pub tree: Vec<Node>,
    /// The checker has run on the current text.
    pub checked: bool,
    /// (indent width << 16 | font pt) the editor's tab stops were set for.
    pub tab_applied: u32,
    /// Text length at the last change notification, so problem offsets past
    /// an edit can shift with the text until the next check.
    pub known_len: i32,
    /// What each character from `styles_at` on was last painted with
    /// (code::UNKNOWN where that isn't known), so repaints only touch
    /// characters whose look changed.
    pub styles: Vec<u16>,
    pub styles_at: i32,
}

impl Document {
    pub fn new(edit: HWND, path: Option<PathBuf>) -> Self {
        Self {
            edit,
            path,
            encoding: Encoding::Utf8,
            eol: Eol::Crlf,
            zoom: 100,
            lines: LineIndex::new(),
            line_total: 0,
            total_stale: false,
            dirty: false,
            sel_prev: CHARRANGE::default(),
            sel_last: CHARRANGE::default(),
            view: None,
            lang: Lang::Plain,
            lang_auto: true,
            indent: Indent { tabs: false, size: 4 },
            syn: vec![State::default()],
            painted: None,
            win_text: Vec::new(),
            win_spans: Vec::new(),
            overlay: Vec::new(),
            problems: Vec::new(),
            tree: Vec::new(),
            checked: false,
            tab_applied: 0,
            known_len: 0,
            styles: Vec::new(),
            styles_at: 0,
        }
    }

    pub fn name(&self) -> String {
        self.path
            .as_deref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".to_string())
    }
}

/// Start offsets of logical lines, valid for the text before `scanned`.
/// Edits truncate it at the first changed offset; lookups extend it only as
/// far as they need, reading the text in bounded chunks. Typing near the
/// viewport therefore costs a few KB of scanning, whatever the file size.
pub struct LineIndex {
    starts: Vec<u32>,
    scanned: i32,
}

/// Characters read per scan step.
const SCAN_CHUNK: i32 = 64 * 1024;

impl LineIndex {
    pub fn new() -> Self {
        Self {
            starts: vec![0],
            scanned: 0,
        }
    }

    /// Forget everything at or after char offset `cp`.
    pub fn invalidate_from(&mut self, cp: i32) {
        let cp = cp.max(0);
        if cp >= self.scanned {
            return;
        }
        // A start equal to cp follows a break before cp, which is intact.
        let keep = self.starts.partition_point(|&s| s as i32 <= cp);
        self.starts.truncate(keep.max(1));
        self.scanned = cp;
    }

    pub fn reset(&mut self) {
        self.starts.truncate(1);
        self.scanned = 0;
    }

    /// Scan forward until offset `cp` is covered (or the text ends).
    /// `read` fills the buffer with text in [from, to).
    fn extend_to(&mut self, cp: i32, len: i32, buf: &mut Vec<u16>, read: &dyn Fn(i32, i32, &mut Vec<u16>)) {
        while self.scanned <= cp && self.scanned < len {
            let from = self.scanned;
            let to = (from + SCAN_CHUNK).min(len);
            read(from, to, buf);
            if buf.is_empty() {
                self.scanned = len;
                break;
            }
            for (k, &c) in buf.iter().enumerate() {
                if c == 13 {
                    self.starts.push(from as u32 + k as u32 + 1);
                }
            }
            self.scanned = from + buf.len() as i32;
        }
    }

    /// (0-based line, its start offset) containing char offset `cp`.
    pub fn line_at(
        &mut self,
        cp: i32,
        len: i32,
        buf: &mut Vec<u16>,
        read: &dyn Fn(i32, i32, &mut Vec<u16>),
    ) -> (usize, i32) {
        self.extend_to(cp, len, buf, read);
        let ln = self.starts.partition_point(|&s| s as i32 <= cp).saturating_sub(1);
        (ln, self.starts[ln] as i32)
    }

    /// Start offset of line `ln`, or None past the last line.
    pub fn line_start(
        &mut self,
        ln: usize,
        len: i32,
        buf: &mut Vec<u16>,
        read: &dyn Fn(i32, i32, &mut Vec<u16>),
    ) -> Option<i32> {
        while self.starts.len() <= ln && self.scanned < len {
            let target = self.scanned;
            self.extend_to(target, len, buf, read);
        }
        self.starts.get(ln).map(|&s| s as i32)
    }

    /// Total line count (scans to the end).
    pub fn count(&mut self, len: i32, buf: &mut Vec<u16>, read: &dyn Fn(i32, i32, &mut Vec<u16>)) -> usize {
        self.extend_to(len, len, buf, read);
        self.starts.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference answer: line starts of `text` computed from scratch.
    fn starts_of(text: &[u16]) -> Vec<i32> {
        std::iter::once(0)
            .chain(text.iter().enumerate().filter(|(_, c)| **c == 13).map(|(k, _)| k as i32 + 1))
            .collect()
    }

    fn reader(text: &[u16]) -> impl Fn(i32, i32, &mut Vec<u16>) + '_ {
        move |a, b, buf: &mut Vec<u16>| {
            buf.clear();
            buf.extend_from_slice(&text[a as usize..b as usize]);
        }
    }

    fn check(ix: &mut LineIndex, text: &[u16]) {
        let want = starts_of(text);
        let len = text.len() as i32;
        let mut buf = Vec::new();
        let read = reader(text);
        for cp in (0..=len).step_by(7).chain([len]) {
            let ln = want.partition_point(|&s| s <= cp) - 1;
            assert_eq!(ix.line_at(cp, len, &mut buf, &read), (ln, want[ln]), "cp {cp}");
        }
        for (ln, &s) in want.iter().enumerate() {
            assert_eq!(ix.line_start(ln, len, &mut buf, &read), Some(s));
        }
        assert_eq!(ix.line_start(want.len(), len, &mut buf, &read), None);
        assert_eq!(ix.count(len, &mut buf, &read), want.len());
    }

    /// Tiny deterministic PRNG so the test needs no dependencies.
    fn rng(seed: &mut u64) -> u64 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        *seed
    }

    #[test]
    fn random_edits_match_full_rescan() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut text: Vec<u16> = Vec::new();
        for _ in 0..20_000 {
            text.push(if rng(&mut seed) % 9 == 0 { 13 } else { b'a' as u16 });
        }
        let mut ix = LineIndex::new();
        check(&mut ix, &text);
        for _ in 0..300 {
            // Replace a random span with random text, as an edit would.
            let at = (rng(&mut seed) as usize) % (text.len() + 1);
            let del = ((rng(&mut seed) as usize) % 40).min(text.len() - at);
            let ins: Vec<u16> = (0..(rng(&mut seed) % 40))
                .map(|_| if rng(&mut seed) % 4 == 0 { 13 } else { b'b' as u16 })
                .collect();
            text.splice(at..at + del, ins);
            // What EN_CHANGE does: trim at (lowest selection seen) - 1.
            ix.invalidate_from(at as i32 - 1);
            check(&mut ix, &text);
        }
    }

    #[test]
    fn empty_and_trailing_break() {
        let mut ix = LineIndex::new();
        check(&mut ix, &[]);
        let text: Vec<u16> = "ab\rcd\r".encode_utf16().collect();
        ix.reset();
        check(&mut ix, &text);
    }
}
