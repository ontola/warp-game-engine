//! F12 flight recorder: bounded state history, no video encoding or GPU readback
//! during play. Snapshots preserve actual gameplay across rendering edits.
use crate::{play::Gg, session::Session, Args};
use app::{Buttons, Controls, Draw, Frame, Game, Pointer};
use growing_guns::{arena::Arena, Breakage, Snapshot, World, DT, TICK_HZ};
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, io::{Read, Write}, path::{Path, PathBuf}, sync::{Arc, Mutex}};

const VERSION: u32 = 1;
const CHUNK_TICKS: usize = 300;
const MAX_CHUNKS: usize = 64; // Guard against rapid arena-reset commands.
const MAX_TICKS: usize = 2400; // 35–40 seconds, including visual warmup.
const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 96 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
pub struct Sample {
    pub state: Snapshot,
    pub broken: Breakage,
    pub slots: Vec<usize>,
    pub sky: String,
    pub tilt: bool,
}
#[derive(Clone, Serialize, Deserialize)]
struct Chunk { arena: Arc<Vec<u8>>, reset: bool, frames: Vec<Arc<Vec<u8>>> }
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Mark { pub frame: usize, pub alpha: f32 }
#[derive(Clone, Serialize, Deserialize)]
pub struct Tape {
    version: u32,
    tick_hz: u32,
    chunks: Vec<Arc<Chunk>>,
    pub mark: Mark,
    pub size: [u32; 2],
}

