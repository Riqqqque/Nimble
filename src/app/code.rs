//! Code-reading features on top of the editor: syntax coloring of the text
//! on screen, matching-bracket and same-word highlights, JSON / YAML
//! checking with squiggles, and the editing commands coders expect.
//!
//! Colors are RichEdit *temporary* formatting: they live beside the text,
//! so they never touch undo history, the modified flag, or what's saved.

use super::*;
use crate::data;
use crate::document::{DocProblem, Indent};
use crate::syntax::{self, Kind, Lang, State};

/// Documents larger than this (in characters) aren't colored...
pub(super) const HIGHLIGHT_LIMIT: i32 = 8 << 20;
/// ...or checked.
pub(super) const CHECK_LIMIT: i32 = 4 << 20;
/// Parsed trees (for the caret path) are kept up to this size.
const TREE_LIMIT: i32 = 2 << 20;
/// Characters colored beyond the screen's edges on very long lines.
const WINDOW_SLACK: i32 = 16 * 1024;
/// Re-check JSON / YAML this long after typing stops.
pub(super) const TIMER_CHECK: usize = 2;
const CHECK_DELAY_MS: u32 = 350;

// Style bits for a character: fore color code (low 6 bits), background
// code, and a wavy underline.
const BG_SHIFT: u16 = 6;
const BG_MATCH: u16 = 1;
const BG_MISMATCH: u16 = 2;
const BG_OCCURRENCE: u16 = 3;
const WAVE: u16 = 1 << 9;
/// Style cache entry for "not known what's painted here".
const UNKNOWN: u16 = u16::MAX;
const FORE_ERROR: u16 = 40;
const FORE_WARN: u16 = 41;

fn fore_code(k: Kind) -> u16 {
    match k {
        Kind::Comment => 1,
        Kind::String => 2,
        Kind::Escape => 3,
        Kind::Number => 4,
        Kind::Keyword => 5,
        Kind::Literal => 6,
        Kind::Type => 7,
        Kind::Function => 8,
        Kind::Key => 9,
        Kind::Bracket(d) => 10 + d as u16,
        Kind::Tag => 13,
        Kind::Attr => 14,
        Kind::Heading => 15,
        Kind::Emphasis => 16,
        Kind::Code => 17,
        Kind::Link => 18,
        Kind::Directive => 19,
        Kind::Variable => 20,
        Kind::Anchor => 21,
        Kind::Punct => 22,
        Kind::Error => FORE_ERROR,
    }
}

/// RichEdit temporary formatting on one reusable TOM range. Temporary
/// formatting merges: properties left unset keep whatever an earlier pass
/// put there, and there's no reliable way to remove it. So every pass sets
/// all three properties, and "plain" means the theme's own colors.
struct Painter {
    range: ITextRange,
    font: ITextFont,
}

impl Painter {
    fn new(doc: &ITextDocument2) -> Option<Self> {
        unsafe {
            let range = doc.Range(0, 0).ok()?;
            let font = range.GetFont().ok()?;
            Some(Self { range, font })
        }
    }

    fn paint(&self, a: i32, b: i32, fore: COLORREF, back: COLORREF, wave: bool) {
        unsafe {
            let _ = self.range.SetRange(a, b);
            let _ = self.font.Reset(tomApplyTmp);
            let _ = self.font.SetForeColor(fore.0 as i32);
            let _ = self.font.SetBackColor(back.0 as i32);
            let _ = self.font.SetUnderline(if wave { tomWave.0 } else { tomNone.0 });
            let _ = self.font.Reset(tomApplyNow);
        }
    }
}

fn is_word(c: u16) -> bool {
    (c as u8).is_ascii_alphanumeric() && c < 0x80 || c == b'_' as u16 || c >= 0xC0 && c != 0xD7 && c != 0xF7
}

impl App {
    // ---- language and indentation -----------------------------------------

    /// Picks the language and indentation for a freshly loaded document.
    pub(super) fn detect_code(&mut self, i: usize) {
        let edit = self.docs[i].edit;
        let len = text_len(edit);
        let mut head = Vec::new();
        get_range_into(edit, 0, len.min(64 * 1024), &mut head);
        let d = &mut self.docs[i];
        if d.lang_auto {
            d.lang = guess_lang(d.path.as_deref(), &head);
        }
        // Files with no indentation yet: code follows the setting, plain text
        // keeps real tab characters like Notepad.
        let tabs = !self.insert_spaces || d.lang == Lang::Plain;
        d.indent = detect_indent(&head).unwrap_or(Indent { tabs, size: self.tab_size as u8 });
        d.known_len = len;
        self.reset_code(i);
    }

    /// Forget all cached coloring and problems for doc `i` and redo them.
    pub(super) fn reset_code(&mut self, i: usize) {
        let d = &mut self.docs[i];
        d.syn.truncate(1);
        d.checked = false;
        if self.docs[i].painted.is_some() || !self.docs[i].overlay.is_empty() {
            self.clear_colors(i);
        }
        self.apply_tab_stops(i);
        self.schedule_check();
        if self.active == Some(i) {
            self.highlight(None);
        }
    }

    fn clear_colors(&mut self, i: usize) {
        let edit = self.docs[i].edit;
        if let Some(doc) = text_document(edit) {
            if let Some(p) = Painter::new(&doc) {
                p.paint(0, text_len(edit), self.theme.edit_fg, self.theme.edit_bg, false);
            }
        }
        let d = &mut self.docs[i];
        d.painted = None;
        d.win_spans.clear();
        d.win_text = Vec::new();
        d.overlay.clear();
        d.styles = Vec::new();
    }

    /// Tab stops every `size` columns of the editor font. TOM's default tab
    /// stop is in points, so it follows zoom by itself and only needs
    /// redoing when the font size or indent width changes.
    pub(super) fn apply_tab_stops(&mut self, i: usize) {
        let size = self.docs[i].indent.size as u32;
        let key = size << 16 | self.edit_pt as u32;
        if self.docs[i].tab_applied == key {
            return;
        }
        let edit = self.docs[i].edit;
        let Some(doc) = text_document(edit) else { return };
        let pts = size as f32 * self.char_em * self.edit_pt as f32;
        let was = sm(edit, EM_GETMODIFY, 0, 0).0;
        unsafe {
            let _ = doc.SetDefaultTabStop(pts);
        }
        sm(edit, EM_SETMODIFY, was as usize, 0);
        self.docs[i].tab_applied = key;
    }

    /// After Save As: a detected language follows the new file name.
    pub(super) fn redetect_language(&mut self, i: usize) {
        let d = &self.docs[i];
        if !d.lang_auto {
            return;
        }
        let mut head = Vec::new();
        get_range_into(d.edit, 0, text_len(d.edit).min(64 * 1024), &mut head);
        let lang = guess_lang(d.path.as_deref(), &head);
        if lang != self.docs[i].lang {
            self.docs[i].lang = lang;
            self.reset_code(i);
            self.update_status();
        }
    }

    pub(super) fn set_language(&mut self, i: usize, lang: Option<Lang>) {
        let d = &mut self.docs[i];
        match lang {
            Some(l) => {
                d.lang = l;
                d.lang_auto = false;
            }
            None => d.lang_auto = true,
        }
        if self.docs[i].lang_auto {
            self.detect_code(i);
        } else {
            self.reset_code(i);
        }
        self.update_status();
    }

    pub(super) fn set_indent(&mut self, i: usize, ind: Indent) {
        self.docs[i].indent = ind;
        self.apply_tab_stops(i);
        self.update_status();
    }

    /// Rewrites leading whitespace with the document's indent width:
    /// spaces to tabs or tabs to spaces.
    pub(super) fn convert_indentation(&mut self, i: usize, to_tabs: bool) {
        let size = self.docs[i].indent.size as usize;
        let text = self.doc_text(i);
        let mut out = Vec::with_capacity(text.len());
        for (n, line) in text.split(|&c| c == 13).enumerate() {
            if n > 0 {
                out.push(13);
            }
            let lead = line.iter().take_while(|&&c| c == 32 || c == 9).count();
            let cols: usize = line[..lead].iter().fold(0, |col, &c| if c == 9 { col / size * size + size } else { col + 1 });
            if to_tabs {
                out.extend(std::iter::repeat_n(9u16, cols / size));
                out.extend(std::iter::repeat_n(32u16, cols % size));
            } else {
                out.extend(std::iter::repeat_n(32u16, cols));
            }
            out.extend_from_slice(&line[lead..]);
        }
        self.docs[i].indent.tabs = to_tabs;
        if out != text {
            let edit = self.docs[i].edit;
            let caret = get_sel(edit).cpMin.min(out.len() as i32);
            self.replace_text(i, 0, text.len() as i32, &out, (caret, caret));
        }
        self.update_status();
    }

