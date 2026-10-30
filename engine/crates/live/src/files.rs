//! The folders a game lets tools read and write (mods, sounds, shaders):
//! paths are relative to a named root, never absolute, never `..`, and
//! must stay inside the root after symlinks are resolved.
use crate::Reload;
use std::path::{Component, Path, PathBuf};

/// A folder tools may see. `extensions` limits writes (and listings) to
/// these file types; empty allows any.
#[derive(Clone, Debug)]
pub struct Root {
    pub name: String,
    pub path: PathBuf,
    pub writable: bool,
    pub extensions: Vec<String>,
    pub reload: Reload,
    /// One line for `describe`: what lives here and what a write does.
    pub about: String,
}

impl Root {
    pub fn new(name: &str, path: impl Into<PathBuf>, reload: Reload, about: &str) -> Self {
        Root { name: name.into(), path: path.into(), writable: true, extensions: Vec::new(), reload, about: about.into() }
    }
    pub fn extensions(mut self, exts: &[&str]) -> Self { self.extensions = exts.iter().map(|e| e.trim_start_matches('.').to_string()).collect(); self }
    pub fn read_only(mut self) -> Self { self.writable = false; self }

    fn allows(&self, path: &Path) -> bool {
        self.extensions.is_empty() || path.extension().and_then(|e| e.to_str()).is_some_and(|e| self.extensions.iter().any(|x| x.eq_ignore_ascii_case(e)))
    }
}

pub const MAX_READ: u64 = 512 * 1024;
pub const MAX_WRITE: usize = 1024 * 1024;
const MAX_LIST: usize = 2000;

/// Split `root/rest` or take `root` + `path`; returns the root and the
/// relative path, checked for shape (no absolute paths, no `..`).
pub fn resolve<'a>(roots: &'a [Root], root: Option<&str>, path: &str) -> Result<(&'a Root, PathBuf), String> {
    let path = path.replace('\\', "/");
    let (root_name, rel) = match root {
        Some(r) => (r.to_string(), path.trim_start_matches("./").to_string()),
        None => match path.split_once('/') { Some((a, b)) => (a.to_string(), b.to_string()), None => (path.clone(), String::new()) },
    };
    let names = || roots.iter().map(|r| r.name.as_str()).collect::<Vec<_>>().join(", ");
    let root = roots.iter().find(|r| r.name == root_name).ok_or_else(|| format!("no folder {root_name:?}; folders: {}", names()))?;
    let rel = PathBuf::from(rel);
    for c in rel.components() {
        match c {
            Component::Normal(_) | Component::CurDir => {}
            _ => return Err(format!("{}: paths are relative to the folder and may not contain '..' or a drive", rel.display())),
        }
    }
    Ok((root, rel))
}

/// The real path, checked to be inside the root after symlinks. For a file
/// that does not exist yet, its nearest existing ancestor is checked.
fn contained(root: &Root, rel: &Path) -> Result<PathBuf, String> {
    let base = root.path.canonicalize().map_err(|e| format!("folder {} ({}): {e}", root.name, root.path.display()))?;
    let full = base.join(rel);
    let mut probe = full.clone();
    while !probe.exists() {
        if !probe.pop() { break; }
    }
    let real = probe.canonicalize().map_err(|e| e.to_string())?;
    if !real.starts_with(&base) { return Err(format!("{}: outside the folder {}", rel.display(), root.name)); }
    Ok(full)
}

pub fn list(roots: &[Root], root: Option<&str>, path: &str) -> Result<serde_json::Value, String> {
    if root.is_none() && path.trim_matches('/').is_empty() {
        return Ok(serde_json::json!({ "folders": roots.iter().map(|r| serde_json::json!({
            "name": r.name, "writable": r.writable, "extensions": r.extensions, "about": r.about, "path": r.path.display().to_string(),
        })).collect::<Vec<_>>() }));
    }
    let (r, rel) = resolve(roots, root, path)?;
    let dir = contained(r, &rel)?;
    let mut out = Vec::new();
    walk(&dir, &dir, 0, r, &mut out);
    out.sort();
    let truncated = out.len() > MAX_LIST;
    out.truncate(MAX_LIST);
    Ok(serde_json::json!({ "folder": r.name, "path": rel.display().to_string(), "files": out, "truncated": truncated }))
}

