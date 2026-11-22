//! Recorded sounds a game names in `samples.lock` (TOML, text only, so it
//! lives in git and in Atomic): each `[file.ID]` says where the recording
//! comes from (a dataset row or a URL) and its sha256; each `[[sample]]`
//! cuts a sound out of a file (`from`, `to`, `db`, `loop`). The files
//! themselves never go in git: `warp run` (and `warp samples DIR`) fetches
//! missing ones into a cache and checks the hash; a packaged game carries
//! them in its `samples/` folder (`warp samples DIR --bundle`).
//!
//! A sample named like a synth patch (`groan`, or variants `groan.1`,
//! `groan.2` ...) replaces that patch once its file is there, so a game
//! sounds the same everywhere, only better once the recordings arrived.
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const LOCK: &str = "samples.lock";

#[derive(Deserialize, Default, Debug)]
#[serde(deny_unknown_fields)]
pub struct Lock {
    #[serde(default)]
    pub source: BTreeMap<String, Source>,
    #[serde(default)]
    pub file: BTreeMap<String, File>,
    #[serde(default)]
    pub sample: Vec<Sample>,
}

/// A dataset served by the Hugging Face datasets server: `rows` is its rows
/// endpoint (dataset, config and split in the query); a row's `column` holds
/// the audio, `id_field` the original's id (checked against the file's `id`).
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub rows: String,
    #[serde(default = "audio_column")]
    pub column: String,
    #[serde(default)]
    pub id_field: Option<String>,
    /// The dataset revision the hashes were taken from (informational: the
    /// hash is what decides).
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(default)]
    pub about: Option<String>,
}

fn audio_column() -> String { "audio".into() }

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct File {
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub row: Option<u64>,
    #[serde(default)]
    pub id: Option<u64>,
    /// A direct download instead of a dataset row.
    #[serde(default)]
    pub url: Option<String>,
    pub sha256: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    pub license: String,
    /// Where a person finds the original.
    #[serde(default)]
    pub link: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Sample {
    pub name: String,
    pub file: String,
    #[serde(default)]
    pub from: f32,
    #[serde(default)]
    pub to: Option<f32>,
    /// Gain in dB (to bring recordings to one loudness).
    #[serde(default)]
    pub db: f32,
    /// Plays until stopped; the end crossfades into the start.
    #[serde(default, rename = "loop")]
    pub looping: bool,
}

pub fn read(game: &Path) -> Result<Option<Lock>, String> {
    let path = game.join(LOCK);
    let Ok(text) = vfs::read_to_string(&path) else { return Ok(None) };
    let lock: Lock = toml::from_str(&text).map_err(|e| format!("{LOCK}: {e}"))?;
    for s in &lock.sample {
        if !lock.file.contains_key(&s.file) { return Err(format!("{LOCK}: sample {:?} names file {:?}, which is not listed", s.name, s.file)); }
    }
    for (id, f) in &lock.file {
        if f.url.is_none() && f.source.as_ref().is_none_or(|s| !lock.source.contains_key(s)) { return Err(format!("{LOCK}: file {id:?} needs `url`, or a `source` that is listed")); }
        if f.url.is_none() && f.row.is_none() { return Err(format!("{LOCK}: file {id:?} needs the dataset `row`")); }
        if f.sha256.len() != 64 { return Err(format!("{LOCK}: file {id:?}: sha256 is 64 hex digits")); }
    }
    Ok(Some(lock))
}

/// The user's sample cache (WARP_SAMPLES_CACHE overrides it).
pub fn cache_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("WARP_SAMPLES_CACHE") { return Some(PathBuf::from(d)); }
    directories::ProjectDirs::from("io", "Ontola", "Warp").map(|p| p.cache_dir().join("samples"))
}

/// Where a file is: the game's own `samples/` folder, else the cache.
pub fn find(game: &Path, sha: &str) -> Option<PathBuf> {
    let own = game.join("samples").join(sha);
    if own.is_file() { return Some(own); }
    cache_dir().map(|d| d.join(sha)).filter(|p| p.is_file())
}

