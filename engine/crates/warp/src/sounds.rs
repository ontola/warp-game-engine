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
    /// The game folder, for `samples.lock` (see crate::samples).
    game: Option<PathBuf>,
    lock_stamp: Option<(SystemTime, u64)>,
    /// Recorded sounds by group (`groan` holds `groan.1`, `groan.2` ...);
    /// a group replaces the synth patch of its name.
    pub sampled: BTreeMap<String, Vec<Arc<CompiledPatch>>>,
    /// Listed files not fetched yet (their sha256).
    waiting: Vec<String>,
    decoded: HashMap<String, Arc<[f32]>>,
    picks: std::cell::Cell<u32>,
    last: std::cell::RefCell<HashMap<String, u32>>,
}

impl Bank {
    pub fn new(dir: Option<PathBuf>) -> Self { Bank { dir, ..Default::default() } }

    /// Also play the recordings the game folder's `samples.lock` names.
    pub fn with_samples(mut self, game: &std::path::Path) -> Self { self.game = Some(game.to_path_buf()); self }

    /// (Re)build the sampled sounds when the lock changed or a file arrived.
    fn scan_samples(&mut self, out: &mut Vec<String>) {
        let Some(game) = self.game.clone() else { return };
        let path = game.join(crate::samples::LOCK);
        let stamp = vfs::stamp(&path);
        let arrived = self.waiting.iter().any(|sha| crate::samples::find(&game, sha).is_some());
        if stamp == self.lock_stamp && !arrived { return; }
        self.lock_stamp = stamp;
        let lock = match crate::samples::read(&game) {
            Ok(Some(l)) => l,
            Ok(None) => { self.sampled.clear(); self.waiting.clear(); return; }
            Err(e) => { out.push(format!("{e} (keeping the last good version)")); self.errors.insert(crate::samples::LOCK.into(), e); return; }
        };
        self.errors.remove(crate::samples::LOCK);
        let mut sampled: BTreeMap<String, Vec<Arc<CompiledPatch>>> = BTreeMap::new();
        let mut waiting = Vec::new();
        let mut decoded = HashMap::new();
        for s in &lock.sample {
            let sha = &lock.file[&s.file].sha256;
            let all = match decoded.get(sha).or_else(|| self.decoded.get(sha)).cloned() {
                Some(a) => a,
                None => match crate::samples::find(&game, sha) {
                    None => { if !waiting.contains(sha) { waiting.push(sha.clone()); } continue; }
                    Some(p) => match audio::sfx::load_clip(&p) {
                        Ok(c) => c.0,
                        Err(e) => { out.push(format!("samples: {}: {e}", s.file)); continue; }
                    },
                },
            };
            decoded.insert(sha.clone(), all.clone());
            let patch = crate::samples::patch(&s.name, crate::samples::cut(&all, s).into(), s.looping);
            match patch.compile() {
                Ok(p) => sampled.entry(crate::samples::group(&s.name).to_string()).or_default().push(Arc::new(p)),
                Err(e) => out.push(format!("samples: {}: {e}", s.name)),
            }
        }
        let n: usize = sampled.values().map(Vec::len).sum();
        out.push(format!("{}: {n} recorded sounds in {} groups{}", crate::samples::LOCK, sampled.len(), if waiting.is_empty() { String::new() } else { format!(" ({} files not fetched yet: the synth versions play)", waiting.len()) }));
        self.sampled = sampled;
        self.waiting = waiting;
        self.decoded = decoded;
    }

    /// Whether `sound.play(name)` plays something (a patch or a recording).
    pub fn has(&self, name: &str) -> bool { self.patches.contains_key(name) || self.is_sampled(name) }

    /// Whether `name` plays a recording (it then takes only `rate` and `gain`).
    pub fn is_sampled(&self, name: &str) -> bool { self.sampled.contains_key(name) || self.sampled.get(crate::samples::group(name)).is_some_and(|v| v.iter().any(|p| p.name == name)) }

    /// Load new and changed files; returns one line per load or error.
    pub fn scan(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        self.scan_samples(&mut out);
        let Some(dir) = &self.dir else { return out };
        let Ok(rd) = vfs::list(dir) else { return out };
        let mut files: Vec<PathBuf> = rd.into_iter().filter(|p| p.extension().is_some_and(|x| x == "luau") && !p.to_string_lossy().ends_with(".d.luau")).collect();
        files.sort();
        for path in files {
            let Some(stamp) = vfs::stamp(&path) else { continue };
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

    /// A recording of the group (a different variant each time), else the
    /// synth patch.
    pub fn get(&self, name: &str) -> Option<Arc<CompiledPatch>> {
        if let Some(v) = self.sampled.get(name).filter(|v| !v.is_empty()) {
            let k = self.picks.get();
            self.picks.set(k.wrapping_add(1));
            // Never the same variant twice in a row.
            let n = v.len() as u32;
            let i = if n > 1 { (k.wrapping_mul(2654435761) >> 7) % (n - 1) } else { 0 };
            let last = self.last_pick(name);
            let i = if n > 1 && i >= last { i + 1 } else { i };
            self.set_last_pick(name, i);
            return Some(v[i as usize % v.len()].clone());
        }
        if let Some(p) = self.sampled.get(crate::samples::group(name)).and_then(|v| v.iter().find(|p| p.name == name)) { return Some(p.clone()); }
        self.patches.get(name).cloned()
    }

    fn last_pick(&self, name: &str) -> u32 { self.last.borrow().get(name).copied().unwrap_or(u32::MAX) }
    fn set_last_pick(&self, name: &str, i: u32) { self.last.borrow_mut().insert(name.to_string(), i); }

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
    /// An online host keeps every play here for its followers.
    pub record: Option<Vec<crate::online::SoundEvent>>,
}

const HANDLES: usize = 512;

impl SoundOut {
    pub fn new(bank: Bank, tx: Option<SoundSender>) -> Self {
        SoundOut { bank, tx, handles: HashMap::new(), order: VecDeque::new(), next: 1, played: 0, recent: VecDeque::new(), ears: Vec::new(), record: None }
    }

    pub fn has_output(&self) -> bool { self.tx.is_some() }

    /// Play a sound; returns its handle (also without an output, so a
    /// script behaves the same in `warp check`).
    pub fn play(&mut self, name: &str, params: &[(String, f32)], at: Option<[f32; 3]>, db: f32, pitch: f32, tick: u64) -> Result<u32, String> {
        let patch = self.bank.find(name)?;
        // A recording stands in for a synth patch: its parameters do not apply.
        let sampled = self.bank.is_sampled(name);
        let params: Vec<(String, f32)> = if sampled { params.iter().filter(|(n, _)| n == "rate" || n == "gain").cloned().collect() } else { params.to_vec() };
        let params = &params[..];
        if !sampled { check_params(&patch, params)?; }
        if let Some(r) = &mut self.record { r.push(crate::online::SoundEvent { name: name.into(), params: params.to_vec(), at, db, pitch }); }
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

    pub fn move_to(&self, id: u32, at: [f32; 3]) {
        if let (Some(tx), Some(h)) = (&self.tx, self.handles.get(&id)) { tx.move_to(h, Place::At(at), 0.0); }
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
