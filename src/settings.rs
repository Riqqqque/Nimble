//! Persisted preferences and remappable shortcuts under
//! HKCU\Software\Nimble, plus the Settings window itself — hand-drawn to
//! match the app chrome (no stock dialog gray).

use std::ffi::c_void;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

use crate::app::{self, App, scale, wide};
use crate::draw::Aa;
use crate::theme::{self, Theme, ThemeMode};

// ---- key bindings ----------------------------------------------------------

pub const MOD_CTRL: u8 = 1;
pub const MOD_ALT: u8 = 2;
pub const MOD_SHIFT: u8 = 4;

/// A keyboard shortcut: modifier bitmask plus a virtual-key code.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Bind {
    pub mods: u8,
    pub vk: u16,
}

impl Bind {
    pub const fn new(mods: u8, vk: u16) -> Self {
        Self { mods, vk }
    }

    /// Registry DWORD: vk in the low word, mods in bits 16-23. A stored vk
    /// of 0 means "explicitly unbound"; a missing value means default.
    pub fn encode(self) -> u32 {
        self.vk as u32 | ((self.mods as u32) << 16)
    }

    pub fn decode(v: u32) -> Option<Bind> {
        let vk = (v & 0xFFFF) as u16;
        if vk == 0 {
            return None;
        }
        Some(Bind {
            mods: ((v >> 16) & 0xFF) as u8,
            vk,
        })
    }

    /// "Ctrl+Shift+S", "F3", "Del"
    pub fn text(self) -> String {
        let mut s = String::new();
        if self.mods & MOD_CTRL != 0 {
            s.push_str("Ctrl+");
        }
        if self.mods & MOD_ALT != 0 {
            s.push_str("Alt+");
        }
        if self.mods & MOD_SHIFT != 0 {
            s.push_str("Shift+");
        }
        s.push_str(&key_name(self.vk));
        s
    }
}

/// One bindable command shown in Settings.
pub struct Action {
    pub cmd: u32,
    pub name: &'static str,
    pub def: Option<Bind>,
}

