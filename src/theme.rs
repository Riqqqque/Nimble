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
