use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::DataExchange::IsClipboardFormatAvailable;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Controls::Dialogs::*;
use windows::Win32::UI::Controls::RichEdit::*;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::{
    DefSubclassProc, DragAcceptFiles, DragFinish, DragQueryFileW, HDROP, RemoveWindowSubclass,
    SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, PWSTR, w};

use crate::document::Document;
use crate::draw::{self, Aa};
use crate::encoding::{self, Encoding};
use crate::findbar::{self, BarMode};
use crate::settings::{self, Action, Bind, MOD_CTRL, MOD_SHIFT};
use crate::theme::{self, Theme, ThemeMode};

const WND_CLASS: PCWSTR = w!("NimbleWindow");
const CP_UTF16: u32 = 1200;
const CF_UNICODETEXT: u32 = 13;

// Command IDs
const IDM_NEW_TAB: u32 = 100;
const IDM_NEW_WINDOW: u32 = 101;
const IDM_OPEN: u32 = 102;
const IDM_SAVE: u32 = 103;
const IDM_SAVE_AS: u32 = 104;
const IDM_CLOSE_TAB: u32 = 105;
const IDM_EXIT: u32 = 106;
const IDM_SETTINGS: u32 = 107;
const IDM_UNDO: u32 = 200;
const IDM_REDO: u32 = 201;
const IDM_CUT: u32 = 202;
const IDM_COPY: u32 = 203;
const IDM_PASTE: u32 = 204;
const IDM_DELETE: u32 = 205;
const IDM_SELECT_ALL: u32 = 206;
const IDM_FIND: u32 = 207;
const IDM_REPLACE: u32 = 208;
const IDM_GOTO: u32 = 209;
const IDM_FIND_NEXT: u32 = 210;
const IDM_FIND_PREV: u32 = 211;
const IDM_DUP_LINE: u32 = 212;
const IDM_WORD_WRAP: u32 = 300;
const IDM_STATUS_BAR: u32 = 301;
const IDM_ZOOM_IN: u32 = 302;
const IDM_ZOOM_OUT: u32 = 303;
const IDM_ZOOM_RESET: u32 = 304;
const IDM_LINE_NUMBERS: u32 = 308;
const IDM_ABOUT: u32 = 400;
const IDM_CHECK_UPDATES: u32 = 401;
// Theme submenu IDs live in theme.rs (IDM_THEME_SYSTEM/LIGHT/DARK)
const IDM_NEXT_TAB: u32 = 500;
const IDM_PREV_TAB: u32 = 501;

/// Every command that can be rebound in Settings, in display order.
pub(crate) const ACTIONS: &[Action] = &[
    Action { cmd: IDM_NEW_TAB, name: "New tab", def: Some(Bind::new(MOD_CTRL, 'T' as u16)) },
    Action { cmd: IDM_NEW_WINDOW, name: "New window", def: Some(Bind::new(MOD_CTRL, 'N' as u16)) },
    Action { cmd: IDM_OPEN, name: "Open…", def: Some(Bind::new(MOD_CTRL, 'O' as u16)) },
    Action { cmd: IDM_SAVE, name: "Save", def: Some(Bind::new(MOD_CTRL, 'S' as u16)) },
    Action { cmd: IDM_SAVE_AS, name: "Save as…", def: Some(Bind::new(MOD_CTRL | MOD_SHIFT, 'S' as u16)) },
    Action { cmd: IDM_CLOSE_TAB, name: "Close tab", def: Some(Bind::new(MOD_CTRL, 'W' as u16)) },
    Action { cmd: IDM_UNDO, name: "Undo", def: Some(Bind::new(MOD_CTRL, 'Z' as u16)) },
    Action { cmd: IDM_REDO, name: "Redo", def: Some(Bind::new(MOD_CTRL, 'Y' as u16)) },
    Action { cmd: IDM_CUT, name: "Cut", def: Some(Bind::new(MOD_CTRL, 'X' as u16)) },
    Action { cmd: IDM_COPY, name: "Copy", def: Some(Bind::new(MOD_CTRL, 'C' as u16)) },
    Action { cmd: IDM_PASTE, name: "Paste", def: Some(Bind::new(MOD_CTRL, 'V' as u16)) },
    Action { cmd: IDM_DELETE, name: "Delete", def: Some(Bind::new(0, VK_DELETE.0)) },
    Action { cmd: IDM_SELECT_ALL, name: "Select all", def: Some(Bind::new(MOD_CTRL, 'A' as u16)) },
    Action { cmd: IDM_DUP_LINE, name: "Duplicate line", def: Some(Bind::new(MOD_CTRL, 'D' as u16)) },
    Action { cmd: IDM_FIND, name: "Find…", def: Some(Bind::new(MOD_CTRL, 'F' as u16)) },
    Action { cmd: IDM_REPLACE, name: "Replace…", def: Some(Bind::new(MOD_CTRL, 'H' as u16)) },
    Action { cmd: IDM_GOTO, name: "Go to line…", def: Some(Bind::new(MOD_CTRL, 'G' as u16)) },
    Action { cmd: IDM_FIND_NEXT, name: "Find next", def: Some(Bind::new(0, VK_F3.0)) },
    Action { cmd: IDM_FIND_PREV, name: "Find previous", def: Some(Bind::new(MOD_SHIFT, VK_F3.0)) },
    Action { cmd: IDM_ZOOM_IN, name: "Zoom in", def: Some(Bind::new(MOD_CTRL, VK_OEM_PLUS.0)) },
    Action { cmd: IDM_ZOOM_OUT, name: "Zoom out", def: Some(Bind::new(MOD_CTRL, VK_OEM_MINUS.0)) },
    Action { cmd: IDM_ZOOM_RESET, name: "Reset zoom", def: Some(Bind::new(MOD_CTRL, '0' as u16)) },
    Action { cmd: IDM_NEXT_TAB, name: "Next tab", def: Some(Bind::new(MOD_CTRL, VK_TAB.0)) },
    Action { cmd: IDM_PREV_TAB, name: "Previous tab", def: Some(Bind::new(MOD_CTRL | MOD_SHIFT, VK_TAB.0)) },
    Action { cmd: IDM_WORD_WRAP, name: "Toggle word wrap", def: None },
    Action { cmd: IDM_LINE_NUMBERS, name: "Toggle line numbers", def: None },
    Action { cmd: IDM_STATUS_BAR, name: "Toggle status bar", def: None },
    Action { cmd: IDM_SETTINGS, name: "Open settings", def: Some(Bind::new(MOD_CTRL, VK_OEM_COMMA.0)) },
];

const EDIT_ID_BASE: usize = 4096;
/// Notifications every editor sends: dirty state, caret moves, scrolling.
const EDIT_EVENTS: u32 = ENM_CHANGE | ENM_SELCHANGE | ENM_SCROLL;
const EDIT_SUBCLASS: usize = 2;
const POPUP_SUBCLASS: usize = 4;

/// The live accelerator table — rebuilt when shortcuts change.
static ACCEL: AtomicUsize = AtomicUsize::new(0);

/// Our window handle while a menu popup is being hooked (same thread only).
static POPUP_HOOK_HWND: AtomicUsize = AtomicUsize::new(0);

#[inline]
pub(crate) fn sm(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> LRESULT {
    unsafe { SendMessageW(hwnd, msg, Some(WPARAM(wparam)), Some(LPARAM(lparam))) }
}

#[inline]
pub(crate) fn loword(v: usize) -> u32 {
    (v & 0xFFFF) as u32
}

#[inline]
pub(crate) fn hiword(v: usize) -> u32 {
    ((v >> 16) & 0xFFFF) as u32
}

#[inline]
fn x_lparam(l: LPARAM) -> i32 {
    (l.0 as i16) as i32
}

#[inline]
fn y_lparam(l: LPARAM) -> i32 {
    ((l.0 >> 16) as i16) as i32
}

pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[inline]
pub(crate) fn scale(v: i32, dpi: u32) -> i32 {
    v * dpi as i32 / 96
}

#[inline]
pub(crate) fn pt_in_rect(rc: &RECT, x: i32, y: i32) -> bool {
    x >= rc.left && x < rc.right && y >= rc.top && y < rc.bottom
}

/// Owner-draw data for one popup menu item; boxed and leaked for the life
/// of the app — menus live as long as the process anyway.
struct MenuItemData {
    label: Vec<u16>, // display text, '&' stripped, no accelerator
    accel: Vec<u16>, // shortcut text ("Ctrl+S") or empty
    accel_col: i32,  // shared accelerator column width for its popup
    label_col: i32,  // shared label column width for its popup
    cmd: u32,        // command id — used to re-label when shortcuts change
    sep: bool,
    radio: bool,
    submenu: bool,
}

/// One entry in the hand-drawn menu strip (File / Edit / View / Help).
struct StripEntry {
    label: Vec<u16>, // display text
    key: u16,        // mnemonic, ASCII lowercase
    menu: HMENU,
    x: i32,
    w: i32,
}

/// Everything build_menu produces: the strip entries plus handles kept for
/// state updates (check marks, radio, enable/gray).
struct MenuSet {
    strip: Vec<StripEntry>,
    popups: Vec<HMENU>, // every popup incl. the theme submenu
    edit: HMENU,
    view: HMENU,
    theme: HMENU,
}

pub(crate) struct App {
    pub(crate) hwnd: HWND,
    menus: MenuSet,
    menu_hot: i32,  // strip index under the cursor, -1 = none
    menu_open: i32, // strip index whose popup is showing, -1 = none
    plus_hot: bool, // cursor over the "+" new-tab glyph
    tab_hot: i32,   // tab index under the cursor, -1 = none
    close_hot: i32, // tab index whose close button is hot, -1 = none
    close_press: i32, // tab index whose close button is pressed
    menu_brush: HBRUSH,
    pub(crate) docs: Vec<Document>,
    pub(crate) active: Option<usize>,
    pub(crate) word_wrap: bool,
    pub(crate) show_status: bool,
    pub(crate) line_numbers: bool,
    pub(crate) auto_indent: bool,
    pub(crate) edit_pt: i32,
    pub(crate) dpi: u32,
    pub(crate) font: HFONT,
    ui_face: String,
    pub(crate) edit_face: String,
    next_id: usize,
    pub(crate) mode: ThemeMode,
    pub(crate) theme: Theme,
    /// Status band segments, right to left: encoding, line endings, zoom,
    /// caret position.
    status_text: [String; 4],
    tab_w: i32,    // cell width, set by layout()
    gutter_w: i32, // line-number gutter width, 0 when off
    pub(crate) binds: Vec<Option<Bind>>,
    pub(crate) findbar: HWND,
    pub(crate) find_query: Vec<u16>,
    pub(crate) settings_hwnd: HWND,
}

pub fn run() {
    unsafe {
        let mode = theme::load_mode();
        // Must run before any windows/menus exist for dark chrome to apply.
        theme::set_preferred_app_mode(theme::effective_dark(mode));
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = LoadLibraryW(w!("msftedit.dll"));
        let _ = InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_TAB_CLASSES | ICC_BAR_CLASSES,
        });

        let hinst = HINSTANCE::from(GetModuleHandleW(None).unwrap_or_default());
        // Resource ID 1 is nimble.ico; fall back to the stock app icon if absent.
        let load_icon = |sx: SYSTEM_METRICS_INDEX, sy: SYSTEM_METRICS_INDEX| -> HICON {
            LoadImageW(
                Some(hinst),
                PCWSTR(1usize as *const u16),
                IMAGE_ICON,
                GetSystemMetrics(sx),
                GetSystemMetrics(sy),
                LR_DEFAULTCOLOR,
            )
            .map(|h| HICON(h.0))
            .or_else(|_| LoadIconW(None, IDI_APPLICATION))
            .unwrap_or_default()
        };
        let icon = load_icon(SM_CXICON, SM_CYICON);
        let icon_sm = load_icon(SM_CXSMICON, SM_CYSMICON);

        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinst,
            hIcon: icon,
            hIconSm: icon_sm,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as *mut c_void),
            lpszClassName: WND_CLASS,
            ..Default::default()
        };
        if RegisterClassExW(&wc) == 0 {
            return;
        }

        let menus = build_menu();
        findbar::register(hinst);
        draw::init();

        let prefs = settings::load_prefs();
        let binds = settings::load_binds(ACTIONS);
        let accel = settings::build_accel(ACTIONS, &binds);
        ACCEL.store(accel.0 as usize, Ordering::Relaxed);

        let app = Box::new(App::new(mode, menus, binds, prefs));
        let app_ptr = Box::into_raw(app);

        let sdpi = GetDpiForSystem();
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            WND_CLASS,
            w!("Nimble"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            scale(760, sdpi),
            scale(540, sdpi),
            None,
            None,
            Some(hinst),
            Some(app_ptr as *const c_void),
        );
        let Ok(hwnd) = hwnd else {
            drop(Box::from_raw(app_ptr));
            return;
        };

        let _ = ShowWindow(hwnd, SW_SHOWDEFAULT);
        let _ = UpdateWindow(hwnd);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            // Shortcuts belong to the main window only — the Settings window
            // must see raw keys while it records a new binding.
            let ours = !msg.hwnd.0.is_null() && GetAncestor(msg.hwnd, GA_ROOT).0 == hwnd.0;
            // Inside a find-bar field, text-editing keys (Ctrl+C/V/X/Z/A,
            // Delete) edit the field, not the document. Other shortcuts
            // (Ctrl+H, F3, ...) still work.
            let app = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
            let in_bar = !app.is_null()
                && !(*app).findbar.0.is_null()
                && GetParent(msg.hwnd).is_ok_and(|p| p.0 == (*app).findbar.0);
            let skip = !ours || (in_bar && is_field_edit_key(&msg));
            let a = ACCEL.load(Ordering::Relaxed);
            if skip || a == 0 || TranslateAcceleratorW(hwnd, HACCEL(a as *mut c_void), &msg) == 0
            {
                let _ = TranslateMessage(&msg);
                let _ = DispatchMessageW(&msg);
            }
        }
    }
}

