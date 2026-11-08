//! Live knobs (live::knobs): the `-- @knobs` blocks in the mods, so the
//! person playing, Claude and the GameNight app can turn the match's feel
//! while it runs (mods/base/knobs.luau). Setting one rewrites the file; the
//! mods reload it on the next poll.
use live::knobs::{self, Knob};
use std::path::{Path, PathBuf};

/// Every knob with its file (relative to mods/), in path order.
pub fn all() -> Vec<(String, Knob)> {
    let Some(dir) = crate::scripts::find_dir() else { return Vec::new() };
    let mut files = Vec::new();
    collect(&dir, &mut files);
    files.sort();
    let mut out = Vec::new();
    for path in files {
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        if !text.contains("-- @knobs") { continue; }
        let rel = path.strip_prefix(&dir).unwrap_or(&path).to_string_lossy().replace('\\', "/");
        out.extend(knobs::parse(&text).into_iter().map(|k| (rel.clone(), k)));
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

/// `name = value  -- about  (file)` per knob.
pub fn list() -> String {
    let all = all();
    if all.is_empty() { return knobs::list(&[]); }
    all.iter().map(|(f, k)| format!("{}  ({f})", knobs::list(std::slice::from_ref(k)))).collect::<Vec<_>>().join("\n")
}

/// Set knob `name` in its file; returns the file (relative to mods/).
pub fn set(name: &str, value: &str) -> Result<String, String> {
    let all = all();
    let (rel, _) = all.iter().find(|(_, k)| k.name == name)
        .ok_or_else(|| format!("no knob {name:?}; knobs: {}", all.iter().map(|(_, k)| k.name.as_str()).collect::<Vec<_>>().join(", ")))?;
    let path = crate::scripts::find_dir().ok_or("no mods folder")?.join(rel);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    std::fs::write(&path, knobs::set(&text, name, value)?).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(rel.clone())
}

/// A number knob as a GameNight setting: percent of its value, its range
/// from the help's "A to B".
pub struct Setting { pub key: String, pub label: String, pub value: i64, pub min: i64, pub max: i64 }

pub const SETTING_PREFIX: &str = "knob.";

pub fn settings() -> Vec<Setting> {
    all().into_iter().filter_map(|(_, k)| {
        let v: f64 = k.value.parse().ok()?;
        let (lo, hi) = range(&k.about).unwrap_or((0.0, (v * 3.0).max(1.0)));
        let pct = |x: f64| (x * 100.0).round() as i64;
        let mut label: String = k.name.replace('_', " ");
        if let Some(c) = label.get_mut(0..1) { c.make_ascii_uppercase(); }
        Some(Setting { key: format!("{SETTING_PREFIX}{}", k.name), label: format!("{label} % (live)"), value: pct(v), min: pct(lo), max: pct(hi) })
    }).collect()
}

/// "..., 0.25 to 3" in a knob's help.
pub(crate) fn range(about: &str) -> Option<(f64, f64)> {
    let words: Vec<&str> = about.split(|c: char| c.is_whitespace() || c == ',').filter(|w| !w.is_empty()).collect();
    let k = words.iter().position(|w| *w == "to")?;
    let (a, b) = (words.get(k.checked_sub(1)?)?.parse().ok()?, words.get(k + 1)?.parse().ok()?);
    (a < b).then_some((a, b))
}

/// Apply a GameNight setting `knob.NAME` (percent); false for other keys.
pub fn from_setting(key: &str, value: &serde_json::Value) -> Option<Result<String, String>> {
    let name = key.strip_prefix(SETTING_PREFIX)?;
    let Some(pct) = value.as_f64().filter(|v| v.is_finite()) else { return Some(Err(format!("{key}: not a number"))) };
    let v = (pct / 100.0 * 1000.0).round() / 1000.0;
    Some(set(name, &format!("{v}")))
}

#[cfg(test)]
mod tests {
    #[test]
    fn range_from_help() {
        assert_eq!(super::range("how fast guns fire, 0.25 to 3"), Some((0.25, 3.0)));
        assert_eq!(super::range("no range"), None);
    }

    #[test]
    fn base_mod_knobs_become_settings() {
        let s = super::settings();
        let d = s.iter().find(|s| s.key == "knob.damage").expect("mods/base/knobs.luau has damage");
        assert_eq!((d.value, d.min, d.max, d.label.as_str()), (100, 25, 300, "Damage % (live)"));
    }
}
