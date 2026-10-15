use std::ffi::c_void;

use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::*;
use windows::Win32::System::Registry::*;
use windows::core::{BOOL, PCSTR, PCWSTR, s, w};

/// What the user picked in View > Theme.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    System = 0,
    Light = 1,
    Dark = 2,
}

impl ThemeMode {
    fn from_u32(v: u32) -> Self {
        match v {
            1 => ThemeMode::Light,
            2 => ThemeMode::Dark,
            _ => ThemeMode::System,
        }
    }
    pub fn command_id(self) -> u32 {
        match self {
            ThemeMode::System => IDM_THEME_SYSTEM,
            ThemeMode::Light => IDM_THEME_LIGHT,
            ThemeMode::Dark => IDM_THEME_DARK,
        }
    }
}

pub const IDM_THEME_SYSTEM: u32 = 305;
pub const IDM_THEME_LIGHT: u32 = 306;
pub const IDM_THEME_DARK: u32 = 307;

#[inline]
pub const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | ((g as u32) << 8) | ((b as u32) << 16))
}

/// Palette for the current effective theme.
#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    pub edit_bg: COLORREF,
    pub edit_fg: COLORREF,
    pub chrome_bg: COLORREF, // tab strip / status bar
    pub tab_sel: COLORREF,   // selected tab fill — matches edit_bg so it merges
    pub tab_fg: COLORREF,
    pub tab_dim: COLORREF, // unselected tab text
    pub tab_hot: COLORREF, // hovered inactive tab
    pub status_fg: COLORREF,
    pub menu_bg: COLORREF, // popup menu fill
    pub menu_fg: COLORREF,
    pub menu_hover: COLORREF, // hot item / strip hover
    pub menu_dim: COLORREF,   // grayed items and accelerators
    pub menu_sep: COLORREF,   // separator lines
    pub accent: COLORREF,     // brand orange, matches the logo
    pub danger: COLORREF,     // "no results" and other error hints
    pub warn: COLORREF,       // warnings (duplicate keys...)
    pub gutter_bg: COLORREF,  // line-number gutter, a shade off the text
    pub gutter_rule: COLORREF, // hairline between gutter and text
    pub syn: Syntax,
}

/// Code colors (VS Code's Dark+ / Light+ families, tuned to Nimble's
/// backgrounds).
#[derive(Clone, Copy)]
pub struct Syntax {
    pub comment: COLORREF,
    pub string: COLORREF,
    pub escape: COLORREF,
    pub number: COLORREF,
    pub keyword: COLORREF,
    pub literal: COLORREF,
    pub type_: COLORREF,
    pub function: COLORREF,
    pub key: COLORREF,
    pub tag: COLORREF,
    pub attr: COLORREF,
    pub heading: COLORREF,
    pub emphasis: COLORREF,
    pub code: COLORREF,
    pub link: COLORREF,
    pub directive: COLORREF,
    pub variable: COLORREF,
    pub anchor: COLORREF,
    pub punct: COLORREF,
    pub brackets: [COLORREF; 3],
    /// Backgrounds: matching bracket pair, unmatched bracket, other
    /// occurrences of the selected word.
    pub match_bg: COLORREF,
    pub mismatch_bg: COLORREF,
    pub occurrence_bg: COLORREF,
}

pub fn light_theme() -> Theme {
    Theme {
        dark: false,
        edit_bg: rgb(0xFF, 0xFF, 0xFF),
        edit_fg: rgb(0x1B, 0x1B, 0x1B),
        chrome_bg: rgb(0xF3, 0xF3, 0xF3),
        tab_sel: rgb(0xFF, 0xFF, 0xFF),
        tab_fg: rgb(0x1B, 0x1B, 0x1B),
        tab_dim: rgb(0x5F, 0x5F, 0x5F),
        tab_hot: rgb(0xE4, 0xE4, 0xE4),
        status_fg: rgb(0x5F, 0x5F, 0x5F),
        menu_bg: rgb(0xF9, 0xF9, 0xF9),
        menu_fg: rgb(0x1B, 0x1B, 0x1B),
        menu_hover: rgb(0xE0, 0xE0, 0xE0),
        menu_dim: rgb(0x8A, 0x8A, 0x8A),
        menu_sep: rgb(0xD4, 0xD4, 0xD4),
        accent: rgb(0xE8, 0x55, 0x1E),
        danger: rgb(0xC4, 0x2B, 0x1C),
        warn: rgb(0xB8, 0x86, 0x00),
        gutter_bg: rgb(0xF8, 0xF8, 0xF8),
        gutter_rule: rgb(0xE3, 0xE3, 0xE3),
        syn: Syntax {
            comment: rgb(0x00, 0x80, 0x00),
            string: rgb(0xA3, 0x15, 0x15),
            escape: rgb(0xEE, 0x00, 0x00),
            number: rgb(0x09, 0x86, 0x58),
            keyword: rgb(0x00, 0x00, 0xFF),
            literal: rgb(0x00, 0x00, 0xFF),
            type_: rgb(0x26, 0x7F, 0x99),
            function: rgb(0x79, 0x5E, 0x26),
            key: rgb(0x04, 0x51, 0xA5),
            tag: rgb(0x80, 0x00, 0x00),
            attr: rgb(0xE5, 0x00, 0x00),
            heading: rgb(0x00, 0x00, 0x80),
            emphasis: rgb(0x00, 0x00, 0x80),
            code: rgb(0xA3, 0x15, 0x15),
            link: rgb(0x00, 0x66, 0xCC),
            directive: rgb(0xAF, 0x00, 0xDB),
            variable: rgb(0x00, 0x10, 0x80),
            anchor: rgb(0xAF, 0x00, 0xDB),
            punct: rgb(0x80, 0x80, 0x80),
            brackets: [rgb(0x04, 0x31, 0xFA), rgb(0x31, 0x93, 0x31), rgb(0x7B, 0x38, 0x14)],
            match_bg: rgb(0xC9, 0xDB, 0xF0),
            mismatch_bg: rgb(0xF6, 0xC9, 0xC6),
            occurrence_bg: rgb(0xE4, 0xE9, 0xF0),
        },
    }
}

