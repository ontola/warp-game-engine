//! Explicit developer screenshots only. No background capture or clipboard work.
use image::ImageEncoder;
use serde_json::Value;
use std::{fs::{self, OpenOptions}, io::Write, path::PathBuf, sync::atomic::{AtomicU64, Ordering}, time::{SystemTime, UNIX_EPOCH}};

static SERIAL: AtomicU64 = AtomicU64::new(0);

pub fn directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../build/screenshots")
}

/// Called on the window event-loop thread, after composing the captured frame.
/// A clipboard/metadata failure never discards the successfully saved PNG.
pub fn save(window: &winit::window::Window, width: u32, height: u32, rgba: &[u8], mut metadata: Value) -> Result<String, String> {
    let expected = u64::from(width) * u64::from(height) * 4;
    if width == 0 || height == 0 || expected != rgba.len() as u64 {
        return Err("Invalid screenshot dimensions or RGBA data".into());
    }
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e|e.to_string())?.as_millis();
    let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
    let name = format!("sideways-{timestamp}-{}-{serial}", std::process::id());
    let directory = directory();
    fs::create_dir_all(&directory).map_err(|e|format!("Screenshot folder: {e}"))?;
    let path = directory.join(format!("{name}.png"));
    let mut file = OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e|format!("Screenshot file: {e}"))?;
    image::codecs::png::PngEncoder::new(&mut file).write_image(rgba, width, height, image::ExtendedColorType::Rgba8).map_err(|e|format!("Screenshot PNG: {e}"))?;
    file.flush().map_err(|e|format!("Screenshot PNG flush: {e}"))?;
    let clipboard = copy_image(window, width, height, rgba);
    metadata["capture"] = serde_json::json!({
        "timestamp_unix_ms": timestamp as u64, "png": path, "size": [width,height],
        "clipboard_image": clipboard.is_ok(), "clipboard_error": clipboard.as_ref().err(),
        "pixels": "same-frame final presentation composite including HUD, sRGB RGBA8",
        "extra_simulation_steps": 0, "extra_scene_renders": 0,
        "executable": std::env::current_exe().ok(),
    });
    let metadata_result = (|| -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(&metadata).map_err(|e|e.to_string())?;
        let mut file = OpenOptions::new().write(true).create_new(true).open(directory.join(format!("{name}.json"))).map_err(|e|e.to_string())?;
        file.write_all(&bytes).and_then(|_|file.flush()).map_err(|e|e.to_string())
    })();
    println!("screenshot: {}", path.display());
    if let Err(e) = &clipboard { eprintln!("screenshot clipboard: {e}"); }
    if let Err(e) = &metadata_result { eprintln!("screenshot metadata: {e}"); }
    Ok(format!("{name}.png | {}{}", if clipboard.is_ok() { "Copied: Ctrl+V" } else { "Saved; clipboard unavailable" }, if metadata_result.is_err() { " | metadata failed" } else { "" }))
}

/// Windows CF_DIB: a BITMAPINFOHEADER followed by bottom-up opaque BGRA rows.
/// A real HWND must own the clipboard; OpenClipboard(NULL) + EmptyClipboard
/// would leave no owner and can make SetClipboardData fail.
#[cfg(any(windows, test))]
fn dib(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let length = u64::from(width) * u64::from(height) * 4;
    if width == 0 || height == 0 || width > i32::MAX as u32 || height > i32::MAX as u32 || length > u32::MAX as u64 || length != rgba.len() as u64 {
        return Err("Invalid clipboard image dimensions".into());
    }
    let mut bytes = vec![0u8; 40 + rgba.len()];
    bytes[0..4].copy_from_slice(&40u32.to_le_bytes());
    bytes[4..8].copy_from_slice(&(width as i32).to_le_bytes());
    bytes[8..12].copy_from_slice(&(height as i32).to_le_bytes());
    bytes[12..14].copy_from_slice(&1u16.to_le_bytes());
    bytes[14..16].copy_from_slice(&32u16.to_le_bytes());
    bytes[20..24].copy_from_slice(&(length as u32).to_le_bytes());
    let row = width as usize * 4;
    for y in 0..height as usize {
        for (src, dst) in rgba[y*row..(y+1)*row].chunks_exact(4).zip(bytes[40+(height as usize-1-y)*row..40+(height as usize-y)*row].chunks_exact_mut(4)) {
            dst.copy_from_slice(&[src[2],src[1],src[0],255]);
        }
    }
    Ok(bytes)
}

#[cfg(windows)]
fn copy_image(window: &winit::window::Window, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    use std::ffi::c_void;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    #[link(name="user32")]
    unsafe extern "system" {
        fn OpenClipboard(owner: *mut c_void) -> i32;
        fn EmptyClipboard() -> i32;
        fn SetClipboardData(format: u32, memory: *mut c_void) -> *mut c_void;
        fn CloseClipboard() -> i32;
    }
    #[link(name="kernel32")]
    unsafe extern "system" {
        fn GlobalAlloc(flags: u32, size: usize) -> *mut c_void;
        fn GlobalLock(memory: *mut c_void) -> *mut c_void;
        fn GlobalUnlock(memory: *mut c_void) -> i32;
        fn GlobalFree(memory: *mut c_void) -> *mut c_void;
    }
    let RawWindowHandle::Win32(handle) = window.window_handle().map_err(|e|e.to_string())?.as_raw() else { return Err("No Win32 window handle".into()); };
    let bytes = dib(width, height, rgba)?;
    // Each allocation stays ours until SetClipboardData succeeds. Every failure
    // frees it; the OS owns it on success. The clipboard is always closed.
    unsafe {
        let allocation = GlobalAlloc(0x0002, bytes.len()); // GMEM_MOVEABLE
        if allocation.is_null() { return Err(std::io::Error::last_os_error().to_string()); }
        let target = GlobalLock(allocation);
        if target.is_null() { let error=std::io::Error::last_os_error();GlobalFree(allocation);return Err(error.to_string()); }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), target.cast::<u8>(), bytes.len());
        GlobalUnlock(allocation);
        if OpenClipboard(handle.hwnd.get() as *mut c_void)==0 { let error=std::io::Error::last_os_error();GlobalFree(allocation);return Err(error.to_string()); }
        let result = if EmptyClipboard()==0 || SetClipboardData(8, allocation).is_null() {
            let error=std::io::Error::last_os_error();GlobalFree(allocation);Err(error.to_string())
        } else { Ok(()) };
        CloseClipboard();
        result
    }
}

#[cfg(not(windows))]
fn copy_image(_: &winit::window::Window, _: u32, _: u32, _: &[u8]) -> Result<(), String> {
    Err("Image clipboard is currently supported on Windows only".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clipboard_dib_preserves_colors_and_flips_rows() {
        let bytes=dib(2,2,&[255,0,0,255, 0,255,0,255, 0,0,255,255, 12,34,56,0]).unwrap();
        assert_eq!(u32::from_le_bytes(bytes[0..4].try_into().unwrap()),40);
        assert_eq!(u16::from_le_bytes(bytes[14..16].try_into().unwrap()),32);
        assert_eq!(&bytes[40..],&[255,0,0,255, 56,34,12,255, 0,0,255,255, 0,255,0,255]);
    }
    #[test]
    fn clipboard_rejects_incomplete_pixels() {
        assert!(dib(2,2,&[0;15]).is_err());
        assert!(dib(0,1,&[]).is_err());
        assert!(dib(u32::MAX,1,&[]).is_err());
    }
}