    /// Marks doc `i` modified without a text change (new line endings or
    /// encoding to save with).
    pub(super) fn mark_dirty(&mut self, i: usize) {
        let edit = self.docs[i].edit;
        sm(edit, EM_SETMODIFY, 1, 0);
        self.docs[i].dirty = true;
        self.refresh_tab(i);
        if self.active == Some(i) {
            self.update_title();
        }
        self.update_status();
    }

    /// The text changed but its EN_CHANGE hasn't arrived yet (RichEdit
    /// reports the caret move of an edit first). Painting now would use
    /// offsets from before the edit.
    fn edit_pending(&self, i: usize) -> bool {
        text_len(self.docs[i].edit) != self.docs[i].known_len
    }

    /// EN_CHANGE: the text changed somewhere in [from, caret] (new
    /// offsets; `from` is a safe lower bound, the caret ends the edit).
    pub(super) fn code_after_edit(&mut self, i: usize, from: i32, caret: i32) {
        let len = text_len(self.docs[i].edit);
        let delta = len - self.docs[i].known_len;
        self.docs[i].known_len = len;
        let line = self.line_at(i, from).0;
        let d = &mut self.docs[i];
        d.syn.truncate(line + 1);
        // The painted-styles cache moves with the text; the edited stretch
        // itself is unknown.
        let at = d.styles_at;
        let n = d.styles.len() as i32;
        if from < at {
            if delta < 0 && from - delta > at {
                d.styles.clear();
            } else {
                d.styles_at += delta;
            }
        } else if from < at + n {
            let rel = (from - at) as usize;
            if delta > 0 {
                d.styles.splice(rel..rel, std::iter::repeat_n(UNKNOWN, delta as usize));
            } else if delta < 0 {
                let cut = ((-delta) as usize).min(d.styles.len() - rel);
                d.styles.drain(rel..rel + cut);
            }
            let end = ((caret + 1 - at).max(0) as usize).min(d.styles.len());
            for v in &mut d.styles[rel.min(end)..end] {
                *v = UNKNOWN;
            }
        }
        if delta != 0 {
            // Keep squiggles on their text until the re-check.
            for p in &mut d.problems {
                if p.start >= from {
                    p.start += delta;
                    p.end += delta;
                } else if p.end > from {
                    p.end = (p.end + delta).max(p.start);
                }
            }
        }
        if self.active == Some(i) {
            self.highlight(Some(from));
        }
        self.schedule_check();
    }

    /// The text changed in ways EN_CHANGE can't pin down (undo, replace
    /// all): drop every cached color and state, then redo the screen.
    pub(super) fn code_text_replaced(&mut self, i: usize) {
        let d = &mut self.docs[i];
        d.syn.truncate(1);
        d.painted = None;
        d.styles.clear();
        d.known_len = text_len(d.edit);
        if self.active == Some(i) {
            self.highlight(None);
        }
        self.schedule_check();
    }

    /// Current value of a checkbox preference, by registry name.
    pub(crate) fn pref(&self, name: &str) -> bool {
        match name {
            "LineNumbers" => self.line_numbers,
            "StatusBar" => self.show_status,
            "WordWrap" => self.word_wrap,
            "AutoIndent" => self.auto_indent,
            "SyntaxHighlight" => self.syntax_on,
            "RainbowBrackets" => self.rainbow,
            "MatchBrackets" => self.match_brackets,
            "Occurrences" => self.occurrences,
            "Validate" => self.validate,
            "AutoClose" => self.auto_close,
            "InsertSpaces" => self.insert_spaces,
            "TrimOnSave" => self.trim_on_save,
            "FinalNewline" => self.final_newline,
            _ => false,
        }
    }

    /// Changes, saves and applies a checkbox preference.
    pub(crate) fn set_pref(&mut self, name: &str, on: bool) {
        let slot = match name {
            "LineNumbers" => &mut self.line_numbers,
            "StatusBar" => &mut self.show_status,
            "WordWrap" => &mut self.word_wrap,
            "AutoIndent" => &mut self.auto_indent,
            "SyntaxHighlight" => &mut self.syntax_on,
            "RainbowBrackets" => &mut self.rainbow,
            "MatchBrackets" => &mut self.match_brackets,
            "Occurrences" => &mut self.occurrences,
            "Validate" => &mut self.validate,
            "AutoClose" => &mut self.auto_close,
            "InsertSpaces" => &mut self.insert_spaces,
            "TrimOnSave" => &mut self.trim_on_save,
            "FinalNewline" => &mut self.final_newline,
            _ => return,
        };
        *slot = on;
        settings::save_pref(name, on as u32);
        match name {
            "LineNumbers" | "StatusBar" | "WordWrap" => self.apply_prefs(),
            "SyntaxHighlight" | "RainbowBrackets" | "MatchBrackets" | "Occurrences" => self.recolor_all(),
            "Validate" => {
                for d in &mut self.docs {
                    d.problems.clear();
                    d.tree.clear();
                    d.checked = false;
                }
                self.recolor_all();
                self.schedule_check();
                self.invalidate_gutter();
                self.update_status();
            }
            _ => {}
        }
        self.check_menu(IDM_SYNTAX, self.syntax_on);
        settings::refresh(self);
    }

    pub(crate) fn set_tab_size(&mut self, size: u32) {
        self.tab_size = size.clamp(1, 8);
        settings::save_pref("TabSize", self.tab_size);
    }

    /// Drop every document's colors; the active one is redone now, the
    /// others when shown.
    pub(super) fn recolor_all(&mut self) {
        for i in 0..self.docs.len() {
            if self.docs[i].painted.is_some() {
                self.clear_colors(i);
            }
        }
        self.highlight(None);
    }

    pub(super) fn unit(&self, i: usize) -> Vec<u16> {
        let ind = self.docs[i].indent;
        if ind.tabs { vec![9] } else { vec![32; ind.size as usize] }
    }

    // ---- coloring ----------------------------------------------------------

    /// Whether doc `i` gets a decorated window at all: syntax colors,
    /// bracket and word highlights, and problem squiggles all live in it.
    fn colors_on(&self, i: usize) -> bool {
        let d = &self.docs[i];
        let any = self.syntax_on || self.match_brackets || self.occurrences || self.validate;
        any && d.lang != Lang::Plain && text_len(d.edit) <= HIGHLIGHT_LIMIT
    }

    /// Lexer state at the start of logical line `ln` (computes and caches
    /// states for the lines before it, reading text in large chunks).
    fn state_at(&mut self, i: usize, ln: usize) -> State {
        let lang = self.docs[i].lang;
        let edit = self.docs[i].edit;
        let len = text_len(edit);
        while self.docs[i].syn.len() <= ln {
            let have = self.docs[i].syn.len() - 1;
            let Some(from) = self.line_start(i, have) else { break };
            let to = (from + 256 * 1024).min(len);
            let mut buf = Vec::new();
            get_range_into(edit, from, to, &mut buf);
            let mut st = self.docs[i].syn[have];
            let mut pos = 0;
            let mut done_any = false;
            while let Some(cr) = buf[pos..].iter().position(|&c| c == 13) {
                st = syntax::lex_line(lang, st, &buf[pos..pos + cr], None);
                self.docs[i].syn.push(st);
                pos += cr + 1;
                done_any = true;
                if self.docs[i].syn.len() > ln {
                    break;
                }
            }
            if !done_any {
                if to >= len {
                    break; // last line: nothing follows it
                }
                // One enormous line: lex it whole.
                let end = self.line_end(i, have);
                get_range_into(edit, from, end, &mut buf);
                st = syntax::lex_line(lang, st, &buf, None);
                self.docs[i].syn.push(st);
            }
        }
        let syn = &self.docs[i].syn;
        syn[ln.min(syn.len() - 1)]
    }