/// Display name for a virtual-key code — ASCII where possible, otherwise
/// the localized scan-code name ("Tab", "F3", "Page Up").
pub fn key_name(vk: u16) -> String {
    match VIRTUAL_KEY(vk) {
        VK_OEM_PLUS => return "Plus".into(),
        VK_OEM_MINUS => return "Minus".into(),
        VK_ADD => return "Num +".into(),
        VK_SUBTRACT => return "Num -".into(),
        _ => {}
    }
    unsafe {
        let ch = MapVirtualKeyW(vk as u32, MAPVK_VK_TO_CHAR);
        if ch != 0 {
            if let Some(c) = char::from_u32(ch) {
                if c.is_ascii_graphic() {
                    return c.to_ascii_uppercase().to_string();
                }
            }
        }
        // The _EX mapping flags extended keys (Delete, Home, arrows) with an
        // E0 prefix; without bit 24 they'd be named as their numpad twins.
        let sc = MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC_EX);
        let ext = if sc & 0xFF00 == 0xE000 { 1 << 24 } else { 0 };
        let mut buf = [0u16; 48];
        let n = GetKeyNameTextW((((sc & 0xFF) << 16) | ext | (1 << 25)) as i32, &mut buf);
        let raw = if n > 0 {
            String::from_utf16_lossy(&buf[..n as usize])
        } else {
            return format!("VK{vk}");
        };
        // GetKeyNameTextW SHOUTS ("PAGE UP") — title-case it.
        raw.split(' ')
            .map(|w| {
                let mut c = w.chars();
                match c.next() {
                    Some(f) => f.to_ascii_uppercase().to_string() + &c.as_str().to_lowercase(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Build the accelerator table from the current bindings.
pub fn build_accel(actions: &[Action], binds: &[Option<Bind>]) -> HACCEL {
    let mut list = Vec::with_capacity(binds.len());
    for (a, b) in actions.iter().zip(binds.iter()) {
        let Some(b) = b else { continue };
        let mut f = FVIRTKEY;
        if b.mods & MOD_CTRL != 0 {
            f |= FCONTROL;
        }
        if b.mods & MOD_ALT != 0 {
            f |= FALT;
        }
        if b.mods & MOD_SHIFT != 0 {
            f |= FSHIFT;
        }
        list.push(ACCEL {
            fVirt: f,
            key: b.vk,
            cmd: a.cmd as u16,
        });
    }
    unsafe { CreateAcceleratorTableW(&list).unwrap_or_default() }
}

// ---- persistence -----------------------------------------------------------

fn reg_read(sub: &str, name: &str) -> Option<u32> {
    unsafe {
        let mut v = 0u32;
        let mut size = size_of::<u32>() as u32;
        let key = wide(&format!("Software\\Nimble\\{sub}"));
        let val = wide(name);
        let r = RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(val.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut v as *mut u32 as *mut c_void),
            Some(&mut size),
        );
        r.is_ok().then_some(v)
    }
}

fn reg_write(sub: &str, name: &str, v: u32) {
    unsafe {
        let key = wide(&format!("Software\\Nimble\\{sub}"));
        let val = wide(name);
        let _ = RegSetKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(val.as_ptr()),
            REG_DWORD.0,
            Some(&v as *const u32 as *const c_void),
            size_of::<u32>() as u32,
        );
    }
}

/// A stored binding wins; none stored falls back to the table default.
pub fn load_binds(actions: &[Action]) -> Vec<Option<Bind>> {
    actions
        .iter()
        .map(|a| match reg_read("Keys", &a.cmd.to_string()) {
            Some(v) => Bind::decode(v),
            None => a.def,
        })
        .collect()
}

pub fn save_bind(cmd: u32, b: Option<Bind>) {
    reg_write("Keys", &cmd.to_string(), b.map(Bind::encode).unwrap_or(0));
}

/// Forget every custom binding (defaults come back).
pub fn reset_binds() {
    unsafe {
        let _ = RegDeleteKeyW(HKEY_CURRENT_USER, w!("Software\\Nimble\\Keys"));
    }
}

#[derive(Clone, Copy)]
pub struct Prefs {
    pub line_numbers: bool,
    pub auto_indent: bool,
    pub word_wrap: bool,
    pub status_bar: bool,
    pub font_pt: i32,
}

pub fn load_prefs() -> Prefs {
    Prefs {
        line_numbers: reg_read("", "LineNumbers").unwrap_or(1) != 0,
        auto_indent: reg_read("", "AutoIndent").unwrap_or(1) != 0,
        word_wrap: reg_read("", "WordWrap").unwrap_or(1) != 0,
        status_bar: reg_read("", "StatusBar").unwrap_or(1) != 0,
        font_pt: reg_read("", "FontPt").unwrap_or(11).clamp(6, 72) as i32,
    }
}

pub fn save_pref(name: &str, v: u32) {
    reg_write("", name, v);
}

// ---- settings window --------------------------------------------------------

const SET_CLASS: PCWSTR = w!("NimbleSettings");

// Hit-test ids for interactive rows.
const ID_THEME: u32 = 1; // + ThemeMode as u32
const ID_CHECK: u32 = 10; // + pref index
const ID_FONT_MINUS: u32 = 20;
const ID_FONT_PLUS: u32 = 21;
const ID_BIND: u32 = 30; // + action index
const ID_RESET: u32 = 60;
const ID_DONE: u32 = 61;
const ID_SCROLLBAR: u32 = 70;

// Pref checkbox rows, in display order.
const PREFS: [(&str, &str); 4] = [
    ("Line numbers", "LineNumbers"),
    ("Status bar", "StatusBar"),
    ("Word wrap", "WordWrap"),
    ("Auto-indent new lines", "AutoIndent"),
];

struct SWin {
    app: *mut App,
    scroll: i32,
    content_h: i32,
    capture: i32, // action index being rebound, -1 = none
    hot: u32,     // hit id under the cursor, 0 = none
    press: u32,   // hit id currently pressed
    drag_scroll: Option<(i32, i32)>, // (grab y, scroll at grab)
    dpi: u32,
}

/// Rows are recomputed for paint and hit-testing from one shared layout.
/// `out` receives (hit id, rect); id 0 = decorative text.
fn build_rows(s: &SWin, w: i32) -> Vec<(u32, RECT, RowKind)> {
    let d = s.dpi as i32;
    let sc = |v: i32| v * d / 96;
    let pad = sc(20);
    let row_h = sc(32);
    let mut y = sc(14);
    let mut rows = Vec::new();
    let header = |t: &'static str, y: &mut i32, rows: &mut Vec<(u32, RECT, RowKind)>| {
        *y += sc(6);
        rows.push((
            0,
            RECT {
                left: pad,
                top: *y,
                right: w - pad,
                bottom: *y + sc(20),
            },
            RowKind::Header(t),
        ));
        *y += sc(28);
    };
    // Appearance
    header("Appearance", &mut y, &mut rows);
    rows.push((
        0,
        RECT {
            left: pad,
            top: y,
            right: w - pad,
            bottom: y + row_h,
        },
        RowKind::ThemeRow,
    ));
    y += row_h + sc(6);
    // Editor
    header("Editor", &mut y, &mut rows);
    for (i, (name, _)) in PREFS.iter().enumerate() {
        rows.push((
            ID_CHECK + i as u32,
            RECT {
                left: pad,
                top: y,
                right: w - pad,
                bottom: y + row_h,
            },
            RowKind::Check(i as u32, name),
        ));
        y += row_h;
    }
    rows.push((
        0,
        RECT {
            left: pad,
            top: y,
            right: w - pad,
            bottom: y + row_h,
        },
        RowKind::FontSize,
    ));
    y += row_h + sc(6);
    // Shortcuts
    header("Keyboard shortcuts", &mut y, &mut rows);
    for i in 0..app::ACTIONS.len() {
        rows.push((
            ID_BIND + i as u32,
            RECT {
                left: pad,
                top: y,
                right: w - pad,
                bottom: y + row_h,
            },
            RowKind::BindRow(i),
        ));
        y += row_h;
    }
    rows
}

/// The three theme pills inside the Appearance row.
fn theme_pills(rc: &RECT, dpi: u32) -> [RECT; 3] {
    let bw = scale(96, dpi);
    let step = bw + scale(8, dpi);
    let left = rc.left + scale(8, dpi);
    std::array::from_fn(|i| RECT {
        left: left + step * i as i32,
        top: rc.top + scale(4, dpi),
        right: left + step * i as i32 + bw,
        bottom: rc.bottom - scale(4, dpi),
    })
}

/// (minus, value, plus) boxes inside the font-size row.
fn font_buttons(rc: &RECT, dpi: u32) -> (RECT, RECT, RECT) {
    let bw = scale(28, dpi);
    let bh = scale(24, dpi);
    let cy = (rc.top + rc.bottom) / 2;
    let minus = RECT {
        left: rc.right - scale(110, dpi),
        top: cy - bh / 2,
        right: rc.right - scale(110, dpi) + bw,
        bottom: cy + bh / 2,
    };
    let plus = RECT {
        left: rc.right - bw,
        top: cy - bh / 2,
        right: rc.right,
        bottom: cy + bh / 2,
    };
    let val = RECT {
        left: minus.right,
        right: plus.left,
        ..minus
    };
    (minus, val, plus)
}

enum RowKind {
    Header(&'static str),
    ThemeRow,
    Check(u32, &'static str),
    FontSize,
    BindRow(usize),
}

fn app_theme(s: &SWin) -> Theme {
    unsafe { (*s.app).theme }
}

fn footer_h(dpi: u32) -> i32 {
    scale(56, dpi)
}

fn scroll_info(s: &SWin, w: i32, h: i32) -> (i32, RECT, RECT) {
    // (max_scroll, track rect, thumb rect)
    let d = s.dpi as i32;
    let view_h = h - footer_h(s.dpi);
    let max_scroll = (s.content_h - view_h).max(0);
    let track = RECT {
        left: w - scale(10, s.dpi),
        top: 0,
        right: w,
        bottom: view_h,
    };
    let _ = d;
    if max_scroll == 0 {
        return (0, track, RECT::default());
    }
    let thumb_h = (view_h * view_h / s.content_h).max(scale(24, s.dpi));
    let ty = s.scroll * (view_h - thumb_h) / max_scroll;
    let thumb = RECT {
        left: track.left + 2,
        top: ty,
        right: track.right - 2,
        bottom: ty + thumb_h,
    };
    (max_scroll, track, thumb)
}

fn hit_test(s: &SWin, w: i32, h: i32, x: i32, y: i32) -> u32 {
    let view_h = h - footer_h(s.dpi);
    let (_, track, thumb) = scroll_info(s, w, h);
    if crate::app::pt_in_rect(&track, x, y) {
        return ID_SCROLLBAR;
    }
    let _ = thumb;
    if y >= view_h {
        // Footer buttons.
        let (_, _, fr, dr) = footer_buttons(s, w, h);
        if crate::app::pt_in_rect(&fr, x, y) {
            return ID_RESET;
        }
        if crate::app::pt_in_rect(&dr, x, y) {
            return ID_DONE;
        }
        return 0;
    }
    for (id, rc, kind) in build_rows(s, w) {
        let rc = RECT {
            top: rc.top - s.scroll,
            bottom: rc.bottom - s.scroll,
            ..rc
        };
        if !crate::app::pt_in_rect(&rc, x, y) {
            continue;
        }
        match kind {
            RowKind::ThemeRow => {
                return theme_pills(&rc, s.dpi)
                    .iter()
                    .position(|p| crate::app::pt_in_rect(p, x, y))
                    .map(|i| ID_THEME + i as u32)
                    .unwrap_or(0);
            }
            RowKind::FontSize => {
                let (minus, _, plus) = font_buttons(&rc, s.dpi);
                if crate::app::pt_in_rect(&minus, x, y) {
                    return ID_FONT_MINUS;
                }
                if crate::app::pt_in_rect(&plus, x, y) {
                    return ID_FONT_PLUS;
                }
                return 0;
            }
            _ if id != 0 => return id,
            _ => {}
        }
    }
    0
}

fn footer_buttons(s: &SWin, w: i32, h: i32) -> (RECT, RECT, RECT, RECT) {
    let fh = footer_h(s.dpi);
    let bh = scale(30, s.dpi);
    let by = h - fh + (fh - bh) / 2;
    let bw = scale(150, s.dpi);
    let reset = RECT {
        left: scale(20, s.dpi),
        top: by,
        right: scale(20, s.dpi) + bw,
        bottom: by + bh,
    };
    let done = RECT {
        left: w - scale(20, s.dpi) - scale(90, s.dpi),
        top: by,
        right: w - scale(20, s.dpi),
        bottom: by + bh,
    };
    (
        RECT {
            left: 0,
            top: h - fh,
            right: w,
            bottom: h,
        },
        RECT::default(),
        reset,
        done,
    )
}

fn paint(s: &SWin, hdc: HDC, w: i32, h: i32) {
    unsafe {
        let app = &*s.app;
        let th = app_theme(s);
        let d = s.dpi;
        let sc = |v: i32| scale(v, d);
        let aa = Aa::new(hdc);

        // Whole window background.
        let all = RECT {
            left: 0,
            top: 0,
            right: w,
            bottom: h,
        };
        let b = CreateSolidBrush(th.edit_bg);
        FillRect(hdc, &all, b);
        let _ = DeleteObject(HGDIOBJ(b.0));

        let view_h = h - footer_h(d);
        let font = app.font;
        let old = SelectObject(hdc, HGDIOBJ(font.0));
        SetBkMode(hdc, TRANSPARENT);

        // Clip scrolling content to the view area.
        let saved = SaveDC(hdc);
        let _ = IntersectClipRect(hdc, 0, 0, w, view_h);

        for (id, rc, kind) in build_rows(s, w) {
            let rc = RECT {
                top: rc.top - s.scroll,
                bottom: rc.bottom - s.scroll,
                ..rc
            };
            if rc.bottom < 0 || rc.top > view_h {
                continue;
            }
            match kind {
                RowKind::Header(t) => {
                    SetTextColor(hdc, th.menu_dim);
                    let mut r = rc;
                    let mut txt = wide(&t.to_uppercase());
                    let n = txt.len() - 1;
                    DrawTextW(
                        hdc,
                        &mut txt[..n],
                        &mut r,
                        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
                    );
                    let mut ln = rc;
                    ln.left += sc(0);
                    ln.top = r.bottom - 2;
                    ln.bottom = ln.top + 1;
                    ln.right = w - sc(20);
                    let bb = CreateSolidBrush(th.menu_sep);
                    FillRect(hdc, &ln, bb);
                    let _ = DeleteObject(HGDIOBJ(bb.0));
                }
                RowKind::ThemeRow => {
                    let modes = [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark];
                    let names = ["System", "Light", "Dark"];
                    let pills = theme_pills(&rc, d);
                    for (i, m) in modes.iter().enumerate() {
                        let on = app.mode == *m;
                        let id = ID_THEME + i as u32;
                        let pill = pills[i];
                        if on {
                            aa.fill_rounded(&pill, sc(6), false, th.accent);
                        } else {
                            aa.fill_rounded(
                                &pill,
                                sc(6),
                                false,
                                if s.hot == id { th.menu_hover } else { th.chrome_bg },
                            );
                            aa.stroke_rounded(&pill, sc(6), th.menu_sep, 1.0);
                        }
                        SetTextColor(
                            hdc,
                            if on {
                                crate::theme::rgb(0xFF, 0xFF, 0xFF)
                            } else {
                                th.menu_fg
                            },
                        );
                        let mut r = pill;
                        let mut t = wide(names[i]);
                        let n = t.len() - 1;
                        DrawTextW(
                            hdc,
                            &mut t[..n],
                            &mut r,
                            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                        );
                    }
                }
                RowKind::Check(idx, name) => {
                    if s.hot == id {
                        aa.fill_rounded(&rc, sc(5), false, th.menu_hover);
                    }
                    let on = match PREFS[idx as usize].1 {
                        "LineNumbers" => app.line_numbers,
                        "StatusBar" => app.show_status,
                        "WordWrap" => app.word_wrap,
                        "AutoIndent" => app.auto_indent,
                        _ => false,
                    };
                    let box_s = sc(16);
                    let bx = RECT {
                        left: rc.left + sc(8),
                        top: (rc.top + rc.bottom - box_s) / 2,
                        right: rc.left + sc(8) + box_s,
                        bottom: (rc.top + rc.bottom + box_s) / 2,
                    };
                    if on {
                        aa.fill_rounded(&bx, sc(4), false, th.accent);
                        // check glyph
                        let cx1 = bx.left + box_s / 4;
                        let cy = (bx.top + bx.bottom) / 2;
                        aa.stroke(
                            &[
                                (cx1, cy),
                                (bx.left + box_s * 2 / 5, bx.bottom - box_s / 4),
                                (bx.right - box_s / 5, bx.top + box_s / 4),
                            ],
                            crate::theme::rgb(0xFF, 0xFF, 0xFF),
                            sc(2).max(1) as f32,
                        );
                    } else {
                        aa.stroke_rounded(&bx, sc(4), th.menu_dim, 1.0);
                    }
                    SetTextColor(hdc, th.menu_fg);
                    let mut r = rc;
                    r.left += sc(34);
                    let mut t = wide(name);
                    let n = t.len() - 1;
                    DrawTextW(hdc, &mut t[..n], &mut r, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
                }
                RowKind::FontSize => {
                    SetTextColor(hdc, th.menu_fg);
                    let mut r = rc;
                    r.left += sc(8);
                    let mut t = wide("Editor font size");
                    let n = t.len() - 1;
                    DrawTextW(hdc, &mut t[..n], &mut r, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
                    let (minus, val, plus) = font_buttons(&rc, d);
                    for (rc2, label, hot_id) in
                        [(minus, "−", ID_FONT_MINUS), (plus, "+", ID_FONT_PLUS)]
                    {
                        let hot = s.hot == hot_id;
                        aa.fill_rounded(
                            &rc2,
                            sc(5),
                            false,
                            if hot { th.menu_hover } else { th.chrome_bg },
                        );
                        aa.stroke_rounded(&rc2, sc(5), th.menu_sep, 1.0);
                        SetTextColor(hdc, th.menu_fg);
                        let mut r3 = rc2;
                        let mut t2 = wide(label);
                        let n2 = t2.len() - 1;
                        DrawTextW(
                            hdc,
                            &mut t2[..n2],
                            &mut r3,
                            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                        );
                    }
                    SetTextColor(hdc, th.menu_fg);
                    let mut rv = val;
                    let mut tv = wide(&format!("{} pt", app.edit_pt));
                    let nv = tv.len() - 1;
                    DrawTextW(
                        hdc,
                        &mut tv[..nv],
                        &mut rv,
                        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                    );
                }
                RowKind::BindRow(i) => {
                    let a = &app::ACTIONS[i];
                    let capturing = s.capture == i as i32;
                    if s.hot == id || capturing {
                        aa.fill_rounded(
                            &rc,
                            sc(5),
                            false,
                            if capturing { th.tab_hot } else { th.menu_hover },
                        );
                    }
                    SetTextColor(hdc, th.menu_fg);
                    let mut r = rc;
                    r.left += sc(8);
                    let mut t = wide(a.name);
                    let n = t.len() - 1;
                    DrawTextW(hdc, &mut t[..n], &mut r, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
                    // Binding pill on the right.
                    let txt = if capturing {
                        "Type shortcut…".to_string()
                    } else {
                        app.binds[i].map(|b| b.text()).unwrap_or_else(|| "—".into())
                    };
                    let pw = sc(130);
                    let pr = RECT {
                        left: rc.right - pw,
                        top: rc.top + sc(5),
                        right: rc.right,
                        bottom: rc.bottom - sc(5),
                    };
                    aa.fill_rounded(&pr, sc(5), false, th.chrome_bg);
                    SetTextColor(hdc, if capturing { th.accent } else { th.menu_dim });
                    let mut r2 = pr;
                    let mut t2 = wide(&txt);
                    let n2 = t2.len() - 1;
                    DrawTextW(
                        hdc,
                        &mut t2[..n2],
                        &mut r2,
                        DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
                    );
                }
            }
        }
        let _ = RestoreDC(hdc, saved);

        // Scrollbar track + thumb.
        let (max_scroll, track, thumb) = scroll_info(s, w, h);
        if max_scroll > 0 {
            let b = CreateSolidBrush(th.chrome_bg);
            FillRect(hdc, &track, b);
            let _ = DeleteObject(HGDIOBJ(b.0));
            aa.fill_rounded(&thumb, sc(3), false, th.menu_dim);
        }

        // Footer: hairline + buttons.
        let (fr, _, reset, done) = footer_buttons(s, w, h);
        let b = CreateSolidBrush(th.edit_bg);
        FillRect(hdc, &fr, b);
        let _ = DeleteObject(HGDIOBJ(b.0));
        let mut ln = fr;
        ln.bottom = ln.top + 1;
        let bb = CreateSolidBrush(th.menu_sep);
        FillRect(hdc, &ln, bb);
        let _ = DeleteObject(HGDIOBJ(bb.0));

        for (r, label, id) in [
            (reset, "Restore defaults", ID_RESET),
            (done, "Done", ID_DONE),
        ] {
            let hot = s.hot == id;
            aa.fill_rounded(
                &r,
                sc(6),
                false,
                if id == ID_DONE {
                    th.accent
                } else if hot {
                    th.menu_hover
                } else {
                    th.chrome_bg
                },
            );
            SetTextColor(
                hdc,
                if id == ID_DONE {
                    crate::theme::rgb(0xFF, 0xFF, 0xFF)
                } else {
                    th.menu_fg
                },
            );
            let mut r2 = r;
            let mut t = wide(label);
            let n = t.len() - 1;
            DrawTextW(hdc, &mut t[..n], &mut r2, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        }

        SelectObject(hdc, old);
    }
}

fn toggle_pref(s: &mut SWin, idx: u32) {
    unsafe {
        let app = &mut *s.app;
        let (name, val) = match PREFS[idx as usize].1 {
            "LineNumbers" => ("LineNumbers", !app.line_numbers),
            "StatusBar" => ("StatusBar", !app.show_status),
            "WordWrap" => ("WordWrap", !app.word_wrap),
            "AutoIndent" => ("AutoIndent", !app.auto_indent),
            _ => return,
        };
        save_pref(name, val as u32);
        match name {
            "LineNumbers" => app.line_numbers = val,
            "StatusBar" => app.show_status = val,
            "WordWrap" => app.word_wrap = val,
            "AutoIndent" => app.auto_indent = val,
            _ => {}
        }
        app.apply_prefs();
    }
}

fn assign_bind(s: &mut SWin, idx: usize, b: Option<Bind>) {
    unsafe {
        let app = &mut *s.app;
        if let Some(nb) = b {
            // One shortcut, one command: clear any other action using it.
            for (j, cur) in app.binds.iter_mut().enumerate() {
                if j != idx && *cur == Some(nb) {
                    *cur = None;
                    save_bind(app::ACTIONS[j].cmd, None);
                }
            }
        }
        app.binds[idx] = b;
        save_bind(app::ACTIONS[idx].cmd, b);
        app.apply_binds();
    }
}

fn on_key(s: &mut SWin, hwnd: HWND, wparam: WPARAM) -> LRESULT {
    let vk = wparam.0 as u16;
    if s.capture >= 0 {
        let i = s.capture as usize;
        match vk as u32 {
            x if x == VK_ESCAPE.0 as u32 => {}
            x if x == VK_BACK.0 as u32 || x == VK_DELETE.0 as u32 => {
                assign_bind(s, i, None);
            }
            // Bare modifiers aren't shortcuts — keep capturing.
            x if [
                VK_SHIFT.0,
                VK_CONTROL.0,
                VK_MENU.0,
                VK_LSHIFT.0,
                VK_RSHIFT.0,
                VK_LCONTROL.0,
                VK_RCONTROL.0,
                VK_LMENU.0,
                VK_RMENU.0,
                VK_LWIN.0,
                VK_RWIN.0,
            ]
            .contains(&(x as u16)) =>
            {
                return LRESULT(0);
            }
            _ => {
                unsafe {
                    let ctrl = (GetKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000) != 0;
                    let alt = (GetKeyState(VK_MENU.0 as i32) as u16 & 0x8000) != 0;
                    let shift = (GetKeyState(VK_SHIFT.0 as i32) as u16 & 0x8000) != 0;
                    let mods = ctrl as u8 * MOD_CTRL | alt as u8 * MOD_ALT | shift as u8 * MOD_SHIFT;
                    assign_bind(s, i, Some(Bind { mods, vk }));
                }
            }
        }
        s.capture = -1;
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        return LRESULT(0);
    }
    if vk == VK_ESCAPE.0 {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
    LRESULT(0)
}

fn on_click(s: &mut SWin, hwnd: HWND, id: u32) {
    unsafe {
        let app = &mut *s.app;
        match id {
            x if (ID_THEME..ID_THEME + 3).contains(&x) => {
                let m = match x - ID_THEME {
                    1 => ThemeMode::Light,
                    2 => ThemeMode::Dark,
                    _ => ThemeMode::System,
                };
                app.set_theme(m);
            }
            x if (ID_CHECK..ID_CHECK + PREFS.len() as u32).contains(&x) => {
                toggle_pref(s, x - ID_CHECK);
            }
            ID_FONT_MINUS => {
                app.edit_pt = (app.edit_pt - 1).clamp(6, 72);
                save_pref("FontPt", app.edit_pt as u32);
                app.apply_font_size();
            }
            ID_FONT_PLUS => {
                app.edit_pt = (app.edit_pt + 1).clamp(6, 72);
                save_pref("FontPt", app.edit_pt as u32);
                app.apply_font_size();
            }
            x if (ID_BIND..ID_BIND + app::ACTIONS.len() as u32).contains(&x) => {
                s.capture = (x - ID_BIND) as i32;
            }
            ID_RESET => {
                for i in 0..app.binds.len() {
                    app.binds[i] = app::ACTIONS[i].def;
                }
                reset_binds();
                app.apply_binds();
                s.capture = -1;
            }
            ID_DONE => {
                let _ = DestroyWindow(hwnd);
                return;
            }
            _ => {}
        }
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

unsafe extern "system" fn settings_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            WM_CREATE => {
                let cs = &*(lparam.0 as *const CREATESTRUCTW);
                let s = cs.lpCreateParams as *mut SWin;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, s as isize);
                return LRESULT(0);
            }
            WM_NCDESTROY => {
                let s = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SWin;
                if !s.is_null() {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    (*(*s).app).settings_hwnd = HWND::default();
                    drop(Box::from_raw(s));
                }
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            }
            _ => {}
        }
        let s = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SWin;
        if s.is_null() {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        let st = &mut *s;
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let (w, h) = (rc.right, rc.bottom);
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                paint(st, hdc, w, h);
                let _ = EndPaint(hwnd, &mut ps);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_MOUSEMOVE => {
                let (x, y) = (
                    (lparam.0 as i16) as i32,
                    ((lparam.0 >> 16) as i16) as i32,
                );
                if let Some((gy, sc0)) = st.drag_scroll {
                    let (_, track, _) = scroll_info(st, w, h);
                    let track_h = track.bottom - track.top;
                    let (_, _, thumb) = scroll_info(st, w, h);
                    let th = thumb.bottom - thumb.top;
                    let max_scroll = (st.content_h - (h - footer_h(st.dpi))).max(1);
                    let range = (track_h - th).max(1);
                    let delta = (y - gy) * max_scroll / range;
                    st.scroll = (sc0 + delta).clamp(0, max_scroll);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    return LRESULT(0);
                }
                let id = hit_test(st, w, h, x, y);
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
            WM_MOUSEWHEEL => {
                let (_, _, _) = scroll_info(st, w, h);
                let max_scroll = (st.content_h - (h - footer_h(st.dpi))).max(0);
                let delta = (wparam.0 >> 16) as i16 as i32;
                st.scroll = (st.scroll - delta / 2).clamp(0, max_scroll);
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = (
                    (lparam.0 as i16) as i32,
                    ((lparam.0 >> 16) as i16) as i32,
                );
                let id = hit_test(st, w, h, x, y);
                if id == ID_SCROLLBAR {
                    let (_, _, thumb) = scroll_info(st, w, h);
                    if crate::app::pt_in_rect(&thumb, x, y) {
                        st.drag_scroll = Some((y, st.scroll));
                    } else {
                        // Click on track = page.
                        let view = h - footer_h(st.dpi);
                        let dir = if y < thumb.top { -1 } else { 1 };
                        let max_scroll = (st.content_h - view).max(0);
                        st.scroll = (st.scroll + dir * view * 4 / 5).clamp(0, max_scroll);
                        st.drag_scroll = Some((y, st.scroll));
                    }
                    let _ = SetCapture(hwnd);
                } else {
                    st.press = id;
                }
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let (x, y) = (
                    (lparam.0 as i16) as i32,
                    ((lparam.0 >> 16) as i16) as i32,
                );
                if st.drag_scroll.is_some() {
                    st.drag_scroll = None;
                    let _ = ReleaseCapture();
                    return LRESULT(0);
                }
                let id = hit_test(st, w, h, x, y);
                if st.press == id && id != 0 && id != ID_SCROLLBAR {
                    on_click(st, hwnd, id);
                }
                st.press = 0;
                LRESULT(0)
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => on_key(st, hwnd, wparam),
            WM_SIZE => {
                st.content_h = build_rows(st, w)
                    .last()
                    .map(|(_, r, _)| r.bottom + scale(10, st.dpi))
                    .unwrap_or(0);
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Follows a theme change made while Settings is open (its own pills, the
/// View menu, or the OS switching modes).
pub fn retheme(app: &App) {
    if app.settings_hwnd.0.is_null() {
        return;
    }
    theme::set_titlebar_dark(app.settings_hwnd, app.theme.dark);
    theme::set_titlebar_color(app.settings_hwnd, app.theme.chrome_bg, app.theme.tab_fg);
    unsafe {
        let _ = InvalidateRect(Some(app.settings_hwnd), None, false);
    }
}

/// Opens (or focuses) the Settings window, centered over the main window.
pub fn open(app: &mut App) {
    unsafe {
        if !app.settings_hwnd.0.is_null() {
            let _ = SetForegroundWindow(app.settings_hwnd);
            return;
        }
        let hinst = HINSTANCE::from(GetModuleHandleW(None).unwrap_or_default());
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(settings_proc),
            hInstance: hinst,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: SET_CLASS,
            ..Default::default()
        };
        let _ = RegisterClassExW(&wc); // already-registered is fine

        let dpi = app.dpi.max(96);
        let (w, h) = (scale(560, dpi), scale(620, dpi));
        let mut pr = RECT::default();
        let _ = GetWindowRect(app.hwnd, &mut pr);
        let x = pr.left + ((pr.right - pr.left) - w) / 2;
        let y = pr.top + ((pr.bottom - pr.top) - h) / 2;

        let st = Box::new(SWin {
            app: app as *mut App,
            scroll: 0,
            content_h: 0,
            capture: -1,
            hot: 0,
            press: 0,
            drag_scroll: None,
            dpi,
        });
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            SET_CLASS,
            w!("Settings"),
            WS_POPUPWINDOW | WS_CAPTION,
            x,
            y,
            w,
            h,
            Some(app.hwnd),
            None,
            Some(hinst),
            Some(Box::into_raw(st) as *const c_void),
        )
        .unwrap_or_default();
        app.settings_hwnd = hwnd;
        theme::set_titlebar_dark(hwnd, app.theme.dark);
        theme::set_titlebar_color(hwnd, app.theme.chrome_bg, app.theme.tab_fg);
        let _ = ShowWindow(hwnd, SW_SHOW);
    }
}
