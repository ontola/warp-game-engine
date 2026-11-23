//! Where a game's files come from. Natively these are `std::fs`; in a
//! browser (wasm32) there is no filesystem, so the page fetches the game's
//! files first and hands them over with [`insert`], and reads find them by
//! path. Writes there change only the copy in memory.
//!
//! On Android the host's games come out of the APK the same way: files
//! handed over with [`insert`] are found first, the filesystem after them.
//!
//! Only what loading and hot-reloading a game needs: read, write, a change
//! stamp, and listing a folder.
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// When a file last changed and its length: a changed stamp means "read it again".
pub type Stamp = (SystemTime, u64);

#[cfg(not(target_arch = "wasm32"))]
mod fs {
    use super::*;
    pub fn read(p: &Path) -> io::Result<Vec<u8>> { std::fs::read(p) }
    pub fn write(p: &Path, bytes: &[u8]) -> io::Result<()> { std::fs::write(p, bytes) }
    pub fn stamp(p: &Path) -> Option<Stamp> { std::fs::metadata(p).ok().map(|m| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len())) }
    pub fn is_file(p: &Path) -> bool { p.is_file() }
    pub fn is_dir(p: &Path) -> bool { p.is_dir() }
    pub fn list(dir: &Path) -> io::Result<Vec<PathBuf>> { std::fs::read_dir(dir)?.map(|e| e.map(|e| e.path())).collect() }
}

#[cfg(any(target_arch = "wasm32", target_os = "android"))]
mod mem {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    static FILES: Mutex<BTreeMap<String, Vec<u8>>> = Mutex::new(BTreeMap::new());

    /// `a/./b/../c` as `a/c`, with `/` between parts and no leading `./`.
    pub fn key(p: &Path) -> String {
        let mut parts: Vec<String> = Vec::new();
        for c in p.components() {
            match c {
                std::path::Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
                std::path::Component::ParentDir => { parts.pop(); }
                _ => {}
            }
        }
        parts.join("/")
    }

    fn missing(p: &Path) -> io::Error { io::Error::new(io::ErrorKind::NotFound, format!("{} is not among the fetched game files", p.display())) }

    pub fn insert(p: &Path, bytes: Vec<u8>) { FILES.lock().unwrap().insert(key(p), bytes); }
    pub fn read(p: &Path) -> io::Result<Vec<u8>> { FILES.lock().unwrap().get(&key(p)).cloned().ok_or_else(|| missing(p)) }
    pub fn write(p: &Path, bytes: &[u8]) -> io::Result<()> { insert(p, bytes.to_vec()); Ok(()) }
    pub fn stamp(p: &Path) -> Option<Stamp> { FILES.lock().unwrap().get(&key(p)).map(|b| (SystemTime::UNIX_EPOCH, b.len() as u64)) }
    pub fn is_file(p: &Path) -> bool { FILES.lock().unwrap().contains_key(&key(p)) }
    pub fn is_dir(p: &Path) -> bool {
        let k = key(p);
        if k.is_empty() { return true; }
        let prefix = format!("{k}/");
        FILES.lock().unwrap().range(prefix.clone()..).next().is_some_and(|(f, _)| f.starts_with(&prefix))
    }
    pub fn list(dir: &Path) -> io::Result<Vec<PathBuf>> {
        let k = key(dir);
        let prefix = if k.is_empty() { String::new() } else { format!("{k}/") };
        let files = FILES.lock().unwrap();
        let mut out: Vec<PathBuf> = Vec::new();
        for f in files.keys().filter(|f| f.starts_with(&prefix)) {
            let first = f[prefix.len()..].split('/').next().unwrap_or_default();
            let child = dir.join(first);
            if out.last() != Some(&child) { out.push(child); }
        }
        if out.is_empty() && !k.is_empty() { return Err(missing(dir)); }
        Ok(out)
    }
}

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
use fs as imp;
#[cfg(target_arch = "wasm32")]
use mem as imp;

/// Android: the files handed over first, then the filesystem.
#[cfg(target_os = "android")]
mod imp {
    use super::*;
    pub fn insert(p: &Path, bytes: Vec<u8>) { mem::insert(p, bytes) }
    pub fn read(p: &Path) -> io::Result<Vec<u8>> { mem::read(p).or_else(|_| fs::read(p)) }
    pub fn write(p: &Path, bytes: &[u8]) -> io::Result<()> { if mem::is_file(p) { mem::write(p, bytes) } else { fs::write(p, bytes) } }
    pub fn stamp(p: &Path) -> Option<Stamp> { mem::stamp(p).or_else(|| fs::stamp(p)) }
    pub fn is_file(p: &Path) -> bool { mem::is_file(p) || fs::is_file(p) }
    pub fn is_dir(p: &Path) -> bool { (!mem::key(p).is_empty() && mem::is_dir(p)) || fs::is_dir(p) }
    pub fn list(dir: &Path) -> io::Result<Vec<PathBuf>> { mem::list(dir).or_else(|_| fs::list(dir)) }
}

#[cfg(any(target_arch = "wasm32", target_os = "android"))]
/// Browser and Android: make `bytes` the contents of `path` (as the game will name it).
pub fn insert(path: impl AsRef<Path>, bytes: Vec<u8>) { imp::insert(path.as_ref(), bytes) }

pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> { imp::read(path.as_ref()) }
pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    String::from_utf8(read(path)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
pub fn write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> io::Result<()> { imp::write(path.as_ref(), bytes.as_ref()) }
pub fn stamp(path: impl AsRef<Path>) -> Option<Stamp> { imp::stamp(path.as_ref()) }
pub fn is_file(path: impl AsRef<Path>) -> bool { imp::is_file(path.as_ref()) }
pub fn is_dir(path: impl AsRef<Path>) -> bool { imp::is_dir(path.as_ref()) }
pub fn exists(path: impl AsRef<Path>) -> bool { is_file(&path) || is_dir(&path) }
/// The entries of `dir` (files and folders), in no particular order.
pub fn list(dir: impl AsRef<Path>) -> io::Result<Vec<PathBuf>> { imp::list(dir.as_ref()) }