fn walk(base: &Path, dir: &Path, depth: usize, root: &Root, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') { continue; }
        let Ok(t) = e.file_type() else { continue };
        if t.is_dir() {
            if depth < 6 { walk(base, &p, depth + 1, root, out); }
        } else if root.allows(&p) && out.len() <= MAX_LIST {
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            out.push(format!("{} ({size} bytes)", p.strip_prefix(base).unwrap_or(&p).display().to_string().replace('\\', "/")));
        }
    }
}

pub fn read(roots: &[Root], root: Option<&str>, path: &str) -> Result<String, String> {
    let (r, rel) = resolve(roots, root, path)?;
    let full = contained(r, &rel)?;
    let meta = std::fs::metadata(&full).map_err(|e| format!("{}/{}: {e}", r.name, rel.display()))?;
    if meta.len() > MAX_READ { return Err(format!("{}/{}: {} bytes, more than the {} byte limit", r.name, rel.display(), meta.len(), MAX_READ)); }
    let bytes = std::fs::read(&full).map_err(|e| e.to_string())?;
    String::from_utf8(bytes).map_err(|_| format!("{}/{}: not UTF-8 text", r.name, rel.display()))
}

/// Lines `offset` (from 1) to `offset + limit - 1` of a text, each with its
/// number, so a long file can be read in parts and edited by what it says.
pub fn lines(text: &str, offset: usize, limit: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    let start = offset.max(1);
    let end = (start - 1 + limit).min(all.len());
    let mut out = String::new();
    for (i, line) in all.iter().enumerate().take(end).skip(start - 1) { out += &format!("{:>5}| {line}\n", i + 1); }
    if end < all.len() { out += &format!("({} lines; read on with offset {})\n", all.len(), end + 1); }
    if out.is_empty() { out = format!("(the file has {} lines)", all.len()); }
    out
}

/// Replace the one place `old` occurs in a file with `new`; returns the root
/// and the relative path, like [`write`]. `old` must occur exactly once, so
/// an edit never lands somewhere unintended.
pub fn edit<'a>(roots: &'a [Root], root: Option<&str>, path: &str, old: &str, new: &str) -> Result<(&'a Root, String), String> {
    if old.is_empty() { return Err("edit_file needs the exact text to replace (old)".into()); }
    let text = read(roots, root, path)?;
    let text = if text.contains(old) || !text.contains("\r\n") { text } else { text.replace("\r\n", "\n") };
    match text.matches(old).count() {
        0 => Err(format!("{path}: the text to replace is not in the file (read it again; copy it exactly, without line numbers)")),
        1 => write(roots, root, path, &text.replacen(old, new, 1)),
        n => Err(format!("{path}: the text to replace occurs {n} times; include more lines around it so it occurs once")),
    }
}

/// Lines that contain `needle` (ignoring case) in a root's files, as
/// `path:line: text`, at most `MAX_FOUND`.
pub fn find(roots: &[Root], root: Option<&str>, needle: &str) -> Result<String, String> {
    const MAX_FOUND: usize = 200;
    if needle.trim().is_empty() { return Err("find_text needs some text to look for".into()); }
    let needle = needle.to_lowercase();
    let mut out = Vec::new();
    for r in roots.iter().filter(|r| root.is_none_or(|n| n == r.name)) {
        let mut files = Vec::new();
        let Ok(base) = r.path.canonicalize() else { continue };
        walk(&base, &base, 0, r, &mut files);
        files.sort();
        for listed in files {
            let rel = listed.rsplit_once(" (").map_or(listed.as_str(), |(p, _)| p);
            let Ok(text) = read(roots, Some(&r.name), rel) else { continue };
            for (i, line) in text.lines().enumerate() {
                if line.to_lowercase().contains(&needle) {
                    if out.len() == MAX_FOUND { out.push(format!("(more than {MAX_FOUND} lines; look for something more specific)")); return Ok(out.join("\n")); }
                    out.push(format!("{}/{rel}:{}: {}", r.name, i + 1, line.trim()));
                }
            }
        }
    }
    Ok(if out.is_empty() { "(not found)".into() } else { out.join("\n") })
}

