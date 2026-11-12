//! Knobs: the values a game marks as tweakable, so a person or an AI can
//! list and change them while playing, and the change lands in the file,
//! ready to commit. A knob block in a Luau file (a Warp game's main.luau,
//! a Growing Guns mod):
//!
//! ```luau
//! -- @knobs
//! local KNOBS = {
//! 	bloom = 0.55, -- glow strength, 0 to 2
//! 	stars = 320, -- how many stars below the arena
//! }
//! -- @end
//! ```
//!
//! Each `name = value,` line between the markers is one knob; the comment
//! after it is its help. Setting a knob rewrites that value in the file,
//! which hot-reloads the module (its `reload()` hook re-applies the look).

//! A game keeps its knobs in its mods folder (one or more files); the
//! folder functions below list and set them across every file, so the live
//! tools, a pause menu and GameNight settings all go through one path. See
//! engine/docs/luau-or-rust.md, "Live reload".

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Knob {
    pub name: String,
    pub value: String,
    pub about: String,
    line: usize,
    /// Byte range of the value inside its line.
    span: (usize, usize),
}

/// The knobs in a source file, in order.
pub fn parse(source: &str) -> Vec<Knob> {
    let mut out = Vec::new();
    let mut inside = false;
    for (i, line) in source.lines().enumerate() {
        let t = line.trim();
        if t.starts_with("-- @knobs") { inside = true; continue; }
        if t.starts_with("-- @end") { inside = false; continue; }
        if !inside { continue; }
        let Some(eq) = line.find('=') else { continue };
        let name = line[..eq].trim();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') { continue; }
        let rest = &line[eq + 1..];
        let (code, about) = match rest.find("--") { Some(c) => (&rest[..c], rest[c + 2..].trim()), None => (rest, "") };
        let value = code.trim().trim_end_matches(',').trim();
        if value.is_empty() { continue; }
        let start = eq + 1 + rest.find(value).unwrap_or(0);
        out.push(Knob { name: name.into(), value: value.into(), about: about.into(), line: i, span: (start, start + value.len()) });
    }
    out
}

/// The source with one knob's value replaced. `value` must be one Luau
/// literal on one line (a number, true/false, a "string" or a {table}).
pub fn set(source: &str, name: &str, value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.contains('\n') || value.contains("--") { return Err(format!("knob {name}: give one value on one line, such as 0.5")); }
    let knobs = parse(source);
    let k = knobs.iter().find(|k| k.name == name)
        .ok_or_else(|| format!("no knob {name:?}; knobs: {}", knobs.iter().map(|k| k.name.as_str()).collect::<Vec<_>>().join(", ")))?;
    let mut out = String::with_capacity(source.len() + value.len());
    for (i, line) in source.split_inclusive('\n').enumerate() {
        if i == k.line { out.push_str(&line[..k.span.0]); out.push_str(value); out.push_str(&line[k.span.1..]); } else { out.push_str(line); }
    }
    Ok(out)
}

/// One line per knob: `name = value  -- about`.
pub fn list(knobs: &[Knob]) -> String {
    if knobs.is_empty() { return "no knobs: mark tweakable values in a Luau file between `-- @knobs` and `-- @end` lines".into(); }
    knobs.iter().map(|k| if k.about.is_empty() { format!("{} = {}", k.name, k.value) } else { format!("{} = {}  -- {}", k.name, k.value, k.about) }).collect::<Vec<_>>().join("\n")
}

/// The value as a number, when it is one.
impl Knob {
    pub fn number(&self) -> Option<f64> { self.value.parse().ok().filter(|v: &f64| v.is_finite()) }
}

/// Every knob in the `*.luau` files under `dir`, with its file (relative to
/// `dir`, `/` separated), in path order.
pub fn in_folder(dir: &Path) -> Vec<(String, Knob)> {
    let mut files = Vec::new();
    collect(dir, &mut files);
    files.sort();
    let mut out = Vec::new();
    for path in files {
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        if !text.contains("-- @knobs") { continue; }
        let rel = path.strip_prefix(dir).unwrap_or(&path).to_string_lossy().replace('\\', "/");
        out.extend(parse(&text).into_iter().map(|k| (rel.clone(), k)));
    }
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() { collect(&p, out); } else if p.extension().is_some_and(|x| x == "luau") { out.push(p); }
    }
}

/// Set knob `name` in whichever file under `dir` holds it; returns that
/// file (relative to `dir`). The game's watcher reloads it like any edit.
pub fn set_in_folder(dir: &Path, name: &str, value: &str) -> Result<String, String> {
    let all = in_folder(dir);
    let (rel, _) = all.iter().find(|(_, k)| k.name == name)
        .ok_or_else(|| format!("no knob {name:?}; knobs: {}", all.iter().map(|(_, k)| k.name.as_str()).collect::<Vec<_>>().join(", ")))?;
    let path = dir.join(rel);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    std::fs::write(&path, set(&text, name, value)?).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(rel.clone())
}

/// `name = value  -- about  (file)` per knob of a folder.
pub fn list_folder(all: &[(String, Knob)]) -> String {
    if all.is_empty() { return list(&[]); }
    all.iter().map(|(f, k)| format!("{}  ({f})", list(std::slice::from_ref(k)))).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "local A = 1\n-- @knobs\nlocal KNOBS = {\n\tbloom = 0.55, -- glow strength\n\tname = \"x\",\n}\n-- @end\nlocal B = { c = 2 }\n";

    #[test]
    fn knobs_list_and_set_in_place() {
        let k = parse(SRC);
        assert_eq!(k.iter().map(|k| (k.name.as_str(), k.value.as_str(), k.about.as_str())).collect::<Vec<_>>(), vec![("bloom", "0.55", "glow strength"), ("name", "\"x\"", "")]);
        let s = set(SRC, "bloom", "0.3").unwrap();
        assert!(s.contains("\tbloom = 0.3, -- glow strength\n") && s.contains("local B = { c = 2 }"), "{s}");
        assert_eq!(s.len(), SRC.len() - 1);
        assert!(set(SRC, "c", "3").unwrap_err().contains("no knob"));
        assert!(set(SRC, "bloom", "1 -- x").is_err());
    }

    #[test]
    fn a_folder_lists_and_sets_knobs_in_their_own_files() {
        let dir = std::env::temp_dir().join(format!("script-knobs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("base")).unwrap();
        std::fs::write(dir.join("base/a.luau"), SRC).unwrap();
        std::fs::write(dir.join("b.luau"), "-- @knobs\nlocal K = {\n\tspeed = 2, -- m/s\n}\n-- @end\n").unwrap();
        let all = in_folder(&dir);
        assert_eq!(all.iter().map(|(f, k)| (f.as_str(), k.name.as_str())).collect::<Vec<_>>(), vec![("b.luau", "speed"), ("base/a.luau", "bloom"), ("base/a.luau", "name")]);
        assert_eq!(set_in_folder(&dir, "bloom", "0.9").unwrap(), "base/a.luau");
        assert_eq!(in_folder(&dir)[1].1.number(), Some(0.9));
        assert!(set_in_folder(&dir, "nope", "1").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
