//! The game's synth patches (`sounds/*.luau`, see engine/docs/audio.md):
//! compiled on load, reloaded when a file changes, a failed load keeping the
//! last good version. Each file's main sound is named after the file;
//! `sound(name, fn)` in a file adds more.
use audio::synth::{self, CompiledPatch, Ear, Handle, Layout, Mixer, Patch, Place, Play, SoundSender};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

#[derive(Default)]
pub struct Bank {
    dir: Option<PathBuf>,
    pub patches: BTreeMap<String, Arc<CompiledPatch>>,
    files: HashMap<PathBuf, (SystemTime, u64)>,
    /// The last load error per file, while its previous sounds stay loaded.
    pub errors: BTreeMap<String, String>,
}

impl Bank {
    pub fn new(dir: Option<PathBuf>) -> Self { Bank { dir, ..Default::default() } }

    /// Load new and changed files; returns one line per load or error.
    pub fn scan(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        let Some(dir) = &self.dir else { return out };
        let Ok(rd) = std::fs::read_dir(dir) else { return out };
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "luau") && !p.to_string_lossy().ends_with(".d.luau")).collect();
        files.sort();
        for path in files {
            let Ok(meta) = std::fs::metadata(&path) else { continue };
            let stamp = (meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), meta.len());
            if self.files.get(&path) == Some(&stamp) { continue; }
            let again = self.files.insert(path.clone(), stamp).is_some();
            let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            match synth::lua::load_file(&path).and_then(|ps| ps.iter().map(Patch::compile).collect::<Result<Vec<_>, _>>()) {
                Ok(ps) => {
                    self.errors.remove(&file);
                    let names: Vec<String> = ps.iter().map(|p| p.name.clone()).collect();
                    for p in ps { self.patches.insert(p.name.clone(), Arc::new(p)); }
                    out.push(format!("sounds/{file}: {} {}", if again { "reloaded" } else { "loaded" }, names.join(", ")));
                }
                Err(e) => {
                    out.push(format!("sounds/{file}: {e} (keeping the last good version)"));
                    self.errors.insert(file, e);
                }
            }
        }
        out
    }

    /// Patches written in a script (a module's exported `sounds` table, so a
    /// one-file game needs no `sounds/` folder): compile `src` as the sound
    /// `name` unless it is unchanged. Returns a line when it (re)loaded or failed.
    pub fn add_source(&mut self, name: &str, src: &str) -> Option<String> {
        let key = format!("inline:{name}");
        let hash = { use std::hash::{Hash, Hasher}; let mut h = std::collections::hash_map::DefaultHasher::new(); src.hash(&mut h); h.finish() };
        let stamp = (SystemTime::UNIX_EPOCH, hash);
        if self.files.get(&PathBuf::from(&key)) == Some(&stamp) { return None; }
        let again = self.files.insert(PathBuf::from(&key), stamp).is_some();
        match synth::lua::load(name, src).and_then(|ps| ps.iter().map(Patch::compile).collect::<Result<Vec<_>, _>>()) {
            Ok(ps) => {
                self.errors.remove(&key);
                let names: Vec<String> = ps.iter().map(|p| p.name.clone()).collect();
                for p in ps { self.patches.insert(p.name.clone(), Arc::new(p)); }
                Some(format!("sound {name}: {} {}", if again { "reloaded" } else { "loaded" }, names.join(", ")))
            }
            Err(e) => {
                self.errors.insert(key, e.clone());
                Some(format!("sound {name}: {e} (keeping the last good version)"))
            }
        }
    }

    pub fn get(&self, name: &str) -> Option<Arc<CompiledPatch>> { self.patches.get(name).cloned() }

    /// A loaded patch, or an error that lists the loaded ones.
    pub fn find(&self, name: &str) -> Result<Arc<CompiledPatch>, String> {
        self.get(name).ok_or_else(|| {
            let names: Vec<&str> = self.patches.keys().map(String::as_str).collect();
            if names.is_empty() { format!("no sound {name:?}: the game has no sounds/*.luau patches (or `sounds` table)") } else { format!("no sound {name:?}; sounds: {}", names.join(", ")) }
        })
    }

    /// Change a parameter's default for every later play; editing the file resets it.
    pub fn set_default(&mut self, name: &str, param: &str, value: f32) -> Result<f32, String> {
        let p = self.find(name)?;
        let (next, old) = p.with_default(param, value)?;
        self.patches.insert(name.to_string(), Arc::new(next));
        Ok(old)
    }
}

