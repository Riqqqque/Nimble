//! Find / Replace / Go-to bar — a slim strip that drops in under the tab
//! strip (like Notepad's). One child window with real EDIT controls plus
//! hand-drawn buttons, so it matches both themes.

use std::ffi::c_void;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Controls::{EM_SETSEL, WM_MOUSELEAVE};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

use crate::app::{self, App, hiword, pt_in_rect, scale, sm, wide};
use crate::draw::{self, Aa};
use crate::theme;

pub const BAR_CLASS: PCWSTR = w!("NimbleFindBar");

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BarMode {
    Find,
    Replace,
    Goto,
}

// Drawn-button ids.
const BTN_NEXT: u32 = 1;
const BTN_PREV: u32 = 2;
const BTN_REPLACE: u32 = 3;
const BTN_ALL: u32 = 4;
const BTN_CLOSE: u32 = 5;
const BTN_GO: u32 = 6;
const EDIT_SUB: usize = 9;

pub struct Bar {
    pub hwnd: HWND,
    edit_find: HWND,
    edit_repl: HWND,
    app: *mut App,
    mode: BarMode,
    hot: u32,
    press: u32,
    dpi: u32,
    brush_edit: HBRUSH,
    /// Right-side hint: "No results", "Replaced 3 matches".
    note: String,
    /// The last search failed — the find field's ring turns red.
    miss: bool,
    /// Set while the bar fills its own field, so that doesn't search.
    quiet: bool,
}

/// Register the bar class once.
pub fn register(hinst: HINSTANCE) {
    unsafe {
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(bar_proc),
            hInstance: hinst,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: BAR_CLASS,
            ..Default::default()
        };
        let _ = RegisterClassExW(&wc);
    }
}