#[derive(Default)]
pub struct Recorder {
    chunks: VecDeque<Arc<Chunk>>,
    bytes: usize,
    frames: usize,
    barrier: bool,
    pub disabled: bool,
    pub last: Option<(Tape, PathBuf)>,
    saving: Arc<Mutex<Option<Result<PathBuf, String>>>>,
    pub error: Option<String>,
}
impl Recorder {
    pub fn barrier(&mut self) { self.barrier = true; }
    pub fn push(&mut self, world: &World, slots: &[usize], sky: &str, tilt: bool) {
        if self.disabled || self.error.is_some() { return; }
        if let Err(e) = self.push_inner(world, slots, sky, tilt) { self.error = Some(e); }
    }
    fn push_inner(&mut self, world: &World, slots: &[usize], sky: &str, tilt: bool) -> Result<(), String> {
        let fresh = self.barrier || self.chunks.back().is_none_or(|c| c.frames.len() >= CHUNK_TICKS)
            || world.events.contains(&growing_guns::Event::NewArena);
        if fresh {
            let arena = encode(&world.arena)?;
            self.bytes += arena.len();
            self.chunks.push_back(Arc::new(Chunk { arena: Arc::new(arena), reset: self.barrier || world.events.contains(&growing_guns::Event::NewArena), frames: Vec::new() }));
            self.barrier = false;
        }
        let bytes = encode(&Sample { state: world.snapshot(), broken: world.breakage.clone(), slots: slots.to_vec(), sky: sky.into(), tilt })?;
        self.bytes += bytes.len();
        self.frames += 1;
        Arc::make_mut(self.chunks.back_mut().unwrap()).frames.push(Arc::new(bytes));
        while self.chunks.len() > 1 && (self.frames > MAX_TICKS || self.chunks.len() > MAX_CHUNKS || self.bytes > MAX_BYTES) {
            let c = self.chunks.pop_front().unwrap();
            self.frames -= c.frames.len();
            self.bytes -= c.arena.len() + c.frames.iter().map(|f| f.len()).sum::<usize>();
        }
        if self.bytes > MAX_BYTES { self.chunks.clear(); self.bytes = 0; self.frames = 0; return Err("Replay exceeded its 64 MiB memory limit".into()); }
        Ok(())
    }
    pub fn tape(&self, alpha: f32, size: [u32; 2]) -> Result<Tape, String> {
        if let Some(e) = &self.error { return Err(e.clone()); }
        let frames: usize = self.chunks.iter().map(|c| c.frames.len()).sum();
        if frames == 0 { return Err("No gameplay recorded yet".into()); }
        Ok(Tape { version: VERSION, tick_hz: TICK_HZ, chunks: self.chunks.iter().cloned().collect(), mark: Mark { frame: frames - 1, alpha }, size })
    }
    /// Clones only Arc handles; compression and disk writes happen off the game thread.
    pub fn save(&mut self, png: &Path, alpha: f32, size: [u32; 2]) -> Result<serde_json::Value, String> {
        if Arc::strong_count(&self.saving) > 1 { return Err("Previous replay is still saving; try F12 again shortly".into()); }
        let tape = self.tape(alpha, size)?;
        let path = png.with_extension("ggreplay");
        self.last = Some((tape.clone(), path.clone()));
        let result = self.saving.clone();
        let output = path.clone();
        let count = tape.len();
        *result.lock().unwrap() = None;
        std::thread::spawn(move || { let saved = tape.write(&output).map(|_| output); *result.lock().unwrap() = Some(saved); });
        Ok(serde_json::json!({ "file": path, "frames": count, "seconds": count as f32 * DT, "alpha": alpha, "status": "saving in background", "open": "F9" }))
    }
    pub fn completion(&mut self) -> Option<Result<PathBuf, String>> { self.saving.lock().unwrap().take() }
}
fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, String> { postcard::to_allocvec(value).map_err(|e| e.to_string()) }
fn decode<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T, String> { postcard::from_bytes(bytes).map_err(|e| format!("Incompatible or damaged replay: {e}")) }
impl Tape {
    pub fn len(&self) -> usize { self.chunks.iter().map(|c| c.frames.len()).sum() }
    fn locate(&self, index: usize) -> Result<(usize, usize), String> {
        let mut i = index;
        for (k, c) in self.chunks.iter().enumerate() { if i < c.frames.len() { return Ok((k, i)); } i -= c.frames.len(); }
        Err(format!("Frame {index} is outside this replay ({} frames)", self.len()))
    }
    fn sample(&self, index: usize) -> Result<Sample, String> { let (c, f) = self.locate(index)?; decode(&self.chunks[c].frames[f]) }
    fn arena(&self, index: usize) -> Result<Arena, String> { let (c, _) = self.locate(index)?; decode(&self.chunks[c].arena) }
    fn validate(&self) -> Result<(), String> {
        if self.version != VERSION || self.tick_hz != TICK_HZ { return Err("Unsupported replay version or tick rate".into()); }
        if self.chunks.is_empty() || self.len() > MAX_TICKS || self.chunks.len() > MAX_CHUNKS || self.chunks.iter().any(|c| c.frames.is_empty() || c.frames.len() > CHUNK_TICKS) { return Err("Invalid replay history".into()); }
        self.validate_mark(self.mark)?;
        if self.size.iter().any(|v| *v == 0 || *v > 8192) { return Err("Invalid replay resolution".into()); }
        for c in &self.chunks {
            let _: Arena = decode(&c.arena)?;
            for f in &c.frames {
                let s: Sample = decode(f)?;
                if s.slots.is_empty() || s.slots.len() > 4 || s.slots.iter().any(|i| *i >= s.state.players.len()) { return Err("Invalid replay player views".into()); }
                crate::sky::SkyChoice::parse(&s.sky)?;
            }
        }
        Ok(())
    }
    fn validate_mark(&self, mark: Mark) -> Result<(), String> {
        self.locate(mark.frame)?;
        if !mark.alpha.is_finite() || !(0.0..=1.0).contains(&mark.alpha) { return Err("Invalid replay interpolation phase".into()); }
        Ok(())
    }
    pub fn write(&self, path: &Path) -> Result<(), String> {
        let temp = path.with_extension("ggreplay.part");
        let file = std::fs::File::create(&temp).map_err(|e| e.to_string())?;
        let mut zip = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
        zip.write_all(&encode(self)?).map_err(|e| e.to_string())?;
        zip.finish().map_err(|e| e.to_string())?.sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(temp, path).map_err(|e| e.to_string())
    }
    pub fn read(path: &Path) -> Result<Self, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut bytes = Vec::new();
        flate2::read::GzDecoder::new(file).take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_FILE_BYTES { return Err("Replay file is too large".into()); }
        let tape: Self = decode(&bytes)?;
        tape.validate()?;
        Ok(tape)
    }
}