/// Keys a single-line text field handles itself.
fn is_field_edit_key(msg: &MSG) -> bool {
    if msg.message != WM_KEYDOWN {
        return false;
    }
    let vk = msg.wParam.0 as u16;
    let ctrl = unsafe { (GetKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000) != 0 };
    vk == VK_DELETE.0
        || vk == VK_BACK.0
        || (ctrl && [b'A', b'C', b'V', b'X', b'Z', b'Y'].contains(&(vk as u8)) && vk < 0x80)
}

/// Splits "Te&xt\tAccel" into (display label, accelerator text).
fn split_item_text(text: &str) -> (String, String) {
    let (name, accel) = text.split_once('\t').unwrap_or((text, ""));
    (name.replace('&', ""), accel.to_string())
}

/// Appends an owner-drawn item. `data` is boxed and its pointer becomes the
/// item's dwItemData — intentionally never freed (menu outlives the window).
fn add_item(menu: HMENU, id: u32, text: &str, radio: bool, submenu: bool, checked: bool) {
    let (label, accel) = split_item_text(text);
    let d = Box::new(MenuItemData {
        label: wide(&label),
        accel: wide(&accel),
        accel_col: 0,
        label_col: 0,
        cmd: id,
        sep: false,
        radio,
        submenu,
    });
    unsafe {
        let _ = AppendMenuW(
            menu,
            MF_OWNERDRAW | MF_ENABLED | if checked { MF_CHECKED } else { MF_UNCHECKED },
            id as usize,
            PCWSTR(Box::into_raw(d) as *const u16),
        );
    }
}

fn add_sep(menu: HMENU) {
    let d = Box::new(MenuItemData {
        label: Vec::new(),
        accel: Vec::new(),
        accel_col: 0,
        label_col: 0,
        cmd: 0,
        sep: true,
        radio: false,
        submenu: false,
    });
    unsafe {
        let _ = AppendMenuW(
            menu,
            MF_OWNERDRAW,
            0,
            PCWSTR(Box::into_raw(d) as *const u16),
        );
    }
}

/// Submenu entries ("Theme") need SetMenuItemInfo: AppendMenuW can't combine
/// MF_POPUP with MF_OWNERDRAW.
fn add_submenu(menu: HMENU, sub: HMENU, text: &str) {
    let (label, _) = split_item_text(text);
    let d = Box::new(MenuItemData {
        label: wide(&label),
        accel: Vec::new(),
        accel_col: 0,
        label_col: 0,
        cmd: 0,
        sep: false,
        radio: false,
        submenu: true,
    });
    unsafe {
        let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, w!("x"));
        let pos = GetMenuItemCount(Some(menu)) - 1;
        let mii = MENUITEMINFOW {
            cbSize: size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_FTYPE | MIIM_DATA,
            fType: MFT_OWNERDRAW,
            dwItemData: Box::into_raw(d) as usize,
            ..Default::default()
        };
        let _ = SetMenuItemInfoW(menu, pos as u32, true, &mii);
    }
}

fn strip_entry(text: &str, menu: HMENU) -> StripEntry {
    let (label, _) = split_item_text(text);
    let key = text
        .chars()
        .skip_while(|&c| c != '&')
        .nth(1)
        .unwrap_or(' ')
        .to_ascii_lowercase() as u16;
    StripEntry {
        label: wide(&label),
        key,
        menu,
        x: 0,
        w: 0,
    }
}

/// The menu bar is drawn by us (Win32 can't theme it), so this builds only
/// the popups plus the strip entries used for hit-testing and labels.
fn build_menu() -> MenuSet {
    unsafe {
        let file = CreatePopupMenu().unwrap_or_default();
        add_item(file, IDM_NEW_TAB, "&New Tab\tCtrl+T", false, false, false);
        add_item(
            file,
            IDM_NEW_WINDOW,
            "New &Window\tCtrl+N",
            false,
            false,
            false,
        );
        add_sep(file);
        add_item(file, IDM_OPEN, "&Open...\tCtrl+O", false, false, false);
        add_item(file, IDM_SAVE, "&Save\tCtrl+S", false, false, false);
        add_item(
            file,
            IDM_SAVE_AS,
            "Save &As...\tCtrl+Shift+S",
            false,
            false,
            false,
        );
        add_sep(file);
        add_item(
            file,
            IDM_CLOSE_TAB,
            "&Close Tab\tCtrl+W",
            false,
            false,
            false,
        );
        add_sep(file);
        add_item(
            file,
            IDM_SETTINGS,
            "&Settings\tCtrl+,",
            false,
            false,
            false,
        );
        add_sep(file);
        add_item(file, IDM_EXIT, "E&xit", false, false, false);

        let edit = CreatePopupMenu().unwrap_or_default();
        add_item(edit, IDM_UNDO, "&Undo\tCtrl+Z", false, false, false);
        add_item(edit, IDM_REDO, "&Redo\tCtrl+Y", false, false, false);
        add_sep(edit);
        add_item(edit, IDM_CUT, "Cu&t\tCtrl+X", false, false, false);
        add_item(edit, IDM_COPY, "&Copy\tCtrl+C", false, false, false);
        add_item(edit, IDM_PASTE, "&Paste\tCtrl+V", false, false, false);
        add_item(edit, IDM_DELETE, "&Delete\tDel", false, false, false);
        add_sep(edit);
        add_item(
            edit,
            IDM_SELECT_ALL,
            "Select &All\tCtrl+A",
            false,
            false,
            false,
        );
        add_item(
            edit,
            IDM_DUP_LINE,
            "D&uplicate Line\tCtrl+D",
            false,
            false,
            false,
        );
        add_sep(edit);
        add_item(edit, IDM_FIND, "&Find...\tCtrl+F", false, false, false);
        add_item(
            edit,
            IDM_REPLACE,
            "&Replace...\tCtrl+H",
            false,
            false,
            false,
        );
        add_item(edit, IDM_GOTO, "&Go To...\tCtrl+G", false, false, false);
        add_item(edit, IDM_FIND_NEXT, "Find &Next\tF3", false, false, false);
        add_item(
            edit,
            IDM_FIND_PREV,
            "Find Pre&vious\tShift+F3",
            false,
            false,
            false,
        );

        let view = CreatePopupMenu().unwrap_or_default();
        add_item(view, IDM_WORD_WRAP, "&Word Wrap", false, false, true);
        add_item(view, IDM_STATUS_BAR, "&Status Bar", false, false, true);
        add_item(
            view,
            IDM_LINE_NUMBERS,
            "&Line Numbers",
            false,
            false,
            true,
        );
        add_sep(view);
        add_item(
            view,
            IDM_ZOOM_IN,
            "Zoom &In\tCtrl+Plus",
            false,
            false,
            false,
        );
        add_item(
            view,
            IDM_ZOOM_OUT,
            "Zoom &Out\tCtrl+Minus",
            false,
            false,
            false,
        );
        add_item(
            view,
            IDM_ZOOM_RESET,
            "&Restore Default Zoom\tCtrl+0",
            false,
            false,
            false,
        );
        add_sep(view);
        let theme = CreatePopupMenu().unwrap_or_default();
        add_item(
            theme,
            theme::IDM_THEME_SYSTEM,
            "Use &System Setting",
            true,
            false,
            false,
        );
        add_item(theme, theme::IDM_THEME_LIGHT, "&Light", true, false, false);
        add_item(theme, theme::IDM_THEME_DARK, "&Dark", true, false, false);
        add_submenu(view, theme, "&Theme");

        let help = CreatePopupMenu().unwrap_or_default();
        add_item(
            help,
            IDM_CHECK_UPDATES,
            "Check for &Updates",
            false,
            false,
            false,
        );
        add_item(help, IDM_ABOUT, "&About Nimble", false, false, false);

        MenuSet {
            strip: vec![
                strip_entry("&File", file),
                strip_entry("&Edit", edit),
                strip_entry("&View", view),
                strip_entry("&Help", help),
            ],
            popups: vec![file, edit, view, theme, help],
            edit,
            view,
            theme,
        }
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        if msg == WM_CREATE {
            let cs = &*(lparam.0 as *const CREATESTRUCTW);
            let app = cs.lpCreateParams as *mut App;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, app as isize);
            (*app).hwnd = hwnd;
            (*app).on_create();
            return LRESULT(0);
        }
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
        if ptr.is_null() {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        let r = (*ptr).handle(msg, wparam, lparam);
        if msg == WM_DESTROY {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Box::from_raw(ptr));
        }
        r
    }
}

impl App {
    fn new(
        mode: ThemeMode,
        menus: MenuSet,
        binds: Vec<Option<Bind>>,
        prefs: settings::Prefs,
    ) -> Self {
        let theme = if theme::effective_dark(mode) {
            theme::dark_theme()
        } else {
            theme::light_theme()
        };
        Self {
            hwnd: HWND::default(),
            menus,
            menu_hot: -1,
            menu_open: -1,
            plus_hot: false,
            tab_hot: -1,
            close_hot: -1,
            close_press: -1,
            menu_brush: HBRUSH::default(),
            docs: Vec::new(),
            active: None,
            word_wrap: prefs.word_wrap,
            show_status: prefs.status_bar,
            line_numbers: prefs.line_numbers,
            auto_indent: prefs.auto_indent,
            edit_pt: prefs.font_pt,
            dpi: 96,
            font: HFONT::default(),
            ui_face: String::new(),
            edit_face: String::new(),
            next_id: EDIT_ID_BASE,
            mode,
            theme,
            status_text: Default::default(),
            tab_w: 0,
            gutter_w: 0,
            binds,
            findbar: HWND::default(),
            find_query: Vec::new(),
            settings_hwnd: HWND::default(),
        }
    }

    fn on_create(&mut self) {
        unsafe {
            self.dpi = GetDpiForWindow(self.hwnd);
            let hdc = GetDC(Some(self.hwnd));
            self.ui_face = pick_face(hdc, &["Segoe UI Variable Text", "Segoe UI"]);
            self.edit_face = pick_face(hdc, &["Cascadia Mono", "Cascadia Code", "Consolas"]);
            let _ = ReleaseDC(Some(self.hwnd), hdc);
            self.font = ui_font(self.dpi, &self.ui_face);

            self.measure_strip();
            self.recompute_menu_metrics();

            DragAcceptFiles(self.hwnd, true);

            self.new_document(None);
            if let Some(arg) = std::env::args_os().nth(1) {
                let p = PathBuf::from(arg);
                if p.is_file() {
                    self.open_path(&p);
                }
            }
            self.apply_theme();
            self.apply_binds();
            self.apply_prefs();
            crate::install::update_check_async();
        }
    }