/// Geometry for every piece of the bar, in bar-client coords. Shared by
/// paint, hit-testing, and child placement.
struct Parts {
    find: RECT,
    repl: RECT,
    buttons: Vec<(u32, RECT, &'static str)>,
    note: RECT,
}

fn layout(s: &Bar, w: i32, h: i32) -> Parts {
    let sc = |v: i32| scale(v, s.dpi);
    let ctl_h = sc(26);
    let top = (h - ctl_h) / 2;
    let close_side = sc(26);
    let close = RECT {
        left: w - close_side - sc(8),
        top: (h - close_side) / 2,
        right: w - sc(8),
        bottom: (h + close_side) / 2,
    };

    let fields = if s.mode == BarMode::Replace { 2 } else { 1 };
    let buttons: &[(u32, &'static str, i32)] = match s.mode {
        BarMode::Find => &[(BTN_NEXT, "Next", 56), (BTN_PREV, "Previous", 76)],
        BarMode::Replace => &[
            (BTN_NEXT, "Next", 56),
            (BTN_PREV, "Previous", 76),
            (BTN_REPLACE, "Replace", 70),
            (BTN_ALL, "Replace all", 92),
        ],
        BarMode::Goto => &[(BTN_GO, "Go", 48)],
    };
    let label_w = sc(if s.mode == BarMode::Goto { 72 } else { 40 });
    let with_w = if fields == 2 { sc(40) } else { 0 };
    let gap = sc(6);
    let fixed: i32 = sc(12)
        + label_w
        + with_w
        + buttons.iter().map(|b| sc(b.2) + gap).sum::<i32>()
        + gap * fields
        + (w - close.left)
        + sc(96); // leave room for the note
    let field_w = if s.mode == BarMode::Goto {
        sc(96)
    } else {
        ((w - fixed) / fields).clamp(sc(110), sc(260))
    };

    let mut x = sc(12) + label_w;
    let find = RECT {
        left: x,
        top,
        right: x + field_w,
        bottom: top + ctl_h,
    };
    x = find.right + gap;
    let mut repl = RECT::default();
    let mut out = Vec::new();
    for &(id, text, bw) in buttons {
        if id == BTN_REPLACE {
            // Second field sits between the search and replace buttons.
            x += with_w;
            repl = RECT {
                left: x,
                top,
                right: x + field_w,
                bottom: top + ctl_h,
            };
            x = repl.right + gap;
        }
        let r = RECT {
            left: x,
            top,
            right: x + sc(bw),
            bottom: top + ctl_h,
        };
        out.push((id, r, text));
        x = r.right + gap;
    }
    out.push((BTN_CLOSE, close, ""));
    Parts {
        find,
        repl,
        buttons: out,
        note: RECT {
            left: x + sc(6),
            top,
            right: close.left - sc(8),
            bottom: top + ctl_h,
        },
    }
}

fn state(bar: HWND) -> Option<&'static mut Bar> {
    if bar.0.is_null() {
        return None;
    }
    unsafe {
        let s = GetWindowLongPtrW(bar, GWLP_USERDATA) as *mut Bar;
        if s.is_null() { None } else { Some(&mut *s) }
    }
}

/// Re-apply theme colors and DPI to the bar and its edit fields.
pub fn retheme(app: &App) {
    let Some(st) = state(app.findbar) else { return };
    unsafe {
        st.dpi = app.dpi.max(96);
        for e in [st.edit_find, st.edit_repl] {
            if e.0.is_null() {
                continue;
            }
            theme::set_control_dark(e, app.theme.dark);
            sm(e, WM_SETFONT, app.font.0 as usize, 1);
        }
        let _ = DeleteObject(HGDIOBJ(st.brush_edit.0));
        st.brush_edit = HBRUSH::default();
        relayout(st);
        let _ = InvalidateRect(Some(app.findbar), None, true);
    }
}

/// Field text including the terminating NUL.
fn field_text(e: HWND) -> Vec<u16> {
    if e.0.is_null() {
        return vec![0];
    }
    let n = sm(e, WM_GETTEXTLENGTH, 0, 0).0.max(0) as usize;
    let mut buf = vec![0u16; n + 1];
    let got = unsafe { GetWindowTextW(e, &mut buf) };
    buf.truncate(got.max(0) as usize + 1);
    buf
}

fn set_field(s: &mut Bar, e: HWND, text: &[u16]) {
    if text.last() != Some(&0) {
        return;
    }
    s.quiet = true;
    unsafe {
        let _ = SetWindowTextW(e, PCWSTR(text.as_ptr()));
    }
    s.quiet = false;
}

/// Places the EDIT children inside their drawn wells, vertically centered
/// on the font (single-line EDITs always draw from the top).
fn relayout(s: &mut Bar) {
    unsafe {
        let mut rc = RECT::default();
        let _ = GetClientRect(s.hwnd, &mut rc);
        let p = layout(s, rc.right, rc.bottom);
        let app = &*s.app;
        let hdc = GetDC(Some(s.hwnd));
        let old = SelectObject(hdc, HGDIOBJ(app.font.0));
        let mut tm = TEXTMETRICW::default();
        let _ = GetTextMetricsW(hdc, &mut tm);
        SelectObject(hdc, old);
        let _ = ReleaseDC(Some(s.hwnd), hdc);
        let pad = scale(8, s.dpi);
        let place = |e: HWND, r: &RECT| {
            let th = tm.tmHeight.min(r.bottom - r.top);
            let _ = MoveWindow(
                e,
                r.left + pad,
                r.top + (r.bottom - r.top - th) / 2,
                (r.right - r.left - pad * 2).max(0),
                th,
                true,
            );
        };
        place(s.edit_find, &p.find);
        if s.mode == BarMode::Replace {
            place(s.edit_repl, &p.repl);
            let _ = ShowWindow(s.edit_repl, SW_SHOW);
        } else {
            let _ = ShowWindow(s.edit_repl, SW_HIDE);
        }
    }
}

fn draw_text(hdc: HDC, text: &str, rc: &RECT, flags: DRAW_TEXT_FORMAT) {
    let mut t = wide(text);
    let n = t.len() - 1;
    if n == 0 {
        return;
    }
    let mut r = *rc;
    unsafe {
        DrawTextW(hdc, &mut t[..n], &mut r, flags | DT_VCENTER | DT_SINGLELINE);
    }
}

fn paint(s: &Bar, hdc: HDC, w: i32, h: i32) {
    unsafe {
        let app = &*s.app;
        let th = app.theme;
        let d = s.dpi;
        let all = RECT {
            left: 0,
            top: 0,
            right: w,
            bottom: h,
        };
        let b = CreateSolidBrush(th.chrome_bg);
        FillRect(hdc, &all, b);
        let _ = DeleteObject(HGDIOBJ(b.0));
        // Bottom hairline separating bar from editor.
        let mut ln = all;
        ln.top = ln.bottom - 1;
        let lb = CreateSolidBrush(th.menu_sep);
        FillRect(hdc, &ln, lb);
        let _ = DeleteObject(HGDIOBJ(lb.0));

        let aa = Aa::new(hdc);
        let p = layout(s, w, h);
        let old = SelectObject(hdc, HGDIOBJ(app.font.0));
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, th.tab_dim);
        let label = if s.mode == BarMode::Goto { "Go to line" } else { "Find" };
        let lr = RECT {
            left: scale(12, d),
            right: p.find.left,
            ..p.find
        };
        draw_text(hdc, label, &lr, DT_LEFT);
        if s.mode == BarMode::Replace {
            let wr = RECT {
                left: p.repl.left - scale(40, d),
                right: p.repl.left,
                ..p.repl
            };
            draw_text(hdc, "With", &wr, DT_LEFT);
        }

        // Field wells: rounded, editor-colored, with a ring that turns
        // accent when focused and red after a failed search.
        let focus = GetFocus();
        for (rc, e) in [(p.find, s.edit_find), (p.repl, s.edit_repl)] {
            if rc.right == 0 {
                continue;
            }
            let ring = if e.0 == s.edit_find.0 && s.miss {
                th.danger
            } else if focus.0 == e.0 {
                th.accent
            } else {
                th.menu_sep
            };
            aa.fill_rounded(&rc, scale(5, d), false, th.edit_bg);
            aa.stroke_rounded(&rc, scale(5, d), ring, 1.0);
        }

        for (id, rc, text) in &p.buttons {
            let hot = s.hot == *id;
            let pressed = s.press == *id;
            if *id == BTN_CLOSE {
                if hot || pressed {
                    aa.fill_rounded(rc, scale(5, d), false, th.menu_hover);
                }
                app::draw_close_glyph(&aa, rc, th.menu_fg, d);
                continue;
            }
            let bg = if pressed {
                th.menu_sep
            } else if hot {
                th.menu_hover
            } else {
                th.edit_bg
            };
            aa.fill_rounded(rc, scale(5, d), false, bg);
            aa.stroke_rounded(rc, scale(5, d), th.menu_sep, 1.0);
            SetTextColor(hdc, th.menu_fg);
            draw_text(hdc, text, rc, DT_CENTER);
        }

        if !s.note.is_empty() && p.note.right > p.note.left {
            SetTextColor(hdc, if s.miss { th.danger } else { th.tab_dim });
            draw_text(hdc, &s.note, &p.note, DT_LEFT | DT_END_ELLIPSIS);
        }
        SelectObject(hdc, old);
    }
}

