//! Reading asset files. Natively these are the filesystem; a browser
//! build has none, so its shell fetches the files first and hands them over
//! with [`preload`], and reads find them by file name.

use std::path::Path;

#[cfg(not(target_arch = "wasm32"))]
pub fn read(path: impl AsRef<Path>) -> std::io::Result<Vec<u8>> { std::fs::read(path) }

/// Decode an image file (format from its contents or extension).
#[cfg(not(target_arch = "wasm32"))]
pub fn open_image(path: impl AsRef<Path>) -> image::ImageResult<image::DynamicImage> { image::open(path) }

#[cfg(target_arch = "wasm32")]
pub use preloaded::*;

#[cfg(target_arch = "wasm32")]
mod preloaded {
    use super::Path;
    use std::{collections::HashMap, sync::Mutex};

    static FILES: Mutex<Option<HashMap<String, Vec<u8>>>> = Mutex::new(None);

    /// Make `bytes` the contents of every path whose file name is `name`.
    pub fn preload(name: &str, bytes: Vec<u8>) {
        FILES.lock().unwrap().get_or_insert_with(HashMap::new).insert(name.to_owned(), bytes);
    }

    pub fn read(path: impl AsRef<Path>) -> std::io::Result<Vec<u8>> {
        let name = path.as_ref().file_name().and_then(|n| n.to_str()).unwrap_or_default();
        FILES.lock().unwrap().as_ref().and_then(|f| f.get(name).cloned())
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, format!("{name} was not preloaded")))
    }

    pub fn open_image(path: impl AsRef<Path>) -> image::ImageResult<image::DynamicImage> {
        let path = path.as_ref();
        let bytes = read(path).map_err(image::ImageError::IoError)?;
        match image::ImageFormat::from_path(path) {
            Ok(format) => image::load_from_memory_with_format(&bytes, format),
            Err(_) => image::load_from_memory(&bytes),
        }
    }
}