    /// Colors the active document's visible text (plus a screen of margin).
    /// `edited_from`: an edit happened at this offset, so everything from
    /// its line on is recolored even if the window hasn't moved.
    pub(super) fn highlight(&mut self, edited_from: Option<i32>) {
        let Some(i) = self.active else { return };
        if edited_from.is_none() && self.edit_pending(i) {
            return;
        }
        if !self.colors_on(i) {
            if self.docs[i].painted.is_some() {
                self.clear_colors(i);
            }
            return;
        }
        let edit = self.docs[i].edit;
        let (vs, ve) = visible_range(edit);
        if edited_from.is_none() {
            if let Some((a, b)) = self.docs[i].painted {
                if a <= vs && ve <= b {
                    return;
                }
            }
        }
        let (l_first, _) = self.line_at(i, vs);
        let (l_last, _) = self.line_at(i, ve);
        let margin = (l_last - l_first).max(20);
        let l0 = l_first.saturating_sub(margin);
        let l1 = l_last + margin;
        let mut st = self.state_at(i, l0);
        let start = self.line_start(i, l0).unwrap_or(0);
        // Lines are lexed from their start (that's what makes their colors
        // right), but only text near the screen is colored or even read
        // past the screen: one minified line can be megabytes long.
        let cap = (ve + WINDOW_SLACK).min(text_len(edit));
        let end = if self.line_at(i, cap).0 <= l1 { cap } else { self.line_end(i, l1) };
        let ws = start.max(vs - WINDOW_SLACK);
        let we = end.max(ws);
        let mut text = Vec::new();
        get_range_into(edit, start, end, &mut text);
        let lang = self.docs[i].lang;
        let mut spans = Vec::new();
        let mut line_spans = Vec::new();
        let mut pos = 0usize;
        let mut ln = l0;
        let (lo, hi) = ((ws - start) as usize, (we - start) as usize);
        loop {
            let e = text[pos..].iter().position(|&c| c == 13).map(|k| pos + k).unwrap_or(text.len());
            line_spans.clear();
            st = syntax::lex_line_within(
                lang,
                st,
                &text[pos..e],
                Some(&mut line_spans),
                lo.saturating_sub(pos),
                hi.saturating_sub(pos),
            );
            for sp in &line_spans {
                let a = (start + (pos + sp.start) as i32).max(ws);
                let b = (start + (pos + sp.end) as i32).min(we);
                if b > a {
                    spans.push((a, b, sp.kind));
                }
            }
            ln += 1;
            // Keep the state cache contiguous as a side benefit (only for
            // lines read to their end).
            if self.docs[i].syn.len() == ln && e < text.len() {
                self.docs[i].syn.push(st);
            }
            if e >= text.len() || e >= hi {
                break;
            }
            pos = e + 1;
        }
        text.truncate(hi);
        text.drain(..lo);
        let d = &mut self.docs[i];
        d.win_text = text;
        d.win_spans = spans;
        let _ = edited_from;
        d.painted = Some((ws, we));
        self.overlays_compute(i);
        self.decorate(i);
    }

    /// Brings the window's colors up to date: works out every character's
    /// look from the spans, problems and overlays, and repaints only the
    /// characters whose look differs from what was painted before.
    fn decorate(&mut self, i: usize) {
        let d = &mut self.docs[i];
        let Some((a, b)) = d.painted else { return };
        if b <= a {
            return;
        }
        let mut styles = vec![0u16; (b - a) as usize];
        if self.syntax_on {
            for &(s, e, k) in &d.win_spans {
                let code = match k {
                    Kind::Bracket(_) if !self.rainbow => 0,
                    Kind::Error => fore_code(k) | WAVE,
                    _ => fore_code(k),
                };
                for p in s.max(a)..e.min(b) {
                    styles[(p - a) as usize] = code;
                }
            }
        }
        for pr in d.problems.iter().filter(|_| self.validate) {
            if pr.end <= a || pr.start >= b {
                continue;
            }
            let code = (if pr.warning { FORE_WARN } else { FORE_ERROR }) | WAVE;
            for p in pr.start.max(a)..pr.end.min(b) {
                let st = &mut styles[(p - a) as usize];
                *st = (*st & !0x3F) | code;
            }
        }
        for &(s, e, bg) in &d.overlay {
            for p in s.max(a)..e.min(b) {
                styles[(p - a) as usize] |= (bg as u16) << BG_SHIFT;
            }
        }
        // What is on screen now, re-aligned to this window.
        let old_at = d.styles_at;
        let old = std::mem::take(&mut d.styles);
        let painted = |p: i32| usize::try_from(p - old_at).ok().and_then(|k| old.get(k).copied()).unwrap_or(UNKNOWN);
        // Stretches whose look changed: (start, end, style).
        let mut todo: Vec<(i32, i32, u16)> = Vec::new();
        let mut k = 0;
        while k < styles.len() {
            let code = styles[k];
            let mut e = k + 1;
            while e < styles.len() && styles[e] == code {
                e += 1;
            }
            let mut p = k;
            while p < e {
                if painted(a + p as i32) == code {
                    p += 1;
                    continue;
                }
                let mut q = p + 1;
                while q < e && painted(a + q as i32) != code {
                    q += 1;
                }
                todo.push((a + p as i32, a + q as i32, code));
                p = q;
            }
            k = e;
        }
        d.styles = styles;
        d.styles_at = a;
        if todo.is_empty() {
            return;
        }
        let Some(doc) = text_document(d.edit) else { return };
        let Some(painter) = Painter::new(&doc) else { return };
        let th = self.theme;
        // Freezing pays off only for many stretches (it redraws the whole
        // control once at the end); a keystroke changes one or two.
        let freeze = todo.len() > 4;
        if freeze {
            unsafe {
                let _ = doc.Freeze();
            }
        }
        for (p, q, code) in todo {
            let fore = match code & 0x3F {
                0 => th.edit_fg,
                c => fore_color(&th, c),
            };
            let back = match (code >> BG_SHIFT) & 0x7 {
                BG_MATCH => th.syn.match_bg,
                BG_MISMATCH => th.syn.mismatch_bg,
                BG_OCCURRENCE => th.syn.occurrence_bg,
                _ => th.edit_bg,
            };
            painter.paint(p, q, fore, back, code & WAVE != 0);
        }
        if freeze {
            unsafe {
                let _ = doc.Unfreeze();
            }
        }
    }

    /// Repaints the whole colored window from scratch. Runs once typing
    /// pauses: edits RichEdit makes on its own (Alt+Backspace undo, drag
    /// and drop, IME) can't be pinned to an offset, and this bounds any
    /// color drift they cause to a moment.
    pub(super) fn repaint_code(&mut self, i: usize) {
        if self.docs[i].painted.is_some() && !self.edit_pending(i) {
            self.docs[i].styles.clear();
            self.decorate(i);
        }
    }

    /// Recompute caret-driven highlights and repaint where they changed.
    pub(super) fn update_overlays(&mut self) {
        let Some(i) = self.active else { return };
        if self.docs[i].painted.is_none() || self.edit_pending(i) {
            return;
        }
        let old = std::mem::take(&mut self.docs[i].overlay);
        self.overlays_compute(i);
        if old != self.docs[i].overlay {
            self.decorate(i);
        }
    }

