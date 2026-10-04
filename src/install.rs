//! Self-installer / updater plumbing.
//!
//! `nimble.exe --install`   — per-user install into %LOCALAPPDATA%\Programs\Nimble,
//!                            Start Menu shortcut, logon update entry, and an
//!                            entry in Settings > Apps (Uninstall registry),
//!                            then launches the installed copy.
//! `nimble.exe --uninstall` — removes everything installed above.
//!
//! The Help menu's "Check for Updates" runs update.ps1 directly, and the
//! installed app runs it silently at launch, at most once a day.
//!
//! The update script is plain PowerShell so the installed footprint stays a
//! single exe. Updates run at logon (registry Run key — Task Scheduler needs
//! admin on some machines) and on the first launch of each day.
//!
//! Install also registers Nimble with Windows as a text editor (ProgID,
//! Capabilities, Open With). Windows only lets the user pick the default
//! app, so Help > Set as Default Text Editor opens that settings page.

use std::ffi::c_void;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::*;
use windows::Win32::System::Registry::*;
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{GUID, Interface, PCWSTR, PWSTR, w};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const UNINSTALL_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Nimble";
const PROGID: &str = "Nimble.Document";
/// Identity shared by the running process, the Start Menu shortcut, and the
/// ProgID. Windows 11's default-app picker only lists desktop apps it can
/// tie to a shortcut this way.
pub const APP_ID: &str = "Rique.Nimble";
/// PKEY_AppUserModel_ID (defined here to avoid another crate feature).
const PKEY_APP_ID: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
    pid: 5,
};
const CAPABILITIES: &str = "Software\\Nimble\\Capabilities";
const APP_KEY: &str = "Software\\Classes\\Applications\\nimble.exe";
/// Types Nimble asks to be the default for. Settings' "Set default" button
/// applies to every one of these, so the list stays to plain-text formats.
const DEFAULT_TYPES: &[&str] = &[".txt", ".log", ".ini", ".cfg", ".conf", ".md"];
/// Types where Nimble is only offered under "Open with".
const OPEN_WITH_TYPES: &[&str] = &[".csv", ".json", ".xml", ".yml", ".yaml", ".toml", ".nfo"];

/// The updater script written next to the installed exe. Swap trick: a running
/// exe can't be overwritten but CAN be renamed, so updates apply immediately.
const UPDATE_PS1: &str = r#"# Nimble updater — checks GitHub releases and swaps nimble.exe in place.
param([switch]$Quiet)
$ErrorActionPreference = 'Stop'
$dir = Split-Path -Parent $MyInvocation.MyCommand.Path
$exe = Join-Path $dir 'nimble.exe'
$verFile = Join-Path $dir 'version.txt'
$log = Join-Path $dir 'update.log'
function Log($m) { "$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss') $m" | Out-File $log -Append -Encoding utf8 }
try {
    $installed = if (Test-Path $verFile) { (Get-Content $verFile -Raw).Trim() } else { '0.0.0' }
    $rel = Invoke-RestMethod -Uri 'https://api.github.com/repos/Riqqqque/Nimble/releases/latest' -Headers @{ 'User-Agent' = 'Nimble-Updater' } -TimeoutSec 15
    $latest = ($rel.tag_name -replace '^v', '')
    $asset = $rel.assets | Where-Object { $_.name -eq 'nimble.exe' } | Select-Object -First 1
    if (-not $asset) { Log "no nimble.exe asset on $($rel.tag_name)"; exit $(if ($Quiet) {0} else {3}) }
    if ([version]$latest -le [version]$installed) { Log "up to date ($installed)"; exit 0 }
    $tmp = Join-Path $dir 'nimble.exe.new'
    Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $tmp -Headers @{ 'User-Agent' = 'Nimble-Updater' } -TimeoutSec 120
    if (Test-Path $exe) { Move-Item $exe (Join-Path $dir 'nimble.exe.old') -Force }
    Move-Item $tmp $exe
    Remove-Item (Join-Path $dir 'nimble.exe.old') -Force -ErrorAction SilentlyContinue
    Set-Content $verFile $latest
    Log "updated $installed -> $latest"
    exit $(if ($Quiet) {0} else {2})
} catch {
    Log "failed: $_"
    exit $(if ($Quiet) {0} else {3})
}
"#;

pub fn install_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").unwrap_or_default();
    PathBuf::from(base).join("Programs").join("Nimble")
}

/// True when this exe is running from the install directory.
pub fn is_installed() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    exe.parent() == Some(install_dir().as_path())
}