pub struct Viewer {
    tape: Tape,
    path: PathBuf,
    game: Gg,
    current: Option<usize>,
    target: Mark,
    pub paused: bool,
    marks: Vec<Mark>,
    message: Option<String>,
    pub overlay: bool,
    exit: bool,
}
impl Viewer {
    pub fn new(tape: Tape, path: PathBuf) -> Result<Self, String> {
        tape.validate()?;
        let game = Self::game(&tape)?;
        let marks_file = path.with_extension("marks.json");
        let marks: Vec<Mark> = if marks_file.exists() {
            serde_json::from_slice(&std::fs::read(marks_file).map_err(|e| e.to_string())?).map_err(|e| format!("Invalid bookmarks: {e}"))?
        } else { Vec::new() };
        for m in &marks { tape.validate_mark(*m)?; }
        let target = marks.last().copied().unwrap_or(tape.mark);
        Ok(Self { tape, path, game, current: None, target, paused: true, marks, message: None, overlay: true, exit: false })
    }
    fn game(tape: &Tape) -> Result<Gg, String> {
        let first = tape.sample(0)?;
        let mut world = World::remote(tape.arena(0)?, first.state.players.len());
        world.apply(first.state);
        world.events.clear();
        let mut game = Gg::new(world, Session::Local, first.slots);
        game.recorder.disabled = true;
        // The stadium's recursive live TV samples wall-clock presentation.
        // Keep its neutral initial texture during repeatable frame review.
        game.vis.scene.live_screen_feeds.clear();
        game.scripts = None;
        game.ui.open(crate::hud::menus::Screen::Play);
        game.light_mut().taa = 0.0; // paused review must not accumulate temporal history.
        game.set_sky(crate::sky::SkyChoice::parse(&first.sky)?)?;
        Ok(game)
    }
    pub fn seek(&mut self, mark: Mark) -> Result<(), String> {
        self.tape.validate_mark(mark)?;
        if self.current.is_some_and(|n| mark.frame < n) { self.game = Self::game(&self.tape)?; self.current = None; }
        self.target = mark;
        self.paused = true;
        Ok(())
    }
    pub fn ready(&self) -> bool { self.current == Some(self.target.frame) }
    fn step(&mut self, delta: isize) {
        let frame = self.target.frame.saturating_add_signed(delta).min(self.tape.len() - 1);
        if let Err(e) = self.seek(Mark { frame, alpha: 1.0 }) { self.message = Some(e); }
    }
    fn bookmark(&mut self) -> Result<(), String> {
        if !self.ready() { return Err("Wait for the selected frame to finish restoring".into()); }
        let mut marks = self.marks.clone();
        marks.retain(|m| *m != self.target);
        marks.push(self.target);
        let path = self.path.with_extension("marks.json");
        let result = serde_json::to_vec_pretty(&marks).map_err(|e| e.to_string()).and_then(|b| std::fs::write(&path, b).map_err(|e| e.to_string()));
        self.message = Some(match &result {
            Ok(()) => { self.marks = marks; format!("Saved frame {}. Reopen after rebuilding.", self.target.frame) }
            Err(e) => format!("Bookmark not saved: {e}"),
        });
        result
    }
    fn toggle(&mut self) {
        if self.ready() && self.target.frame + 1 == self.tape.len() { self.step(-(self.target.frame as isize)); }
        self.paused = !self.paused;
    }
}
impl Game for Viewer {
    fn quit(&self) -> bool { self.exit }
    fn function_key(&mut self, n: u8) -> bool { if n == 9 { self.exit = true; true } else { false } }
    fn fixed_quality(&self) -> bool { true }
    fn local_players(&self) -> usize { 1 }
    fn mouse_capture(&self) -> Option<bool> { Some(false) }
    fn tick(&mut self, controls: &[Controls]) {
        let c = controls.first().copied().unwrap_or_default();
        if c.pressed.has(Buttons::SOUTH | Buttons::START) { self.toggle(); }
        if c.pressed.has(Buttons::LEFT) { self.step(-1); }
        if c.pressed.has(Buttons::RIGHT) { self.step(1); }
        if c.pressed.has(Buttons::WEST) { let _ = self.bookmark(); }
        if !self.paused && self.ready() {
            if self.target.frame + 1 < self.tape.len() { self.target = Mark { frame: self.target.frame + 1, alpha: 1.0 }; }
            else { self.paused = true; }
        }
    }
    fn pointer(&mut self, p: Pointer, size: [u32; 2]) {
        let Some([x, y]) = p.position.filter(|_| p.pressed) else { return };
        let s = size[1] as f32 / 900.0;
        if y < size[1] as f32 - 118.0 * s { return; }
        if y > size[1] as f32 - 48.0 * s {
            let t = ((x - 32.0 * s) / (size[0] as f32 - 64.0 * s)).clamp(0.0, 1.0);
            let mark = self.marks.iter().chain(std::iter::once(&self.tape.mark)).copied().find(|m| {
                let mx = 32.0 * s + (size[0] as f32 - 64.0 * s) * m.frame as f32 / (self.tape.len() - 1).max(1) as f32;
                (mx - x).abs() <= 8.0 * s
            }).unwrap_or(Mark { frame: (t * (self.tape.len() - 1) as f32).round() as usize, alpha: 1.0 });
            if let Err(e) = self.seek(mark) { self.message = Some(e); }
        } else {
            match (x / s) as u32 { 20..=150 => self.toggle(), 151..=245 => self.step(-1), 246..=340 => self.step(1), 341..=520 => { let _ = self.bookmark(); }, 521..=700 => { if let Err(e) = self.seek(self.tape.mark) { self.message = Some(e); } }, _ => {} }
        }
    }
    fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> {
        // Seeking runs the real visual event/physics path and GPU debris at
        // every tick. Never skip straight to a snapshot with missing effects.
        // Bound work per paint so the review UI can show reconstruction progress.
        let start = self.current.map_or(0, |n| n + 1);
        let end = self.target.frame.min(start + 119);
        let dt = frame.dt;
        let alpha = frame.alpha;
        frame.dt = 0.0;
        frame.alpha = 1.0;
        for i in start..=end {
            let sample = self.tape.sample(i)?;
            let (chunk, offset) = self.tape.locate(i)?;
            let arena = (offset == 0 && (i == 0 || self.tape.chunks[chunk].reset))
                .then(|| decode::<Arena>(&self.tape.chunks[chunk].arena)).transpose()?;
            self.game.replay_apply(sample, arena)?;
            // Build/upload only when needed; dispatch simulation without shading.
            self.game.replay_gpu_tick(frame)?;
            self.current = Some(i);
        }
        frame.alpha = if self.ready() { self.target.alpha } else { 1.0 };
        let mut draw = self.game.draw(frame)?;
        frame.dt = dt;
        frame.alpha = alpha;
        if self.overlay {
            use crate::hud::style::{Label, BODY};
            let [w, h] = frame.size.map(|n| n as f32);
            let s = h / 900.0;
            let clip = [0.0, 0.0, w, h];
            draw.hud.begin_overlay();
            draw.hud.rect([0.0, h - 124.0 * s, w, 124.0 * s], [0.025, 0.035, 0.055, 0.96], clip);
            let state = if !self.ready() { "RESTORING" } else if self.paused { "PAUSED" } else { "PLAYING" };
            let n = self.current.unwrap_or(0);
            let label = format!("REPLAY  /  {state}     frame {n} / {}     {:.2}s     tick {}", self.tape.len() - 1, n as f32 * DT, self.game.world.tick);
            Label::new(BODY, 20.0 * s, [0.78, 0.86, 0.95, 1.0]).draw(&mut draw.hud, &label, 32.0 * s, h - 103.0 * s, clip);
            for (left, right, text) in [(20.0, 150.0, if self.paused { "PLAY [SPACE]" } else { "PAUSE [SPACE]" }), (151.0, 245.0, "< FRAME"), (246.0, 340.0, "FRAME >"), (341.0, 520.0, "SAVE FRAME [R]"), (521.0, 700.0, "F12 MOMENT")] {
                draw.hud.rect([left * s, h - 88.0 * s, (right - left - 5.0) * s, 36.0 * s], [0.12, 0.18, 0.26, 1.0], clip);
                Label::new(BODY, 20.0 * s, [1.0; 4]).center().draw(&mut draw.hud, text, (left + right - 5.0) * 0.5 * s, h - 69.0 * s, clip);
            }
            Label::new(BODY, 17.0 * s, [0.6, 0.72, 0.84, 1.0]).right().draw(&mut draw.hud, self.message.as_deref().unwrap_or("Click timeline to seek   |   F9 / Esc closes replay"), w - 32.0 * s, h - 69.0 * s, clip);
            let bar = [32.0 * s, h - 30.0 * s, w - 64.0 * s, 5.0 * s];
            draw.hud.rect(bar, [0.18, 0.23, 0.3, 1.0], clip);
            let x = |m: Mark| bar[0] + bar[2] * m.frame as f32 / (self.tape.len() - 1).max(1) as f32;
            for m in self.marks.iter().chain(std::iter::once(&self.tape.mark)) { draw.hud.rect([x(*m) - 2.0 * s, bar[1] - 5.0 * s, 4.0 * s, 15.0 * s], [1.0, 0.72, 0.24, 1.0], clip); }
            draw.hud.rect([x(Mark { frame: n, alpha: 1.0 }) - 4.0 * s, bar[1] - 7.0 * s, 8.0 * s, 19.0 * s], [0.3, 0.78, 1.0, 1.0], clip);
        }
        Ok(draw)
    }
    fn split(&self) -> render::SplitLayout { self.game.split() }
    fn state(&self) -> Option<String> { self.game.state() }
    fn feedback_replay(&mut self, _path: &Path, _alpha: f32) -> Result<Option<serde_json::Value>, String> {
        self.bookmark()?;
        Ok(Some(serde_json::json!({"file": self.path, "frame": self.target.frame, "alpha": self.target.alpha})))
    }
}