    fn handle(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        unsafe {
            match msg {
                WM_SIZE => {
                    self.layout();
                    LRESULT(0)
                }
                WM_COMMAND => self.on_command(wparam, lparam),
                WM_NOTIFY => self.on_notify(lparam),
                WM_INITMENUPOPUP => {
                    self.on_init_menu_popup(wparam);
                    LRESULT(0)
                }
                WM_DROPFILES => {
                    self.on_drop(wparam);
                    LRESULT(0)
                }
                WM_PAINT => {
                    let mut ps = PAINTSTRUCT::default();
                    let hdc = BeginPaint(self.hwnd, &mut ps);
                    self.paint_menu_strip(hdc);
                    self.paint_tabs(hdc);
                    self.paint_gutter(hdc);
                    self.paint_status(hdc);
                    let _ = EndPaint(self.hwnd, &mut ps);
                    LRESULT(0)
                }
                WM_MOUSEMOVE => {
                    self.on_mouse_move(x_lparam(lparam), y_lparam(lparam));
                    LRESULT(0)
                }
                WM_MOUSELEAVE => {
                    if self.menu_hot >= 0 || self.tab_hot >= 0 || self.plus_hot {
                        self.menu_hot = -1;
                        self.tab_hot = -1;
                        self.close_hot = -1;
                        self.plus_hot = false;
                        self.invalidate_strip();
                        self.invalidate_tabs();
                    }
                    LRESULT(0)
                }
                WM_LBUTTONDOWN => {
                    let (x, y) = (x_lparam(lparam), y_lparam(lparam));
                    if let Some(i) = self.strip_hit(x, y) {
                        self.menu_loop(i);
                        return LRESULT(0);
                    }
                    if let Some(i) = self.close_at(x, y) {
                        self.close_press = i as i32;
                        let _ = SetCapture(self.hwnd);
                        return LRESULT(0);
                    }
                    if let Some(i) = self.tab_at(x, y) {
                        self.set_active(i);
                        return LRESULT(0);
                    }
                    DefWindowProcW(self.hwnd, msg, wparam, lparam)
                }
                WM_LBUTTONUP => {
                    let (x, y) = (x_lparam(lparam), y_lparam(lparam));
                    if self.close_press >= 0 {
                        let i = self.close_press;
                        self.close_press = -1;
                        let _ = ReleaseCapture();
                        if self.close_at(x, y) == Some(i as usize) {
                            self.close_tab(i as usize);
                        }
                        return LRESULT(0);
                    }
                    if pt_in_rect(&self.plus_rect(), x, y) {
                        self.new_document(None);
                        return LRESULT(0);
                    }
                    DefWindowProcW(self.hwnd, msg, wparam, lparam)
                }
                WM_MBUTTONUP => {
                    if let Some(i) = self.tab_at(x_lparam(lparam), y_lparam(lparam)) {
                        self.close_tab(i);
                        return LRESULT(0);
                    }
                    DefWindowProcW(self.hwnd, msg, wparam, lparam)
                }
                WM_LBUTTONDBLCLK => {
                    let (x, y) = (x_lparam(lparam), y_lparam(lparam));
                    if y >= self.menu_h()
                        && y < self.menu_h() + self.tab_strip_h()
                        && self.tab_at(x, y).is_none()
                    {
                        self.new_document(None);
                        return LRESULT(0);
                    }
                    DefWindowProcW(self.hwnd, msg, wparam, lparam)
                }
                WM_MOUSEWHEEL => {
                    // Ctrl+wheel zooms the editor.
                    if (wparam.0 as u16 & 0x0008) != 0 {
                        self.zoom(if (wparam.0 >> 16) as i16 > 0 { 1 } else { -1 });
                        return LRESULT(0);
                    }
                    DefWindowProcW(self.hwnd, msg, wparam, lparam)
                }
                WM_SYSKEYDOWN => {
                    if wparam.0 == VK_F10.0 as usize {
                        self.menu_loop(0);
                        LRESULT(0)
                    } else {
                        DefWindowProcW(self.hwnd, msg, wparam, lparam)
                    }
                }
                WM_SYSCHAR => {
                    let ch = (wparam.0 as u32 as u8 as char).to_ascii_lowercase();
                    if let Some(i) = self.menus.strip.iter().position(|m| m.key == ch as u16) {
                        self.menu_loop(i);
                        LRESULT(0)
                    } else {
                        DefWindowProcW(self.hwnd, msg, wparam, lparam)
                    }
                }
                WM_MEASUREITEM => {
                    let mi = &mut *(lparam.0 as *mut MEASUREITEMSTRUCT);
                    if mi.CtlType == ODT_MENU {
                        self.measure_menu_item(mi);
                    }
                    LRESULT(1)
                }
                WM_DRAWITEM => {
                    let dis = &*(lparam.0 as *const DRAWITEMSTRUCT);
                    if dis.CtlType == ODT_MENU {
                        self.draw_menu_item(dis);
                    }
                    LRESULT(1)
                }
                WM_ERASEBKGND => {
                    let mut rc = RECT::default();
                    let _ = GetClientRect(self.hwnd, &mut rc);
                    let b = CreateSolidBrush(self.theme.chrome_bg);
                    FillRect(HDC(wparam.0 as *mut c_void), &rc, b);
                    let _ = DeleteObject(HGDIOBJ(b.0));
                    LRESULT(1)
                }
                WM_SETTINGCHANGE | WM_SYSCOLORCHANGE => {
                    if self.mode == ThemeMode::System {
                        self.apply_theme();
                    }
                    DefWindowProcW(self.hwnd, msg, wparam, lparam)
                }
                WM_GETMINMAXINFO => {
                    let mmi = &mut *(lparam.0 as *mut MINMAXINFO);
                    mmi.ptMinTrackSize.x = scale(360, self.dpi);
                    mmi.ptMinTrackSize.y = scale(240, self.dpi);
                    LRESULT(0)
                }
                // Empty menu-strip and tab-strip space drags the window.
                WM_NCHITTEST => {
                    let r = DefWindowProcW(self.hwnd, msg, wparam, lparam);
                    if r.0 == HTCLIENT as isize {
                        let mut p = POINT {
                            x: x_lparam(lparam),
                            y: y_lparam(lparam),
                        };
                        let _ = ScreenToClient(self.hwnd, &mut p);
                        if p.y < self.menu_h() {
                            if self.strip_hit(p.x, p.y).is_none() {
                                return LRESULT(HTCAPTION as isize);
                            }
                        } else if p.y < self.menu_h() + self.tab_strip_h()
                            && self.tab_at(p.x, p.y).is_none()
                            && self.close_at(p.x, p.y).is_none()
                            && !pt_in_rect(&self.plus_rect(), p.x, p.y)
                        {
                            return LRESULT(HTCAPTION as isize);
                        }
                    }
                    r
                }
                WM_SETFOCUS => {
                    if let Some(e) = self.active_edit() {
                        let _ = SetFocus(Some(e));
                    }
                    LRESULT(0)
                }
                WM_DPICHANGED => {
                    // GetDpiForWindow already reports the new DPI here.
                    self.refresh_dpi();
                    let rc = &*(lparam.0 as *const RECT);
                    let _ = SetWindowPos(
                        self.hwnd,
                        None,
                        rc.left,
                        rc.top,
                        rc.right - rc.left,
                        rc.bottom - rc.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                    LRESULT(0)
                }
                WM_CLOSE => {
                    for i in 0..self.docs.len() {
                        if !self.confirm_save(i) {
                            return LRESULT(0);
                        }
                    }
                    let _ = DestroyWindow(self.hwnd);
                    LRESULT(0)
                }
                WM_DESTROY => {
                    let _ = DeleteObject(HGDIOBJ(self.font.0));
                    PostQuitMessage(0);
                    LRESULT(0)
                }
                _ => DefWindowProcW(self.hwnd, msg, wparam, lparam),
            }
        }
    }

    // ---- documents -------------------------------------------------------

    fn new_document(&mut self, path: Option<PathBuf>) {
        unsafe {
            let id = self.next_id;
            self.next_id += 1;
            let style = WINDOW_STYLE(
                (WS_CHILD | WS_VSCROLL | WS_HSCROLL).0
                    | ES_MULTILINE as u32
                    | ES_WANTRETURN as u32
                    | ES_AUTOVSCROLL as u32
                    | ES_AUTOHSCROLL as u32
                    | ES_NOHIDESEL as u32,
            );
            let Ok(edit) = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                MSFTEDIT_CLASS,
                PCWSTR::null(),
                style,
                0,
                0,
                0,
                0,
                Some(self.hwnd),
                Some(HMENU(id as *mut c_void)),
                None,
                None,
            ) else {
                return;
            };
            sm(edit, EM_SETEVENTMASK, 0, EDIT_EVENTS as isize);
            sm(edit, EM_EXLIMITTEXT, 0, 0x4000_0000);
            sm(edit, EM_AUTOURLDETECT, 0, 0);
            sm(
                edit,
                EM_SETTYPOGRAPHYOPTIONS,
                (TO_ADVANCEDTYPOGRAPHY | TO_SIMPLELINEBREAK) as usize,
                (TO_ADVANCEDTYPOGRAPHY | TO_SIMPLELINEBREAK) as isize,
            );
            sm(
                edit,
                EM_SETTARGETDEVICE,
                0,
                if self.word_wrap { 0 } else { 1 },
            );
            set_editor_font(edit, &self.edit_face, self.edit_pt);
            apply_editor_theme(edit, &self.theme);
            let _ = SetWindowSubclass(edit, Some(edit_proc), EDIT_SUBCLASS, self.hwnd.0 as usize);
            sm(edit, EM_SETMODIFY, 0, 0);

            let mut doc = Document::new(edit, path);
            if let Some(p) = doc.path.clone() {
                match load_file(edit, &p) {
                    Ok(enc) => doc.encoding = enc,
                    Err(e) => {
                        let msg = wide(&format!("Cannot open \"{}\":\n{}", p.display(), e));
                        MessageBoxW(
                            Some(self.hwnd),
                            PCWSTR(msg.as_ptr()),
                            w!("Nimble"),
                            MB_OK | MB_ICONERROR,
                        );
                        let _ = DestroyWindow(edit);
                        return;
                    }
                }
            }
            self.docs.push(doc);
            let idx = self.docs.len() - 1;
            self.set_active(idx);
            self.layout();
        }
    }

    fn set_active(&mut self, idx: usize) {
        if idx >= self.docs.len() {
            return;
        }
        self.active = Some(idx);
        self.invalidate_tabs();
        unsafe {
            for (i, d) in self.docs.iter().enumerate() {
                let _ = ShowWindow(d.edit, if i == idx { SW_SHOW } else { SW_HIDE });
            }
            let _ = SetFocus(Some(self.docs[idx].edit));
        }
        self.update_title();
        self.update_status();
    }

    fn cycle_tab(&mut self, dir: i32) {
        if self.docs.is_empty() {
            return;
        }
        let cur = self.active.unwrap_or(0) as i32;
        let n = self.docs.len() as i32;
        self.set_active((cur + dir).rem_euclid(n) as usize);
    }

    fn close_tab(&mut self, idx: usize) {
        if idx >= self.docs.len() || !self.confirm_save(idx) {
            return;
        }
        let doc = self.docs.remove(idx);
        unsafe {
            let _ = RemoveWindowSubclass(doc.edit, Some(edit_proc), EDIT_SUBCLASS);
            let _ = DestroyWindow(doc.edit);
        }
        self.invalidate_tabs();
        if self.docs.is_empty() {
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
            return;
        }
        self.active = None;
        self.set_active(idx.min(self.docs.len() - 1));
        self.layout();
    }

    fn active_edit(&self) -> Option<HWND> {
        self.active.and_then(|i| self.docs.get(i)).map(|d| d.edit)
    }

    // ---- tabs / chrome ---------------------------------------------------

    /// Tab text only — a modified doc gets a drawn accent dot, not a `*`.
    fn tab_label(&self, idx: usize) -> String {
        self.docs[idx].name()
    }

    fn tab_modified(&self, idx: usize) -> bool {
        idx < self.docs.len() && sm(self.docs[idx].edit, EM_GETMODIFY, 0, 0).0 != 0
    }

