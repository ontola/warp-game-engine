#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod bench;
#[cfg(windows)]
mod ui;

#[cfg(windows)]
fn copy_luau(text: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|error| error.to_string())?;
    clipboard.set_text(text).map_err(|error| error.to_string())
}

#[cfg(windows)]
fn main() {
    if let Err(error) = ui::run() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
        let title = windows_sys::core::w!("Warp Engine Generator");
        let message: Vec<u16> = error.encode_utf16().chain(Some(0)).collect();
        unsafe { MessageBoxW(std::ptr::null_mut(), message.as_ptr(), title, MB_ICONERROR | MB_OK) };
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Warp Engine Generator uses the Windows desktop and audio device.");
    std::process::exit(1);
}