pub fn open_latest() -> Result<Viewer, String> {
    let path = latest()?;
    Viewer::new(Tape::read(&path)?, path)
}

fn latest() -> Result<PathBuf, String> {
    std::fs::read_dir(app::screenshot::directory()).map_err(|e| e.to_string())?
        .filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "ggreplay"))
        .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok()).ok_or("No saved replays yet. Press F12 during play.".into())
}
pub fn run(args: &Args) -> Result<(), String> {
    let input = args.list.get(1).map(String::as_str).unwrap_or("latest");
    let path = if input == "latest" { latest()? } else { PathBuf::from(input) };
    let tape = Tape::read(&path)?;
    let default_size = (tape.size[0], tape.size[1]);
    let mut viewer = Viewer::new(tape, path)?;
    if let Some(n) = args.value("--frame") {
        viewer.seek(Mark { frame: n.parse().map_err(|_| "--frame must be an integer")?, alpha: args.num("--alpha", 1.0) })?;
    }
    let (w, h) = args.size(default_size);
    if w == 0 || h == 0 || w > 8192 || h > 8192 { return Err("Size must be between 1 and 8192".into()); }
    if let Some(out) = args.value("--out") {
        viewer.overlay = args.flag("--ui");
        let gpu = render::Gpu::new()?;
        let mut capture = app::Headless::new(&gpu, [w, h], TICK_HZ as f32)?;
        loop {
            let rgba = capture.frame(&mut viewer)?;
            if viewer.ready() {
                render::write_png(Path::new(out), w, h, &rgba)?;
                println!("replay frame {} alpha {}: {out}", viewer.target.frame, viewer.target.alpha);
                break;
            }
        }
        if let Some(out) = args.value("--state") { std::fs::write(out, viewer.state().unwrap_or_default()).map_err(|e| e.to_string())?; }
        Ok(())
    } else {
        app::run(app::Config { title: "Growing Guns | Replay".into(), size: [w, h], tick_hz: TICK_HZ as f32, ..Default::default() }, &mut viewer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rolling_history_roundtrips_actual_states_and_is_bounded() {
        let mut world = World::new(3, &[false, true, true, true]);
        world.release_cage();
        let mut recorder = Recorder::default();
        let mut elapsed = std::time::Duration::ZERO;
        for i in 0..2705 {
            world.step(&[growing_guns::Input { mv: [0.15, 0.4], fire: true, look: [0.006, 0.0], ..Default::default() }]);
            let start = std::time::Instant::now();
            recorder.push(&world, &[0], "godot", true);
            elapsed += start.elapsed();
            if i == 150 { recorder.barrier(); }
        }
        let tape = recorder.tape(0.375, [1600, 900]).unwrap();
        tape.validate().unwrap();
        assert!(tape.len() <= MAX_TICKS);
        assert!(tape.len() >= MAX_TICKS - CHUNK_TICKS);
        assert!(recorder.bytes <= MAX_BYTES);
        let path = std::env::temp_dir().join(format!("gg-replay-test-{}.ggreplay", std::process::id()));
        tape.write(&path).unwrap();
        let saved = Tape::read(&path).unwrap();
        let last = saved.sample(saved.len() - 1).unwrap();
        assert_eq!(encode(&last.state).unwrap(), encode(&world.snapshot()).unwrap());
        let mut restored = World::remote(saved.arena(0).unwrap(), last.state.players.len());
        for (i, c) in saved.chunks.iter().enumerate() {
            if i == 0 || c.reset { restored.arena = decode(&c.arena).unwrap(); }
            for f in &c.frames {
                let sample: Sample = decode(f).unwrap();
                restored.arena.apply_breakage(&sample.broken);
                restored.apply(sample.state);
            }
        }
        assert_eq!(world.digest(), restored.digest());
        assert_eq!(encode(&world.arena).unwrap(), encode(&restored.arena).unwrap());
        println!("recorder: {:.3} ms/tick, {} bytes memory, {} bytes saved, {:.2}s history", elapsed.as_secs_f64() * 1000.0 / 2705.0, recorder.bytes, path.metadata().unwrap().len(), saved.len() as f32 * DT);
        std::fs::remove_file(path).unwrap();
        let mut bad = saved.clone(); bad.version += 1; assert!(bad.validate().is_err());
        assert!(saved.validate_mark(Mark { frame: saved.len(), alpha: 1.0 }).is_err());
        assert!(saved.validate_mark(Mark { frame: 0, alpha: f32::NAN }).is_err());
    }

    #[test]
    fn saved_tape_is_immutable_while_play_continues() {
        let mut world = World::new(5, &[true; 4]);
        let mut recorder = Recorder::default();
        world.step(&[]); recorder.push(&world, &[0], "godot", true);
        let saved = recorder.tape(0.25, [800, 450]).unwrap();
        let bytes = encode(&saved).unwrap();
        for _ in 0..400 { world.step(&[]); recorder.push(&world, &[0], "godot", true); }
        assert_eq!(encode(&saved).unwrap(), bytes);
        assert_eq!(saved.len(), 1);
        assert_eq!(saved.mark.alpha, 0.25);
    }

    /// Serial Metal fixture: exercises F12's screenshot/replay hook, rewinds,
    /// restores an exact sub-tick, and checks paused frames do not animate.
    #[test]
    #[ignore = "GPU; run serially without another game rendering"]
    fn f12_replay_gpu_review() {
        app::clipboard::memory_only();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../build/replay-review");
        std::fs::create_dir_all(&root).unwrap();
        unsafe { std::env::set_var("WARP_SCREENSHOTS", &root); std::env::set_var("WARP_LIVE_DIR", root.join("feedback")); }
        let mut game = Gg::new(World::new(3, &[false, true, true, true]), Session::Local, vec![0]);
        for cmd in ["bots pause", "cage", "health 0 50000", "special 0 ion_cannon", "aim 0 0 0 0"] { game.command(cmd).unwrap(); }
        for _ in 0..30 { game.tick(&[Controls::default()]); }
        game.tick(&[Controls { held: Buttons(Buttons::LEFT_TRIGGER), ..Default::default() }]);
        for _ in 0..225 { game.tick(&[Controls::default()]); }
        let gpu = render::Gpu::new().unwrap();
        let mut headless = app::Headless::new(&gpu, [1600, 900], TICK_HZ as f32).unwrap();
        let rgba = headless.frame_at(&mut game, 0.375).unwrap();
        app::screenshot::save_with(app::screenshot::Shot { title: "Replay review", size: [1600, 900], rgba: &rgba, metadata: serde_json::json!({"test_data": true}) }, |path| game.feedback_replay(path, 0.375)).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if let Some(done) = game.recorder.completion() { done.unwrap(); break; }
            assert!(std::time::Instant::now() < deadline, "replay write timed out");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let (_, path) = game.recorder.last.clone().unwrap();
        let tape = Tape::read(&path).unwrap();
        assert_eq!(tape.mark.alpha, 0.375);
        assert_eq!(encode(&tape.sample(tape.mark.frame).unwrap().state).unwrap(), encode(&game.world.snapshot()).unwrap());
        std::fs::write(root.join("latest.txt"), path.to_string_lossy().as_bytes()).unwrap();
        let mut viewer = Viewer::new(tape.clone(), path.clone()).unwrap();
        let render_ready = |h: &mut app::Headless, v: &mut Viewer| -> Vec<u8> {
            loop { let rgba = h.frame(v).unwrap(); if v.ready() { break rgba; } }
        };
        let first = render_ready(&mut headless, &mut viewer);
        render::write_png(&root.join("paused.png"), 1600, 900, &first).unwrap();
        viewer.overlay = false;
        let clean = render_ready(&mut headless, &mut viewer);
        let again = render_ready(&mut headless, &mut viewer);
        render::write_png(&root.join("again.png"), 1600, 900, &again).unwrap();
        render::write_png(&root.join("clean.png"), 1600, 900, &clean).unwrap();
        let changed = clean.iter().zip(&again).filter(|(a, b)| a != b).count();
        assert!(changed == 0, "paused frame animated: {changed} differing channels");
        render::write_png(&root.join("clean.png"), 1600, 900, &clean).unwrap();
        viewer.seek(Mark { frame: 90, alpha: 1.0 }).unwrap();
        render_ready(&mut headless, &mut viewer);
        viewer.seek(tape.mark).unwrap();
        let restored = render_ready(&mut headless, &mut viewer);
        render::write_png(&root.join("restored.png"), 1600, 900, &restored).unwrap();
        let changed = clean.iter().zip(&restored).filter(|(a, b)| a != b).count();
        assert!(changed == 0, "backward/forward seek changed the marked frame: {changed} differing channels");
        viewer.bookmark().unwrap();
        let reopened = Viewer::new(Tape::read(&path).unwrap(), path).unwrap();
        assert_eq!(reopened.target, tape.mark);
        // Continue actual gameplay through detonation; replay must advance
        // GPU shards too, not merely restore the beam's analytic clock.
        for _ in 0..40 { game.tick(&[Controls::default()]); }
        let impact_tape = game.recorder.tape(1.0, [1600, 900]).unwrap();
        let impact_path = root.join("impact.ggreplay");
        impact_tape.write(&impact_path).unwrap();
        let mut impact = Viewer::new(impact_tape.clone(), impact_path).unwrap();
        let before = render_ready(&mut headless, &mut impact);
        render::write_png(&root.join("impact.png"), 1600, 900, &before).unwrap();
        let paused = render_ready(&mut headless, &mut impact);
        assert!(before == paused, "GPU debris moved while paused");
        impact.seek(Mark { frame: 40, alpha: 1.0 }).unwrap();
        render_ready(&mut headless, &mut impact);
        impact.seek(impact_tape.mark).unwrap();
        let after = render_ready(&mut headless, &mut impact);
        assert!(before == after, "GPU debris changed after rewind");
        assert!(game.function_key(9));
        assert_eq!(game.mouse_capture(), Some(false));
        assert!(game.fixed_quality());
        assert!(game.escape());
        assert!(!game.fixed_quality());
        game.recorder.last = None; // A fresh process has no in-memory capture.
        assert!(game.function_key(9));
        assert!(game.fixed_quality(), "F9 did not reopen the saved file");
        assert!(game.escape());
        println!("GPU review saved to {}", root.display());
    }
}
