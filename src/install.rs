//! Self-installer / updater plumbing.
//!
//! `nimble.exe --install`   — per-user install into %LOCALAPPDATA%\Programs\Nimble,
//!                            Start Menu shortcut, logon update entry, and an
//!                            entry in Settings > Apps (Uninstall registry).
//! `nimble.exe --uninstall` — removes everything installed above.
//! `nimble.exe --update`    — runs the update script now (the Help menu uses
//!                            this too). The app also runs it silently at
//!                            launch when installed.
//!
//! The update script is plain PowerShell so the installed footprint stays a
//! single exe. Updates run at logon (registry Run key — Task Scheduler needs
//! admin on some machines) and again each time Nimble starts.

use std::ffi::c_void;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use windows::core::{w, Interface, PCWSTR};
use windows::Win32::System::Com::*;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const UNINSTALL_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Nimble";

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
        Ok(msg) => msgbox(&format!("Nimble {VERSION} installed.\n\n{msg}"), "Nimble", MB_ICONINFORMATION),
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
    Ok(format!(
        "Location: {}\nUpdates: automatic (at sign-in and every launch)",
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
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
            .map_err(win_err)?;
        let exe = dir.join("nimble.exe");
        let exe_w = wide(&exe.to_string_lossy());
        let dir_w = wide(&dir.to_string_lossy());
        let lnk_w = wide(&lnk.to_string_lossy());
        link.SetPath(PCWSTR(exe_w.as_ptr())).map_err(win_err)?;
        link.SetIconLocation(PCWSTR(exe_w.as_ptr()), 0).map_err(win_err)?;
        link.SetWorkingDirectory(PCWSTR(dir_w.as_ptr())).map_err(win_err)?;
        link.SetDescription(w!("Nimble")).map_err(win_err)?;
        let file: IPersistFile = link.cast().map_err(win_err)?;
        file.Save(PCWSTR(lnk_w.as_ptr()), true).map_err(win_err)?;
        CoUninitialize();
    }
    Ok(())
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
        for (name, val) in [("NoModify", 1u32), ("NoRepair", 1u32), ("EstimatedSize", size_kb)] {
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
    // Self-delete: the running exe can't be removed, so hand off to a detached
    // cmd that waits a beat, then deletes the binary and the directory.
    let _ = std::fs::remove_file(dir.join("update.ps1"));
    let _ = std::fs::remove_file(dir.join("version.txt"));
    let _ = std::fs::remove_file(dir.join("update.log"));
    let _ = std::fs::remove_file(dir.join("nimble.exe.old"));
    let exe = std::env::current_exe().unwrap_or_default();
    let _ = Command::new("cmd.exe")
        .args([
            "/c",
            &format!(
                "ping 127.0.0.1 -n 3 > nul & del /q \"{}\" & rmdir \"{}\"",
                exe.display(),
                dir.display()
            ),
        ])
        .creation_flags(0x0800_0000)
        .spawn();
    msgbox("Nimble uninstalled.", "Nimble", MB_ICONINFORMATION);
}

/// Silent background check at app launch — fire-and-forget so startup stays
/// instant. Only runs when this exe is the installed copy.
pub fn update_check_async() {
    if !is_installed() {
        return;
    }
    let ps = install_dir().join("update.ps1");
    let _ = Command::new("powershell.exe")
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-ExecutionPolicy", "Bypass", "-File"])
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
        MessageBoxW(None, PCWSTR(wide(text).as_ptr()), PCWSTR(wide(title).as_ptr()), flags);
    }
}

fn win_err(e: windows::core::Error) -> std::io::Error {
    std::io::Error::other(e.message())
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