fn sha256(b: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

/// Download with the system's curl (on Windows 10+, macOS and Linux; it
/// follows the system's proxy and certificates), a few tries.
fn get(url: &str) -> Result<Vec<u8>, String> {
    let mut last = String::new();
    for k in 0..5 {
        if k > 0 { std::thread::sleep(std::time::Duration::from_secs(2 * k)); }
        match std::process::Command::new("curl").args(["-sSfL", "--max-time", "180", url]).output() {
            Ok(o) if o.status.success() => return Ok(o.stdout),
            Ok(o) => last = String::from_utf8_lossy(&o.stderr).trim().to_string(),
            Err(e) => return Err(format!("curl: {e} (warp fetches samples with curl)")),
        }
    }
    Err(last)
}

fn fetch_one(lock: &Lock, f: &File) -> Result<Vec<u8>, String> {
    if let Some(url) = &f.url { return get(url); }
    let src = &lock.source[f.source.as_deref().unwrap_or_default()];
    let row = f.row.unwrap_or_default();
    let url = format!("{}{}offset={row}&length=1", src.rows, if src.rows.contains('?') { "&" } else { "?" });
    let json: serde_json::Value = serde_json::from_slice(&get(&url)?).map_err(|e| format!("{url}: {e}"))?;
    let r = &json["rows"][0]["row"];
    if let (Some(field), Some(id)) = (&src.id_field, f.id) {
        if r[field.as_str()].as_u64() != Some(id) { return Err(format!("row {row} is no longer {field} {id} (the dataset changed)")); }
    }
    let a = &r[src.column.as_str()];
    let audio = a[0]["src"].as_str().or_else(|| a["src"].as_str()).ok_or_else(|| format!("row {row} has no {} file", src.column))?;
    get(audio)
}

/// Fetch every listed file that is not here yet into `into` (the cache, or
/// the game's `samples/` for a bundle). Returns one line per file fetched.
pub fn fetch(game: &Path, into: Option<&Path>, say: &mut dyn FnMut(String)) -> Result<usize, String> {
    let Some(lock) = read(game)? else { return Ok(0) };
    let dir = match into { Some(d) => d.to_path_buf(), None => cache_dir().ok_or("no cache directory")? };
    let mut errors = Vec::new();
    let mut got = 0;
    for (id, f) in &lock.file {
        let target = dir.join(&f.sha256);
        if target.is_file() { continue; }
        if into.is_some() {
            if let Some(have) = find(game, &f.sha256) {
                std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                std::fs::copy(&have, &target).map_err(|e| format!("{}: {e}", target.display()))?;
                got += 1;
                continue;
            }
        } else if find(game, &f.sha256).is_some() { continue; }
        match fetch_one(&lock, f).and_then(|b| { let h = sha256(&b); if h == f.sha256 { Ok(b) } else { Err(format!("hash {h} is not the listed {}", f.sha256)) } }) {
            Ok(b) => {
                std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                let tmp = dir.join(format!("{}.part", f.sha256));
                std::fs::write(&tmp, &b).and_then(|_| std::fs::rename(&tmp, &target)).map_err(|e| format!("{}: {e}", target.display()))?;
                say(format!("samples: fetched {id} ({} kB, {})", b.len() / 1024, f.license));
                got += 1;
            }
            Err(e) => errors.push(format!("{id}: {e}")),
        }
    }
    if errors.is_empty() { Ok(got) } else { Err(format!("samples: {} not fetched: {}", errors.len(), errors.join("; "))) }
}

/// Fetch in the background (a played game: the sounds swap in once here).
pub fn fetch_later(game: &Path) {
    let game = game.to_path_buf();
    let Ok(Some(lock)) = read(&game) else { return };
    if lock.file.values().all(|f| find(&game, &f.sha256).is_some()) { return; }
    let work = move || {
        if let Err(e) = fetch(&game, None, &mut |l| eprintln!("{l}")) { eprintln!("{e} (the synth versions play instead)"); }
    };
    // No threads in a browser (and no files to fetch to): the synth plays.
    #[cfg(target_arch = "wasm32")]
    drop(work);
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::spawn(work);
}

/// A CREDITS text for the files (CC0 needs none, but it is kind and lets a
/// person find the originals).
pub fn credits(lock: &Lock) -> String {
    let mut out = String::from("Recorded sounds\n\n");
    for (id, f) in &lock.file {
        out.push_str(&format!("- {}{} ({}){}\n", f.title.as_deref().unwrap_or(id), f.author.as_ref().map(|a| format!(" by {a}")).unwrap_or_default(), f.license, f.link.as_ref().map(|l| format!(", {l}")).unwrap_or_default()));
    }
    out
}

/// Cut, level and fade one sample out of a decoded file.
pub fn cut(all: &[f32], s: &Sample) -> Vec<f32> {
    let sr = audio::SAMPLE_RATE;
    let a = ((s.from.max(0.0) * sr) as usize).min(all.len());
    let b = s.to.map_or(all.len(), |t| ((t * sr) as usize).clamp(a, all.len()));
    let gain = 10f32.powf(s.db / 20.0);
    let mut x: Vec<f32> = all[a..b].iter().map(|v| v * gain).collect();
    let n = x.len();
    if s.looping {
        // Fold the last half second into the start: the loop has no seam.
        let f = ((0.5 * sr) as usize).min(n / 3);
        if f > 0 {
            let tail: Vec<f32> = x[n - f..].to_vec();
            x.truncate(n - f);
            for (k, t) in tail.iter().enumerate() {
                let w = k as f32 / f as f32;
                x[k] = x[k] * (w * std::f32::consts::FRAC_PI_2).sin() + t * (w * std::f32::consts::FRAC_PI_2).cos();
            }
        }
    } else {
        let f = ((0.004 * sr) as usize).min(n / 2);
        for k in 0..f {
            let w = k as f32 / f as f32;
            x[k] *= w;
            x[n - 1 - k] *= w;
        }
    }
    x
}

/// The patch that plays a cut sample: `rate` (1 = as recorded) and `gain`
/// (linear, for beds whose level the game moves).
pub fn patch(name: &str, samples: Arc<[f32]>, looping: bool) -> audio::synth::Patch {
    use audio::synth::{In, Node, Patch, Unit};
    Patch {
        name: name.to_string(),
        nodes: vec![
            Node { unit: Unit::Clip { name: name.to_string(), samples, looping }, inputs: vec![In::P("rate".into())] },
            Node { unit: Unit::Mul, inputs: vec![In::N(0), In::P("gain".into())] },
        ],
        out: 1,
        params: vec![("rate".into(), 1.0), ("gain".into(), 1.0)],
        looping,
        ..Default::default()
    }
}

/// The group a variant belongs to: `groan.2` is one of `groan`.
pub fn group(name: &str) -> &str {
    match name.rsplit_once('.') { Some((g, n)) if !g.is_empty() && n.chars().all(|c| c.is_ascii_digit()) && !n.is_empty() => g, _ => name }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_group_and_loops_have_no_seam() {
        assert_eq!(group("groan.12"), "groan");
        assert_eq!(group("groan"), "groan");
        assert_eq!(group("v1.x"), "v1.x");
        let sr = audio::SAMPLE_RATE as usize;
        let all: Vec<f32> = (0..sr * 3).map(|i| (i as f32 * 0.01).sin()).collect();
        let s = Sample { name: "bed".into(), file: "f".into(), from: 0.5, to: Some(2.5), db: -6.0, looping: true };
        let x = cut(&all, &s);
        assert_eq!(x.len(), sr * 2 - sr / 2);
        let one = Sample { looping: false, ..s };
        let y = cut(&all, &one);
        assert_eq!(y[0], 0.0);
        assert!((y[sr] - all[sr + sr / 2] * 0.501).abs() < 0.01);
    }

    #[test]
    fn lock_files_must_be_listed() {
        let dir = std::env::temp_dir().join(format!("warp-samples-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(LOCK), "[[sample]]\nname = \"a\"\nfile = \"missing\"\n").unwrap();
        assert!(read(&dir).unwrap_err().contains("not listed"));
        std::fs::write(dir.join(LOCK), "[file.f]\nurl = \"https://example.org/a.flac\"\nsha256 = \"00\"\nlicense = \"CC0-1.0\"\n").unwrap();
        assert!(read(&dir).unwrap_err().contains("64 hex"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
