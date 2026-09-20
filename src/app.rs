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
use crate::encoding::{self, Encoding};
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
const IDM_UNDO: u32 = 200;
const IDM_REDO: u32 = 201;
const IDM_CUT: u32 = 202;
const IDM_COPY: u32 = 203;
const IDM_PASTE: u32 = 204;
const IDM_DELETE: u32 = 205;
const IDM_SELECT_ALL: u32 = 206;
const IDM_WORD_WRAP: u32 = 300;
const IDM_STATUS_BAR: u32 = 301;
const IDM_ZOOM_IN: u32 = 302;
const IDM_ZOOM_OUT: u32 = 303;
const IDM_ZOOM_RESET: u32 = 304;
const IDM_ABOUT: u32 = 400;
const IDM_CHECK_UPDATES: u32 = 401;
// Theme submenu IDs live in theme.rs (IDM_THEME_SYSTEM/LIGHT/DARK)
const IDM_NEXT_TAB: u32 = 500;
const IDM_PREV_TAB: u32 = 501;

const EDIT_ID_BASE: usize = 4096;
const TAB_SUBCLASS: usize = 1;
const EDIT_SUBCLASS: usize = 2;
const STATUS_SUBCLASS: usize = 3;
const POPUP_SUBCLASS: usize = 4;

/// Our window handle while a menu popup is being hooked (same thread only).
static POPUP_HOOK_HWND: AtomicUsize = AtomicUsize::new(0);