    fn overlays_compute(&mut self, i: usize) {
        let d = &self.docs[i];
        let mut out = Vec::new();
        let Some((ws, _)) = d.painted else {
            self.docs[i].overlay = out;
            return;
        };
        let sel = get_sel(d.edit);
        let text = &d.win_text;
        let at = |cp: i32| -> Option<u16> { usize::try_from(cp - ws).ok().and_then(|k| text.get(k).copied()) };

        // Matching bracket around the caret.
        if self.match_brackets && sel.cpMin == sel.cpMax {
            let c = sel.cpMin;
            let is_bracket = |cp: i32| d.win_spans.binary_search_by(|s| {
                if s.1 <= cp { std::cmp::Ordering::Less } else if s.0 > cp { std::cmp::Ordering::Greater } else { std::cmp::Ordering::Equal }
            }).ok().is_some_and(|k| matches!(d.win_spans[k].2, Kind::Bracket(_)));
            let here = [c, c - 1].into_iter().find(|&p| at(p).is_some_and(|ch| b"()[]{}".contains(&(ch as u8)) && ch < 0x80) && is_bracket(p));
            if let Some(p) = here {
                match find_partner(d, p) {
                    Some(q) => {
                        out.push((p.min(q), p.min(q) + 1, BG_MATCH as u8));
                        out.push((p.max(q), p.max(q) + 1, BG_MATCH as u8));
                    }
                    // Unmatched only if the colored window reaches the end
                    // of the text in the direction the partner would be;
                    // otherwise it's just off screen.
                    None => {
                        let opener = at(p).is_some_and(|c| c == 0x28 || c == 0x5B || c == 0x7B);
                        let (ws, we) = d.painted.unwrap_or_default();
                        let sure = if opener { we >= text_len(d.edit) } else { ws <= 0 };
                        if sure {
                            out.push((p, p + 1, BG_MISMATCH as u8));
                        }
                    }
                }
            }
        }
        // Other occurrences of the selection, or of the word at the caret.
        if self.occurrences {
            let needle: Option<(Vec<u16>, bool)> = if sel.cpMin != sel.cpMax {
                let (a, b) = ((sel.cpMin - ws) as usize, (sel.cpMax - ws) as usize);
                (sel.cpMin >= ws && b <= text.len() && b - a <= 200 && !text[a..b].contains(&13) && b > a)
                    .then(|| (text[a..b].to_vec(), text[a..b].iter().all(|&c| is_word(c))))
            } else if d.lang.is_code() {
                let c = sel.cpMin - ws;
                if c >= 0 && (c as usize) <= text.len() {
                    let c = c as usize;
                    let mut s = c;
                    while s > 0 && is_word(text[s - 1]) {
                        s -= 1;
                    }
                    let mut e = c;
                    while e < text.len() && is_word(text[e]) {
                        e += 1;
                    }
                    (e - s >= 2 && !text[s..e].iter().all(|&ch| (b'0' as u16..=b'9' as u16).contains(&ch))).then(|| (text[s..e].to_vec(), true))
                } else {
                    None
                }
            } else {
                None
            };
            if let Some((n, whole)) = needle {
                let mut hits = Vec::new();
                let mut k = 0;
                while k + n.len() <= text.len() && hits.len() < 400 {
                    if text[k..k + n.len()] == n[..]
                        && (!whole || ((k == 0 || !is_word(text[k - 1])) && (k + n.len() == text.len() || !is_word(text[k + n.len()]))))
                    {
                        hits.push(k);
                        k += n.len();
                    } else {
                        k += 1;
                    }
                }
                if hits.len() >= 2 {
                    for h in hits {
                        let a = ws + h as i32;
                        // The selection keeps its own highlight.
                        if sel.cpMin != sel.cpMax && a == sel.cpMin {
                            continue;
                        }
                        out.push((a, a + n.len() as i32, BG_OCCURRENCE as u8));
                    }
                }
            }
        }
        self.docs[i].overlay = out;
    }

    // ---- checking ------------------------------------------------------------