pub fn install() {
    match do_install() {
        Ok(msg) => {
            msgbox(
                &format!("Nimble {VERSION} installed.\n\n{msg}"),
                "Nimble",
                MB_ICONINFORMATION,
            );
            let _ = Command::new(install_dir().join("nimble.exe")).spawn();
        }
        Err(e) => msgbox(&format!("Install failed: {e}"), "Nimble", MB_ICONERROR),
    }
}

fn do_install() -> std::io::Result<String> {
    let dir = install_dir();
    std::fs::create_dir_all(&dir)?;
    let dst = dir.join("nimble.exe");
    let exe = std::env::current_exe()?;
    if exe != dst {
        // A previously installed copy may still be running; rename-then-copy
        // survives that case too.
        if dst.exists() {
            let _ = std::fs::rename(&dst, dir.join("nimble.exe.old"));
        }
        std::fs::copy(&exe, &dst)?;
        let _ = std::fs::remove_file(dir.join("nimble.exe.old"));
    }
    std::fs::write(dir.join("version.txt"), VERSION)?;
    std::fs::write(dir.join("update.ps1"), UPDATE_PS1)?;
    register_run_key(&dir)?;
    create_shortcut(&dir)?;
    register_uninstall(&dir)?;
    register_types(&dir)?;
    Ok(format!(
        "Location: {}\nUpdates: automatic (at sign-in and once a day at launch)\n\nTo make Nimble your default text editor: Help > Set as Default Text Editor.",
        dir.display()
    ))
}

/// HKCU\...\Run entry: update.ps1 runs hidden at every logon.
fn register_run_key(dir: &std::path::Path) -> std::io::Result<()> {
    let cmd = format!(
        "powershell.exe -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File \"{}\" -Quiet",
        dir.join("update.ps1").display()
    );
    let vdata = wide(&cmd);
    unsafe {
        let r = RegSetKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(wide(RUN_KEY).as_ptr()),
            w!("Nimble Updater"),
            REG_SZ.0,
            Some(vdata.as_ptr() as *const c_void),
            (vdata.len() * 2) as u32,
        );
        if r.is_err() {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

fn create_shortcut(dir: &std::path::Path) -> std::io::Result<()> {
    let lnk_dir = PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default())
        .join("Microsoft\\Windows\\Start Menu\\Programs");
    let lnk = lnk_dir.join("Nimble.lnk");
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let link: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(win_err)?;
        let exe = dir.join("nimble.exe");
        let exe_w = wide(&exe.to_string_lossy());
        let dir_w = wide(&dir.to_string_lossy());
        let lnk_w = wide(&lnk.to_string_lossy());
        link.SetPath(PCWSTR(exe_w.as_ptr())).map_err(win_err)?;
        link.SetIconLocation(PCWSTR(exe_w.as_ptr()), 0)
            .map_err(win_err)?;
        link.SetWorkingDirectory(PCWSTR(dir_w.as_ptr()))
            .map_err(win_err)?;
        link.SetDescription(w!("Nimble")).map_err(win_err)?;
        let store: IPropertyStore = link.cast().map_err(win_err)?;
        let id = lpwstr_propvariant(APP_ID);
        store.SetValue(&PKEY_APP_ID, &id).map_err(win_err)?;
        store.Commit().map_err(win_err)?;
        let file: IPersistFile = link.cast().map_err(win_err)?;
        file.Save(PCWSTR(lnk_w.as_ptr()), true).map_err(win_err)?;
        CoUninitialize();
    }
    Ok(())
}

/// A VT_LPWSTR PROPVARIANT (what shell properties expect; the crate's
/// From<&str> makes a BSTR). PROPVARIANT's Drop frees the string.
fn lpwstr_propvariant(s: &str) -> PROPVARIANT {
    let w = wide(s);
    unsafe {
        let p = CoTaskMemAlloc(w.len() * 2) as *mut u16;
        let mut pv = PROPVARIANT::default();
        if p.is_null() {
            return pv;
        }
        std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
        let inner = &mut *pv.Anonymous.Anonymous;
        inner.vt = VT_LPWSTR;
        inner.Anonymous.pwszVal = PWSTR(p);
        pv
    }
}