    /// Repaint a tab (label or dirty state changed).
    fn refresh_tab(&self, idx: usize) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), Some(&self.tab_cell(idx)), false);
        }
    }

    fn update_title(&self) {
        let title = match self.active.and_then(|i| self.docs.get(i)) {
            Some(d) => {
                let star = if sm(d.edit, EM_GETMODIFY, 0, 0).0 != 0 {
                    "*"
                } else {
                    ""
                };
                format!("{}{} - Nimble", star, d.name())
            }
            None => "Nimble".into(),
        };
        let t = wide(&title);
        unsafe {
            let _ = SetWindowTextW(self.hwnd, PCWSTR(t.as_ptr()));
        }
    }

    /// The window's monitor DPI isn't bound during WM_CREATE — re-query on
    /// every layout so a missed initial WM_DPICHANGED can't leave us at 96
    /// while the window actually renders at e.g. 125%.
    fn refresh_dpi(&mut self) {
        let d = unsafe { GetDpiForWindow(self.hwnd) };
        if d == 0 || d == self.dpi {
            return;
        }
        self.dpi = d;
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.font.0));
            self.font = ui_font(d, &self.ui_face);
        }
        findbar::retheme(self);
        self.measure_strip();
        self.recompute_menu_metrics();
    }

    pub(crate) fn layout(&mut self) {
        self.refresh_dpi();
        unsafe {
            let mut rc = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut rc);
            let (cx, cy) = (rc.right, rc.bottom);
            let menu_h = self.menu_h();
            let tab_h = self.tab_strip_h();
            let bar_h = findbar::height(self);
            let status_h = self.status_h();
            // Tabs shrink to fit like Notepad's, down to a sensible minimum.
            let n = self.docs.len().max(1) as i32;
            let pad = scale(4, self.dpi);
            self.tab_w =
                ((cx - pad * 2 - tab_h) / n).clamp(scale(64, self.dpi), scale(176, self.dpi));
            // Line-number gutter: width follows the active doc's line count.
            self.gutter_w = self.wanted_gutter_w();
            if !self.findbar.0.is_null() {
                let _ = SetWindowPos(
                    self.findbar,
                    None,
                    0,
                    menu_h + tab_h,
                    cx,
                    bar_h,
                    SWP_NOZORDER,
                );
            }
            let edit_y = menu_h + tab_h + bar_h;
            let edit_h = (cy - edit_y - status_h).max(0);
            for d in &self.docs {
                let _ = SetWindowPos(
                    d.edit,
                    None,
                    self.gutter_w,
                    edit_y,
                    cx - self.gutter_w,
                    edit_h,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                // Internal margins so text doesn't hug the window edge.
                let mut fr = RECT {
                    left: scale(10, self.dpi),
                    top: scale(6, self.dpi),
                    right: (cx - self.gutter_w - scale(10, self.dpi)).max(0),
                    bottom: (edit_h - scale(4, self.dpi)).max(0),
                };
                sm(d.edit, EM_SETRECT, 0, &mut fr as *mut RECT as isize);
            }
        }
    }

    fn update_status(&mut self) {
        if !self.show_status {
            return;
        }
        let Some(i) = self.active else { return };
        let (edit, zoom, enc) = {
            let d = &self.docs[i];
            (d.edit, d.zoom, d.encoding.label())
        };
        // After an undo RichEdit can select its final paragraph mark too.
        let caret = get_sel(edit).cpMax.min(text_len(edit));
        let (line, line_start) = self.line_at(i, caret);
        let col = caret - line_start;
        self.status_text = [
            enc.to_string(),
            "Windows (CRLF)".to_string(),
            format!("{zoom}%"),
            format!("Ln {}, Col {}", line + 1, col + 1),
        ];
        self.invalidate_status();
    }

    // ---- commands --------------------------------------------------------

    fn on_command(&mut self, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if lparam.0 != 0 {
            if hiword(wparam.0) == EN_VSCROLL {
                self.invalidate_gutter();
            } else if hiword(wparam.0) == EN_CHANGE {
                let edit = HWND(lparam.0 as *mut c_void);
                if let Some(i) = self.docs.iter().position(|d| d.edit.0 == edit.0) {
                    self.docs[i].lines_dirty = true;
                    self.refresh_tab(i);
                    if self.active == Some(i) {
                        self.update_title();
                        self.invalidate_gutter();
                    }
                }
            }
            return LRESULT(0);
        }
        unsafe {
            match loword(wparam.0) {
                IDM_NEW_TAB => self.new_document(None),
                IDM_NEW_WINDOW => self.new_window(),
                IDM_OPEN => self.open_dialog(),
                IDM_SAVE => {
                    if let Some(i) = self.active {
                        self.save_doc(i);
                    }
                }
                IDM_SAVE_AS => {
                    if let Some(i) = self.active {
                        self.save_doc_as(i);
                    }
                }
                IDM_CLOSE_TAB => {
                    if let Some(i) = self.active {
                        self.close_tab(i);
                    }
                }
                IDM_EXIT => {
                    sm(self.hwnd, WM_CLOSE, 0, 0);
                }
                IDM_SETTINGS => settings::open(self),
                IDM_UNDO => {
                    if let Some(e) = self.active_edit() {
                        sm(e, EM_UNDO, 0, 0);
                    }
                }
                IDM_REDO => {
                    if let Some(e) = self.active_edit() {
                        sm(e, EM_REDO, 0, 0);
                    }
                }
                IDM_CUT => {
                    if let Some(e) = self.active_edit() {
                        sm(e, WM_CUT, 0, 0);
                    }
                }
                IDM_COPY => {
                    if let Some(e) = self.active_edit() {
                        sm(e, WM_COPY, 0, 0);
                    }
                }
                IDM_PASTE => {
                    if let Some(e) = self.active_edit() {
                        sm(e, WM_PASTE, 0, 0);
                    }
                }
                IDM_DELETE => {
                    // The Del accelerator fires for every Delete press, so
                    // with no selection hand the key to the editor itself.
                    if let Some(e) = self.active_edit() {
                        let sel = get_sel(e);
                        if sel.cpMin == sel.cpMax {
                            sm(e, WM_KEYDOWN, VK_DELETE.0 as usize, 0);
                        } else {
                            sm(e, EM_REPLACESEL, 1, w!("").0 as isize);
                        }
                    }
                }
                IDM_SELECT_ALL => {
                    if let Some(e) = self.active_edit() {
                        sm(e, EM_SETSEL, 0, -1);
                    }
                }
                IDM_WORD_WRAP => self.toggle_wrap(),
                IDM_STATUS_BAR => self.toggle_status_bar(),
                IDM_LINE_NUMBERS => self.toggle_line_numbers(),
                IDM_FIND => findbar::open(self, BarMode::Find),
                IDM_REPLACE => findbar::open(self, BarMode::Replace),
                IDM_GOTO => findbar::open(self, BarMode::Goto),
                IDM_FIND_NEXT => self.find_next(true),
                IDM_FIND_PREV => self.find_next(false),
                IDM_DUP_LINE => self.duplicate_line(),
                IDM_ZOOM_IN => self.zoom(1),
                IDM_ZOOM_OUT => self.zoom(-1),
                IDM_ZOOM_RESET => self.zoom(0),
                theme::IDM_THEME_SYSTEM => self.set_theme(ThemeMode::System),
                theme::IDM_THEME_LIGHT => self.set_theme(ThemeMode::Light),
                theme::IDM_THEME_DARK => self.set_theme(ThemeMode::Dark),
                IDM_NEXT_TAB => self.cycle_tab(1),
                IDM_PREV_TAB => self.cycle_tab(-1),
                IDM_CHECK_UPDATES => self.check_updates(),
                IDM_ABOUT => self.about(),
                _ => return DefWindowProcW(self.hwnd, WM_COMMAND, wparam, lparam),
            }
        }
        LRESULT(0)
    }

    fn on_notify(&mut self, lparam: LPARAM) -> LRESULT {
        unsafe {
            let hdr = &*(lparam.0 as *const NMHDR);
            let is_edit = self.docs.iter().any(|d| d.edit.0 == hdr.hwndFrom.0);
            if !is_edit {
                return LRESULT(0);
            }
            if hdr.code == EN_SELCHANGE {
                if self.active_edit().map(|e| e.0) == Some(hdr.hwndFrom.0) {
                    self.update_status();
                    self.invalidate_gutter();
                }
            }
        }
        LRESULT(0)
    }

    fn on_init_menu_popup(&mut self, wparam: WPARAM) {
        let menu = HMENU(wparam.0 as *mut c_void);
        if menu.0 != self.menus.edit.0 {
            return;
        }
        let Some(edit) = self.active_edit() else {
            return;
        };
        unsafe {
            let can_undo = sm(edit, EM_CANUNDO, 0, 0).0 != 0;
            let can_redo = sm(edit, EM_CANREDO, 0, 0).0 != 0;
            let mut s = 0i32;
            let mut e = 0i32;
            sm(
                edit,
                EM_GETSEL,
                &mut s as *mut i32 as usize,
                &mut e as *mut i32 as isize,
            );
            let has_sel = s != e;
            let can_paste = IsClipboardFormatAvailable(CF_UNICODETEXT).is_ok();
            let flag = |on: bool| MF_BYCOMMAND | if on { MF_ENABLED } else { MF_GRAYED };
            let _ = EnableMenuItem(menu, IDM_UNDO, flag(can_undo));
            let _ = EnableMenuItem(menu, IDM_REDO, flag(can_redo));
            let _ = EnableMenuItem(menu, IDM_CUT, flag(has_sel));
            let _ = EnableMenuItem(menu, IDM_COPY, flag(has_sel));
            let _ = EnableMenuItem(menu, IDM_DELETE, flag(has_sel));
            let _ = EnableMenuItem(menu, IDM_PASTE, flag(can_paste));
        }
    }

    fn on_drop(&mut self, wparam: WPARAM) {
        unsafe {
            let hdrop = HDROP(wparam.0 as *mut c_void);
            let n = DragQueryFileW(hdrop, u32::MAX, None);
            for i in 0..n {
                let len = DragQueryFileW(hdrop, i, None) as usize;
                let mut buf = vec![0u16; len + 1];
                DragQueryFileW(hdrop, i, Some(&mut buf));
                self.open_path(&PathBuf::from(String::from_utf16_lossy(&buf[..len])));
            }
            DragFinish(hdrop);
        }
    }

    fn new_window(&self) {
        if let Ok(exe) = std::env::current_exe() {
            let _ = std::process::Command::new(exe).spawn();
        }
    }

    fn toggle_wrap(&mut self) {
        self.word_wrap = !self.word_wrap;
        settings::save_pref("WordWrap", self.word_wrap as u32);
        self.apply_prefs();
    }

    fn toggle_status_bar(&mut self) {
        self.show_status = !self.show_status;
        settings::save_pref("StatusBar", self.show_status as u32);
        self.apply_prefs();
    }

    fn toggle_line_numbers(&mut self) {
        self.line_numbers = !self.line_numbers;
        settings::save_pref("LineNumbers", self.line_numbers as u32);
        self.apply_prefs();
    }

    fn check_menu(&self, cmd: u32, on: bool) {
        unsafe {
            let _ = CheckMenuItem(
                self.menus.view,
                cmd,
                (MF_BYCOMMAND | if on { MF_CHECKED } else { MF_UNCHECKED }).0,
            );
        }
    }

    /// Pushes the view preferences (wrap, status bar, line numbers) out to
    /// the menus, every editor, and the layout. Called at startup and
    /// whenever the View menu or Settings changes one.
    pub(crate) fn apply_prefs(&mut self) {
        self.check_menu(IDM_WORD_WRAP, self.word_wrap);
        self.check_menu(IDM_STATUS_BAR, self.show_status);
        self.check_menu(IDM_LINE_NUMBERS, self.line_numbers);
        for d in &self.docs {
            sm(d.edit, EM_SETTARGETDEVICE, 0, if self.word_wrap { 0 } else { 1 });
        }
        self.layout();
        self.update_status();
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    /// Re-applies the editor font size from Settings to every tab.
    pub(crate) fn apply_font_size(&mut self) {
        for d in &self.docs {
            set_editor_font(d.edit, &self.edit_face, self.edit_pt);
        }
        self.layout();
        self.invalidate_gutter();
    }

    /// Rebuilds the accelerator table from the current bindings and
    /// relabels the menus so their shortcut column matches.
    pub(crate) fn apply_binds(&mut self) {
        unsafe {
            let table = settings::build_accel(ACTIONS, &self.binds);
            let old = ACCEL.swap(table.0 as usize, Ordering::Relaxed);
            if old != 0 && old != table.0 as usize {
                let _ = DestroyAcceleratorTable(HACCEL(old as *mut c_void));
            }
            for &m in &self.menus.popups {
                for pos in 0..GetMenuItemCount(Some(m)).max(0) as u32 {
                    let Some(d) = menu_item_data(m, pos) else { continue };
                    if (*d).cmd == 0 {
                        continue;
                    }
                    if let Some(k) = ACTIONS.iter().position(|a| a.cmd == (*d).cmd) {
                        let text = self.binds[k].map(|b| b.text()).unwrap_or_default();
                        (*d).accel = wide(&text);
                    }
                }
            }
        }
        self.recompute_menu_metrics();
    }

    fn zoom(&mut self, dir: i32) {
        let Some(i) = self.active else { return };
        let d = &mut self.docs[i];
        d.zoom = if dir == 0 {
            100
        } else {
            (d.zoom + dir * 10).clamp(10, 500)
        };
        sm(d.edit, EM_SETZOOM, (d.zoom / 10) as usize, 10);
        self.update_status();
        // Gutter numbers follow the editor's zoomed text size.
        self.layout();
        self.invalidate_gutter();
    }

    fn check_updates(&self) {
        unsafe {
            if !crate::install::is_installed() {
                MessageBoxW(
                    Some(self.hwnd),
                    w!(
                        "Nimble is not installed. Run \"nimble.exe --install\" to install it and enable updates."
                    ),
                    w!("Nimble"),
                    MB_OK | MB_ICONINFORMATION,
                );
                return;
            }
            match crate::install::check_updates() {
                Some(2) => MessageBoxW(
                    Some(self.hwnd),
                    w!("Nimble was updated. Restart Nimble to use the new version."),
                    w!("Nimble"),
                    MB_OK | MB_ICONINFORMATION,
                ),
                Some(0) => MessageBoxW(
                    Some(self.hwnd),
                    w!("Nimble is up to date."),
                    w!("Nimble"),
                    MB_OK | MB_ICONINFORMATION,
                ),
                _ => MessageBoxW(
                    Some(self.hwnd),
                    w!("Couldn't check for updates. Try again later."),
                    w!("Nimble"),
                    MB_OK | MB_ICONWARNING,
                ),
            };
        }
    }

    fn about(&self) {
        unsafe {
            MessageBoxW(
                Some(self.hwnd),
                w!("Nimble\n\nA blazing fast, featherweight native notepad for Windows."),
                w!("About Nimble"),
                MB_OK | MB_ICONINFORMATION,
            );
        }
    }

    // ---- theme -------------------------------------------------------------

    pub(crate) fn set_theme(&mut self, mode: ThemeMode) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        theme::save_mode(mode);
        self.apply_theme();
    }

    fn apply_theme(&mut self) {
        let dark = theme::effective_dark(self.mode);
        self.theme = if dark {
            theme::dark_theme()
        } else {
            theme::light_theme()
        };
        theme::set_preferred_app_mode(dark);
        theme::set_titlebar_dark(self.hwnd, dark);
        theme::set_titlebar_color(self.hwnd, self.theme.chrome_bg, self.theme.menu_fg);
        for d in &self.docs {
            apply_editor_theme(d.edit, &self.theme);
        }
        findbar::retheme(self);
        settings::retheme(self);
        unsafe {
            let _ = CheckMenuRadioItem(
                self.menus.theme,
                theme::IDM_THEME_SYSTEM,
                theme::IDM_THEME_DARK,
                self.mode.command_id(),
                MF_BYCOMMAND.0,
            );
            // Popup edge/background pixels outside item rects use this brush.
            let brush = CreateSolidBrush(self.theme.menu_bg);
            let info = MENUINFO {
                cbSize: size_of::<MENUINFO>() as u32,
                fMask: MIM_BACKGROUND,
                hbrBack: brush,
                ..Default::default()
            };
            for &m in &self.menus.popups {
                let _ = SetMenuInfo(m, &info);
            }
            if !self.menu_brush.0.is_null() {
                let _ = DeleteObject(HGDIOBJ(self.menu_brush.0));
            }
            self.menu_brush = brush;
            let _ = RedrawWindow(
                Some(self.hwnd),
                None,
                None,
                RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_FRAME,
            );
        }
    }

    // ---- hand-drawn tab strip -----------------------------------------------
    // The strip is painted by the main window itself — no SysTabControl32, so
    // no default frames, focus rects, or theme edges can leak through.

    fn tab_strip_h(&self) -> i32 {
        scale(32, self.dpi)
    }

    fn status_h(&self) -> i32 {
        if self.show_status {
            scale(24, self.dpi)
        } else {
            0
        }
    }

    /// Top of the editing area (tab strip bottom + find bar when open).
    fn edit_top(&self) -> i32 {
        self.menu_h() + self.tab_strip_h() + findbar::height(self)
    }

    /// Client-coord cell for tab `i` inside the strip.
    fn tab_cell(&self, i: usize) -> RECT {
        let pad = scale(4, self.dpi);
        let top = self.menu_h();
        RECT {
            left: pad + i as i32 * self.tab_w,
            top,
            right: pad + (i as i32 + 1) * self.tab_w,
            bottom: top + self.tab_strip_h(),
        }
    }

    fn tab_at(&self, x: i32, y: i32) -> Option<usize> {
        if y < self.menu_h() || y >= self.menu_h() + self.tab_strip_h() {
            return None;
        }
        let pad = scale(4, self.dpi);
        let i = (x - pad) / self.tab_w.max(1);
        if i >= 0 && (i as usize) < self.docs.len() && x >= pad {
            Some(i as usize)
        } else {
            None
        }
    }

    /// Close-button box inside a tab cell.
    fn close_rect_for(&self, cell: &RECT) -> RECT {
        let side = scale(18, self.dpi);
        let cy = (cell.top + cell.bottom) / 2;
        let right = cell.right - scale(8, self.dpi);
        RECT {
            left: right - side,
            top: cy - side / 2,
            right,
            bottom: cy + (side - side / 2),
        }
    }

    fn close_at(&self, x: i32, y: i32) -> Option<usize> {
        let i = self.tab_at(x, y)?;
        let visible = self.tab_hot == i as i32 || self.active == Some(i);
        if !visible {
            return None;
        }
        pt_in_rect(&self.close_rect_for(&self.tab_cell(i)), x, y).then_some(i)
    }

    /// The "+" button sits just after the last tab cell.
    fn plus_rect(&self) -> RECT {
        let side = scale(22, self.dpi);
        let strip_top = self.menu_h();
        let top = strip_top + (self.tab_strip_h() - side) / 2;
        let left = scale(4, self.dpi) + self.docs.len() as i32 * self.tab_w + scale(4, self.dpi);
        RECT {
            left,
            top,
            right: left + side,
            bottom: top + side,
        }
    }

    fn invalidate_tabs(&self) {
        unsafe {
            let mut rc = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut rc);
            rc.top = self.menu_h();
            rc.bottom = rc.top + self.tab_strip_h();
            let _ = InvalidateRect(Some(self.hwnd), Some(&rc), false);
        }
    }

    fn invalidate_gutter(&mut self) {
        // Digit-count growth needs a relayout so the gutter can widen.
        if self.line_numbers && self.wanted_gutter_w() != self.gutter_w {
            self.layout();
            return;
        }
        if self.gutter_w <= 0 {
            return;
        }
        unsafe {
            let mut rc = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut rc);
            let top = self.edit_top();
            let band = RECT {
                left: 0,
                top,
                right: self.gutter_w,
                bottom: rc.bottom,
            };
            let _ = InvalidateRect(Some(self.hwnd), Some(&band), false);
        }
    }

    /// Gutter width for the active doc's line count (0 when disabled).
    fn wanted_gutter_w(&mut self) -> i32 {
        let Some(i) = self.active.filter(|_| self.line_numbers) else {
            return 0;
        };
        let lines = self.line_starts(i).len().max(1);
        let digits = (lines.ilog10() as i32 + 1).max(2);
        unsafe {
            let font = self.gutter_font();
            let hdc = GetDC(Some(self.hwnd));
            let old = SelectObject(hdc, HGDIOBJ(font.0));
            let digit_w = text_extent(hdc, &wide("0"));
            SelectObject(hdc, old);
            let _ = ReleaseDC(Some(self.hwnd), hdc);
            let _ = DeleteObject(HGDIOBJ(font.0));
            scale(14, self.dpi) + digits * digit_w + scale(10, self.dpi)
        }
    }

    /// Editor face at the active tab's zoomed size, so gutter numbers sit
    /// on the same baselines as the text. Caller deletes it.
    fn gutter_font(&self) -> HFONT {
        let zoom = self.active.map(|i| self.docs[i].zoom).unwrap_or(100);
        let px = self.edit_pt * self.dpi as i32 * zoom / (72 * 100);
        let face = wide(&self.edit_face);
        unsafe {
            CreateFontW(
                -px.max(6),
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                PCWSTR(face.as_ptr()),
            )
        }
    }

    // ---- logical lines --------------------------------------------------

    /// Logical line starts for doc `i`, rebuilt after any edit. RichEdit
    /// stores line breaks as a single CR, which is what char offsets count.
    fn line_starts(&mut self, i: usize) -> &[i32] {
        let d = &mut self.docs[i];
        if d.lines_dirty {
            let text = get_text_raw(d.edit);
            d.line_starts.clear();
            d.line_starts.push(0);
            d.line_starts.extend(
                text.iter()
                    .enumerate()
                    .filter(|&(_, &c)| c == 13)
                    .map(|(k, _)| k as i32 + 1),
            );
            d.lines_dirty = false;
        }
        &self.docs[i].line_starts
    }

    /// (0-based logical line, its start offset) for char offset `cp`.
    fn line_at(&mut self, i: usize, cp: i32) -> (usize, i32) {
        let starts = self.line_starts(i);
        let ln = starts.partition_point(|&s| s <= cp).saturating_sub(1);
        (ln, starts[ln])
    }

    /// 1-based logical line of the caret in the active tab.
    pub(crate) fn caret_line(&mut self) -> usize {
        let Some(i) = self.active else { return 1 };
        let caret = get_sel(self.docs[i].edit).cpMax;
        self.line_at(i, caret).0 + 1
    }

    /// End offset of logical line `ln`, excluding its line break.
    fn line_end(&mut self, i: usize, ln: usize) -> i32 {
        let edit = self.docs[i].edit;
        match self.line_starts(i).get(ln + 1) {
            Some(&next) => next - 1,
            None => text_len(edit),
        }
    }

    /// Unified hover tracking for the menu strip and tab strip.
    fn on_mouse_move(&mut self, x: i32, y: i32) {
        let menu_hit = self.strip_hit(x, y).map(|i| i as i32).unwrap_or(-1);
        let tab_hit = self.tab_at(x, y).map(|i| i as i32).unwrap_or(-1);
        let close_hit = self.close_at(x, y).map(|i| i as i32).unwrap_or(-1);
        let plus_hit = pt_in_rect(&self.plus_rect(), x, y);
        if menu_hit != self.menu_hot {
            self.menu_hot = menu_hit;
            self.invalidate_strip();
        }
        if tab_hit != self.tab_hot
            || close_hit != self.close_hot
            || plus_hit != self.plus_hot
        {
            self.tab_hot = tab_hit;
            self.close_hot = close_hit;
            self.plus_hot = plus_hit;
            self.invalidate_tabs();
        }
        if menu_hit >= 0 || tab_hit >= 0 || plus_hit {
            unsafe {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: self.hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut tme);
            }
        }
    }

    /// Paints the whole tab strip — background, hairline, pills, labels.
    fn paint_tabs(&self, hdc: HDC) {
        unsafe {
            let th = self.theme;
            let mut rc = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut rc);
            let top = self.menu_h();
            let band = RECT {
                left: 0,
                top,
                right: rc.right,
                bottom: top + self.tab_strip_h(),
            };
            let b = CreateSolidBrush(th.chrome_bg);
            FillRect(hdc, &band, b);
            let _ = DeleteObject(HGDIOBJ(b.0));

            // Hairline across the strip bottom — the selected tab's pill is
            // drawn over it so the active tab merges into the editor.
            let mut ln = band;
            ln.top = ln.bottom - 1;
            let hb = CreateSolidBrush(th.menu_sep);
            FillRect(hdc, &ln, hb);
            let _ = DeleteObject(HGDIOBJ(hb.0));

            let aa = Aa::new(hdc);
            let rad = scale(6, self.dpi);
            for i in 0..self.docs.len() {
                let cell = self.tab_cell(i);
                let selected = self.active == Some(i);
                let hot = self.tab_hot == i as i32;
                if selected || hot {
                    let mut pill = cell;
                    pill.left += scale(2, self.dpi);
                    pill.right -= scale(2, self.dpi);
                    pill.top += scale(3, self.dpi);
                    pill.bottom += rad; // clipped at band bottom — top corners stay round
                    let saved = SaveDC(hdc);
                    let _ = IntersectClipRect(
                        hdc,
                        band.left,
                        band.top,
                        band.right,
                        band.bottom,
                    );
                    aa.fill_rounded(
                        &pill,
                        rad,
                        true,
                        if selected { th.tab_sel } else { th.tab_hot },
                    );
                    let _ = RestoreDC(hdc, saved);
                }
                self.draw_tab_contents(&aa, hdc, i, &cell, selected);
            }

            self.draw_plus(&aa);
        }
    }

    /// Label + dirty dot + close button for one tab inside `cell`.
    fn draw_tab_contents(&self, aa: &Aa, hdc: HDC, i: usize, cell: &RECT, selected: bool) {
        unsafe {
            let th = self.theme;
            let fg = if selected { th.tab_fg } else { th.tab_dim };

            // Dirty dot: accent-colored, replaces the classic '*' prefix.
            let dirty = self.tab_modified(i);
            let mut text_rc = *cell;
            text_rc.left += scale(12, self.dpi);
            text_rc.right -= scale(30, self.dpi); // room for the close button
            if dirty {
                let r = scale(3, self.dpi).max(2);
                let cy = (cell.top + cell.bottom) / 2;
                let dx = text_rc.left + r;
                aa.fill_ellipse(
                    &RECT {
                        left: dx - r,
                        top: cy - r,
                        right: dx + r,
                        bottom: cy + r,
                    },
                    th.accent,
                );
                text_rc.left += r * 2 + scale(6, self.dpi);
            }

            let mut label = wide(&self.tab_label(i));
            let n = label.len().saturating_sub(1);
            let old = SelectObject(hdc, HGDIOBJ(self.font.0));
            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, fg);
            DrawTextW(
                hdc,
                &mut label[..n],
                &mut text_rc,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
            );
            SelectObject(hdc, old);

            // Close button on the selected or hovered tab.
            if selected || self.tab_hot == i as i32 {
                let cr = self.close_rect_for(cell);
                if self.close_hot == i as i32 || self.close_press == i as i32 {
                    aa.fill_ellipse(&cr, th.menu_hover);
                }
                draw_close_glyph(aa, &cr, fg, self.dpi);
            }
        }
    }

    /// Line-number gutter — numbers the active editor's logical lines, with
    /// wrapped continuation rows left blank. The caret's line is brighter.
    fn paint_gutter(&mut self, hdc: HDC) {
        if self.gutter_w <= 0 {
            return;
        }
        let Some(i) = self.active else {
            return;
        };
        let edit = self.docs[i].edit;
        let caret_line = self.line_at(i, get_sel(edit).cpMax).0;
        unsafe {
            let th = self.theme;
            let mut rc = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut rc);
            let top = self.edit_top();
            let bottom = rc.bottom - self.status_h();
            let band = RECT {
                left: 0,
                top,
                right: self.gutter_w,
                bottom,
            };
            let b = CreateSolidBrush(th.edit_bg);
            FillRect(hdc, &band, b);
            let _ = DeleteObject(HGDIOBJ(b.0));

            let font = self.gutter_font();
            let old = SelectObject(hdc, HGDIOBJ(font.0));
            SetBkMode(hdc, TRANSPARENT);
            let saved = SaveDC(hdc);
            let _ = IntersectClipRect(hdc, band.left, band.top, band.right, band.bottom);
            let first = sm(edit, EM_GETFIRSTVISIBLELINE, 0, 0).0.max(0);
            let row_h = scale(40, self.dpi);
            for row in first..first + 2000 {
                let charidx = sm(edit, EM_LINEINDEX, row as usize, 0).0;
                if charidx < 0 {
                    break;
                }
                let mut pt = POINTL::default();
                sm(
                    edit,
                    EM_POSFROMCHAR,
                    &mut pt as *mut POINTL as usize,
                    charidx,
                );
                let y = top + pt.y;
                if y > bottom {
                    break;
                }
                let (ln, start) = self.line_at(i, charidx as i32);
                if start != charidx as i32 {
                    continue; // wrapped continuation row — no number
                }
                SetTextColor(hdc, if ln == caret_line { th.tab_fg } else { th.menu_dim });
                let mut r = RECT {
                    left: 0,
                    top: y,
                    right: band.right - scale(10, self.dpi),
                    bottom: y + row_h,
                };
                let mut t = wide(&(ln + 1).to_string());
                let n = t.len() - 1;
                DrawTextW(hdc, &mut t[..n], &mut r, DT_RIGHT | DT_TOP | DT_SINGLELINE);
            }
            let _ = RestoreDC(hdc, saved);
            SelectObject(hdc, old);
            let _ = DeleteObject(HGDIOBJ(font.0));
        }
    }

    fn status_band(&self) -> RECT {
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rc);
        }
        rc.top = rc.bottom - self.status_h();
        rc
    }

    fn invalidate_status(&self) {
        if self.show_status {
            unsafe {
                let _ = InvalidateRect(Some(self.hwnd), Some(&self.status_band()), false);
            }
        }
    }

    /// Status band: flat chrome strip with a hairline on top and segments
    /// laid out from the right edge, split by short dividers.
    fn paint_status(&self, hdc: HDC) {
        if !self.show_status {
            return;
        }
        unsafe {
            let th = self.theme;
            let band = self.status_band();
            let b = CreateSolidBrush(th.chrome_bg);
            FillRect(hdc, &band, b);
            let _ = DeleteObject(HGDIOBJ(b.0));
            let sep = CreateSolidBrush(th.menu_sep);
            let mut ln = band;
            ln.bottom = ln.top + 1;
            FillRect(hdc, &ln, sep);

            let old = SelectObject(hdc, HGDIOBJ(self.font.0));
            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, th.status_fg);
            let pad = scale(12, self.dpi);
            let mut right = band.right - scale(6, self.dpi);
            for (k, text) in self.status_text.iter().enumerate() {
                if text.is_empty() {
                    continue;
                }
                let mut t = wide(text);
                let n = t.len() - 1;
                let w = text_extent(hdc, &t) + pad * 2;
                let mut r = RECT {
                    left: right - w,
                    top: band.top + 1,
                    right,
                    bottom: band.bottom,
                };
                DrawTextW(hdc, &mut t[..n], &mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                right -= w;
                if k + 1 < self.status_text.len() {
                    let inset = scale(6, self.dpi);
                    let div = RECT {
                        left: right,
                        top: band.top + inset,
                        right: right + 1,
                        bottom: band.bottom - inset,
                    };
                    FillRect(hdc, &div, sep);
                }
            }
            SelectObject(hdc, old);
            let _ = DeleteObject(HGDIOBJ(sep.0));
        }
    }

    // ---- menu strip ---------------------------------------------------------
    // The menu bar is drawn by hand: Win32 menu bars can't be themed, so the
    // window has no attached menu. The strip paints File/Edit/View/Help across
    // the top of the client area; clicking opens the real popup via
    // TrackPopupMenuEx, and every popup item is owner-drawn (ODT_MENU).

    fn menu_h(&self) -> i32 {
        scale(30, self.dpi)
    }

    fn measure_strip(&mut self) {
        unsafe {
            let hdc = GetDC(Some(self.hwnd));
            let old = SelectObject(hdc, HGDIOBJ(self.font.0));
            let mut x = scale(4, self.dpi);
            for e in &mut self.menus.strip {
                e.x = x;
                e.w = text_extent(hdc, &e.label) + 2 * scale(10, self.dpi);
                x += e.w;
            }
            SelectObject(hdc, old);
            let _ = ReleaseDC(Some(self.hwnd), hdc);
        }
    }

    /// Recomputes the accelerator column width for every popup so the
    /// shortcut text aligns down the right side of each menu.
    fn recompute_menu_metrics(&self) {
        unsafe {
            let hdc = GetDC(Some(self.hwnd));
            let old = SelectObject(hdc, HGDIOBJ(self.font.0));
            for &m in &self.menus.popups {
                let n = GetMenuItemCount(Some(m));
                if n < 0 {
                    continue;
                }
                let mut max_w = 0;
                let mut max_l = 0;
                for i in 0..n as u32 {
                    if let Some(d) = menu_item_data(m, i) {
                        let w = text_extent(hdc, &(*d).accel);
                        max_w = max_w.max(w);
                        let lw = text_extent(hdc, &(*d).label);
                        max_l = max_l.max(lw);
                    }
                }
                for i in 0..n as u32 {
                    if let Some(d) = menu_item_data(m, i) {
                        (*d).accel_col = max_w;
                        (*d).label_col = max_l;
                        // Windows caches owner-drawn item sizes; touching
                        // the type makes it send WM_MEASUREITEM again.
                        let mii = MENUITEMINFOW {
                            cbSize: size_of::<MENUITEMINFOW>() as u32,
                            fMask: MIIM_FTYPE,
                            fType: MFT_OWNERDRAW,
                            ..Default::default()
                        };
                        let _ = SetMenuItemInfoW(m, i, true, &mii);
                    }
                }
            }
            SelectObject(hdc, old);
            let _ = ReleaseDC(Some(self.hwnd), hdc);
        }
    }

    fn strip_hit(&self, x: i32, y: i32) -> Option<usize> {
        if y < 0 || y >= self.menu_h() {
            return None;
        }
        self.menus
            .strip
            .iter()
            .position(|e| x >= e.x && x < e.x + e.w)
    }

    fn invalidate_strip(&self) {
        unsafe {
            let mut rc = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut rc);
            rc.bottom = self.menu_h();
            let _ = InvalidateRect(Some(self.hwnd), Some(&rc), false);
        }
    }

    /// Shows the popup for strip item `idx`; while open, clicking a different
    /// strip entry chains into that menu like a classic menu bar. A CBT hook
    /// watches for the popup window so its corners can be rounded.
    fn menu_loop(&mut self, mut idx: usize) {
        unsafe {
            POPUP_HOOK_HWND.store(self.hwnd.0 as usize, Ordering::Relaxed);
            let hook = SetWindowsHookExW(WH_CBT, Some(menu_hook_proc), None, GetCurrentThreadId())
                .unwrap_or_default();
            loop {
                self.menu_open = idx as i32;
                self.menu_hot = idx as i32;
                self.invalidate_strip();
                let e = &self.menus.strip[idx];
                // TPM wants screen coordinates.
                let mut pt = POINT {
                    x: e.x,
                    y: self.menu_h(),
                };
                let _ = ClientToScreen(self.hwnd, &mut pt);
                let cmd = TrackPopupMenuEx(
                    e.menu,
                    (TPM_LEFTALIGN | TPM_TOPALIGN | TPM_RETURNCMD).0,
                    pt.x,
                    pt.y,
                    self.hwnd,
                    None,
                )
                .0;
                self.menu_open = -1;
                self.invalidate_strip();
                if cmd != 0 {
                    self.on_command(WPARAM(cmd as usize), LPARAM(0));
                    break;
                }
                // Dismissed: only chain when the cursor landed on a
                // different strip item (i.e. the click that closed us).
                let mut p = POINT::default();
                let _ = GetCursorPos(&mut p);
                let _ = ScreenToClient(self.hwnd, &mut p);
                match self.strip_hit(p.x, p.y) {
                    Some(n) if n != idx => idx = n,
                    _ => break,
                }
            }
            if !hook.0.is_null() {
                let _ = UnhookWindowsHookEx(hook);
            }
            POPUP_HOOK_HWND.store(0, Ordering::Relaxed);
            if let Some(e) = self.active_edit() {
                let _ = SetFocus(Some(e));
            }
        }
    }

    fn paint_menu_strip(&self, hdc: HDC) {
        unsafe {
            let menu_h = self.menu_h();
            // Fill the strip band itself — partial invalidates can't rely on
            // WM_ERASEBKGND having run first.
            let mut band = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut band);
            band.bottom = menu_h;
            let bb = CreateSolidBrush(self.theme.chrome_bg);
            FillRect(hdc, &band, bb);
            let _ = DeleteObject(HGDIOBJ(bb.0));

            let aa = Aa::new(hdc);
            let old = SelectObject(hdc, HGDIOBJ(self.font.0));
            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, self.theme.menu_fg);
            for (i, e) in self.menus.strip.iter().enumerate() {
                let mut rc = RECT {
                    left: e.x,
                    top: 0,
                    right: e.x + e.w,
                    bottom: menu_h,
                };
                if i as i32 == self.menu_hot || i as i32 == self.menu_open {
                    let mut pill = rc;
                    pill.top += scale(3, self.dpi);
                    pill.bottom -= scale(3, self.dpi);
                    pill.left += scale(1, self.dpi);
                    pill.right -= scale(1, self.dpi);
                    aa.fill_rounded(&pill, scale(4, self.dpi), false, self.theme.menu_hover);
                }
                let mut label = e.label.clone();
                let n = label.len().saturating_sub(1);
                DrawTextW(
                    hdc,
                    &mut label[..n],
                    &mut rc,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                );
            }
            SelectObject(hdc, old);
        }
    }

    fn measure_menu_item(&self, mi: &mut MEASUREITEMSTRUCT) {
        unsafe {
            let d = mi.itemData as *const MenuItemData;
            if d.is_null() || (*d).sep {
                mi.itemWidth = 1;
                mi.itemHeight = (scale(9, self.dpi) * self.dpi as i32 / 96) as u32;
                return;
            }
            let hdc = GetDC(Some(self.hwnd));
            let old = SelectObject(hdc, HGDIOBJ(self.font.0));
            let text_h = text_height(hdc, &(*d).label);
            SelectObject(hdc, old);
            let _ = ReleaseDC(Some(self.hwnd), hdc);
            let gutter = scale(28, self.dpi);
            let accel_w = (*d).accel_col;
            let gap = if accel_w > 0 { scale(20, self.dpi) } else { 0 };
            let tail = if (*d).submenu {
                scale(24, self.dpi)
            } else {
                scale(14, self.dpi)
            };
            let w = gutter + (*d).label_col + gap + accel_w + tail;
            let h = (text_h + scale(12, self.dpi)).max(scale(24, self.dpi));
            // Windows rescales owner-drawn item metrics from the menu's
            // create-time DPI (system, 96) to the popup's display DPI — the
            // shrink factor is 96/dpi — so pre-compensate by dpi/96.
            mi.itemWidth = (w * self.dpi as i32 / 96) as u32;
            mi.itemHeight = (h * self.dpi as i32 / 96) as u32;
        }
    }

    fn draw_menu_item(&self, dis: &DRAWITEMSTRUCT) {
        unsafe {
            let th = self.theme;
            let d = dis.itemData as *const MenuItemData;
            let bg = CreateSolidBrush(th.menu_bg);
            FillRect(dis.hDC, &dis.rcItem, bg);
            let _ = DeleteObject(HGDIOBJ(bg.0));
            if d.is_null() || (*d).sep {
                // Separator: a single hairline, inset past the check gutter.
                let mut ln = dis.rcItem;
                ln.left += scale(28, self.dpi);
                ln.right -= scale(8, self.dpi);
                ln.top = (ln.top + ln.bottom) / 2;
                ln.bottom = ln.top + 1;
                let b = CreateSolidBrush(th.menu_sep);
                FillRect(dis.hDC, &ln, b);
                let _ = DeleteObject(HGDIOBJ(b.0));
                return;
            }
            let d = &*d;
            let aa = Aa::new(dis.hDC);
            let selected = (dis.itemState.0 & ODS_SELECTED.0) != 0;
            let grayed = (dis.itemState.0 & (ODS_GRAYED.0 | ODS_DISABLED.0)) != 0;
            if selected && !grayed {
                // Rounded hover pill, inset a few px from the popup edge.
                let mut pill = dis.rcItem;
                pill.left += scale(3, self.dpi);
                pill.right -= scale(3, self.dpi);
                pill.top += 1;
                pill.bottom -= 1;
                aa.fill_rounded(&pill, scale(4, self.dpi), false, th.menu_hover);
            }
            let fg = if grayed { th.menu_dim } else { th.menu_fg };
            let old = SelectObject(dis.hDC, HGDIOBJ(self.font.0));
            SetBkMode(dis.hDC, TRANSPARENT);
            SetTextColor(dis.hDC, fg);
            let gutter = scale(28, self.dpi);
            // Check gutter: vector-drawn tick or radio dot (no font dependency).
            if (dis.itemState.0 & ODS_CHECKED.0) != 0 {
                let rc = RECT {
                    left: dis.rcItem.left + scale(4, self.dpi),
                    top: dis.rcItem.top,
                    right: dis.rcItem.left + gutter,
                    bottom: dis.rcItem.bottom,
                };
                if d.radio {
                    draw_radio_glyph(&aa, &rc, fg, self.dpi);
                } else {
                    draw_check_glyph(&aa, &rc, fg, self.dpi);
                }
            }
            let mut rc = dis.rcItem;
            rc.left += gutter;
            // Fixed columns: label sits after the check gutter, accelerators
            // left-align in their own column right after the widest label —
            // never anchored to the popup's right edge, which Windows can
            // size tighter than the items under mixed DPI.
            let accel_x = rc.left + d.label_col + scale(20, self.dpi);
            rc.right = accel_x - scale(6, self.dpi);
            let mut label = d.label.clone();
            let n = label.len().saturating_sub(1);
            DrawTextW(
                dis.hDC,
                &mut label[..n],
                &mut rc,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE,
            );
            let mut rrc = dis.rcItem;
            if d.submenu {
                rrc.left = accel_x;
                draw_chevron_glyph(&aa, &rrc, th.menu_dim, self.dpi);
            } else if d.accel.len() > 1 {
                let mut accel = d.accel.clone();
                let n = accel.len().saturating_sub(1);
                rrc.left = accel_x;
                rrc.right = accel_x + d.accel_col;
                SetTextColor(dis.hDC, th.menu_dim);
                DrawTextW(
                    dis.hDC,
                    &mut accel[..n],
                    &mut rrc,
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE,
                );
            }
            SelectObject(dis.hDC, old);
        }
    }

    /// The "+" new-tab button right after the last tab.
    fn draw_plus(&self, aa: &Aa) {
        let rc = self.plus_rect();
        if self.plus_hot {
            aa.fill_ellipse(&rc, self.theme.tab_hot);
        }
        draw_plus_glyph(aa, &rc, self.theme.tab_fg, self.dpi);
    }

    // ---- find / replace / go to -------------------------------------------

    /// Current selection as a search seed: single line, short, non-empty.
    pub(crate) fn selected_text(&self) -> Option<String> {
        let edit = self.active_edit()?;
        let sel = get_sel(edit);
        let n = sel.cpMax - sel.cpMin;
        if n <= 0 || n > 256 {
            return None;
        }
        let text = get_range(edit, sel.cpMin, sel.cpMax);
        if text.iter().any(|&c| c == 13 || c == 10) {
            return None;
        }
        Some(String::from_utf16_lossy(&text))
    }

    /// F3 / Shift+F3 / Enter in the find bar. Wraps around once.
    pub(crate) fn find_next(&mut self, forward: bool) {
        let q = findbar::query(self);
        if q.len() <= 1 {
            findbar::open(self, BarMode::Find);
            return;
        }
        self.find_query = q.clone();
        let Some(edit) = self.active_edit() else { return };
        let sel = get_sel(edit);
        let hit = if forward {
            find_range(edit, &q, sel.cpMax, -1, true).or_else(|| find_range(edit, &q, 0, -1, true))
        } else {
            find_range(edit, &q, sel.cpMin, 0, false)
                .or_else(|| find_range(edit, &q, text_len(edit), 0, false))
        };
        self.show_hit(edit, hit);
    }

    /// Find-as-you-type: re-search from the start of the current match so
    /// extending the query keeps the same hit selected.
    pub(crate) fn find_incremental(&mut self) {
        let q = findbar::query(self);
        let Some(edit) = self.active_edit() else { return };
        if q.len() <= 1 {
            findbar::set_note(self, "", false);
            return;
        }
        let from = get_sel(edit).cpMin;
        let hit =
            find_range(edit, &q, from, -1, true).or_else(|| find_range(edit, &q, 0, -1, true));
        self.show_hit(edit, hit);
    }

    fn show_hit(&mut self, edit: HWND, hit: Option<CHARRANGE>) {
        match hit {
            Some(r) => {
                set_sel(edit, r.cpMin, r.cpMax);
                sm(edit, EM_SCROLLCARET, 0, 0);
                findbar::set_note(self, "", false);
            }
            None => findbar::set_note(self, "No results", true),
        }
        self.invalidate_gutter();
    }

    /// Replaces the selection if it is a match, then moves to the next one.
    pub(crate) fn replace_next(&mut self) {
        let q = findbar::query(self);
        let Some(edit) = self.active_edit().filter(|_| q.len() > 1) else {
            return;
        };
        let sel = get_sel(edit);
        if sel.cpMax > sel.cpMin && find_range(edit, &q, sel.cpMin, sel.cpMax, true) == Some(sel) {
            let rep = findbar::repl_text(self);
            sm(edit, EM_REPLACESEL, 1, rep.as_ptr() as isize);
        }
        self.find_next(true);
    }

    /// Replaces every match in one edit, so a single Undo reverts it all.
    pub(crate) fn replace_all(&mut self) {
        let q = findbar::query(self);
        let Some(edit) = self.active_edit().filter(|_| q.len() > 1) else {
            return;
        };
        let mut hits = Vec::new();
        let mut pos = 0;
        while let Some(r) = find_range(edit, &q, pos, -1, true) {
            if r.cpMax <= r.cpMin {
                break;
            }
            hits.push(r);
            pos = r.cpMax;
        }
        if hits.is_empty() {
            findbar::set_note(self, "No results", true);
            return;
        }
        let rep = findbar::repl_text(self);
        let rep = &rep[..rep.len() - 1];
        let text = get_text_raw(edit);
        if text.len() as i32 != text_len(edit) {
            // Offsets don't line up with the plain text (embedded objects
            // from a rich paste) — replace match by match, last first.
            for r in hits.iter().rev() {
                set_sel(edit, r.cpMin, r.cpMax);
                let mut w = rep.to_vec();
                w.push(0);
                sm(edit, EM_REPLACESEL, 1, w.as_ptr() as isize);
            }
        } else {
            // Rewrite only the span from the first match to the last, so
            // Undo restores (and selects) as little as possible.
            let span_start = hits[0].cpMin;
            let span_end = hits[hits.len() - 1].cpMax;
            let mut out = Vec::new();
            let mut last = span_start as usize;
            for r in &hits {
                out.extend_from_slice(&text[last..r.cpMin as usize]);
                out.extend_from_slice(rep);
                last = r.cpMax as usize;
            }
            out.push(0);
            let caret = get_sel(edit).cpMin;
            let mut scroll = POINT::default();
            sm(edit, EM_GETSCROLLPOS, 0, &mut scroll as *mut POINT as isize);
            sm(edit, WM_SETREDRAW, 0, 0);
            set_sel(edit, span_start, span_end);
            sm(edit, EM_REPLACESEL, 1, out.as_ptr() as isize);
            let c = caret.min(text_len(edit));
            set_sel(edit, c, c);
            sm(edit, EM_SETSCROLLPOS, 0, &mut scroll as *mut POINT as isize);
            sm(edit, WM_SETREDRAW, 1, 0);
            unsafe {
                let _ = InvalidateRect(Some(edit), None, true);
            }
        }
        let n = hits.len();
        findbar::set_note(self, &format!("{n} replaced"), false);
        self.invalidate_gutter();
    }

    /// Enter in the bar's Go To mode: jump to the typed line number.
    pub(crate) fn goto_from_bar(&mut self) {
        let q = findbar::query(self);
        let typed = String::from_utf16_lossy(&q[..q.len().saturating_sub(1)]);
        let Some(i) = self.active else { return };
        let Ok(n) = typed.trim().parse::<usize>() else {
            findbar::set_note(self, "Type a line number", true);
            return;
        };
        let count = self.line_starts(i).len();
        let cp = self.line_starts(i)[n.clamp(1, count) - 1];
        let edit = self.docs[i].edit;
        set_sel(edit, cp, cp);
        sm(edit, EM_SCROLLCARET, 0, 0);
        self.close_findbar();
    }

    pub(crate) fn close_findbar(&mut self) {
        findbar::close(self);
        if let Some(e) = self.active_edit() {
            unsafe {
                let _ = SetFocus(Some(e));
            }
        }
    }

    /// Ctrl+D: copies the line(s) touched by the selection below
    /// themselves and moves the selection onto the copy.
    fn duplicate_line(&mut self) {
        let Some(i) = self.active else { return };
        let edit = self.docs[i].edit;
        let sel = get_sel(edit);
        let (first, start) = self.line_at(i, sel.cpMin);
        // A selection ending at a line start doesn't include that line.
        let last_cp = if sel.cpMax > sel.cpMin { sel.cpMax - 1 } else { sel.cpMax };
        let last = self.line_at(i, last_cp).0.max(first);
        let end = self.line_end(i, last);
        let mut ins = vec![13u16];
        ins.extend(get_range(edit, start, end));
        ins.push(0);
        set_sel(edit, end, end);
        sm(edit, EM_REPLACESEL, 1, ins.as_ptr() as isize);
        let shift = end - start + 1;
        set_sel(edit, sel.cpMin + shift, sel.cpMax + shift);
        sm(edit, EM_SCROLLCARET, 0, 0);
    }

    /// Enter with auto-indent on: carry the current line's leading
    /// whitespace onto the new line. Returns false to let RichEdit handle
    /// a plain Enter.
    fn auto_indent_newline(&mut self, edit: HWND) -> bool {
        let Some(i) = self.docs.iter().position(|d| d.edit.0 == edit.0) else {
            return false;
        };
        let sel = get_sel(edit);
        let (_, start) = self.line_at(i, sel.cpMin);
        let indent: Vec<u16> = get_range(edit, start, sel.cpMin)
            .into_iter()
            .take_while(|&c| c == b' ' as u16 || c == b'\t' as u16)
            .collect();
        if indent.is_empty() {
            return false;
        }
        let mut ins = vec![13u16];
        ins.extend(indent);
        ins.push(0);
        sm(edit, EM_REPLACESEL, 1, ins.as_ptr() as isize);
        true
    }

    // ---- files -----------------------------------------------------------

    fn open_dialog(&mut self) {
        if let Some(path) = file_dialog(self.hwnd, true, None) {
            self.open_path(&path);
        }
    }

    fn open_path(&mut self, path: &Path) {
        let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if let Some(i) = self
            .docs
            .iter()
            .position(|d| d.path.as_deref() == Some(canon.as_path()))
        {
            self.set_active(i);
            return;
        }
        // A lone untouched "Untitled" tab (the one every window starts
        // with) gives way to the file instead of lingering beside it.
        let blank = (self.docs.len() == 1
            && self.docs[0].path.is_none()
            && text_len(self.docs[0].edit) == 0
            && sm(self.docs[0].edit, EM_GETMODIFY, 0, 0).0 == 0)
            .then(|| self.docs[0].edit);
        let before = self.docs.len();
        self.new_document(Some(canon));
        if let Some(edit) = blank.filter(|_| self.docs.len() > before) {
            unsafe {
                let _ = RemoveWindowSubclass(edit, Some(edit_proc), EDIT_SUBCLASS);
                let _ = DestroyWindow(edit);
            }
            self.docs.remove(0);
            self.active = Some(self.docs.len() - 1);
            self.layout();
            self.invalidate_tabs();
        }
    }

    fn confirm_save(&mut self, i: usize) -> bool {
        if i >= self.docs.len() || sm(self.docs[i].edit, EM_GETMODIFY, 0, 0).0 == 0 {
            return true;
        }
        self.set_active(i);
        let text = wide(&format!(
            "Do you want to save changes to {}?",
            self.docs[i].name()
        ));
        unsafe {
            match MessageBoxW(
                Some(self.hwnd),
                PCWSTR(text.as_ptr()),
                w!("Nimble"),
                MB_YESNOCANCEL | MB_ICONWARNING,
            ) {
                IDYES => self.save_doc(i),
                IDNO => true,
                _ => false,
            }
        }
    }

    fn save_doc(&mut self, i: usize) -> bool {
        if i >= self.docs.len() {
            return true;
        }
        if self.docs[i].path.is_none() {
            return self.save_doc_as(i);
        }
        let path = self.docs[i].path.clone().unwrap();
        self.write_doc(i, &path)
    }

    fn save_doc_as(&mut self, i: usize) -> bool {
        if i >= self.docs.len() {
            return false;
        }
        match file_dialog(self.hwnd, false, Some(&self.docs[i].name())) {
            Some(path) => {
                self.docs[i].path = Some(path.clone());
                self.write_doc(i, &path)
            }
            None => false,
        }
    }

    fn write_doc(&mut self, i: usize, path: &Path) -> bool {
        let units = get_text(self.docs[i].edit);
        let bytes = encoding::encode(&units, self.docs[i].encoding);
        match std::fs::write(path, &bytes) {
            Ok(()) => {
                sm(self.docs[i].edit, EM_SETMODIFY, 0, 0);
                self.refresh_tab(i);
                if self.active == Some(i) {
                    self.update_title();
                }
                true
            }
            Err(e) => {
                let msg = wide(&format!("Cannot save \"{}\":\n{}", path.display(), e));
                unsafe {
                    MessageBoxW(
                        Some(self.hwnd),
                        PCWSTR(msg.as_ptr()),
                        w!("Nimble"),
                        MB_OK | MB_ICONERROR,
                    );
                }
                false
            }
        }
    }
}