#[inline]
fn sm(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> LRESULT {
    unsafe { SendMessageW(hwnd, msg, Some(WPARAM(wparam)), Some(LPARAM(lparam))) }
}

#[inline]
fn loword(v: usize) -> u32 {
    (v & 0xFFFF) as u32
}

#[inline]
fn hiword(v: usize) -> u32 {
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

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[inline]
fn scale(v: i32, dpi: u32) -> i32 {
    v * dpi as i32 / 96
}

#[inline]
fn pt_in_rect(rc: &RECT, x: i32, y: i32) -> bool {
    x >= rc.left && x < rc.right && y >= rc.top && y < rc.bottom
}

/// Owner-draw data for one popup menu item; boxed and leaked for the life
/// of the app — menus live as long as the process anyway.
struct MenuItemData {
    label: Vec<u16>, // display text, '&' stripped, no accelerator
    accel: Vec<u16>, // shortcut text ("Ctrl+S") or empty
    accel_col: i32,  // shared accelerator column width for its popup
    label_col: i32,  // shared label column width for its popup
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

struct App {
    hwnd: HWND,
    tab: HWND,
    status: HWND,
    menus: MenuSet,
    menu_hot: i32,  // strip index under the cursor, -1 = none
    menu_open: i32, // strip index whose popup is showing, -1 = none
    plus_hot: bool, // cursor over the "+" new-tab glyph
    tab_hot: i32,   // tab index under the cursor, -1 = none
    close_hot: i32, // tab index whose close button is hot, -1 = none
    menu_brush: HBRUSH,
    docs: Vec<Document>,
    active: Option<usize>,
    word_wrap: bool,
    show_status: bool,
    dpi: u32,
    font: HFONT,
    ui_face: String,
    edit_face: String,
    next_id: usize,
    mode: ThemeMode,
    theme: Theme,
    status_text: [[u16; 96]; 5],
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
        let accel = create_accel();

        let app = Box::new(App::new(mode, menus));
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
            if TranslateAcceleratorW(hwnd, accel, &msg) == 0 {
                let _ = TranslateMessage(&msg);
                let _ = DispatchMessageW(&msg);
            }
        }
    }
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

        let view = CreatePopupMenu().unwrap_or_default();
        add_item(view, IDM_WORD_WRAP, "&Word Wrap", false, false, true);
        add_item(view, IDM_STATUS_BAR, "&Status Bar", false, false, true);
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

fn create_accel() -> HACCEL {
    let ctrl = |key: u16, cmd: u32| ACCEL {
        fVirt: FVIRTKEY | FCONTROL,
        key,
        cmd: cmd as u16,
    };
    let ctrl_shift = |key: u16, cmd: u32| ACCEL {
        fVirt: FVIRTKEY | FCONTROL | FSHIFT,
        key,
        cmd: cmd as u16,
    };
    let accels = [
        ctrl(b'N' as u16, IDM_NEW_WINDOW),
        ctrl(b'T' as u16, IDM_NEW_TAB),
        ctrl(b'O' as u16, IDM_OPEN),
        ctrl(b'S' as u16, IDM_SAVE),
        ctrl_shift(b'S' as u16, IDM_SAVE_AS),
        ctrl(b'W' as u16, IDM_CLOSE_TAB),
        ctrl(b'Z' as u16, IDM_UNDO),
        ctrl(b'Y' as u16, IDM_REDO),
        ctrl(b'A' as u16, IDM_SELECT_ALL),
        ctrl(VK_TAB.0, IDM_NEXT_TAB),
        ctrl_shift(VK_TAB.0, IDM_PREV_TAB),
        ctrl(VK_OEM_PLUS.0, IDM_ZOOM_IN),
        ctrl(VK_OEM_MINUS.0, IDM_ZOOM_OUT),
        ctrl(b'0' as u16, IDM_ZOOM_RESET),
    ];
    unsafe { CreateAcceleratorTableW(&accels).unwrap_or_default() }
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
    fn new(mode: ThemeMode, menus: MenuSet) -> Self {
        let theme = if theme::effective_dark(mode) {
            theme::dark_theme()
        } else {
            theme::light_theme()
        };
        Self {
            hwnd: HWND::default(),
            tab: HWND::default(),
            status: HWND::default(),
            menus,
            menu_hot: -1,
            menu_open: -1,
            plus_hot: false,
            tab_hot: -1,
            close_hot: -1,
            menu_brush: HBRUSH::default(),
            docs: Vec::new(),
            active: None,
            word_wrap: true,
            show_status: true,
            dpi: 96,
            font: HFONT::default(),
            ui_face: String::new(),
            edit_face: String::new(),
            next_id: EDIT_ID_BASE,
            mode,
            theme,
            status_text: [[0; 96]; 5],
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

            self.tab = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                WC_TABCONTROL,
                PCWSTR::null(),
                WINDOW_STYLE(
                    (WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS).0
                        | TCS_OWNERDRAWFIXED
                        | TCS_FIXEDWIDTH
                        | TCS_FOCUSNEVER,
                ),
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
            sm(self.tab, WM_SETFONT, self.font.0 as usize, 1);
            let _ = SetWindowSubclass(self.tab, Some(tab_proc), TAB_SUBCLASS, self.hwnd.0 as usize);

            self.status = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                STATUSCLASSNAMEW,
                PCWSTR::null(),
                WS_CHILD,
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
            sm(self.status, WM_SETFONT, self.font.0 as usize, 1);
            let _ = SetWindowSubclass(
                self.status,
                Some(status_proc),
                STATUS_SUBCLASS,
                self.hwnd.0 as usize,
            );

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
            self.layout();
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
                    let _ = EndPaint(self.hwnd, &mut ps);
                    LRESULT(0)
                }
                WM_MOUSEMOVE => {
                    self.on_strip_hover(x_lparam(lparam), y_lparam(lparam));
                    LRESULT(0)
                }
                WM_MOUSELEAVE => {
                    if self.menu_hot >= 0 {
                        self.menu_hot = -1;
                        self.invalidate_strip();
                    }
                    LRESULT(0)
                }
                WM_LBUTTONDOWN => {
                    if let Some(i) = self.strip_hit(x_lparam(lparam), y_lparam(lparam)) {
                        self.menu_loop(i);
                        LRESULT(0)
                    } else {
                        DefWindowProcW(self.hwnd, msg, wparam, lparam)
                    }
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
                    } else if dis.hwndItem.0 == self.tab.0 {
                        self.draw_tab(dis);
                    } else if dis.hwndItem.0 == self.status.0 {
                        self.draw_status_part(dis);
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
                // Empty menu-strip space acts as a drag handle for the window.
                WM_NCHITTEST => {
                    let r = DefWindowProcW(self.hwnd, msg, wparam, lparam);
                    if r.0 == HTCLIENT as isize {
                        let mut p = POINT {
                            x: x_lparam(lparam),
                            y: y_lparam(lparam),
                        };
                        let _ = ScreenToClient(self.hwnd, &mut p);
                        if p.y < self.menu_h() && self.strip_hit(p.x, p.y).is_none() {
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
            sm(
                edit,
                EM_SETEVENTMASK,
                0,
                (ENM_CHANGE | ENM_SELCHANGE) as isize,
            );
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
            set_editor_font(edit, &self.edit_face);
            apply_editor_theme(edit, &self.theme);
            let _ = SetWindowSubclass(edit, Some(edit_proc), EDIT_SUBCLASS, self.hwnd.0 as usize);
            sm(edit, EM_SETMODIFY, 0, 0);

            let mut doc = Document {
                edit,
                path,
                encoding: Encoding::Utf8,
                zoom: 100,
            };
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
            self.insert_tab_item(idx);
            self.set_active(idx);
            self.layout();
        }
    }

    fn set_active(&mut self, idx: usize) {
        if idx >= self.docs.len() {
            return;
        }
        self.active = Some(idx);
        sm(self.tab, TCM_SETCURSEL, idx, 0);
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
        sm(self.tab, TCM_DELETEITEM, idx, 0);
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

    fn insert_tab_item(&self, idx: usize) {
        let text = wide(&self.tab_label(idx));
        let item = TCITEMW {
            mask: TCIF_TEXT,
            pszText: PWSTR(text.as_ptr() as *mut u16),
            ..Default::default()
        };
        sm(self.tab, TCM_INSERTITEMW, idx, &item as *const _ as isize);
    }

    fn refresh_tab(&self, idx: usize) {
        let text = wide(&self.tab_label(idx));
        let item = TCITEMW {
            mask: TCIF_TEXT,
            pszText: PWSTR(text.as_ptr() as *mut u16),
            ..Default::default()
        };
        sm(self.tab, TCM_SETITEMW, idx, &item as *const _ as isize);
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
            sm(self.tab, WM_SETFONT, self.font.0 as usize, 1);
            sm(self.status, WM_SETFONT, self.font.0 as usize, 1);
        }
        self.measure_strip();
        self.recompute_menu_metrics();
    }

    fn layout(&mut self) {
        self.refresh_dpi();
        unsafe {
            let mut rc = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut rc);
            let (cx, cy) = (rc.right, rc.bottom);
            let menu_h = self.menu_h();
            let tab_h = scale(32, self.dpi);
            let plus_w = tab_h; // room for the strip-drawn "+" after last tab
            let status_h = if self.show_status {
                scale(23, self.dpi)
            } else {
                0
            };
            // Tabs shrink to fit like Notepad's, down to a sensible minimum.
            let n = self.docs.len().max(1) as i32;
            let tab_w = ((cx - plus_w) / n).clamp(scale(64, self.dpi), scale(176, self.dpi));
            sm(
                self.tab,
                TCM_SETITEMSIZE,
                0,
                ((tab_w & 0xFFFF) | (tab_h << 16)) as isize,
            );
            let _ = SetWindowPos(self.tab, None, 0, menu_h, cx, tab_h, SWP_NOZORDER);
            if self.show_status {
                let _ = ShowWindow(self.status, SW_SHOW);
                let _ = SetWindowPos(
                    self.status,
                    None,
                    0,
                    cy - status_h,
                    cx,
                    status_h,
                    SWP_NOZORDER,
                );
                self.update_status_parts(cx);
            } else {
                let _ = ShowWindow(self.status, SW_HIDE);
            }
            let edit_h = (cy - menu_h - tab_h - status_h).max(0);
            for d in &self.docs {
                let _ = SetWindowPos(
                    d.edit,
                    None,
                    0,
                    menu_h + tab_h,
                    cx,
                    edit_h,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                // Internal margins so text doesn't hug the window edge.
                let mut fr = RECT {
                    left: scale(10, self.dpi),
                    top: scale(6, self.dpi),
                    right: (cx - scale(10, self.dpi)).max(0),
                    bottom: (edit_h - scale(4, self.dpi)).max(0),
                };
                sm(d.edit, EM_SETRECT, 0, &mut fr as *mut RECT as isize);
            }
        }
    }

    fn update_status_parts(&self, cx: i32) {
        let d = self.dpi as i32;
        let edges = [
            cx - 430 * d / 96,
            cx - 320 * d / 96,
            cx - 250 * d / 96,
            cx - 140 * d / 96,
            -1,
        ];
        sm(
            self.status,
            SB_SETPARTS,
            edges.len(),
            edges.as_ptr() as isize,
        );
        // Part 0 gets owner-draw too so it doesn't render with the sunken
        // edge — the pointer is stored for later WM_DRAWITEM, so it must be
        // a static, not a stack local.
        static EMPTY_PART: [u16; 1] = [0];
        sm(
            self.status,
            SB_SETTEXTW,
            SBT_OWNERDRAW as usize,
            EMPTY_PART.as_ptr() as isize,
        );
        // SB_SETTEXT is dropped for parts that don't exist yet, so re-attach
        // the stored buffers every time the parts are (re)created here.
        for i in 1..self.status_text.len() {
            sm(
                self.status,
                SB_SETTEXTW,
                i | SBT_OWNERDRAW as usize,
                self.status_text[i].as_ptr() as isize,
            );
        }
    }

    fn set_status_part(&mut self, part: usize, text: &str) {
        let w = wide(text);
        let n = (w.len() - 1).min(95);
        self.status_text[part][..n].copy_from_slice(&w[..n]);
        self.status_text[part][n] = 0;
        sm(
            self.status,
            SB_SETTEXTW,
            part | SBT_OWNERDRAW as usize,
            self.status_text[part].as_ptr() as isize,
        );
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
        let mut lo = 0i32;
        let mut hi = 0i32;
        sm(
            edit,
            EM_GETSEL,
            &mut lo as *mut i32 as usize,
            &mut hi as *mut i32 as isize,
        );
        let line = sm(edit, EM_EXLINEFROMCHAR, 0, hi as isize).0 as i64;
        let line_start = sm(edit, EM_LINEINDEX, line as usize, 0).0 as i64;
        let col = hi as i64 - line_start;
        self.set_status_part(1, &format!("Ln {}, Col {}", line + 1, col + 1));
        self.set_status_part(2, &format!("{}%", zoom));
        self.set_status_part(3, "Windows (CRLF)");
        self.set_status_part(4, enc);
    }

    // ---- commands --------------------------------------------------------

    fn on_command(&mut self, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if lparam.0 != 0 {
            if hiword(wparam.0) == EN_CHANGE {
                let edit = HWND(lparam.0 as *mut c_void);
                if let Some(i) = self.docs.iter().position(|d| d.edit.0 == edit.0) {
                    self.refresh_tab(i);
                    if self.active == Some(i) {
                        self.update_title();
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
                    if let Some(e) = self.active_edit() {
                        sm(e, EM_REPLACESEL, 1, w!("").0 as isize);
                    }
                }
                IDM_SELECT_ALL => {
                    if let Some(e) = self.active_edit() {
                        sm(e, EM_SETSEL, 0, -1);
                    }
                }
                IDM_WORD_WRAP => self.toggle_wrap(),
                IDM_STATUS_BAR => self.toggle_status_bar(),
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
            if hdr.hwndFrom.0 == self.tab.0 {
                if hdr.code == TCN_SELCHANGE {
                    let sel = sm(self.tab, TCM_GETCURSEL, 0, 0).0;
                    if sel >= 0 && (sel as usize) < self.docs.len() {
                        self.set_active(sel as usize);
                    }
                }
            } else if hdr.code == EN_SELCHANGE {
                if self.active_edit().map(|e| e.0) == Some(hdr.hwndFrom.0) {
                    self.update_status();
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
        unsafe {
            let _ = CheckMenuItem(
                self.menus.view,
                IDM_WORD_WRAP,
                (MF_BYCOMMAND
                    | if self.word_wrap {
                        MF_CHECKED
                    } else {
                        MF_UNCHECKED
                    })
                .0,
            );
            for d in &self.docs {
                sm(
                    d.edit,
                    EM_SETTARGETDEVICE,
                    0,
                    if self.word_wrap { 0 } else { 1 },
                );
            }
        }
    }

    fn toggle_status_bar(&mut self) {
        self.show_status = !self.show_status;
        unsafe {
            let _ = CheckMenuItem(
                self.menus.view,
                IDM_STATUS_BAR,
                (MF_BYCOMMAND
                    | if self.show_status {
                        MF_CHECKED
                    } else {
                        MF_UNCHECKED
                    })
                .0,
            );
        }
        self.layout();
        self.update_status();
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

    fn set_theme(&mut self, mode: ThemeMode) {
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
        theme::set_control_dark(self.tab, dark);
        theme::set_control_dark(self.status, dark);
        sm(
            self.status,
            SB_SETBKCOLOR,
            0,
            self.theme.chrome_bg.0 as isize,
        );
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

    /// Index of the tab under a strip-space point, if any.
    fn tab_hit(&self, tab: HWND, x: i32, y: i32) -> Option<usize> {
        let mut ht = TCHITTESTINFO {
            pt: POINT { x, y },
            ..Default::default()
        };
        let i = sm(tab, TCM_HITTEST, 0, &mut ht as *mut TCHITTESTINFO as isize).0;
        (i >= 0 && (i as usize) < self.docs.len()).then_some(i as usize)
    }

    /// Index of the tab whose close button is under the point. Only counts
    /// where the button is visible — the selected or hovered tab.
    fn tab_close_hit(&self, tab: HWND, x: i32, y: i32) -> Option<usize> {
        let i = self.tab_hit(tab, x, y)?;
        let visible = i as i32 == self.tab_hot || self.active == Some(i);
        if !visible {
            return None;
        }
        let mut ir = RECT::default();
        sm(tab, TCM_GETITEMRECT, i, &mut ir as *mut RECT as isize);
        pt_in_rect(&self.tab_close_rect(&ir), x, y).then_some(i)
    }

    /// Close-button box inside a tab's item rect.
    fn tab_close_rect(&self, item: &RECT) -> RECT {
        let side = scale(18, self.dpi);
        let cy = (item.top + item.bottom) / 2;
        let right = item.right - scale(7, self.dpi);
        RECT {
            left: right - side,
            top: cy - side / 2,
            right,
            bottom: cy + (side - side / 2),
        }
    }

    fn draw_tab(&self, dis: &DRAWITEMSTRUCT) {
        unsafe {
            let i = dis.itemID as usize;
            if i >= self.docs.len() {
                return;
            }
            let th = self.theme;
            let selected = (dis.itemState.0 & ODS_SELECTED.0) != 0;

            // Flat base fill only — the control draws its raised frame around
            // the selected item *outside* rcItem, so the rounded pill is drawn
            // later over the whole cell in tab_proc's paint overlay.
            let bg = if selected {
                th.tab_sel
            } else if self.tab_hot == i as i32 {
                th.tab_hot
            } else {
                th.chrome_bg
            };
            let brush = CreateSolidBrush(bg);
            FillRect(dis.hDC, &dis.rcItem, brush);
            let _ = DeleteObject(HGDIOBJ(brush.0));

            self.draw_tab_contents(dis.hDC, i, &dis.rcItem, selected);
        }
    }

    /// Label + dirty dot + close button for one tab, positioned inside `cell`
    /// (the owner-draw item rect or the full cell in the paint overlay).
    fn draw_tab_contents(&self, hdc: HDC, i: usize, cell: &RECT, selected: bool) {
        unsafe {
            let th = self.theme;
            let fg = if selected { th.tab_fg } else { th.tab_dim };

            // Dirty dot: accent-colored, replaces the classic '*' prefix.
            let dirty = self.tab_modified(i);
            let mut text_rc = *cell;
            text_rc.left += scale(10, self.dpi);
            text_rc.right -= scale(28, self.dpi); // leave room for the close button
            if dirty {
                let r = scale(3, self.dpi).max(2);
                let cy = (cell.top + cell.bottom) / 2;
                let dx = text_rc.left + r;
                let b = CreateSolidBrush(th.accent);
                let oldb = SelectObject(hdc, HGDIOBJ(b.0));
                let np = SelectObject(hdc, GetStockObject(NULL_PEN));
                let _ = Ellipse(hdc, dx - r, cy - r, dx + r, cy + r);
                SelectObject(hdc, np);
                SelectObject(hdc, oldb);
                let _ = DeleteObject(HGDIOBJ(b.0));
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
                let cr = self.tab_close_rect(cell);
                if self.close_hot == i as i32 {
                    let b = CreateSolidBrush(th.menu_hover);
                    let saved = SaveDC(hdc);
                    let _ = IntersectClipRect(hdc, cell.left, cell.top, cell.right, cell.bottom);
                    let oldb = SelectObject(hdc, HGDIOBJ(b.0));
                    let np = SelectObject(hdc, GetStockObject(NULL_PEN));
                    let _ = Ellipse(hdc, cr.left, cr.top, cr.right, cr.bottom);
                    SelectObject(hdc, np);
                    SelectObject(hdc, oldb);
                    let _ = DeleteObject(HGDIOBJ(b.0));
                    let _ = RestoreDC(hdc, saved);
                }
                draw_close_glyph(hdc, &cr, fg, self.dpi);
            }
        }
    }

    /// Post-paint overlay (runs from tab_proc after the control paints): draws
    /// the rounded-top pill over the *whole* cell for the selected/hovered
    /// tab, which covers the control's default frame edge that sits just
    /// outside the owner-draw item rect.
    fn draw_tabs(&self, hdc: HDC) {
        unsafe {
            let count = sm(self.tab, TCM_GETITEMCOUNT, 0, 0).0.max(0) as usize;
            let th = self.theme;
            let rad = scale(6, self.dpi);
            let mut client = RECT::default();
            let _ = GetClientRect(self.tab, &mut client);
            for i in 0..count {
                let selected = self.active == Some(i);
                let hot = self.tab_hot == i as i32;
                if !selected && !hot {
                    continue;
                }
                let mut cell = RECT::default();
                sm(
                    self.tab,
                    TCM_GETITEMRECT,
                    i,
                    &mut cell as *mut RECT as isize,
                );
                let saved = SaveDC(hdc);
                let _ =
                    IntersectClipRect(hdc, client.left, client.top, client.right, client.bottom);
                let b = CreateSolidBrush(if selected { th.tab_sel } else { th.tab_hot });
                if selected {
                    // The control draws the selected item's frame edge outside
                    // rcItem — flat-fill the whole cell band up to the strip
                    // top first so no frame sliver survives.
                    let mut cover = cell;
                    cover.left -= scale(4, self.dpi);
                    cover.right += scale(4, self.dpi);
                    cover.top = client.top;
                    FillRect(hdc, &cover, b);
                }
                let mut pill = cell;
                pill.left += scale(1, self.dpi);
                pill.right -= scale(1, self.dpi);
                if selected {
                    pill.top = client.top;
                }
                pill.bottom += rad; // clipped — only the top corners stay round
                let oldb = SelectObject(hdc, HGDIOBJ(b.0));
                let np = SelectObject(hdc, GetStockObject(NULL_PEN));
                let _ = RoundRect(
                    hdc,
                    pill.left,
                    pill.top,
                    pill.right,
                    pill.bottom,
                    rad * 2,
                    rad * 2,
                );
                SelectObject(hdc, np);
                SelectObject(hdc, oldb);
                let _ = DeleteObject(HGDIOBJ(b.0));
                let _ = RestoreDC(hdc, saved);
                self.draw_tab_contents(hdc, i, &cell, selected);
            }
        }
    }

    fn draw_status_part(&self, dis: &DRAWITEMSTRUCT) {
        unsafe {
            let bg = CreateSolidBrush(self.theme.chrome_bg);
            FillRect(dis.hDC, &dis.rcItem, bg);
            let _ = DeleteObject(HGDIOBJ(bg.0));
            if dis.itemData == 0 {
                return;
            }
            let ptr = dis.itemData as *const u16;
            let mut len = 0usize;
            while len < 96 && *ptr.add(len) != 0 {
                len += 1;
            }
            if len == 0 {
                return; // DrawTextW dereferences the buffer — never pass empty
            }
            let mut s = std::slice::from_raw_parts(ptr, len).to_vec();
            let old = SelectObject(dis.hDC, HGDIOBJ(self.font.0));
            SetBkMode(dis.hDC, TRANSPARENT);
            SetTextColor(dis.hDC, self.theme.status_fg);
            let mut rc = dis.rcItem;
            DrawTextW(
                dis.hDC,
                &mut s,
                &mut rc,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
            );
            SelectObject(dis.hDC, old);
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

    fn on_strip_hover(&mut self, x: i32, y: i32) {
        let hit = self.strip_hit(x, y).map(|i| i as i32).unwrap_or(-1);
        if hit != self.menu_hot {
            self.menu_hot = hit;
            self.invalidate_strip();
        }
        if hit >= 0 {
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
                    let rad = scale(4, self.dpi);
                    let mut pill = rc;
                    pill.top += scale(3, self.dpi);
                    pill.bottom -= scale(3, self.dpi);
                    pill.left += scale(1, self.dpi);
                    pill.right -= scale(1, self.dpi);
                    let b = CreateSolidBrush(self.theme.menu_hover);
                    let oldb = SelectObject(hdc, HGDIOBJ(b.0));
                    let np = SelectObject(hdc, GetStockObject(NULL_PEN));
                    let _ = RoundRect(
                        hdc,
                        pill.left,
                        pill.top,
                        pill.right,
                        pill.bottom,
                        rad * 2,
                        rad * 2,
                    );
                    SelectObject(hdc, np);
                    SelectObject(hdc, oldb);
                    let _ = DeleteObject(HGDIOBJ(b.0));
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
            let selected = (dis.itemState.0 & ODS_SELECTED.0) != 0;
            let grayed = (dis.itemState.0 & (ODS_GRAYED.0 | ODS_DISABLED.0)) != 0;
            if selected && !grayed {
                // Rounded hover pill, inset a few px from the popup edge.
                let rad = scale(4, self.dpi);
                let mut pill = dis.rcItem;
                pill.left += scale(3, self.dpi);
                pill.right -= scale(3, self.dpi);
                pill.top += 1;
                pill.bottom -= 1;
                let b = CreateSolidBrush(th.menu_hover);
                let oldb = SelectObject(dis.hDC, HGDIOBJ(b.0));
                let np = SelectObject(dis.hDC, GetStockObject(NULL_PEN));
                let _ = RoundRect(
                    dis.hDC,
                    pill.left,
                    pill.top,
                    pill.right,
                    pill.bottom,
                    rad * 2,
                    rad * 2,
                );
                SelectObject(dis.hDC, np);
                SelectObject(dis.hDC, oldb);
                let _ = DeleteObject(HGDIOBJ(b.0));
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
                    draw_radio_glyph(dis.hDC, &rc, fg, self.dpi);
                } else {
                    draw_check_glyph(dis.hDC, &rc, fg, self.dpi);
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
                draw_chevron_glyph(dis.hDC, &rrc, th.menu_dim, self.dpi);
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

    /// The "+" new-tab glyph sits right after the last tab, drawn inside the
    /// strip's empty area — same hit-test region used by tab_proc's clicks.
    fn plus_rect(&self) -> RECT {
        let side = scale(22, self.dpi);
        let top = (scale(32, self.dpi) - side) / 2;
        let count = sm(self.tab, TCM_GETITEMCOUNT, 0, 0).0.max(0);
        let mut left = scale(4, self.dpi);
        if count > 0 {
            let mut ir = RECT::default();
            sm(
                self.tab,
                TCM_GETITEMRECT,
                (count - 1) as usize,
                &mut ir as *mut RECT as isize,
            );
            left = ir.right + scale(4, self.dpi);
        }
        RECT {
            left,
            top,
            right: left + side,
            bottom: top + side,
        }
    }

    fn draw_plus(&self, hdc: HDC) {
        unsafe {
            let rc = self.plus_rect();
            if self.plus_hot {
                let b = CreateSolidBrush(self.theme.tab_hot);
                let oldb = SelectObject(hdc, HGDIOBJ(b.0));
                let np = SelectObject(hdc, GetStockObject(NULL_PEN));
                let _ = Ellipse(hdc, rc.left, rc.top, rc.right, rc.bottom);
                SelectObject(hdc, np);
                SelectObject(hdc, oldb);
                let _ = DeleteObject(HGDIOBJ(b.0));
            }
            draw_plus_glyph(hdc, &rc, self.theme.tab_fg, self.dpi);
        }
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
        self.new_document(Some(canon));
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

fn set_editor_font(edit: HWND, face: &str) {
    let mut cf = CHARFORMAT2W::default();
    cf.Base.cbSize = size_of::<CHARFORMAT2W>() as u32;
    cf.Base.dwMask = CFM_FACE | CFM_SIZE | CFM_CHARSET;
    cf.Base.yHeight = 220; // 11pt, in twips
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
    sm(
        edit,
        EM_SETEVENTMASK,
        0,
        (ENM_CHANGE | ENM_SELCHANGE) as isize,
    );
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
unsafe extern "system" fn tab_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    refdata: usize,
) -> LRESULT {
    unsafe {
        match msg {
            // Fill the strip background in the theme color; the control's own
            // erase would otherwise paint the default face color.
            WM_ERASEBKGND => {
                let parent = HWND(refdata as *mut c_void);
                let app = GetWindowLongPtrW(parent, GWLP_USERDATA) as *mut App;
                if app.is_null() {
                    return DefSubclassProc(hwnd, msg, wparam, lparam);
                }
                let mut rc = RECT::default();
                let _ = GetClientRect(hwnd, &mut rc);
                let b = CreateSolidBrush((*app).theme.chrome_bg);
                FillRect(HDC(wparam.0 as *mut c_void), &rc, b);
                let _ = DeleteObject(HGDIOBJ(b.0));
                LRESULT(1)
            }
            m if m == WM_PAINT || m == WM_PRINT || m == WM_PRINTCLIENT => {
                let r = DefSubclassProc(hwnd, msg, wparam, lparam);
                let parent = HWND(refdata as *mut c_void);
                let app = GetWindowLongPtrW(parent, GWLP_USERDATA) as *mut App;
                if !app.is_null() {
                    let a = &*app;
                    // WM_PRINT/WM_PRINTCLIENT supply a DC; WM_PAINT needs GetDC.
                    let hdc = if msg == WM_PAINT {
                        GetDC(Some(hwnd))
                    } else {
                        HDC(wparam.0 as *mut c_void)
                    };
                    let mut rc = RECT::default();
                    let _ = GetClientRect(hwnd, &mut rc);
                    let b = CreateSolidBrush(a.theme.chrome_bg);
                    // Empty strip to the right of the last tab
                    let count = sm(hwnd, TCM_GETITEMCOUNT, 0, 0).0 as i32;
                    if count > 0 {
                        let mut ir = RECT::default();
                        sm(
                            hwnd,
                            TCM_GETITEMRECT,
                            (count - 1) as usize,
                            &mut ir as *mut RECT as isize,
                        );
                        if ir.right < rc.right {
                            let mut gap = rc;
                            gap.left = ir.right;
                            FillRect(hdc, &gap, b);
                        }
                    }
                    // Cover the default 3D edge under the tabs.
                    let mut edge = rc;
                    edge.top = edge.bottom - scale(2, a.dpi).max(1);
                    FillRect(hdc, &edge, b);
                    let _ = DeleteObject(HGDIOBJ(b.0));
                    // Pills + contents over the full cells (covers the control's
                    // frame edge around the selected tab).
                    a.draw_tabs(hdc);
                    a.draw_plus(hdc);
                    if msg == WM_PAINT {
                        let _ = ReleaseDC(Some(hwnd), hdc);
                    }
                }
                r
            }
            WM_MOUSEMOVE => {
                let parent = HWND(refdata as *mut c_void);
                let app = GetWindowLongPtrW(parent, GWLP_USERDATA) as *mut App;
                if !app.is_null() {
                    let a = &mut *app;
                    let (x, y) = (x_lparam(lparam), y_lparam(lparam));
                    let now = pt_in_rect(&a.plus_rect(), x, y);
                    if now != a.plus_hot {
                        a.plus_hot = now;
                        let rc = a.plus_rect();
                        let _ = InvalidateRect(Some(hwnd), Some(&rc), false);
                    }
                    // Track which tab / close button is under the cursor.
                    let ti = a.tab_hit(hwnd, x, y).map(|t| t as i32).unwrap_or(-1);
                    let ci = a.tab_close_hit(hwnd, x, y).map(|t| t as i32).unwrap_or(-1);
                    if ti != a.tab_hot || ci != a.close_hot {
                        let old_t = a.tab_hot;
                        let old_c = a.close_hot;
                        a.tab_hot = ti;
                        a.close_hot = ci;
                        for i in [old_t, ti, old_c, ci] {
                            if i >= 0 {
                                let mut ir = RECT::default();
                                sm(
                                    hwnd,
                                    TCM_GETITEMRECT,
                                    i as usize,
                                    &mut ir as *mut RECT as isize,
                                );
                                let _ = InvalidateRect(Some(hwnd), Some(&ir), false);
                            }
                        }
                    }
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut tme);
                }
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            WM_MOUSELEAVE => {
                let parent = HWND(refdata as *mut c_void);
                let app = GetWindowLongPtrW(parent, GWLP_USERDATA) as *mut App;
                if !app.is_null() {
                    let a = &mut *app;
                    let stale = [a.tab_hot, a.close_hot];
                    a.plus_hot = false;
                    a.tab_hot = -1;
                    a.close_hot = -1;
                    let rc = a.plus_rect();
                    let _ = InvalidateRect(Some(hwnd), Some(&rc), false);
                    for i in stale {
                        if i >= 0 {
                            let mut ir = RECT::default();
                            sm(
                                hwnd,
                                TCM_GETITEMRECT,
                                i as usize,
                                &mut ir as *mut RECT as isize,
                            );
                            let _ = InvalidateRect(Some(hwnd), Some(&ir), false);
                        }
                    }
                }
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            WM_LBUTTONDOWN => {
                // Swallow presses on a tab's close button so the click doesn't
                // also select the tab underneath.
                let parent = HWND(refdata as *mut c_void);
                let app = GetWindowLongPtrW(parent, GWLP_USERDATA) as *mut App;
                if !app.is_null()
                    && (*app)
                        .tab_close_hit(hwnd, x_lparam(lparam), y_lparam(lparam))
                        .is_some()
                {
                    return LRESULT(0);
                }
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            WM_LBUTTONUP => {
                let parent = HWND(refdata as *mut c_void);
                let app = GetWindowLongPtrW(parent, GWLP_USERDATA) as *mut App;
                if !app.is_null() {
                    let a = &*app;
                    let (x, y) = (x_lparam(lparam), y_lparam(lparam));
                    if pt_in_rect(&a.plus_rect(), x, y) {
                        (*app).new_document(None);
                        return LRESULT(0);
                    }
                    if let Some(i) = a.tab_close_hit(hwnd, x, y) {
                        (*app).close_tab(i);
                        return LRESULT(0);
                    }
                }
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            // Empty tab-strip space is a drag handle for the window. The control
            // reports HTTRANSPARENT there (routing clicks to the parent), so test
            // the point against the client rect ourselves: tabs and the plus
            // button stay client hits, the rest becomes caption for dragging.
            WM_NCHITTEST => {
                let r = DefSubclassProc(hwnd, msg, wparam, lparam);
                let parent = HWND(refdata as *mut c_void);
                let app = GetWindowLongPtrW(parent, GWLP_USERDATA) as *mut App;
                if app.is_null() {
                    return r;
                }
                let a = &*app;
                let mut p = POINT {
                    x: x_lparam(lparam),
                    y: y_lparam(lparam),
                };
                let _ = ScreenToClient(hwnd, &mut p);
                let mut rc = RECT::default();
                let _ = GetClientRect(hwnd, &mut rc);
                if !pt_in_rect(&rc, p.x, p.y) {
                    return r;
                }
                if a.tab_hit(hwnd, p.x, p.y).is_some() || pt_in_rect(&a.plus_rect(), p.x, p.y) {
                    LRESULT(HTCLIENT as isize)
                } else {
                    LRESULT(HTCAPTION as isize)
                }
            }
            WM_MBUTTONUP | WM_LBUTTONDBLCLK => {
                let r = DefSubclassProc(hwnd, msg, wparam, lparam);
                let parent = HWND(refdata as *mut c_void);
                let app = GetWindowLongPtrW(parent, GWLP_USERDATA) as *mut App;
                if !app.is_null() {
                    let mut ht = TCHITTESTINFO {
                        pt: POINT {
                            x: x_lparam(lparam),
                            y: y_lparam(lparam),
                        },
                        ..Default::default()
                    };
                    let idx = SendMessageW(
                        hwnd,
                        TCM_HITTEST,
                        Some(WPARAM(0)),
                        Some(LPARAM(&mut ht as *mut TCHITTESTINFO as isize)),
                    )
                    .0;
                    if msg == WM_MBUTTONUP {
                        if idx >= 0 {
                            (*app).close_tab(idx as usize);
                        }
                    } else if idx < 0 {
                        (*app).new_document(None);
                    }
                }
                r
            }
            _ => DefSubclassProc(hwnd, msg, wparam, lparam),
        }
    }
}

// ---- vector glyphs ---------------------------------------------------------
// Chrome icons are drawn with pens, not fonts — crisp at any DPI and immune
// to missing icon fonts.

/// Selects a fresh round-cap pen and returns the previous object. Caller must
/// `glyph_end` to restore and free it.
fn glyph_begin(hdc: HDC, color: COLORREF, dpi: u32) -> (HPEN, HGDIOBJ) {
    unsafe {
        let lb = LOGBRUSH {
            lbStyle: BS_SOLID,
            lbColor: color,
            ..Default::default()
        };
        let pen = ExtCreatePen(
            PEN_STYLE(PS_COSMETIC.0 | PS_ENDCAP_ROUND.0 | PS_JOIN_ROUND.0),
            scale(1, dpi).max(1) as u32 + 1,
            &lb,
            None,
        );
        let old = SelectObject(hdc, HGDIOBJ(pen.0));
        (pen, old)
    }
}

fn glyph_end(hdc: HDC, pen: HPEN, old: HGDIOBJ) {
    unsafe {
        SelectObject(hdc, old);
        let _ = DeleteObject(HGDIOBJ(pen.0));
    }
}

fn line(hdc: HDC, x1: i32, y1: i32, x2: i32, y2: i32) {
    unsafe {
        let _ = MoveToEx(hdc, x1, y1, None);
        let _ = LineTo(hdc, x2, y2);
    }
}

fn draw_plus_glyph(hdc: HDC, rc: &RECT, color: COLORREF, dpi: u32) {
    let (pen, old) = glyph_begin(hdc, color, dpi);
    let cx = (rc.left + rc.right) / 2;
    let cy = (rc.top + rc.bottom) / 2;
    let half = (rc.right - rc.left) * 7 / 22;
    line(hdc, cx - half, cy, cx + half + 1, cy);
    line(hdc, cx, cy - half, cx, cy + half + 1);
    glyph_end(hdc, pen, old);
}

fn draw_close_glyph(hdc: HDC, rc: &RECT, color: COLORREF, dpi: u32) {
    let (pen, old) = glyph_begin(hdc, color, dpi);
    let cx = (rc.left + rc.right) / 2;
    let cy = (rc.top + rc.bottom) / 2;
    let half = (rc.right - rc.left) * 5 / 18;
    line(hdc, cx - half, cy - half, cx + half + 1, cy + half + 1);
    line(hdc, cx + half, cy - half, cx - half - 1, cy + half + 1);
    glyph_end(hdc, pen, old);
}

fn draw_check_glyph(hdc: HDC, rc: &RECT, color: COLORREF, dpi: u32) {
    let (pen, old) = glyph_begin(hdc, color, dpi);
    let w = rc.right - rc.left;
    let h = rc.bottom - rc.top;
    let pts = [
        POINT {
            x: rc.left + w * 7 / 24,
            y: rc.top + h * 12 / 24,
        },
        POINT {
            x: rc.left + w * 11 / 24,
            y: rc.top + h * 16 / 24,
        },
        POINT {
            x: rc.left + w * 18 / 24,
            y: rc.top + h * 7 / 24,
        },
    ];
    unsafe {
        let _ = Polyline(hdc, &pts);
    }
    glyph_end(hdc, pen, old);
}

fn draw_radio_glyph(hdc: HDC, rc: &RECT, color: COLORREF, _dpi: u32) {
    unsafe {
        let r = ((rc.right - rc.left) / 8).max(3);
        let cx = (rc.left + rc.right) / 2;
        let cy = (rc.top + rc.bottom) / 2;
        let b = CreateSolidBrush(color);
        let oldb = SelectObject(hdc, HGDIOBJ(b.0));
        let np = SelectObject(hdc, GetStockObject(NULL_PEN));
        let _ = Ellipse(hdc, cx - r, cy - r, cx + r, cy + r);
        SelectObject(hdc, np);
        SelectObject(hdc, oldb);
        let _ = DeleteObject(HGDIOBJ(b.0));
    }
}

fn draw_chevron_glyph(hdc: HDC, rc: &RECT, color: COLORREF, dpi: u32) {
    let (pen, old) = glyph_begin(hdc, color, dpi);
    let h = rc.bottom - rc.top;
    let cx = rc.right - scale(6, dpi);
    let cy = (rc.top + rc.bottom) / 2;
    let pts = [
        POINT {
            x: cx - 2,
            y: cy - h * 3 / 16,
        },
        POINT { x: cx + 2, y: cy },
        POINT {
            x: cx - 2,
            y: cy + h * 3 / 16,
        },
    ];
    unsafe {
        let _ = Polyline(hdc, &pts);
    }
    glyph_end(hdc, pen, old);
}

// ---- subclasses ------------------------------------------------------------

/// Status bar: flat hairline on top instead of the default 3D edge.
unsafe extern "system" fn status_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    refdata: usize,
) -> LRESULT {
    unsafe {
        if msg == WM_PAINT {
            let r = DefSubclassProc(hwnd, msg, wparam, lparam);
            let parent = HWND(refdata as *mut c_void);
            let app = GetWindowLongPtrW(parent, GWLP_USERDATA) as *mut App;
            if !app.is_null() {
                let hdc = GetDC(Some(hwnd));
                let mut rc = RECT::default();
                let _ = GetClientRect(hwnd, &mut rc);
                rc.bottom = rc.top + 1;
                let b = CreateSolidBrush((*app).theme.menu_sep);
                FillRect(hdc, &rc, b);
                let _ = DeleteObject(HGDIOBJ(b.0));
                let _ = ReleaseDC(Some(hwnd), hdc);
            }
            return r;
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }
}

/// CBT hook active while a TrackPopupMenuEx popup is open: rounds the
/// corners of each menu window ("#32768") it sees being created or activated.
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
                // The popup keeps getting resized while TPM lays it out, so
                // the rounded region has to follow its size — subclass it and
                // re-apply on WM_WINDOWPOSCHANGED once the layout settles.
                let _ = SetWindowSubclass(hwnd, Some(popup_proc), POPUP_SUBCLASS, 0);
            }
        }
        CallNextHookEx(None, code, wparam, lparam)
    }
}

/// Popup menu window: keep a rounded window region matched to the live size
/// (applied after TPM finishes layout, not at create time).
unsafe extern "system" fn popup_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _refdata: usize,
) -> LRESULT {
    unsafe {
        if msg == WM_WINDOWPOSCHANGED {
            let r = DefSubclassProc(hwnd, msg, wparam, lparam);
            // SetWindowRgn re-triggers WM_WINDOWPOSCHANGED — only rebuild the
            // region when the size actually changed.
            let wp = &*(lparam.0 as *const WINDOWPOS);
            if (wp.flags & SWP_NOSIZE) == SWP_NOSIZE {
                return r;
            }
            let (w, h) = (wp.cx, wp.cy);
            if w > 0 && h > 0 {
                let key = hwnd.0 as usize;
                let mut sizes = POPUP_SIZES.lock().unwrap();
                let entry = sizes.iter_mut().find(|(k, _)| *k == key);
                let stale = match &entry {
                    Some((_, s)) => *s != (w, h),
                    None => true,
                };
                if stale {
                    match entry {
                        Some(e) => e.1 = (w, h),
                        None => sizes.push((key, (w, h))),
                    }
                    drop(sizes);
                    let dpi = GetDpiForWindow(hwnd).max(96);
                    let rad = scale(8, dpi);
                    let rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, rad * 2, rad * 2);
                    let _ = SetWindowRgn(hwnd, Some(rgn), true); // system owns rgn
                }
            }
            return r;
        }
        if msg == WM_NCDESTROY {
            POPUP_SIZES
                .lock()
                .unwrap()
                .retain(|(k, _)| *k != hwnd.0 as usize);
            let _ = RemoveWindowSubclass(hwnd, Some(popup_proc), POPUP_SUBCLASS);
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }
}

/// Last-applied region size per popup window, so WM_WINDOWPOSCHANGED echoes
/// of our own SetWindowRgn don't loop.
static POPUP_SIZES: std::sync::Mutex<Vec<(usize, (i32, i32))>> = std::sync::Mutex::new(Vec::new());

/// Editor: Ctrl+mouse wheel zooms.
unsafe extern "system" fn edit_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    refdata: usize,
) -> LRESULT {
    unsafe {
        if msg == WM_MOUSEWHEEL && (wparam.0 as u16 & 0x0008) != 0 {
            let parent = HWND(refdata as *mut c_void);
            let app = GetWindowLongPtrW(parent, GWLP_USERDATA) as *mut App;
            if !app.is_null() {
                let delta = (wparam.0 >> 16) as i16;
                (*app).zoom(if delta > 0 { 1 } else { -1 });
            }
            return LRESULT(0);
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }
}