fn run_button(s: &mut Bar, id: u32) {
    unsafe {
        let app = &mut *s.app;
        match id {
            BTN_NEXT => app.find_next(true),
            BTN_PREV => app.find_next(false),
            BTN_REPLACE => app.replace_next(),
            BTN_ALL => app.replace_all(),
            BTN_GO => app.goto_from_bar(),
            BTN_CLOSE => app.close_findbar(),
            _ => {}
        }
    }
}

/// Enter/Escape/F3/Tab inside the bar's edit fields.
unsafe extern "system" fn bar_edit_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    refdata: usize,
) -> LRESULT {
    unsafe {
        let bar = HWND(refdata as *mut c_void);
        if let Some(s) = state(bar) {
            let shift = (GetKeyState(VK_SHIFT.0 as i32) as u16 & 0x8000) != 0;
            if msg == WM_KEYDOWN {
                match VIRTUAL_KEY(wparam.0 as u16) {
                    VK_RETURN => {
                        match s.mode {
                            BarMode::Goto => run_button(s, BTN_GO),
                            BarMode::Replace if hwnd.0 == s.edit_repl.0 => {
                                run_button(s, if shift { BTN_ALL } else { BTN_REPLACE })
                            }
                            _ => run_button(s, if shift { BTN_PREV } else { BTN_NEXT }),
                        }
                        return LRESULT(0);
                    }
                    VK_ESCAPE => {
                        run_button(s, BTN_CLOSE);
                        return LRESULT(0);
                    }
                    VK_F3 => {
                        run_button(s, if shift { BTN_PREV } else { BTN_NEXT });
                        return LRESULT(0);
                    }
                    VK_TAB if s.mode == BarMode::Replace => {
                        let other = if hwnd.0 == s.edit_find.0 { s.edit_repl } else { s.edit_find };
                        let _ = SetFocus(Some(other));
                        sm(other, EM_SETSEL, 0, -1);
                        return LRESULT(0);
                    }
                    _ => {}
                }
            }
            // Swallow the chars for keys handled above so EDIT doesn't beep.
            if msg == WM_CHAR && matches!(wparam.0, 0x0D | 0x1B | 0x09) {
                return LRESULT(0);
            }
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }
}

fn button_at(s: &Bar, hwnd: HWND, lparam: LPARAM) -> u32 {
    let (x, y) = ((lparam.0 as i16) as i32, ((lparam.0 >> 16) as i16) as i32);
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
    }
    layout(s, rc.right, rc.bottom)
        .buttons
        .iter()
        .find(|(_, r, _)| pt_in_rect(r, x, y))
        .map(|(i, _, _)| *i)
        .unwrap_or(0)
}