// ---- helpers --------------------------------------------------------------

/// Text metrics for a null-terminated UTF-16 buffer.
fn text_size(hdc: HDC, s: &[u16]) -> SIZE {
    let n = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    let mut sz = SIZE::default();
    unsafe {
        let _ = GetTextExtentPoint32W(hdc, &s[..n], &mut sz);
    }
    sz
}

fn text_extent(hdc: HDC, s: &[u16]) -> i32 {
    text_size(hdc, s).cx
}

fn text_height(hdc: HDC, s: &[u16]) -> i32 {
    text_size(hdc, s).cy
}

/// Fetches the owner-draw data pointer of the menu item at `pos`.
unsafe fn menu_item_data(menu: HMENU, pos: u32) -> Option<*mut MenuItemData> {
    let mut mii = MENUITEMINFOW {
        cbSize: size_of::<MENUITEMINFOW>() as u32,
        fMask: MIIM_DATA,
        ..Default::default()
    };
    unsafe {
        if GetMenuItemInfoW(menu, pos, true, &mut mii).is_ok() && mii.dwItemData != 0 {
            Some(mii.dwItemData as *mut MenuItemData)
        } else {
            None
        }
    }
}

/// Returns the first font face name that actually exists on this system —
/// CreateFontW silently substitutes missing faces, so verify via GetTextFace.
fn pick_face(hdc: HDC, candidates: &[&str]) -> String {
    unsafe {
        for name in candidates {
            let wname = wide(name);
            let f = CreateFontW(
                -12,
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                DEFAULT_QUALITY,
                0,
                PCWSTR(wname.as_ptr()),
            );
            if f.0.is_null() {
                continue;
            }
            let old = SelectObject(hdc, HGDIOBJ(f.0));
            let mut buf = [0u16; 64];
            let n = GetTextFaceW(hdc, Some(&mut buf));
            SelectObject(hdc, old);
            let _ = DeleteObject(HGDIOBJ(f.0));
            let actual = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
            if actual.eq_ignore_ascii_case(name) {
                return (*name).to_string();
            }
        }
        candidates.last().map(|s| s.to_string()).unwrap_or_default()
    }
}