fn register_uninstall(dir: &std::path::Path) -> std::io::Result<()> {
    let exe = dir.join("nimble.exe");
    let exe_s = exe.to_string_lossy();
    let dir_s = dir.to_string_lossy();
    let uninstall = format!("\"{exe_s}\" --uninstall");
    let size_kb: u32 = (std::fs::metadata(&exe).map(|m| m.len()).unwrap_or(0) / 1024) as u32;
    unsafe {
        let vals: [(&str, &str); 6] = [
            ("DisplayName", "Nimble"),
            ("DisplayVersion", VERSION),
            ("Publisher", "Nimble"),
            ("DisplayIcon", &exe_s),
            ("InstallLocation", &dir_s),
            ("UninstallString", &uninstall),
        ];
        for (name, val) in vals {
            let vname = wide(name);
            let vdata = wide(val);
            let r = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(wide(UNINSTALL_KEY).as_ptr()),
                PCWSTR(vname.as_ptr()),
                REG_SZ.0,
                Some(vdata.as_ptr() as *const c_void),
                (vdata.len() * 2) as u32,
            );
            if r.is_err() {
                return Err(std::io::Error::last_os_error());
            }
        }
        for (name, val) in [
            ("NoModify", 1u32),
            ("NoRepair", 1u32),
            ("EstimatedSize", size_kb),
        ] {
            let vname = wide(name);
            let r = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(wide(UNINSTALL_KEY).as_ptr()),
                PCWSTR(vname.as_ptr()),
                REG_DWORD.0,
                Some(&val as *const u32 as *const c_void),
                4,
            );
            if r.is_err() {
                return Err(std::io::Error::last_os_error());
            }
        }
    }
    Ok(())
}

fn set_value(key: &str, name: Option<&str>, data: &[u8], kind: REG_VALUE_TYPE) -> std::io::Result<()> {
    let key_w = wide(key);
    let name_w = name.map(wide);
    let r = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key_w.as_ptr()),
            name_w.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr())),
            kind.0,
            Some(data.as_ptr() as *const c_void),
            data.len() as u32,
        )
    };
    if r.is_err() {
        return Err(std::io::Error::other(format!("registry write failed: {key}")));
    }
    Ok(())
}

fn set_str(key: &str, name: Option<&str>, val: &str) -> std::io::Result<()> {
    let v = wide(val);
    let bytes = unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2) };
    set_value(key, name, bytes, REG_SZ)
}

/// Registers Nimble as a text editor: a ProgID that opens files in it, an
/// Applications entry (for "Open with"), and Capabilities listed under
/// RegisteredApplications so it appears in Settings > Default apps. None
/// of this changes the user's current defaults.
pub fn register_types(dir: &std::path::Path) -> std::io::Result<()> {
    let exe = dir.join("nimble.exe");
    let exe_s = exe.to_string_lossy();
    let open = format!("\"{exe_s}\" \"%1\"");
    let icon = format!("\"{exe_s}\",0");
    let progid = format!("Software\\Classes\\{PROGID}");
    set_str(&progid, None, "Text Document")?;
    set_str(&progid, Some("AppUserModelID"), APP_ID)?;
    set_str(&format!("{progid}\\DefaultIcon"), None, &icon)?;
    set_str(&format!("{progid}\\shell\\open\\command"), None, &open)?;

    set_str(APP_KEY, Some("FriendlyAppName"), "Nimble")?;
    set_str(&format!("{APP_KEY}\\DefaultIcon"), None, &icon)?;
    set_str(&format!("{APP_KEY}\\shell\\open\\command"), None, &open)?;

    set_str(CAPABILITIES, Some("ApplicationName"), "Nimble")?;
    set_str(
        CAPABILITIES,
        Some("ApplicationDescription"),
        "A fast, lightweight native text editor.",
    )?;
    set_str(CAPABILITIES, Some("ApplicationIcon"), &icon)?;
    for ext in DEFAULT_TYPES.iter().chain(OPEN_WITH_TYPES) {
        set_str(&format!("{APP_KEY}\\SupportedTypes"), Some(ext), "")?;
        set_value(
            &format!("Software\\Classes\\{ext}\\OpenWithProgids"),
            Some(PROGID),
            &[],
            REG_NONE,
        )?;
    }
    for ext in DEFAULT_TYPES {
        set_str(&format!("{CAPABILITIES}\\FileAssociations"), Some(ext), PROGID)?;
    }
    set_str("Software\\RegisteredApplications", Some("Nimble"), CAPABILITIES)?;
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    Ok(())
}

fn unregister_types() {
    unsafe {
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(wide(&format!("Software\\Classes\\{PROGID}")).as_ptr()));
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(wide(APP_KEY).as_ptr()));
        let _ = RegDeleteKeyValueW(
            HKEY_CURRENT_USER,
            w!("Software\\RegisteredApplications"),
            w!("Nimble"),
        );
        for ext in DEFAULT_TYPES.iter().chain(OPEN_WITH_TYPES) {
            let key = wide(&format!("Software\\Classes\\{ext}\\OpenWithProgids"));
            let name = wide(PROGID);
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr()), PCWSTR(name.as_ptr()));
        }
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
    }
}