pub fn dark_theme() -> Theme {
    Theme {
        dark: true,
        edit_bg: rgb(0x20, 0x20, 0x20),
        edit_fg: rgb(0xE8, 0xE8, 0xE8),
        chrome_bg: rgb(0x19, 0x19, 0x19),
        tab_sel: rgb(0x20, 0x20, 0x20),
        tab_fg: rgb(0xEC, 0xEC, 0xEC),
        tab_dim: rgb(0x9A, 0x9A, 0x9A),
        tab_hot: rgb(0x2A, 0x2A, 0x2A),
        status_fg: rgb(0xA0, 0xA0, 0xA0),
        menu_bg: rgb(0x26, 0x26, 0x26),
        menu_fg: rgb(0xEC, 0xEC, 0xEC),
        menu_hover: rgb(0x38, 0x38, 0x38),
        menu_dim: rgb(0x8A, 0x8A, 0x8A),
        menu_sep: rgb(0x40, 0x40, 0x40),
        accent: rgb(0xE8, 0x55, 0x1E),
        danger: rgb(0xFF, 0x7A, 0x6E),
        warn: rgb(0xE5, 0xC0, 0x7B),
        gutter_bg: rgb(0x1C, 0x1C, 0x1C),
        gutter_rule: rgb(0x33, 0x33, 0x33),
        syn: Syntax {
            comment: rgb(0x6A, 0x99, 0x55),
            string: rgb(0xCE, 0x91, 0x78),
            escape: rgb(0xD7, 0xBA, 0x7D),
            number: rgb(0xB5, 0xCE, 0xA8),
            keyword: rgb(0x56, 0x9C, 0xD6),
            literal: rgb(0x56, 0x9C, 0xD6),
            type_: rgb(0x4E, 0xC9, 0xB0),
            function: rgb(0xDC, 0xDC, 0xAA),
            key: rgb(0x9C, 0xDC, 0xFE),
            tag: rgb(0x56, 0x9C, 0xD6),
            attr: rgb(0x9C, 0xDC, 0xFE),
            heading: rgb(0x56, 0x9C, 0xD6),
            emphasis: rgb(0xC5, 0x86, 0xC0),
            code: rgb(0xCE, 0x91, 0x78),
            link: rgb(0x4F, 0xC1, 0xFF),
            directive: rgb(0xC5, 0x86, 0xC0),
            variable: rgb(0x9C, 0xDC, 0xFE),
            anchor: rgb(0xC5, 0x86, 0xC0),
            punct: rgb(0x80, 0x80, 0x80),
            brackets: [rgb(0xFF, 0xD7, 0x00), rgb(0xDA, 0x70, 0xD6), rgb(0x17, 0x9F, 0xFF)],
            match_bg: rgb(0x3E, 0x4D, 0x63),
            mismatch_bg: rgb(0x6B, 0x2C, 0x2C),
            occurrence_bg: rgb(0x34, 0x3A, 0x46),
        },
    }
}

// ---- persistence ----------------------------------------------------------

/// 0 = follow system, 1 = light, 2 = dark.
pub fn load_mode() -> ThemeMode {
    unsafe {
        let mut v = 0u32;
        let mut size = size_of::<u32>() as u32;
        let r = RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Nimble"),
            w!("Theme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut v as *mut u32 as *mut c_void),
            Some(&mut size),
        );
        if r.is_ok() {
            ThemeMode::from_u32(v)
        } else {
            ThemeMode::System
        }
    }
}

pub fn save_mode(mode: ThemeMode) {
    let v = mode as u32;
    unsafe {
        let _ = RegSetKeyValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Nimble"),
            w!("Theme"),
            REG_DWORD.0,
            Some(&v as *const u32 as *const c_void),
            size_of::<u32>() as u32,
        );
    }
}

pub fn system_is_dark() -> bool {
    unsafe {
        let mut v = 1u32;
        let mut size = size_of::<u32>() as u32;
        let r = RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut v as *mut u32 as *mut c_void),
            Some(&mut size),
        );
        r.is_ok() && v == 0
    }
}

