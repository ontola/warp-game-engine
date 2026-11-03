#![cfg_attr(windows, windows_subsystem = "windows")]
fn main() {
    app::startup::init("Growing Guns", Some(include_bytes!("../../assets/branding/logo.png"))); growing_guns_app::desktop_main(); }