/// Parameters a patch does not have are errors (a typo would be silent).
pub fn check_params(patch: &CompiledPatch, params: &[(String, f32)]) -> Result<(), String> {
    for (n, _) in params {
        if !patch.params.iter().any(|p| &p.0 == n) {
            let have: Vec<&str> = patch.params.iter().map(|p| p.0.as_str()).collect();
            return Err(format!("sound {} has no parameter {n:?} (has: {})", patch.name, if have.is_empty() { "none".into() } else { have.join(", ") }));
        }
    }
    Ok(())
}

/// Where sounds go: the speakers, an offline mixer, or nowhere (`warp check`).
pub struct SoundOut {
    pub bank: Bank,
    tx: Option<SoundSender>,
    handles: HashMap<u32, Handle>,
    order: VecDeque<u32>,
    next: u32,
    /// Plays so far, and the last few (name, tick), for `state`.
    pub played: u64,
    pub recent: VecDeque<(String, u64)>,
    ears: Vec<Ear>,
}

const HANDLES: usize = 512;

impl SoundOut {
    pub fn new(bank: Bank, tx: Option<SoundSender>) -> Self {
        SoundOut { bank, tx, handles: HashMap::new(), order: VecDeque::new(), next: 1, played: 0, recent: VecDeque::new(), ears: Vec::new() }
    }

    pub fn has_output(&self) -> bool { self.tx.is_some() }

    /// Play a sound; returns its handle (also without an output, so a
    /// script behaves the same in `warp check`).
    pub fn play(&mut self, name: &str, params: &[(String, f32)], at: Option<[f32; 3]>, db: f32, pitch: f32, tick: u64) -> Result<u32, String> {
        let patch = self.bank.find(name)?;
        check_params(&patch, params)?;
        let id = self.next;
        self.next = self.next.wrapping_add(1).max(1);
        self.played += 1;
        self.recent.push_back((name.to_string(), tick));
        while self.recent.len() > 8 { self.recent.pop_front(); }
        if let Some(tx) = &self.tx {
            let refs: Vec<(&str, f32)> = params.iter().map(|(n, v)| (n.as_str(), *v)).collect();
            let place = at.map_or(Place::Head, Place::At);
            let h = tx.play(&patch, &refs, Play { place, reverb: 0.15, ..Default::default() }.db(db).pitch(pitch.clamp(0.05, 8.0)).seed(id));
            self.handles.insert(id, h);
            self.order.push_back(id);
            while self.order.len() > HANDLES { if let Some(old) = self.order.pop_front() { self.handles.remove(&old); } }
        }
        Ok(id)
    }

    pub fn set(&self, id: u32, param: &str, value: f32) {
        if let (Some(tx), Some(h)) = (&self.tx, self.handles.get(&id)) { tx.set(h, param, value); }
    }

    pub fn stop(&self, id: u32) {
        if let (Some(tx), Some(h)) = (&self.tx, self.handles.get(&id)) { tx.release(h); }
    }

    pub fn stop_all(&self) { if let Some(tx) = &self.tx { tx.stop_all(); } }

    /// Listeners at the cameras (sent only when they move).
    pub fn ears(&mut self, ears: Vec<Ear>) {
        let same = ears.len() == self.ears.len() && ears.iter().zip(&self.ears).all(|(a, b)| a.pos == b.pos && a.forward == b.forward);
        if same { return; }
        if let Some(tx) = &self.tx { tx.ears(ears.clone()); }
        self.ears = ears;
    }
}

/// A mixer for offline use (scripts record it; `serve` drains it).
pub fn offline() -> (SoundSender, Mixer) {
    let (tx, mix) = synth::mixer(Layout::stereo(), 128, audio::SAMPLE_RATE);
    tx.room(1.2, 0.4, 0.5);
    (tx, mix)
}
