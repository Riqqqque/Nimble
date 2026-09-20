use std::ffi::c_void;
use std::path::{Path, PathBuf};

use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::DataExchange::IsClipboardFormatAvailable;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::Controls::Dialogs::*;
use windows::Win32::UI::Controls::RichEdit::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::{
    DefSubclassProc, DragAcceptFiles, DragFinish, DragQueryFileW, RemoveWindowSubclass,
    SetWindowSubclass, HDROP,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::document::Document;
use crate::encoding::{self, Encoding};

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
const IDM_NEXT_TAB: u32 = 500;
const IDM_PREV_TAB: u32 = 501;
const ID_PLUS: u32 = 600;

const EDIT_ID_BASE: usize = 4096;
const TAB_SUBCLASS: usize = 1;
const EDIT_SUBCLASS: usize = 2;

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

struct App {
    hwnd: HWND,
    tab: HWND,
    plus: HWND,
    status: HWND,
    edit_menu: HMENU,
    docs: Vec<Document>,
    active: Option<usize>,
    word_wrap: bool,
    show_status: bool,
    dpi: u32,
    font: HFONT,
    next_id: usize,
}

pub fn run() {
    unsafe {
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

        let menu = build_menu();
        let accel = create_accel();

        let app = Box::new(App::new());
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
            Some(menu),
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

fn build_menu() -> HMENU {
    unsafe {
        let bar = CreateMenu().unwrap_or_default();

        let file = CreatePopupMenu().unwrap_or_default();
        let _ = AppendMenuW(bar, MF_POPUP, file.0 as usize, w!("&File"));
        let _ = AppendMenuW(file, MF_STRING, IDM_NEW_TAB as usize, w!("&New Tab\tCtrl+T"));
        let _ = AppendMenuW(file, MF_STRING, IDM_NEW_WINDOW as usize, w!("New &Window\tCtrl+N"));
        let _ = AppendMenuW(file, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(file, MF_STRING, IDM_OPEN as usize, w!("&Open...\tCtrl+O"));
        let _ = AppendMenuW(file, MF_STRING, IDM_SAVE as usize, w!("&Save\tCtrl+S"));
        let _ = AppendMenuW(file, MF_STRING, IDM_SAVE_AS as usize, w!("Save &As...\tCtrl+Shift+S"));
        let _ = AppendMenuW(file, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(file, MF_STRING, IDM_CLOSE_TAB as usize, w!("&Close Tab\tCtrl+W"));
        let _ = AppendMenuW(file, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(file, MF_STRING, IDM_EXIT as usize, w!("E&xit"));

        let edit = CreatePopupMenu().unwrap_or_default();
        let _ = AppendMenuW(bar, MF_POPUP, edit.0 as usize, w!("&Edit"));
        let _ = AppendMenuW(edit, MF_STRING, IDM_UNDO as usize, w!("&Undo\tCtrl+Z"));
        let _ = AppendMenuW(edit, MF_STRING, IDM_REDO as usize, w!("&Redo\tCtrl+Y"));
        let _ = AppendMenuW(edit, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(edit, MF_STRING, IDM_CUT as usize, w!("Cu&t\tCtrl+X"));
        let _ = AppendMenuW(edit, MF_STRING, IDM_COPY as usize, w!("&Copy\tCtrl+C"));
        let _ = AppendMenuW(edit, MF_STRING, IDM_PASTE as usize, w!("&Paste\tCtrl+V"));
        let _ = AppendMenuW(edit, MF_STRING, IDM_DELETE as usize, w!("&Delete\tDel"));
        let _ = AppendMenuW(edit, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(edit, MF_STRING, IDM_SELECT_ALL as usize, w!("Select &All\tCtrl+A"));

        let view = CreatePopupMenu().unwrap_or_default();
        let _ = AppendMenuW(bar, MF_POPUP, view.0 as usize, w!("&View"));
        let _ = AppendMenuW(view, MF_STRING, IDM_WORD_WRAP as usize, w!("&Word Wrap"));
        let _ = AppendMenuW(view, MF_STRING, IDM_STATUS_BAR as usize, w!("&Status Bar"));
        let _ = AppendMenuW(view, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(view, MF_STRING, IDM_ZOOM_IN as usize, w!("Zoom &In\tCtrl+Plus"));
        let _ = AppendMenuW(view, MF_STRING, IDM_ZOOM_OUT as usize, w!("Zoom &Out\tCtrl+Minus"));
        let _ = AppendMenuW(view, MF_STRING, IDM_ZOOM_RESET as usize, w!("&Restore Default Zoom\tCtrl+0"));

        let help = CreatePopupMenu().unwrap_or_default();
        let _ = AppendMenuW(bar, MF_POPUP, help.0 as usize, w!("&Help"));
        let _ = AppendMenuW(help, MF_STRING, IDM_ABOUT as usize, w!("&About Nimble"));

        bar
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
    fn new() -> Self {
        Self {
            hwnd: HWND::default(),
            tab: HWND::default(),
            plus: HWND::default(),
            status: HWND::default(),
            edit_menu: HMENU::default(),
            docs: Vec::new(),
            active: None,
            word_wrap: true,
            show_status: true,
            dpi: 96,
            font: HFONT::default(),
            next_id: EDIT_ID_BASE,
        }
    }

    fn on_create(&mut self) {
        unsafe {
        self.dpi = GetDpiForWindow(self.hwnd);
        self.font = ui_font(self.dpi);

        self.tab = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            WC_TABCONTROL,
            PCWSTR::null(),
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
            0, 0, 0, 0,
            Some(self.hwnd),
            None,
            None,
            None,
        )
        .unwrap_or_default();
        sm(self.tab, WM_SETFONT, self.font.0 as usize, 1);
        let _ = SetWindowSubclass(
            self.tab,
            Some(tab_proc),
            TAB_SUBCLASS,
            self.hwnd.0 as usize,
        );

        self.plus = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            w!("+"),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(BS_PUSHBUTTON as u32),
            0, 0, 0, 0,
            Some(self.hwnd),
            Some(HMENU(ID_PLUS as *mut c_void)),
            None,
            None,
        )
        .unwrap_or_default();
        sm(self.plus, WM_SETFONT, self.font.0 as usize, 1);

        self.status = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            STATUSCLASSNAMEW,
            PCWSTR::null(),
            WS_CHILD,
            0, 0, 0, 0,
            Some(self.hwnd),
            None,
            None,
            None,
        )
        .unwrap_or_default();
        sm(self.status, WM_SETFONT, self.font.0 as usize, 1);

        self.edit_menu = GetSubMenu(GetMenu(self.hwnd), 1);

        DragAcceptFiles(self.hwnd, true);

        self.new_document(None);
        if let Some(arg) = std::env::args_os().nth(1) {
            let p = PathBuf::from(arg);
            if p.is_file() {
                self.open_path(&p);
            }
        }
        self.layout();
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
                WM_SETFOCUS => {
                    if let Some(e) = self.active_edit() {
                        let _ = SetFocus(Some(e));
                    }
                    LRESULT(0)
                }
                WM_DPICHANGED => {
                    self.dpi = hiword(wparam.0);
                    let _ = DeleteObject(HGDIOBJ(self.font.0));
                    self.font = ui_font(self.dpi);
                    sm(self.tab, WM_SETFONT, self.font.0 as usize, 1);
                    sm(self.plus, WM_SETFONT, self.font.0 as usize, 1);
                    sm(self.status, WM_SETFONT, self.font.0 as usize, 1);
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
                WS_EX_CLIENTEDGE,
                MSFTEDIT_CLASS,
                PCWSTR::null(),
                style,
                0, 0, 0, 0,
                Some(self.hwnd),
                Some(HMENU(id as *mut c_void)),
                None,
                None,
            ) else {
                return;
            };
            sm(edit, EM_SETEVENTMASK, 0, (ENM_CHANGE | ENM_SELCHANGE) as isize);
            sm(edit, EM_EXLIMITTEXT, 0, 0x4000_0000);
            sm(edit, EM_AUTOURLDETECT, 0, 0);
            sm(
                edit,
                EM_SETTYPOGRAPHYOPTIONS,
                (TO_ADVANCEDTYPOGRAPHY | TO_SIMPLELINEBREAK) as usize,
                (TO_ADVANCEDTYPOGRAPHY | TO_SIMPLELINEBREAK) as isize,
            );
            sm(edit, EM_SETTARGETDEVICE, 0, if self.word_wrap { 0 } else { 1 });
            set_editor_font(edit);
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
                        let msg =
                            wide(&format!("Cannot open \"{}\":\n{}", p.display(), e));
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

    fn tab_label(&self, idx: usize) -> String {
        let d = &self.docs[idx];
        if sm(d.edit, EM_GETMODIFY, 0, 0).0 != 0 {
            format!("* {}", d.name())
        } else {
            d.name()
        }
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
        unsafe { let _ = SetWindowTextW(self.hwnd, PCWSTR(t.as_ptr())); }
    }

    fn layout(&self) {
        unsafe {
            let mut rc = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut rc);
            let (cx, cy) = (rc.right, rc.bottom);
            let tab_h = scale(26, self.dpi);
            let plus_w = scale(30, self.dpi);
            let status_h = if self.show_status {
                scale(22, self.dpi)
            } else {
                0
            };
            let _ = SetWindowPos(
                self.tab,
                None,
                0,
                0,
                (cx - plus_w).max(0),
                tab_h,
                SWP_NOZORDER,
            );
            let _ = SetWindowPos(
                self.plus,
                None,
                cx - plus_w,
                0,
                plus_w,
                tab_h,
                SWP_NOZORDER,
            );
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
            let edit_h = (cy - tab_h - status_h).max(0);
            for d in &self.docs {
                let _ = SetWindowPos(
                    d.edit,
                    None,
                    0,
                    tab_h,
                    cx,
                    edit_h,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
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
        sm(self.status, SB_SETPARTS, edges.len(), edges.as_ptr() as isize);
    }

    fn set_status_part(&self, part: usize, text: &str) {
        let t = wide(text);
        sm(self.status, SB_SETTEXTW, part, t.as_ptr() as isize);
    }

    fn update_status(&self) {
        if !self.show_status {
            return;
        }
        let Some(i) = self.active else { return };
        let d = &self.docs[i];
        let mut lo = 0i32;
        let mut hi = 0i32;
        sm(
            d.edit,
            EM_GETSEL,
            &mut lo as *mut i32 as usize,
            &mut hi as *mut i32 as isize,
        );
        let line = sm(d.edit, EM_EXLINEFROMCHAR, 0, hi as isize).0 as i64;
        let line_start = sm(d.edit, EM_LINEINDEX, line as usize, 0).0 as i64;
        let col = hi as i64 - line_start;
        self.set_status_part(1, &format!("Ln {}, Col {}", line + 1, col + 1));
        self.set_status_part(2, &format!("{}%", d.zoom));
        self.set_status_part(3, "Windows (CRLF)");
        self.set_status_part(4, d.encoding.label());
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
                IDM_NEW_TAB | ID_PLUS => self.new_document(None),
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
                IDM_NEXT_TAB => self.cycle_tab(1),
                IDM_PREV_TAB => self.cycle_tab(-1),
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
        if menu.0 != self.edit_menu.0 {
            return;
        }
        let Some(edit) = self.active_edit() else { return };
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
            let flag = |on: bool| {
                MF_BYCOMMAND
                    | if on {
                        MF_ENABLED
                    } else {
                        MF_GRAYED
                    }
            };
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
            let menu = GetMenu(self.hwnd);
            CheckMenuItem(
                menu,
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
                sm(d.edit, EM_SETTARGETDEVICE, 0, if self.word_wrap { 0 } else { 1 });
            }
        }
    }

    fn toggle_status_bar(&mut self) {
        self.show_status = !self.show_status;
        unsafe {
            let menu = GetMenu(self.hwnd);
            CheckMenuItem(
                menu,
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

fn ui_font(dpi: u32) -> HFONT {
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
        w!("Segoe UI"),
        )
    }
}

fn set_editor_font(edit: HWND) {
    let mut cf = CHARFORMAT2W::default();
    cf.Base.cbSize = size_of::<CHARFORMAT2W>() as u32;
    cf.Base.dwMask = CFM_FACE | CFM_SIZE | CFM_CHARSET;
    cf.Base.yHeight = 220; // 11pt, in twips
    cf.Base.bCharSet = DEFAULT_CHARSET;
    let face = wide("Consolas");
    cf.Base.szFaceName[..8].copy_from_slice(&face[..8]);
    sm(edit, EM_SETCHARFORMAT, SCF_ALL as usize, &cf as *const _ as isize);
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