pub fn effective_dark(mode: ThemeMode) -> bool {
    match mode {
        ThemeMode::System => system_is_dark(),
        ThemeMode::Light => false,
        ThemeMode::Dark => true,
    }
}

// ---- OS-level dark mode ----------------------------------------------------

/// Flips the process-wide dark-mode flag (undocumented uxtheme ordinals,
/// stable since Windows 10 1809). This is what makes menus, dialogs, and
/// message boxes render dark without any owner-drawing.
/// Call before creating windows for full effect.
pub fn set_preferred_app_mode(dark: bool) {
    unsafe {
        let Ok(ux) = LoadLibraryW(w!("uxtheme.dll")) else {
            return;
        };
        // SetPreferredAppMode: 0=Default, 1=AllowDark, 2=ForceDark, 3=ForceLight
        if let Some(f) = GetProcAddress(ux, PCSTR(135usize as *const u8)) {
            let f: unsafe extern "system" fn(i32) -> i32 = core::mem::transmute(f);
            f(if dark { 2 } else { 3 });
        } else if let Some(f) = GetProcAddress(ux, PCSTR(132usize as *const u8)) {
            // Fallback for older builds: AllowDarkModeForApp
            let f: unsafe extern "system" fn(i32) -> i32 = core::mem::transmute(f);
            f(dark as i32);
        }
        // RefreshImmersiveColorPolicyState (104) + FlushMenuThemes (134)
        for ord in [104usize, 134usize] {
            if let Some(f) = GetProcAddress(ux, PCSTR(ord as *const u8)) {
                let f: unsafe extern "system" fn() = core::mem::transmute(f);
                f();
            }
        }
    }
}

/// Dark title bar / non-client area via DwmSetWindowAttribute.
pub fn set_titlebar_dark(hwnd: HWND, dark: bool) {
    unsafe {
        let Ok(dwm) = LoadLibraryW(w!("dwmapi.dll")) else {
            return;
        };
        let Some(f) = GetProcAddress(dwm, s!("DwmSetWindowAttribute")) else {
            return;
        };
        let f: unsafe extern "system" fn(HWND, u32, *const c_void, u32) -> i32 =
            core::mem::transmute(f);
        let v = BOOL(dark as i32);
        // 20 = DWMWA_USE_IMMERSIVE_DARK_MODE (Win11 / newer Win10), 19 on older builds
        for attr in [20u32, 19] {
            if f(hwnd, attr, &v as *const BOOL as *const c_void, 4) == 0 {
                break;
            }
        }
    }
}

/// Tints the title bar to match the theme (Win11; silently ignored elsewhere).
/// 35 = DWMWA_CAPTION_COLOR, 36 = DWMWA_TEXT_COLOR.
pub fn set_titlebar_color(hwnd: HWND, caption: COLORREF, text: COLORREF) {
    unsafe {
        let Ok(dwm) = GetModuleHandleW(w!("dwmapi.dll")) else {
            return;
        };
        let Some(f) = GetProcAddress(dwm, s!("DwmSetWindowAttribute")) else {
            return;
        };
        let f: unsafe extern "system" fn(HWND, u32, *const c_void, u32) -> i32 =
            core::mem::transmute(f);
        let _ = f(hwnd, 35, &caption as *const COLORREF as *const c_void, 4);
        let _ = f(hwnd, 36, &text as *const COLORREF as *const c_void, 4);
    }
}

/// Rounded corners for popup menu windows (Win11 composes them smoothly;
/// harmless on older builds).
/// 33 = DWMWA_WINDOW_CORNER_PREFERENCE, 2 = DWMWCP_ROUND.
pub fn round_corners(hwnd: HWND) {
    unsafe {
        let Ok(dwm) = GetModuleHandleW(w!("dwmapi.dll")) else {
            return;
        };
        let Some(f) = GetProcAddress(dwm, s!("DwmSetWindowAttribute")) else {
            return;
        };
        let f: unsafe extern "system" fn(HWND, u32, *const c_void, u32) -> i32 =
            core::mem::transmute(f);
        let v = 2u32;
        let _ = f(hwnd, 33, &v as *const u32 as *const c_void, 4);
    }
}

/// Applies (or clears) the "DarkMode_Explorer" visual style on a control —
/// gives dark scrollbars, buttons, etc. when supported.
pub fn set_control_dark(hwnd: HWND, dark: bool) {
    unsafe {
        let Ok(ux) = GetModuleHandleW(w!("uxtheme.dll")) else {
            return;
        };
        let Some(f) = GetProcAddress(ux, s!("SetWindowTheme")) else {
            return;
        };
        let f: unsafe extern "system" fn(HWND, PCWSTR, PCWSTR) -> i32 = core::mem::transmute(f);
        let name = wide0("DarkMode_Explorer");
        let _ = f(
            hwnd,
            if dark {
                PCWSTR(name.as_ptr())
            } else {
                PCWSTR::null()
            },
            PCWSTR::null(),
        );
    }
}

fn wide0(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