fn ui_font(dpi: u32, face: &str) -> HFONT {
    let wface = wide(face);
    unsafe {
        CreateFontW(
            -((9 * dpi as i32 + 36) / 72),
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            PCWSTR(wface.as_ptr()),
        )
    }
}

/// Applies the editor face and size. Like apply_editor_theme, this keeps the
/// modified flag and mutes EN_CHANGE so a font change never dirties a tab.
fn set_editor_font(edit: HWND, face: &str, pt: i32) {
    let was_modified = sm(edit, EM_GETMODIFY, 0, 0).0;
    sm(edit, EM_SETEVENTMASK, 0, 0);
    let mut cf = CHARFORMAT2W::default();
    cf.Base.cbSize = size_of::<CHARFORMAT2W>() as u32;
    cf.Base.dwMask = CFM_FACE | CFM_SIZE | CFM_CHARSET;
    cf.Base.yHeight = pt * 20; // twips
    cf.Base.bCharSet = DEFAULT_CHARSET;
    let face = wide(face);
    let n = (face.len() - 1).min(31);
    cf.Base.szFaceName[..n].copy_from_slice(&face[..n]);
    sm(
        edit,
        EM_SETCHARFORMAT,
        SCF_ALL as usize,
        &cf as *const _ as isize,
    );
    sm(edit, EM_SETMODIFY, was_modified as usize, 0);
    sm(edit, EM_SETEVENTMASK, 0, EDIT_EVENTS as isize);
}