    pub(super) fn schedule_check(&self) {
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_CHECK, CHECK_DELAY_MS, None);
        }
    }

    /// Re-runs the JSON / YAML checker on doc `i` (or clears its results).
    pub(super) fn check(&mut self, i: usize) {
        let edit = self.docs[i].edit;
        let len = text_len(edit);
        let lang = self.docs[i].lang;
        // Untitled documents pick up a language once they have content.
        if self.docs[i].lang_auto && self.docs[i].path.is_none() && self.docs[i].lang == Lang::Plain {
            let mut head = Vec::new();
            get_range_into(edit, 0, len.min(64 * 1024), &mut head);
            let sniffed = guess_lang(None, &head);
            if sniffed != Lang::Plain {
                self.docs[i].lang = sniffed;
                self.reset_code(i);
                return;
            }
        }
        let wanted = self.validate && matches!(lang, Lang::Json | Lang::Yaml) && len <= CHECK_LIMIT;
        let mut problems = Vec::new();
        let mut tree = Vec::new();
        if wanted {
            let mut text = Vec::new();
            get_range_into(edit, 0, len, &mut text);
            let parsed = match lang {
                Lang::Json => {
                    let name = self.docs[i].path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
                    crate::json::parse(&text, crate::json::options_for(name.as_deref()))
                }
                _ => crate::yaml::parse(&text),
            };
            for p in parsed.problems.iter().take(200) {
                let start = (p.start as i32).clamp(0, len);
                let end = (p.end as i32).clamp(start, len).max((start + 1).min(len));
                let line = self.line_at(i, start).0;
                problems.push(DocProblem { start, end, line, message: p.message.clone(), warning: p.warning });
            }
            problems.sort_by_key(|p| p.start);
            if len <= TREE_LIMIT {
                tree = parsed.docs;
            }
        }
        let changed = problems.len() != self.docs[i].problems.len()
            || problems.iter().zip(&self.docs[i].problems).any(|(a, b)| a.start != b.start || a.end != b.end || a.message != b.message);
        let d = &mut self.docs[i];
        d.problems = problems;
        d.tree = tree;
        d.checked = wanted;
        if changed && self.active == Some(i) {
            if self.docs[i].painted.is_some() {
                self.decorate(i);
            } else {
                self.highlight(None);
            }
            self.invalidate_gutter();
        }
        if self.active == Some(i) {
            self.update_status();
        }
    }

    /// Text for the left side of the status bar: the first problem, or
    /// where the caret is in the JSON / YAML structure.
    pub(super) fn compute_note(&mut self, caret: i32) -> (String, u8) {
        let Some(i) = self.active else { return (String::new(), 0) };
        let d = &self.docs[i];
        if !d.problems.is_empty() {
            let errors = d.problems.iter().filter(|p| !p.warning).count();
            let p = d.problems.iter().find(|p| p.end >= caret).unwrap_or(&d.problems[0]);
            let total = d.problems.len();
            let count = if total > 1 { format!("{total} problems · ") } else { String::new() };
            let mark = if errors > 0 { "\u{2715}" } else { "\u{26A0}" };
            return (format!("{mark} {count}Ln {}: {}", p.line + 1, p.message), if errors > 0 { 1 } else { 3 });
        }
        let path = data::path_at(&d.tree, caret as usize).filter(|p| p != "$");
        if d.checked && text_len(d.edit) > 0 {
            let mut note = format!("\u{2713} Valid {}", d.lang.name());
            if let Some(p) = path {
                note.push_str("  \u{00B7}  ");
                note.push_str(&p);
            }
            return (note, 2);
        }
        (path.unwrap_or_default(), 0)
    }

    /// Mouse over the editor: explain the problem under it in a tooltip.
    pub(super) fn hover_problem(&mut self, edit: HWND, x: i32, y: i32) {
        let found = self.problem_at_point(edit, x, y);
        if found.as_ref().map(|p| (p.start, p.end)) == self.tip_for {
            return;
        }
        let Some(p) = found else {
            self.hide_tip();
            return;
        };
        self.tip_for = Some((p.start, p.end));
        let text = wide(&format!("{}{}", if p.warning { "\u{26A0} " } else { "" }, p.message));
        unsafe {
            if self.tip.0.is_null() {
                self.tip = CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_TRANSPARENT,
                    TOOLTIPS_CLASSW,
                    PCWSTR::null(),
                    WINDOW_STYLE(WS_POPUP.0 | TTS_NOPREFIX | TTS_ALWAYSTIP),
                    0,
                    0,
                    0,
                    0,
                    Some(self.hwnd),
                    None,
                    None,
                    None,
                )
                .unwrap_or_default();
                let ti = tool_info(self.hwnd, &text);
                sm(self.tip, TTM_ADDTOOLW, 0, &ti as *const _ as isize);
                sm(self.tip, TTM_SETMAXTIPWIDTH, 0, scale(420, self.dpi) as isize);
            }
            theme::set_control_dark(self.tip, self.theme.dark);
            let ti = tool_info(self.hwnd, &text);
            sm(self.tip, TTM_UPDATETIPTEXTW, 0, &ti as *const _ as isize);
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let pos = ((pt.y + scale(20, self.dpi)) as u32) << 16 | ((pt.x + scale(4, self.dpi)) as u32 & 0xFFFF);
            sm(self.tip, TTM_TRACKPOSITION, 0, pos as isize);
            sm(self.tip, TTM_TRACKACTIVATE, 1, &ti as *const _ as isize);
            // Hear when the mouse leaves the editor, to hide the tip.
            let mut tme = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: edit,
                dwHoverTime: 0,
            };
            let _ = TrackMouseEvent(&mut tme);
        }
    }

    pub(super) fn hide_tip(&mut self) {
        if self.tip_for.take().is_some() && !self.tip.0.is_null() {
            let ti = tool_info(self.hwnd, &[0]);
            sm(self.tip, TTM_TRACKACTIVATE, 0, &ti as *const _ as isize);
        }
    }

    /// The problem whose text is under client point (x, y) of `edit`.
    fn problem_at_point(&self, edit: HWND, x: i32, y: i32) -> Option<DocProblem> {
        let i = self.active.filter(|&i| self.docs[i].edit.0 == edit.0)?;
        let d = &self.docs[i];
        if !self.validate || d.problems.is_empty() {
            return None;
        }
        let pt = POINTL { x, y };
        let cp = sm(edit, EM_CHARFROMPOS, 0, &pt as *const POINTL as isize).0 as i32;
        // EM_CHARFROMPOS snaps to the nearest character; make sure the
        // mouse is really on it.
        let mut a = POINTL::default();
        let mut b = POINTL::default();
        sm(edit, EM_POSFROMCHAR, &mut a as *mut POINTL as usize, cp as isize);
        sm(edit, EM_POSFROMCHAR, &mut b as *mut POINTL as usize, (cp + 1) as isize);
        let line_h = (self.edit_pt * self.dpi as i32 * d.zoom / 7200) * 4 / 3;
        let right = if b.y == a.y && b.x > a.x { b.x } else { a.x + line_h / 2 };
        if x < a.x || x >= right || y < a.y || y >= a.y + line_h.max(1) {
            return None;
        }
        d.problems.iter().find(|p| p.start <= cp && cp < p.end.max(p.start + 1)).cloned()
    }

    pub(super) fn goto_problem(&mut self, forward: bool) {
        let Some(i) = self.active else { return };
        let edit = self.docs[i].edit;
        let caret = get_sel(edit).cpMin;
        let ps = &self.docs[i].problems;
        if ps.is_empty() {
            return;
        }
        let target = if forward {
            ps.iter().find(|p| p.start > caret).or(ps.first())
        } else {
            ps.iter().rev().find(|p| p.start < caret).or(ps.last())
        };
        if let Some(p) = target {
            let (a, b) = (p.start, p.end);
            set_sel(edit, a, b);
            sm(edit, EM_SCROLLCARET, 0, 0);
            unsafe {
                let _ = SetFocus(Some(edit));
            }
        }
    }

    pub(super) fn copy_path(&mut self) {
        let Some(i) = self.active else { return };
        let caret = get_sel(self.docs[i].edit).cpMin as usize;
        if let Some(path) = data::path_at(&self.docs[i].tree, caret) {
            set_clipboard(self.hwnd, &path);
        }
    }

    // ---- whole-document tools ----------------------------------------------

    fn doc_text(&self, i: usize) -> Vec<u16> {
        let edit = self.docs[i].edit;
        let mut t = Vec::new();
        get_range_into(edit, 0, text_len(edit), &mut t);
        t
    }

    /// Replaces [a, b) with `text` as one undo step, keeping the view put.
    pub(super) fn replace_text(&mut self, i: usize, a: i32, b: i32, text: &[u16], sel_after: (i32, i32)) {
        let edit = self.docs[i].edit;
        let mut scroll = POINT::default();
        sm(edit, EM_GETSCROLLPOS, 0, &mut scroll as *mut POINT as isize);
        sm(edit, WM_SETREDRAW, 0, 0);
        set_sel(edit, a, b);
        let mut buf = text.to_vec();
        buf.push(0);
        sm(edit, EM_REPLACESEL, 1, buf.as_ptr() as isize);
        set_sel(edit, sel_after.0, sel_after.1);
        sm(edit, EM_SETSCROLLPOS, 0, &mut scroll as *mut POINT as isize);
        sm(edit, WM_SETREDRAW, 1, 0);
        unsafe {
            let _ = InvalidateRect(Some(edit), None, true);
        }
    }

    fn note(&mut self, msg: &str) {
        let wmsg = wide(msg);
        unsafe {
            MessageBoxW(Some(self.hwnd), PCWSTR(wmsg.as_ptr()), w!("Nimble"), MB_OK | MB_ICONINFORMATION);
        }
    }

    /// JSON flavor for doc `i`, from its file name.
    fn json_options(&self, i: usize) -> crate::json::Options {
        let name = self.docs[i].path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
        crate::json::options_for(name.as_deref())
    }

    /// Parses for the reformatting tools, which keep comments where they can.
    fn parse_lenient(&self, i: usize, text: &[u16]) -> data::Parsed {
        match self.docs[i].lang {
            Lang::Yaml => crate::yaml::parse(text),
            _ => crate::json::parse(text, crate::json::Options { jsonc: true, ..self.json_options(i) }),
        }
    }

    /// JSON Lines must stay one value per line.
    fn refuse_json_lines(&mut self, i: usize, what: &str) -> bool {
        if self.docs[i].lang == Lang::Json && self.json_options(i).lines {
            self.note(&format!("{what} doesn't apply to JSON Lines files (one value per line)."));
            return true;
        }
        false
    }

    fn first_error(&mut self, i: usize, parsed: &data::Parsed) -> bool {
        if let Some(p) = parsed.problems.iter().find(|p| !p.warning) {
            let line = self.line_at(i, p.start as i32).0;
            let msg = format!("Fix this first (line {}): {}", line + 1, p.message);
            self.note(&msg);
            let edit = self.docs[i].edit;
            set_sel(edit, p.start as i32, p.end as i32);
            sm(edit, EM_SCROLLCARET, 0, 0);
            return true;
        }
        false
    }

    /// Format Document: JSON re-indented (comments kept); YAML re-written
    /// in a clean block style when it has no comments to lose.
    pub(super) fn format_document(&mut self) {
        let Some(i) = self.active else { return };
        let text = self.doc_text(i);
        let unit: String = String::from_utf16_lossy(&self.unit(i));
        if self.refuse_json_lines(i, "Format Document") {
            return;
        }
        let out = match self.docs[i].lang {
            Lang::Json => {
                let parsed = self.parse_lenient(i, &text);
                if self.first_error(i, &parsed) {
                    return;
                }
                crate::json::format(&text, &unit)
            }
            Lang::Yaml => {
                if text.iter().any(|&c| c == b'#' as u16) && yaml_has_comments(&text) {
                    self.note("This YAML has comments, which reformatting would remove, so it was left as is.");
                    return;
                }
                let parsed = self.parse_lenient(i, &text);
                if self.first_error(i, &parsed) {
                    return;
                }
                let size = if self.docs[i].indent.tabs { 2 } else { self.docs[i].indent.size as usize };
                parsed.docs.iter().map(|d| data::to_yaml(d, size)).collect::<Vec<_>>().join("---\n")
            }
            other => {
                self.note(&format!("Format Document works on JSON and YAML (this is {}).", other.name()));
                return;
            }
        };
        self.replace_all_text(i, &out);
    }

    fn replace_all_text(&mut self, i: usize, out: &str) {
        let edit = self.docs[i].edit;
        let new: Vec<u16> = out.replace("\r\n", "\r").replace('\n', "\r").encode_utf16().collect();
        let old = self.doc_text(i);
        if new == old {
            return;
        }
        let caret = get_sel(edit).cpMin.min(new.len() as i32);
        self.replace_text(i, 0, old.len() as i32, &new, (caret, caret));
    }

    pub(super) fn minify(&mut self) {
        let Some(i) = self.active else { return };
        if self.docs[i].lang != Lang::Json {
            self.note("Minify works on JSON documents.");
            return;
        }
        if self.refuse_json_lines(i, "Minify") {
            return;
        }
        let text = self.doc_text(i);
        let parsed = self.parse_lenient(i, &text);
        if self.first_error(i, &parsed) {
            return;
        }
        let out = crate::json::minify(&text);
        self.replace_all_text(i, &out);
    }

    pub(super) fn sort_keys(&mut self) {
        let Some(i) = self.active else { return };
        if self.docs[i].lang != Lang::Json {
            self.note("Sort Keys works on JSON documents.");
            return;
        }
        if self.refuse_json_lines(i, "Sort Keys") {
            return;
        }
        let text = self.doc_text(i);
        if crate::json::has_comments(&text) {
            self.note("This JSON has comments, which sorting would remove, so it was left as is.");
            return;
        }
        let parsed = self.parse_lenient(i, &text);
        if self.first_error(i, &parsed) || parsed.docs.is_empty() {
            return;
        }
        let unit: String = String::from_utf16_lossy(&self.unit(i));
        let out = data::to_json(&parsed.docs[0], Some(&unit), true);
        self.replace_all_text(i, &out);
    }

    /// Opens the JSON as YAML (or the YAML as JSON) in a new tab.
    pub(super) fn convert(&mut self) {
        let Some(i) = self.active else { return };
        let lang = self.docs[i].lang;
        if !matches!(lang, Lang::Json | Lang::Yaml) {
            self.note("Convert works on JSON and YAML documents.");
            return;
        }
        let text = self.doc_text(i);
        let parsed = self.parse_lenient(i, &text);
        if self.first_error(i, &parsed) || parsed.docs.is_empty() {
            return;
        }
        let (out, to) = if lang == Lang::Json {
            let docs: Vec<String> = parsed.docs.iter().map(|d| data::to_yaml(d, 2)).collect();
            (docs.join("---
"), Lang::Yaml)
        } else if parsed.docs.len() == 1 {
            (data::to_json(&parsed.docs[0], Some("  "), false), Lang::Json)
        } else {
            let all = data::Node { start: 0, end: 0, value: data::Value::Array(parsed.docs.clone()) };
            (data::to_json(&all, Some("  "), false), Lang::Json)
        };
        if !self.new_document(None) {
            return;
        }
        let j = self.docs.len() - 1;
        self.docs[j].lang = to;
        self.docs[j].lang_auto = false;
        let edit = self.docs[j].edit;
        let w: Vec<u16> = out.replace('\n', "\r").encode_utf16().chain(std::iter::once(0)).collect();
        sm(edit, EM_REPLACESEL, 1, w.as_ptr() as isize);
        set_sel(edit, 0, 0);
        sm(edit, EM_SCROLLCARET, 0, 0);
        self.detect_code(j);
    }

    pub(super) fn trim_trailing_ws(&mut self, i: usize) {
        let text = self.doc_text(i);
        let mut out: Vec<u16> = Vec::with_capacity(text.len());
        let mut changed = false;
        for (n, line) in text.split(|&c| c == 13).enumerate() {
            if n > 0 {
                out.push(13);
            }
            let keep = line.len() - line.iter().rev().take_while(|&&c| c == 32 || c == 9).count();
            changed |= keep != line.len();
            out.extend_from_slice(&line[..keep]);
        }
        if changed {
            let edit = self.docs[i].edit;
            let caret = get_sel(edit).cpMin.min(out.len() as i32);
            self.replace_text(i, 0, text.len() as i32, &out, (caret, caret));
        }
    }

    pub(super) fn ensure_final_newline(&mut self, i: usize) {
        let edit = self.docs[i].edit;
        let len = text_len(edit);
        if len == 0 {
            return;
        }
        let mut last = Vec::new();
        get_range_into(edit, len - 1, len, &mut last);
        if last.first() != Some(&13) {
            let sel = get_sel(edit);
            self.replace_text(i, len, len, &[13], (sel.cpMin, sel.cpMax));
        }
    }

    // ---- line commands ---------------------------------------------------------

    /// First and last logical line touched by the selection.
    fn sel_lines(&mut self, i: usize) -> (usize, usize, CHARRANGE) {
        let sel = get_sel(self.docs[i].edit);
        let first = self.line_at(i, sel.cpMin).0;
        let last_cp = if sel.cpMax > sel.cpMin { sel.cpMax - 1 } else { sel.cpMax };
        let last = self.line_at(i, last_cp).0.max(first);
        (first, last, sel)
    }

    /// Text of lines [first, last] (no trailing break) and its span.
    fn lines_text(&mut self, i: usize, first: usize, last: usize) -> (i32, i32, Vec<u16>) {
        let a = self.line_start(i, first).unwrap_or(0);
        let b = self.line_end(i, last);
        let mut t = Vec::new();
        get_range_into(self.docs[i].edit, a, b, &mut t);
        (a, b, t)
    }

    /// Ctrl+] / Tab on a multi-line selection: indent (or outdent) lines.
    pub(super) fn shift_lines(&mut self, outdent: bool) {
        let Some(i) = self.active else { return };
        let (first, last, sel) = self.sel_lines(i);
        let (a, b, text) = self.lines_text(i, first, last);
        let unit = self.unit(i);
        let width = self.docs[i].indent.size as usize;
        let mut out = Vec::with_capacity(text.len() + unit.len() * (last - first + 1));
        let mut delta_first = 0i32;
        let mut delta_total = 0i32;
        for (n, line) in text.split(|&c| c == 13).enumerate() {
            if n > 0 {
                out.push(13);
            }
            let d = if outdent {
                let remove = if line.first() == Some(&9) { 1 } else { line.iter().take(width).take_while(|&&c| c == 32).count() };
                out.extend_from_slice(&line[remove..]);
                -(remove as i32)
            } else if line.iter().all(|&c| c == 32 || c == 9) {
                out.extend_from_slice(line);
                0
            } else {
                out.extend_from_slice(&unit);
                out.extend_from_slice(line);
                unit.len() as i32
            };
            if n == 0 {
                delta_first = d;
            }
            delta_total += d;
        }
        if delta_total == 0 && !outdent {
            return;
        }
        let new_a = (sel.cpMin + delta_first).max(a);
        let new_b = (sel.cpMax + delta_total).max(new_a);
        let sel_after = if sel.cpMin == sel.cpMax { (new_a, new_a) } else { (new_a, new_b) };
        self.replace_text(i, a, b, &out, sel_after);
    }

    pub(super) fn toggle_comment(&mut self) {
        let Some(i) = self.active else { return };
        let lang = self.docs[i].lang;
        let Some(token) = lang.line_comment() else {
            if let Some((open, close)) = lang.block_comment() {
                self.toggle_block_comment(i, open, close);
            }
            return;
        };
        let token: Vec<u16> = token.encode_utf16().collect();
        let (first, last, sel) = self.sel_lines(i);
        let (a, b, text) = self.lines_text(i, first, last);
        let lines: Vec<&[u16]> = text.split(|&c| c == 13).collect();
        let indent_of = |l: &[u16]| l.iter().take_while(|&&c| c == 32 || c == 9).count();
        let content: Vec<&&[u16]> = lines.iter().filter(|l| l.iter().any(|&c| c != 32 && c != 9)).collect();
        if content.is_empty() {
            return;
        }
        let starts_with = |l: &[u16]| {
            let k = indent_of(l);
            l.len() >= k + token.len() && l[k..k + token.len()].iter().zip(&token).all(|(&x, &y)| x == y || x | 32 == y | 32)
        };
        let uncomment = content.iter().all(|l| starts_with(l));
        let col = content.iter().map(|l| indent_of(l)).min().unwrap_or(0);
        let mut out = Vec::new();
        let mut delta_first = 0;
        let mut delta_total = 0i32;
        for (n, l) in lines.iter().enumerate() {
            if n > 0 {
                out.push(13);
            }
            let blank = l.iter().all(|&c| c == 32 || c == 9);
            let d = if blank {
                out.extend_from_slice(l);
                0
            } else if uncomment {
                let k = indent_of(l);
                let mut cut = token.len();
                if l.get(k + cut) == Some(&32) {
                    cut += 1;
                }
                out.extend_from_slice(&l[..k]);
                out.extend_from_slice(&l[k + cut..]);
                -(cut as i32)
            } else {
                out.extend_from_slice(&l[..col]);
                out.extend_from_slice(&token);
                out.push(32);
                out.extend_from_slice(&l[col..]);
                token.len() as i32 + 1
            };
            if n == 0 {
                delta_first = d;
            }
            delta_total += d;
        }
        let new_a = (sel.cpMin + delta_first).max(a);
        let sel_after = if sel.cpMin == sel.cpMax { (new_a, new_a) } else { (new_a, (sel.cpMax + delta_total).max(new_a)) };
        self.replace_text(i, a, b, &out, sel_after);
    }

    fn toggle_block_comment(&mut self, i: usize, open: &str, close: &str) {
        let (first, last, _) = self.sel_lines(i);
        let (a, b, text) = self.lines_text(i, first, last);
        let o: Vec<u16> = open.encode_utf16().collect();
        let c: Vec<u16> = close.encode_utf16().collect();
        let s = text.iter().position(|&ch| ch != 32 && ch != 9).unwrap_or(0);
        let e = text.len() - text.iter().rev().take_while(|&&ch| ch == 32 || ch == 9).count();
        let body = &text[s..e];
        let out: Vec<u16> = if body.starts_with(&o) && body.ends_with(&c) && body.len() >= o.len() + c.len() {
            let mut inner = &body[o.len()..body.len() - c.len()];
            if inner.first() == Some(&32) {
                inner = &inner[1..];
            }
            if inner.last() == Some(&32) {
                inner = &inner[..inner.len() - 1];
            }
            [&text[..s], inner, &text[e..]].concat()
        } else {
            [&text[..s], &o[..], &[32][..], body, &[32][..], &c[..], &text[e..]].concat()
        };
        let end = a + out.len() as i32;
        self.replace_text(i, a, b, &out, (a, end));
    }

    pub(super) fn move_lines(&mut self, up: bool) {
        let Some(i) = self.active else { return };
        let (first, last, sel) = self.sel_lines(i);
        if (up && first == 0) || (!up && self.line_start(i, last + 1).is_none()) {
            return;
        }
        let (lo, hi) = if up { (first - 1, last) } else { (first, last + 1) };
        let (a, b, text) = self.lines_text(i, lo, hi);
        let lines: Vec<&[u16]> = text.split(|&c| c == 13).collect();
        let mut order: Vec<&[u16]> = lines.clone();
        let shift;
        if up {
            let moved = order.remove(0);
            order.push(moved);
            shift = -(lines[0].len() as i32 + 1);
        } else {
            let moved = order.pop().unwrap();
            order.insert(0, moved);
            shift = lines[lines.len() - 1].len() as i32 + 1;
        }
        let out = order.join(&13u16);
        self.replace_text(i, a, b, &out, (sel.cpMin + shift, sel.cpMax + shift));
    }

    pub(super) fn delete_lines(&mut self) {
        let Some(i) = self.active else { return };
        let (first, last, _) = self.sel_lines(i);
        let a = self.line_start(i, first).unwrap_or(0);
        let (a, b) = if let Some(next) = self.line_start(i, last + 1) {
            (a, next)
        } else if first > 0 {
            (self.line_end(i, first - 1), self.line_end(i, last))
        } else {
            (a, self.line_end(i, last))
        };
        self.replace_text(i, a, b, &[], (a, a));
    }

    pub(super) fn select_line(&mut self) {
        let Some(i) = self.active else { return };
        let (first, last, _) = self.sel_lines(i);
        let a = self.line_start(i, first).unwrap_or(0);
        let b = self.line_start(i, last + 1).unwrap_or_else(|| text_len(self.docs[i].edit));
        set_sel(self.docs[i].edit, a, b);
    }

    /// Ctrl+Shift+\: jump to the partner of the bracket at the caret.
    pub(super) fn goto_matching_bracket(&mut self) {
        let Some(i) = self.active else { return };
        self.highlight(None);
        let edit = self.docs[i].edit;
        let caret = get_sel(edit).cpMin;
        let d = &self.docs[i];
        let Some((ws, _)) = d.painted else { return };
        let text = &d.win_text;
        let is_bracket_at = |p: i32| {
            usize::try_from(p - ws).ok().and_then(|k| text.get(k)).is_some_and(|&c| c < 0x80 && b"()[]{}".contains(&(c as u8)))
                && d.win_spans.iter().any(|s| s.0 == p && matches!(s.2, Kind::Bracket(_)))
        };
        let Some(p) = [caret, caret - 1].into_iter().find(|&p| is_bracket_at(p)) else { return };
        if let Some(q) = find_partner(d, p) {
            // Land on the same side of the partner as the caret was.
            let target = if p == caret { q } else { q + 1 };
            set_sel(edit, target, target);
            sm(edit, EM_SCROLLCARET, 0, 0);
        }
    }

    // ---- typing ------------------------------------------------------------------

    /// Tab / Shift+Tab. Multi-line selections indent whole lines; otherwise
    /// a tab or the spaces to the next tab stop is inserted.
    pub(super) fn on_tab(&mut self, edit: HWND, shift: bool) -> bool {
        let Some(i) = self.docs.iter().position(|d| d.edit.0 == edit.0) else { return false };
        let sel = get_sel(edit);
        let (first, last, _) = self.sel_lines(i);
        if shift || last > first {
            self.shift_lines(shift);
            return true;
        }
        let ind = self.docs[i].indent;
        if ind.tabs {
            return false; // RichEdit inserts the tab itself
        }
        let line_start = self.line_at(i, sel.cpMin).1;
        let mut before = Vec::new();
        get_range_into(edit, line_start, sel.cpMin, &mut before);
        let col: usize = before.iter().map(|&c| if c == 9 { ind.size as usize } else { 1 }).sum();
        let n = ind.size as usize - col % ind.size as usize;
        let spaces: Vec<u16> = std::iter::repeat_n(32u16, n).chain(std::iter::once(0)).collect();
        sm(edit, EM_REPLACESEL, 1, spaces.as_ptr() as isize);
        true
    }

    /// Typed characters: closing pairs are inserted with their opener and
    /// typed over when already there; a selection gets wrapped.
    pub(super) fn on_char(&mut self, edit: HWND, ch: u16) -> bool {
        let Some(i) = self.docs.iter().position(|d| d.edit.0 == edit.0) else { return false };
        if !self.auto_close || !self.docs[i].lang.is_code() {
            return false;
        }
        let pairs: &[(u16, u16)] = &[(0x28, 0x29), (0x5B, 0x5D), (0x7B, 0x7D), (0x22, 0x22), (0x27, 0x27), (0x60, 0x60)];
        let sel = get_sel(edit);
        let len = text_len(edit);
        let mut around = Vec::new();
        get_range_into(edit, (sel.cpMin - 1).max(0), (sel.cpMax + 1).min(len), &mut around);
        let prev = if sel.cpMin > 0 { around.first().copied() } else { None };
        let next = if sel.cpMax < len { around.last().copied() } else { None };
        let lang = self.docs[i].lang;
        // Typing a closer that's already next: step over it.
        if sel.cpMin == sel.cpMax && pairs.iter().any(|&(_, c)| c == ch) && next == Some(ch) {
            let quote = ch == 0x22 || ch == 0x27 || ch == 0x60;
            if !quote || prev.is_some_and(|p| p != 0x5C) {
                set_sel(edit, sel.cpMin + 1, sel.cpMin + 1);
                return true;
            }
        }
        let Some(&(open, close)) = pairs.iter().find(|&&(o, _)| o == ch) else { return false };
        let quote = open == close;
        if quote {
            // Quotes pair only between words: not in "it's", not after a backslash.
            if prev.is_some_and(|p| is_word(p) || p == 0x5C) || next.is_some_and(is_word) {
                return false;
            }
            if open == 0x27 && matches!(lang, Lang::Rust) || open == 0x60 && !matches!(lang, Lang::JavaScript | Lang::TypeScript | Lang::Markdown | Lang::Go) {
                return false;
            }
        } else if sel.cpMin == sel.cpMax && next.is_some_and(|n| is_word(n)) {
            return false;
        }
        if sel.cpMin != sel.cpMax {
            let mut inner = Vec::new();
            get_range_into(edit, sel.cpMin, sel.cpMax, &mut inner);
            if inner.contains(&13) && quote {
                return false;
            }
            let mut out = vec![open];
            out.extend_from_slice(&inner);
            out.push(close);
            out.push(0);
            sm(edit, EM_REPLACESEL, 1, out.as_ptr() as isize);
            set_sel(edit, sel.cpMin + 1, sel.cpMax + 1);
            return true;
        }
        let out = [open, close, 0];
        sm(edit, EM_REPLACESEL, 1, out.as_ptr() as isize);
        set_sel(edit, sel.cpMin + 1, sel.cpMin + 1);
        true
    }

    /// Backspace between an empty pair removes both halves.
    pub(super) fn on_backspace(&mut self, edit: HWND) -> bool {
        let Some(i) = self.docs.iter().position(|d| d.edit.0 == edit.0) else { return false };
        if !self.auto_close || !self.docs[i].lang.is_code() {
            return false;
        }
        let sel = get_sel(edit);
        if sel.cpMin != sel.cpMax || sel.cpMin == 0 {
            return false;
        }
        let mut two = Vec::new();
        get_range_into(edit, sel.cpMin - 1, sel.cpMin + 1, &mut two);
        let pair = matches!(two.as_slice(), [0x28, 0x29] | [0x5B, 0x5D] | [0x7B, 0x7D] | [0x22, 0x22] | [0x27, 0x27] | [0x60, 0x60]);
        if pair {
            set_sel(edit, sel.cpMin - 1, sel.cpMin + 1);
            sm(edit, EM_REPLACESEL, 1, [0u16].as_ptr() as isize);
            return true;
        }
        false
    }

    /// Enter: carry the line's indentation, add a level after an opening
    /// bracket (or a trailing ':' in YAML / Python), and split an empty
    /// bracket pair onto three lines.
    pub(super) fn smart_newline(&mut self, edit: HWND) -> bool {
        let Some(i) = self.docs.iter().position(|d| d.edit.0 == edit.0) else { return false };
        let sel = get_sel(edit);
        let (_, start) = self.line_at(i, sel.cpMin);
        let mut before = Vec::new();
        get_range_into(edit, start, sel.cpMin, &mut before);
        let indent: Vec<u16> = before.iter().copied().take_while(|&c| c == 32 || c == 9).collect();
        let lang = self.docs[i].lang;
        let last = before.iter().rev().find(|&&c| c != 32 && c != 9).copied();
        let opens = lang.is_code() && matches!(last, Some(0x7B) | Some(0x5B) | Some(0x28))
            || lang.colon_indents() && last == Some(0x3A);
        let len = text_len(edit);
        let mut after = Vec::new();
        get_range_into(edit, sel.cpMax, (sel.cpMax + 1).min(len), &mut after);
        let closes = opens && matches!((last, after.first()), (Some(0x7B), Some(&0x7D)) | (Some(0x5B), Some(&0x5D)) | (Some(0x28), Some(&0x29)));
        if indent.is_empty() && !opens {
            return false; // a plain Enter
        }
        let mut ins = vec![13u16];
        ins.extend_from_slice(&indent);
        if opens {
            ins.extend(self.unit(i));
        }
        let caret = sel.cpMin + ins.len() as i32;
        if closes {
            ins.push(13);
            ins.extend_from_slice(&indent);
        }
        ins.push(0);
        sm(edit, EM_REPLACESEL, 1, ins.as_ptr() as isize);
        if closes {
            set_sel(edit, caret, caret);
        }
        true
    }
}

/// The partner of the bracket at `p` among the window's bracket tokens.
fn find_partner(d: &crate::document::Document, p: i32) -> Option<i32> {
    let ws = d.painted?.0;
    let ch = *d.win_text.get((p - ws) as usize)?;
    let brackets: Vec<(i32, u16)> = d
        .win_spans
        .iter()
        .filter(|s| matches!(s.2, Kind::Bracket(_)))
        .map(|s| (s.0, d.win_text[(s.0 - ws) as usize]))
        .collect();
    let k = brackets.iter().position(|b| b.0 == p)?;
    let (open, close) = match ch {
        0x28 | 0x29 => (0x28, 0x29),
        0x5B | 0x5D => (0x5B, 0x5D),
        _ => (0x7B, 0x7D),
    };
    let mut depth = 0i32;
    if ch == open {
        for &(q, c) in &brackets[k..] {
            if c == open {
                depth += 1;
            } else if c == close {
                depth -= 1;
                if depth == 0 {
                    return Some(q);
                }
            }
        }
    } else {
        for &(q, c) in brackets[..=k].iter().rev() {
            if c == close {
                depth += 1;
            } else if c == open {
                depth -= 1;
                if depth == 0 {
                    return Some(q);
                }
            }
        }
    }
    None
}

fn fore_color(th: &Theme, code: u16) -> COLORREF {
    let s = &th.syn;
    match code {
        1 => s.comment,
        2 => s.string,
        3 => s.escape,
        4 => s.number,
        5 => s.keyword,
        6 => s.literal,
        7 => s.type_,
        8 => s.function,
        9 => s.key,
        10..=12 => s.brackets[(code - 10) as usize],
        13 => s.tag,
        14 => s.attr,
        15 => s.heading,
        16 => s.emphasis,
        17 => s.code,
        18 => s.link,
        19 => s.directive,
        20 => s.variable,
        21 => s.anchor,
        22 => s.punct,
        FORE_WARN => th.warn,
        _ => th.danger,
    }
}

/// Language from the file name, else from the text itself. Plain-text
/// extensions stay plain: a log that starts with "[12:00]" isn't JSON.
fn guess_lang(path: Option<&std::path::Path>, head: &[u16]) -> Lang {
    if let Some(p) = path {
        let l = Lang::from_path(p);
        if l != Lang::Plain {
            return l;
        }
        let ext = p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
        if matches!(ext.as_deref(), Some("txt" | "text" | "log" | "csv" | "tsv")) {
            return Lang::Plain;
        }
    }
    match Lang::sniff(head) {
        Lang::Json if !looks_like_json(head) => Lang::Plain,
        l => l,
    }
}

/// The start of a document parses as JSON (it may be cut off mid-way).
fn looks_like_json(head: &[u16]) -> bool {
    let parsed = crate::json::parse(head, crate::json::Options { jsonc: true, lines: false });
    let truncated = |p: &data::Problem| {
        p.message.contains("never closed") || p.message.contains("end of file") || p.end + 2 >= head.len()
    };
    parsed.problems.iter().filter(|p| !p.warning && !truncated(p)).count() == 0
}

/// First and last character offsets showing in the editor.
fn visible_range(edit: HWND) -> (i32, i32) {
    let first = sm(edit, EM_GETFIRSTVISIBLELINE, 0, 0).0.max(0);
    let start = sm(edit, EM_LINEINDEX, first as usize, 0).0.max(0) as i32;
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(edit, &mut rc);
    }
    let pt = POINTL { x: rc.right, y: rc.bottom };
    let end = sm(edit, EM_CHARFROMPOS, 0, &pt as *const POINTL as isize).0 as i32;
    (start, end.max(start))
}

