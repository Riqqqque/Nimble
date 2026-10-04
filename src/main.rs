#![windows_subsystem = "windows"]

mod app;
mod data;
mod document;
mod draw;
mod encoding;
mod fileio;
mod findbar;
mod install;
mod json;
mod settings;
mod syntax;
mod theme;
mod yaml;

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--install") => install::install(),
        Some("--uninstall") => install::uninstall(),
        _ => app::run(),
    }
}
