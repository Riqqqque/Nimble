#![windows_subsystem = "windows"]

mod app;
mod document;
mod encoding;
mod install;
mod theme;

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--install") => install::install(),
        Some("--uninstall") => install::uninstall(),
        _ => app::run(),
    }
}