/// Indentation style from a document's first lines: tabs, or the most
/// common step between space-indented lines.
fn detect_indent(head: &[u16]) -> Option<Indent> {
    let mut tabs = 0;
    let mut spaces = 0;
    let mut steps = [0usize; 9];
    let mut prev = 0usize;
    for line in head.split(|&c| c == 13).take(2000) {
        if line.iter().all(|&c| c == 32 || c == 9) {
            continue;
        }
        if line.first() == Some(&9) {
            tabs += 1;
            continue;
        }
        let n = line.iter().take_while(|&&c| c == 32).count();
        if n > 0 {
            spaces += 1;
        }
        let step = n.abs_diff(prev);
        if (2..=8).contains(&step) {
            steps[step] += 1;
        }
        prev = n;
    }
    if tabs > spaces && tabs > 0 {
        return Some(Indent { tabs: true, size: 4 });
    }
    if spaces == 0 {
        return None;
    }
    let best = [2usize, 4, 3, 8].into_iter().max_by_key(|&s| (steps[s], s == 4 || s == 2))?;
    (steps[best] > 0).then_some(Indent { tabs: false, size: best as u8 })
}

/// True if the YAML has a real comment (not a '#' inside a value).
fn yaml_has_comments(text: &[u16]) -> bool {
    let mut st = State::default();
    let mut spans = Vec::new();
    for line in text.split(|&c| c == 13) {
        spans.clear();
        st = syntax::lex_line(Lang::Yaml, st, line, Some(&mut spans));
        if spans.iter().any(|s| s.kind == Kind::Comment) {
            return true;
        }
    }
    false
}