/// Re-registers (cheap, and repairs installs made by older versions) and
/// opens Nimble's page in Settings > Default apps, where the user picks it.
pub fn open_default_apps() -> std::io::Result<()> {
    register_types(&install_dir())?;
    unsafe {
        let r = ShellExecuteW(
            None,
            w!("open"),
            w!("ms-settings:defaultapps?registeredAppUser=Nimble"),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
        if r.0 as usize <= 32 {
            return Err(std::io::Error::other("couldn't open Settings"));
        }
    }
    Ok(())
}

pub fn uninstall() {
    let dir = install_dir();
    let lnk = PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default())
        .join("Microsoft\\Windows\\Start Menu\\Programs\\Nimble.lnk");
    let _ = std::fs::remove_file(&lnk);
    unsafe {
        let _ = RegDeleteKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(wide(RUN_KEY).as_ptr()),
            w!("Nimble Updater"),
        );
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(wide(UNINSTALL_KEY).as_ptr()));
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, w!("Software\\Nimble"));
    }
    unregister_types();
    // Self-delete: a running exe can't be deleted, but it CAN be renamed.
    // Rename to nimble.exe.old up front, then hand off to a detached
    // PowerShell that waits for this PID to exit, deletes the leftover, and
    // removes the install dir only if empty — so a fast reinstall survives.
    let exe = std::env::current_exe().unwrap_or_default();
    let old = dir.join("nimble.exe.old");
    let ours = exe.parent() == Some(dir.as_path());
    if ours {
        let _ = std::fs::remove_file(&old);
        let _ = std::fs::rename(&exe, &old);
    }
    let _ = std::fs::remove_file(dir.join("update.ps1"));
    let _ = std::fs::remove_file(dir.join("version.txt"));
    let _ = std::fs::remove_file(dir.join("update.log"));
    msgbox("Nimble uninstalled.", "Nimble", MB_ICONINFORMATION);
    let pid = std::process::id();
    let target = if old.exists() { old } else { exe };
    let cleanup = if ours {
        format!(
            "Remove-Item '{}' -Force -EA SilentlyContinue;if(-not(Get-ChildItem '{}' -Force -EA SilentlyContinue)){{Remove-Item '{}' -Force -EA SilentlyContinue}}",
            target.display().to_string().replace('\'', "''"),
            dir.display().to_string().replace('\'', "''"),
            dir.display().to_string().replace('\'', "''"),
        )
    } else {
        String::new()
    };
    if !cleanup.is_empty() {
        let _ = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-WindowStyle",
                "Hidden",
                "-Command",
                &format!(
                    "for($i=0;$i -lt 90;$i++){{if(-not(Get-Process -Id {pid} -EA SilentlyContinue)){{break}};Start-Sleep 1}};{cleanup}"
                ),
            ])
            .creation_flags(0x0800_0000)
            .spawn();
    }
}

/// Silent background check at app launch — fire-and-forget so startup stays
/// instant. Only runs when this exe is the installed copy, and at most once
/// a day: as the default editor Nimble starts often, and each check costs a
/// PowerShell process. The sign-in check covers the rest.
pub fn update_check_async() {
    if !is_installed() {
        return;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut last = 0u64;
    let mut size = 8u32;
    unsafe {
        let _ = RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Nimble"),
            w!("LastUpdateCheck"),
            RRF_RT_REG_QWORD,
            None,
            Some(&mut last as *mut u64 as *mut c_void),
            Some(&mut size),
        );
    }
    if now.saturating_sub(last) < 24 * 60 * 60 {
        return;
    }
    let _ = set_value("Software\\Nimble", Some("LastUpdateCheck"), &now.to_le_bytes(), REG_QWORD);
    let ps = install_dir().join("update.ps1");
    let _ = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&ps)
        .arg("-Quiet")
        .creation_flags(0x0800_0000)
        .spawn();
}

/// Runs the updater now and reports. Returns the script exit code:
/// 0 = already current, 2 = updated, 3 = check failed.
pub fn check_updates() -> Option<i32> {
    let ps = install_dir().join("update.ps1");
    if !ps.exists() {
        return None;
    }
    Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&ps)
        .creation_flags(0x0800_0000)
        .status()
        .ok()
        .and_then(|s| s.code())
}

fn msgbox(text: &str, title: &str, flags: MESSAGEBOX_STYLE) {
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(wide(text).as_ptr()),
            PCWSTR(wide(title).as_ptr()),
            flags,
        );
    }
}

fn win_err(e: windows::core::Error) -> std::io::Error {
    std::io::Error::other(e.message())
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