/// Recolors an editor control for the active theme. Preserves the modified
/// flag and silences EN_CHANGE while formatting so the transient dirty state
/// can't reach the tab label or title bar.
fn apply_editor_theme(edit: HWND, th: &Theme) {
    let was_modified = sm(edit, EM_GETMODIFY, 0, 0).0;
    sm(edit, EM_SETEVENTMASK, 0, 0);
    sm(edit, EM_SETBKGNDCOLOR, 0, th.edit_bg.0 as isize);
    let mut cf = CHARFORMAT2W::default();
    cf.Base.cbSize = size_of::<CHARFORMAT2W>() as u32;
    cf.Base.dwMask = CFM_COLOR;
    cf.Base.crTextColor = th.edit_fg;
    sm(
        edit,
        EM_SETCHARFORMAT,
        SCF_ALL as usize,
        &cf as *const _ as isize,
    );
    sm(edit, EM_SETMODIFY, was_modified as usize, 0);
    sm(edit, EM_SETEVENTMASK, 0, EDIT_EVENTS as isize);
    theme::set_control_dark(edit, th.dark);
}

fn load_file(edit: HWND, path: &Path) -> std::io::Result<Encoding> {
    let bytes = std::fs::read(path)?;
    let (units, enc) = encoding::decode(&bytes);
    let st = SETTEXTEX {
        flags: ST_DEFAULT,
        codepage: CP_UTF16,
    };
    sm(
        edit,
        EM_SETTEXTEX,
        &st as *const SETTEXTEX as usize,
        units.as_ptr() as isize,
    );
    sm(edit, EM_EMPTYUNDOBUFFER, 0, 0);
    sm(edit, EM_SETMODIFY, 0, 0);
    sm(edit, EM_SETSEL, 0, 0);
    sm(edit, EM_SCROLLCARET, 0, 0);
    Ok(enc)
}