/// The one tool of the problem tooltip (`text` must be null-terminated).
fn tool_info(owner: HWND, text: &[u16]) -> TTTOOLINFOW {
    TTTOOLINFOW {
        cbSize: size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_TRACK | TTF_ABSOLUTE,
        hwnd: owner,
        uId: 1,
        lpszText: PWSTR(text.as_ptr() as *mut u16),
        ..Default::default()
    }
}

/// Puts text on the clipboard.
pub(super) fn set_clipboard(owner: HWND, text: &str) {
    use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
    let w: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        if OpenClipboard(Some(owner)).is_err() {
            return;
        }
        let _ = EmptyClipboard();
        if let Ok(h) = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2) {
            let p = GlobalLock(h) as *mut u16;
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
                let _ = GlobalUnlock(h);
                let _ = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(h.0)));
            }
        }
        let _ = CloseClipboard();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.replace('\n', "\r").encode_utf16().collect()
    }

    #[test]
    fn indentation_detection() {
        assert_eq!(detect_indent(&w("a:\n  b:\n    c: 1\n  d: 2\n")), Some(Indent { tabs: false, size: 2 }));
        assert_eq!(detect_indent(&w("{\n    \"a\": {\n        \"b\": 1\n    }\n}")), Some(Indent { tabs: false, size: 4 }));
        assert_eq!(detect_indent(&w("fn x() {\n\tlet a;\n\tlet b;\n}")), Some(Indent { tabs: true, size: 4 }));
        assert_eq!(detect_indent(&w("plain\ntext\n")), None);
    }

    #[test]
    fn yaml_comment_detection() {
        assert!(yaml_has_comments(&w("a: 1 # note\n")));
        assert!(!yaml_has_comments(&w("a: \"x # y\"\nb: c#d\n")));
    }
}