unsafe extern "system" fn bar_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            WM_CREATE => {
                let cs = &*(lparam.0 as *const CREATESTRUCTW);
                let s = cs.lpCreateParams as *mut Bar;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, s as isize);
                (*s).hwnd = hwnd;
                (*s).dpi = GetDpiForWindow(hwnd).max(96);
                for slot in 0..2 {
                    let e = CreateWindowExW(
                        WINDOW_EX_STYLE(0),
                        w!("EDIT"),
                        PCWSTR::null(),
                        WINDOW_STYLE(
                            (WS_CHILD | WS_VISIBLE | WS_TABSTOP).0 | ES_AUTOHSCROLL as u32,
                        ),
                        0,
                        0,
                        0,
                        0,
                        Some(hwnd),
                        None,
                        None,
                        None,
                    )
                    .unwrap_or_default();
                    sm(e, WM_SETFONT, (*(*s).app).font.0 as usize, 1);
                    let _ = SetWindowSubclass(e, Some(bar_edit_proc), EDIT_SUB, hwnd.0 as usize);
                    if slot == 0 {
                        (*s).edit_find = e;
                    } else {
                        (*s).edit_repl = e;
                    }
                }
                return LRESULT(0);
            }
            WM_NCDESTROY => {
                let s = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Bar;
                if !s.is_null() {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    let _ = DeleteObject(HGDIOBJ((*s).brush_edit.0));
                    drop(Box::from_raw(s));
                }
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            }
            _ => {}
        }
        let Some(st) = state(hwnd) else {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        };
        match msg {
            WM_SIZE => {
                relayout(st);
                LRESULT(0)
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                let mut rc = RECT::default();
                let _ = GetClientRect(hwnd, &mut rc);
                draw::buffered(hdc, &rc, |mem| paint(st, mem, rc.right, rc.bottom));
                let _ = EndPaint(hwnd, &mut ps);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_CTLCOLOREDIT => {
                // Field colors come straight from the theme.
                let app = &*st.app;
                let hdc = HDC(wparam.0 as *mut c_void);
                SetBkColor(hdc, app.theme.edit_bg);
                SetTextColor(hdc, app.theme.edit_fg);
                if st.brush_edit.0.is_null() {
                    st.brush_edit = CreateSolidBrush(app.theme.edit_bg);
                }
                LRESULT(st.brush_edit.0 as isize)
            }
            WM_COMMAND => {
                let code = hiword(wparam.0);
                let from = lparam.0 as *mut c_void;
                if code == EN_CHANGE && from == st.edit_find.0 && !st.quiet {
                    if st.mode == BarMode::Goto {
                        st.note.clear();
                        st.miss = false;
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    } else {
                        (*st.app).find_incremental();
                    }
                } else if code == EN_SETFOCUS || code == EN_KILLFOCUS {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let id = button_at(st, hwnd, lparam);
                if id != st.hot {
                    st.hot = id;
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut tme);
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                if st.hot != 0 {
                    st.hot = 0;
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                st.press = button_at(st, hwnd, lparam);
                if st.press != 0 {
                    let _ = SetCapture(hwnd);
                }
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let id = button_at(st, hwnd, lparam);
                let pressed = st.press;
                st.press = 0;
                let _ = ReleaseCapture();
                let _ = InvalidateRect(Some(hwnd), None, false);
                if pressed == id && id != 0 {
                    // May destroy the bar (Close, Go) — touch nothing after.
                    run_button(st, id);
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Create (or re-show) the bar under the tab strip in `mode`, seeding the
/// field: the caret's line for Go To, otherwise the selection or the last
/// search.
pub fn open(app: &mut App, mode: BarMode) {
    unsafe {
        let fresh = app.findbar.0.is_null();
        if fresh {
            let st = Box::new(Bar {
                hwnd: HWND::default(),
                edit_find: HWND::default(),
                edit_repl: HWND::default(),
                app: app as *mut App,
                mode,
                hot: 0,
                press: 0,
                dpi: app.dpi.max(96),
                brush_edit: HBRUSH::default(),
                note: String::new(),
                miss: false,
                quiet: false,
            });
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                BAR_CLASS,
                PCWSTR::null(),
                WS_CHILD | WS_CLIPCHILDREN,
                0,
                0,
                0,
                0,
                Some(app.hwnd),
                None,
                None,
                Some(Box::into_raw(st) as *const c_void),
            )
            .unwrap_or_default();
            app.findbar = hwnd;
        }
        let selected = app.selected_text();
        let line = app.caret_line();
        let Some(s) = state(app.findbar) else { return };
        let prev = s.mode;
        s.mode = mode;
        s.note.clear();
        s.miss = false;
        if fresh {
            theme::set_control_dark(s.edit_find, app.theme.dark);
            theme::set_control_dark(s.edit_repl, app.theme.dark);
        }
        let seed = match mode {
            BarMode::Goto => Some(wide(&line.to_string())),
            _ => selected
                .map(|t| wide(&t))
                .or_else(|| (fresh || prev == BarMode::Goto).then(|| app.find_query.clone())),
        };
        if let Some(seed) = seed {
            set_field(s, s.edit_find, &seed);
        }
        let _ = ShowWindow(app.findbar, SW_SHOW);
        app.layout();
        relayout(s);
        let _ = InvalidateRect(Some(app.findbar), None, false);
        let _ = SetFocus(Some(s.edit_find));
        sm(s.edit_find, EM_SETSEL, 0, -1);
    }
}

/// Destroy the bar (idempotent), remembering the search for F3.
pub fn close(app: &mut App) {
    let Some(s) = state(app.findbar) else { return };
    if s.mode != BarMode::Goto {
        app.find_query = field_text(s.edit_find);
    }
    unsafe {
        let _ = DestroyWindow(app.findbar);
    }
    app.findbar = HWND::default();
    app.layout();
}

/// Shows a hint on the bar's right side; `miss` also turns the field red.
pub fn set_note(app: &App, note: &str, miss: bool) {
    let Some(s) = state(app.findbar) else { return };
    if s.note == note && s.miss == miss {
        return;
    }
    s.note = note.to_string();
    s.miss = miss;
    unsafe {
        let _ = InvalidateRect(Some(app.findbar), None, false);
    }
}

/// The find field's text (NUL-terminated), or the last search when the bar
/// is closed.
pub fn query(app: &App) -> Vec<u16> {
    match state(app.findbar) {
        Some(s) => field_text(s.edit_find),
        None if app.find_query.is_empty() => vec![0],
        None => app.find_query.clone(),
    }
}

/// The replace field's text (NUL-terminated).
pub fn repl_text(app: &App) -> Vec<u16> {
    match state(app.findbar) {
        Some(s) if s.mode == BarMode::Replace => field_text(s.edit_repl),
        _ => vec![0],
    }
}

/// Height the bar occupies when visible.
pub fn height(app: &App) -> i32 {
    if app.findbar.0.is_null() {
        0
    } else {
        scale(42, app.dpi)
    }
}