fn get_text(edit: HWND) -> Vec<u16> {
    let mut gtl = GETTEXTLENGTHEX {
        flags: GTL_NUMCHARS | GTL_USECRLF,
        codepage: CP_UTF16,
    };
    let len = sm(
        edit,
        EM_GETTEXTLENGTHEX,
        &mut gtl as *mut GETTEXTLENGTHEX as usize,
        0,
    )
    .0 as usize;
    if len == 0 {
        return Vec::new();
    }
    let mut buf = vec![0u16; len + 1];
    let mut gt = GETTEXTEX {
        cb: (buf.len() * 2) as u32,
        flags: GT_USECRLF,
        codepage: CP_UTF16,
        ..Default::default()
    };
    let n = sm(
        edit,
        EM_GETTEXTEX,
        &mut gt as *mut GETTEXTEX as usize,
        buf.as_mut_ptr() as isize,
    )
    .0 as usize;
    buf.truncate(n.min(len));
    buf
}

/// Editor text without CRLF translation — offsets match RichEdit's char
/// positions one-to-one (line breaks are a single CR).
fn get_text_raw(edit: HWND) -> Vec<u16> {
    let len = text_len(edit).max(0) as usize;
    if len == 0 {
        return Vec::new();
    }
    let mut buf = vec![0u16; len + 1];
    let mut gt = GETTEXTEX {
        cb: (buf.len() * 2) as u32,
        flags: GT_DEFAULT,
        codepage: CP_UTF16,
        ..Default::default()
    };
    let n = sm(
        edit,
        EM_GETTEXTEX,
        &mut gt as *mut GETTEXTEX as usize,
        buf.as_mut_ptr() as isize,
    )
    .0 as usize;
    buf.truncate(n.min(len));
    buf
}

/// Character count in RichEdit's own units (CR line breaks).
fn text_len(edit: HWND) -> i32 {
    let mut gtl = GETTEXTLENGTHEX {
        flags: GTL_NUMCHARS | GTL_PRECISE,
        codepage: CP_UTF16,
    };
    sm(
        edit,
        EM_GETTEXTLENGTHEX,
        &mut gtl as *mut GETTEXTLENGTHEX as usize,
        0,
    )
    .0 as i32
}

fn get_sel(edit: HWND) -> CHARRANGE {
    let mut r = CHARRANGE::default();
    sm(edit, EM_EXGETSEL, 0, &mut r as *mut CHARRANGE as isize);
    r
}

fn set_sel(edit: HWND, from: i32, to: i32) {
    let mut r = CHARRANGE {
        cpMin: from,
        cpMax: to,
    };
    sm(edit, EM_EXSETSEL, 0, &mut r as *mut CHARRANGE as isize);
}

/// Text in [from, to) as UTF-16, no terminator.
fn get_range(edit: HWND, from: i32, to: i32) -> Vec<u16> {
    if to <= from {
        return Vec::new();
    }
    let mut buf = vec![0u16; (to - from) as usize + 1];
    let mut tr = TEXTRANGEW {
        chrg: CHARRANGE {
            cpMin: from,
            cpMax: to,
        },
        lpstrText: PWSTR(buf.as_mut_ptr()),
    };
    let n = sm(edit, EM_GETTEXTRANGE, 0, &mut tr as *mut TEXTRANGEW as isize).0;
    buf.truncate(n.max(0) as usize);
    buf
}

/// Case-insensitive RichEdit search for null-terminated `q` from `from`
/// toward `to` (backward when `forward` is false; `to` = -1 is the end).
fn find_range(edit: HWND, q: &[u16], from: i32, to: i32, forward: bool) -> Option<CHARRANGE> {
    let mut ft = FINDTEXTEXW {
        chrg: CHARRANGE {
            cpMin: from,
            cpMax: to,
        },
        lpstrText: PCWSTR(q.as_ptr()),
        chrgText: CHARRANGE::default(),
    };
    let flags = if forward { FR_DOWN.0 } else { 0 };
    let r = sm(
        edit,
        EM_FINDTEXTEXW,
        flags as usize,
        &mut ft as *mut FINDTEXTEXW as isize,
    )
    .0;
    if r >= 0 { Some(ft.chrgText) } else { None }
}

fn file_dialog(owner: HWND, open: bool, initial: Option<&str>) -> Option<PathBuf> {
    unsafe {
        let mut buf = vec![0u16; 32768];
        if let Some(name) = initial {
            let n = wide(name);
            let copy = (n.len() - 1).min(buf.len() - 1);
            buf[..copy].copy_from_slice(&n[..copy]);
        }
        let mut ofn = OPENFILENAMEW {
            lStructSize: size_of::<OPENFILENAMEW>() as u32,
            hwndOwner: owner,
            lpstrFilter: w!("Text Documents (*.txt)\0*.txt\0All Files (*.*)\0*.*\0\0"),
            lpstrFile: PWSTR(buf.as_mut_ptr()),
            nMaxFile: buf.len() as u32,
            lpstrDefExt: w!("txt"),
            Flags: if open {
                OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_HIDEREADONLY
            } else {
                OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST
            },
            lpstrTitle: if open { w!("Open") } else { w!("Save As") },
            ..Default::default()
        };
        let ok = if open {
            GetOpenFileNameW(&mut ofn)
        } else {
            GetSaveFileNameW(&mut ofn)
        };
        if !ok.as_bool() {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        if len == 0 {
            return None;
        }
        Some(PathBuf::from(String::from_utf16_lossy(&buf[..len])))
    }
}

// ---- subclasses ------------------------------------------------------------

/// Tab strip: middle-click closes a tab, double-click on empty space opens one.
// ---- vector glyphs ---------------------------------------------------------
// Chrome icons are drawn through GDI+ (Aa) — anti-aliased, crisp at any DPI,
// and immune to missing icon fonts.

fn draw_plus_glyph(aa: &Aa, rc: &RECT, color: COLORREF, dpi: u32) {
    let cx = (rc.left + rc.right) / 2;
    let cy = (rc.top + rc.bottom) / 2;
    let half = (rc.right - rc.left) * 7 / 22;
    let w = scale(1, dpi).max(1) as f32;
    aa.stroke(&[(cx - half, cy), (cx + half, cy)], color, w);
    aa.stroke(&[(cx, cy - half), (cx, cy + half)], color, w);
}

pub(crate) fn draw_close_glyph(aa: &Aa, rc: &RECT, color: COLORREF, dpi: u32) {
    let cx = (rc.left + rc.right) / 2;
    let cy = (rc.top + rc.bottom) / 2;
    let half = (rc.right - rc.left) * 5 / 18;
    let w = scale(1, dpi).max(1) as f32;
    aa.stroke(&[(cx - half, cy - half), (cx + half, cy + half)], color, w);
    aa.stroke(&[(cx + half, cy - half), (cx - half, cy + half)], color, w);
}

fn draw_check_glyph(aa: &Aa, rc: &RECT, color: COLORREF, dpi: u32) {
    let w = rc.right - rc.left;
    let h = rc.bottom - rc.top;
    aa.stroke(
        &[
            (rc.left + w * 7 / 24, rc.top + h * 12 / 24),
            (rc.left + w * 11 / 24, rc.top + h * 16 / 24),
            (rc.left + w * 18 / 24, rc.top + h * 7 / 24),
        ],
        color,
        (scale(1, dpi).max(1) + 1) as f32,
    );
}

fn draw_radio_glyph(aa: &Aa, rc: &RECT, color: COLORREF, _dpi: u32) {
    let r = ((rc.right - rc.left) / 8).max(3);
    let cx = (rc.left + rc.right) / 2;
    let cy = (rc.top + rc.bottom) / 2;
    aa.fill_ellipse(
        &RECT {
            left: cx - r,
            top: cy - r,
            right: cx + r,
            bottom: cy + r,
        },
        color,
    );
}

fn draw_chevron_glyph(aa: &Aa, rc: &RECT, color: COLORREF, dpi: u32) {
    let h = rc.bottom - rc.top;
    let cx = rc.right - scale(6, dpi);
    let cy = (rc.top + rc.bottom) / 2;
    aa.stroke(
        &[(cx - 2, cy - h * 3 / 16), (cx + 2, cy), (cx - 2, cy + h * 3 / 16)],
        color,
        scale(1, dpi).max(1) as f32,
    );
}

// ---- subclasses ------------------------------------------------------------

/// CBT hook active while a TrackPopupMenuEx popup is open: subclasses each
/// menu window ("#32768") so its frame is themed. Corner rounding is left to
/// DWM (Windows 11 draws popups rounded natively) — SetWindowRgn would clip
/// with visibly jagged stair-steps.
unsafe extern "system" fn menu_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if code == HCBT_CREATEWND as i32 || code == HCBT_ACTIVATE as i32 {
            let hwnd = HWND(wparam.0 as *mut c_void);
            let mut name = [0u16; 32];
            let n = GetClassNameW(hwnd, &mut name);
            if n == 6
                && &name[..6]
                    == [
                        b'#' as u16,
                        b'3' as u16,
                        b'2' as u16,
                        b'7' as u16,
                        b'6' as u16,
                        b'8' as u16,
                    ]
            {
                theme::round_corners(hwnd);
                let _ = SetWindowSubclass(hwnd, Some(popup_proc), POPUP_SUBCLASS, 0);
            }
        }
        CallNextHookEx(None, code, wparam, lparam)
    }
}

/// Popup menu window: the system frames popups in a bright white that clashes
/// with dark themes. Paint the non-client frame with the menu's border color
/// so popups match their owner-drawn items.
unsafe extern "system" fn popup_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _refdata: usize,
) -> LRESULT {
    unsafe {
        if msg == WM_NCPAINT {
            let r = DefSubclassProc(hwnd, msg, wparam, lparam);
            // Frame color follows the *popup's* theme — look it up on the
            // owning window (stored when the hook installed the subclass).
            let parent = GetWindowLongPtrW(hwnd, GWLP_HWNDPARENT);
            let owner = if parent != 0 {
                HWND(parent as *mut c_void)
            } else {
                HWND(POPUP_HOOK_HWND.load(Ordering::Relaxed) as *mut c_void)
            };
            let app = GetWindowLongPtrW(owner, GWLP_USERDATA) as *mut App;
            let color = if app.is_null() {
                COLORREF(0x00707070)
            } else {
                (*app).theme.menu_sep
            };
            let dc = GetWindowDC(Some(hwnd));
            let mut rc = RECT::default();
            let _ = GetWindowRect(hwnd, &mut rc);
            rc.right -= rc.left;
            rc.bottom -= rc.top;
            rc.left = 0;
            rc.top = 0;
            let b = CreateSolidBrush(color);
            let _ = FrameRect(dc, &rc, b);
            let _ = DeleteObject(HGDIOBJ(b.0));
            let _ = ReleaseDC(Some(hwnd), dc);
            return r;
        }
        if msg == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(hwnd, Some(popup_proc), POPUP_SUBCLASS);
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }
}

/// Editor: Ctrl+wheel zooms, Enter auto-indents, scrolling repaints the
/// line-number gutter.
unsafe extern "system" fn edit_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    refdata: usize,
) -> LRESULT {
    unsafe {
        let parent = HWND(refdata as *mut c_void);
        let app = GetWindowLongPtrW(parent, GWLP_USERDATA) as *mut App;
        if app.is_null() {
            return DefSubclassProc(hwnd, msg, wparam, lparam);
        }
        match msg {
            WM_MOUSEWHEEL if (wparam.0 as u16 & 0x0008) != 0 => {
                let delta = (wparam.0 >> 16) as i16;
                (*app).zoom(if delta > 0 { 1 } else { -1 });
                LRESULT(0)
            }
            WM_CHAR if wparam.0 == 0x0D && (*app).auto_indent && (*app).auto_indent_newline(hwnd) => {
                LRESULT(0)
            }
            WM_MOUSEWHEEL | WM_VSCROLL => {
                let r = DefSubclassProc(hwnd, msg, wparam, lparam);
                (*app).invalidate_gutter();
                r
            }
            _ => DefSubclassProc(hwnd, msg, wparam, lparam),
        }
    }
}