/// Write a file; returns the root (for its reload) and the relative path.
pub fn write<'a>(roots: &'a [Root], root: Option<&str>, path: &str, text: &str) -> Result<(&'a Root, String), String> {
    let (r, rel) = resolve(roots, root, path)?;
    if !r.writable { return Err(format!("folder {} is read-only", r.name)); }
    if rel.as_os_str().is_empty() { return Err("write_file needs a file name".into()); }
    if !r.allows(&rel) { return Err(format!("{}: folder {} takes only .{} files", rel.display(), r.name, r.extensions.join(", ."))); }
    if text.len() > MAX_WRITE { return Err(format!("{} bytes, more than the {MAX_WRITE} byte limit", text.len())); }
    let full = contained(r, &rel)?;
    if let Some(dir) = full.parent() { std::fs::create_dir_all(dir).map_err(|e| e.to_string())?; }
    // Write beside and rename, so a watcher never reads half a file.
    let tmp = full.with_extension(format!("{}.live-tmp", full.extension().and_then(|e| e.to_str()).unwrap_or("")));
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", rel.display()))?;
    std::fs::rename(&tmp, &full).map_err(|e| { let _ = std::fs::remove_file(&tmp); format!("{}: {e}", rel.display()) })?;
    Ok((r, rel.display().to_string().replace('\\', "/")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_stay_inside_their_root() {
        let dir = std::env::temp_dir().join(format!("live-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("mods/base")).unwrap();
        let roots = vec![Root::new("mods", dir.join("mods"), Reload::Game, "").extensions(&["luau"])];
        assert!(write(&roots, None, "mods/../escape.luau", "x").is_err());
        assert!(write(&roots, None, "/etc/passwd", "x").is_err());
        assert!(write(&roots, Some("mods"), "a.txt", "x").is_err(), "extension");
        assert!(write(&roots, None, "sounds/a.luau", "x").is_err(), "unknown root");
        let (_, rel) = write(&roots, None, "mods/live/new.luau", "return {}").unwrap();
        assert_eq!(rel, "live/new.luau");
        assert_eq!(read(&roots, Some("mods"), "live/new.luau").unwrap(), "return {}");
        let l = list(&roots, None, "mods").unwrap();
        assert_eq!(l["files"][0], "live/new.luau (9 bytes)");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/tmp", dir.join("mods/out")).unwrap();
            assert!(write(&roots, None, "mods/out/x.luau", "x").unwrap_err().contains("outside"));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn long_files_are_read_in_parts_found_and_edited_in_one_place() {
        let dir = std::env::temp_dir().join(format!("live-edit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let roots = vec![Root::new("game", &dir, Reload::Game, "").extensions(&["luau"])];
        write(&roots, None, "game/main.luau", "local speed = 1\nlocal hp = 3\nlocal hp_max = 3\n").unwrap();
        assert_eq!(lines(&read(&roots, None, "game/main.luau").unwrap(), 2, 1), "    2| local hp = 3\n(3 lines; read on with offset 3)\n");
        assert_eq!(find(&roots, None, "HP").unwrap(), "game/main.luau:2: local hp = 3\ngame/main.luau:3: local hp_max = 3");
        assert!(edit(&roots, None, "game/main.luau", " = 3", " = 5").unwrap_err().contains("2 times"));
        assert!(edit(&roots, None, "game/main.luau", "local mana", "x").unwrap_err().contains("not in the file"));
        edit(&roots, None, "game/main.luau", "local hp = 3", "local hp = 5").unwrap();
        assert_eq!(read(&roots, None, "game/main.luau").unwrap(), "local speed = 1\nlocal hp = 5\nlocal hp_max = 3\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
