//! F4 flight recorder: bounded state history, no video encoding or GPU readback
//! during play. Snapshots preserve actual gameplay across rendering edits.
use crate::{play::Gg, session::Session, Args};
use app::{Buttons, Controls, Draw, Frame, Game, Pointer};
use growing_guns::{arena::Arena, Breakage, Snapshot, World, DT, TICK_HZ};
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, io::{Read, Write}, path::{Path, PathBuf}, sync::{Arc, Mutex}};

// A paint stays responsive while canonical effects are reconstructed. This is
// a time budget, not 120 arbitrarily expensive ticks in a single UI frame.
const SEEK_BUDGET: std::time::Duration = std::time::Duration::from_millis(6);
const SCRUB_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(80);
const VERSION: u32 = 3;
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
    inputs: VecDeque<(u64, u64, Vec<growing_guns::Input>)>,
    edits: VecDeque<(u64, serde_json::Value)>,
    recorded_steps: u64,
    dropped_edits: u64,
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
    pub fn inputs(&mut self, tick: u64, seed: u64, inputs: &[growing_guns::Input]) {
        if self.disabled {return;}
        self.recorded_steps += 1;
        self.inputs.push_back((tick, seed, inputs.to_vec()));
        if self.inputs.len() > 5 * TICK_HZ as usize {self.inputs.pop_front();}
        while self.edits.front().is_some_and(|(at,_)|self.recorded_steps - at > 5 * TICK_HZ as u64) {
            self.edits.pop_front();
        }
    }

    pub fn edit(&mut self, tick:u64, seed:u64, tool:&str, args:&serde_json::Value) {
        if self.disabled {return;}
        if self.edits.len()==128 {self.edits.pop_front(); self.dropped_edits+=1;}
        let args=if args.to_string().len() <= 8192 {args.clone()} else {
            self.dropped_edits+=1;
            serde_json::json!({"omitted":"arguments exceeded 8 KiB"})
        };
        self.edits.push_back((self.recorded_steps, serde_json::json!({"after_tick":tick,"arena_seed":seed,"tool":tool,"arguments":args})));
    }

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
        if Arc::strong_count(&self.saving) > 1 { return Err("Previous replay is still saving; try F4 again shortly".into()); }
        let tape = self.tape(alpha, size)?;
        let path = png.with_extension("ggreplay");
        self.last = Some((tape.clone(), path.clone()));
        let result = self.saving.clone();
        let output = path.clone();
        let count = tape.len();
        *result.lock().unwrap() = None;
        std::thread::spawn(move || { let saved = tape.write(&output).map(|_| output); *result.lock().unwrap() = Some(saved); });
        Ok(serde_json::json!({ "file": path, "frames": count, "seconds": count as f32 * DT, "alpha": alpha, "status": "saving in background", "open": "F11",
            "input_history":{"hz":TICK_HZ,"rows_after_step_tick_seed_inputs":self.inputs,
            "live_edits":self.edits.iter().map(|(_,v)|v).collect::<Vec<_>>(),"omitted_edits_since_start":self.dropped_edits} }))
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
    fn sample(&self, index: usize) -> Result<Sample, String> { let (c, f) = self.locate(index)?; self.decode_frame(&self.chunks[c].frames[f]) }
    fn arena(&self, index: usize) -> Result<Arena, String> { let (c, _) = self.locate(index)?; decode(&self.chunks[c].arena) }
    fn decode_frame(&self, bytes: &[u8]) -> Result<Sample, String> {
        if self.version == 2 {
            // V2 placed the breakage record immediately after the snapshot.
            // Insert an empty length-prefixed list at that boundary.
            let (_, tail) = postcard::take_from_bytes::<Snapshot<()>>(bytes).map_err(|e| e.to_string())?;
            let mut frame = bytes[..bytes.len() - tail.len()].to_vec();
            frame.extend(encode(&Vec::<glam::Vec3>::new())?);
            frame.extend_from_slice(tail);
            return decode(&frame);
        }
        decode(bytes)
    }

    fn validate(&self) -> Result<(), String> {
        if !(2..=VERSION).contains(&self.version) || self.tick_hz != TICK_HZ { return Err("Unsupported replay version or tick rate".into()); }
        if self.chunks.is_empty() || self.len() > MAX_TICKS || self.chunks.len() > MAX_CHUNKS || self.chunks.iter().any(|c| c.frames.is_empty() || c.frames.len() > CHUNK_TICKS) { return Err("Invalid replay history".into()); }
        self.validate_mark(self.mark)?;
        if self.size.iter().any(|v| *v == 0 || *v > 8192) { return Err("Invalid replay resolution".into()); }
        for c in &self.chunks {
            let _: Arena = decode(&c.arena)?;
            for f in &c.frames {
                let s = self.decode_frame(f)?;
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
    window: render::editor_window::EditorWindow,
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
    dragging: bool,
    scrub_changed: Option<std::time::Instant>,
    clock: Option<(std::time::Instant, f64)>,
    sounds: Option<crate::sounds::Sounds>,
    music: Option<(crate::music::Shared, crate::music::Director)>,
    music_db: f32,
    crowd: Option<(audio::sfx::SfxSender, crate::crowd_sound::Samples)>,
    audio_running: bool,
    rewind_reuses: usize,
    pub lens: Lens,
}

/// Photo-style depth of field for frame review (off by default): F toggles,
/// right-click focuses on that point, Q / E open and close the aperture.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Lens {
    pub on: bool,
    pub f_stop: f32,
    /// Autofocus point as a fraction of the window, or `focus_m` if above zero.
    pub focus: [f32; 2],
    pub focus_m: f32,
}
impl Default for Lens {
    fn default() -> Self { Self { on: false, f_stop: 2.8, focus: [0.5, 0.5], focus_m: 0.0 } }
}
const FILM_MM: f32 = 70.0;
const STOPS: [f32; 9] = [1.4, 2.0, 2.8, 4.0, 5.6, 8.0, 11.0, 16.0, 22.0];
impl Lens {
    /// One full stop wider (negative) or narrower (positive).
    fn stop(&mut self, by: isize) {
        let i = STOPS.iter().position(|&f| f >= self.f_stop - 0.05).unwrap_or(STOPS.len() - 1);
        self.f_stop = STOPS[(i as isize + by).clamp(0, STOPS.len() as isize - 1) as usize];
    }
    /// One lens per view. With several views each focuses on its own centre.
    fn settings(&self, views: usize) -> Vec<render::depth_of_field::Settings> {
        (0..views).map(|_| render::depth_of_field::Settings {
            f_stop: if self.on { self.f_stop } else { 0.0 },
            focus_m: self.focus_m,
            focus_point: if views == 1 { self.focus } else { [0.5, 0.5] },
            // The game's wide field of view on 24 mm film keeps nearly
            // everything sharp; medium-format film gives a photo-mode look.
            sensor_mm: FILM_MM,
            ..Default::default()
        }).collect()
    }
    fn label(&self) -> String {
        if !self.on { return "F lens".into(); }
        let focus = if self.focus_m > 0.0 { format!("{:.1} m", self.focus_m) } else { "right-click focus".into() };
        format!("LENS f/{}  Q/E aperture  {focus}  F off", self.f_stop)
    }
}
impl Viewer {
    fn layout(&self,size:[u32;2])->render::editor_window::WindowFrame {
        let [w,h]=size.map(|n|n as f32);
        let s=(h/900.).min(w/760.).max(0.01);
        self.window.frame(size,[8.*s,h-174.*s,w-16.*s,168.*s],[720.*s,150.*s],s)
    }

    pub fn new(tape: Tape, path: PathBuf) -> Result<Self, String> {
        tape.validate()?;
        let game = Self::game(&tape)?;
        let marks_file = path.with_extension("marks.json");
        let marks: Vec<Mark> = if marks_file.exists() {
            serde_json::from_slice(&std::fs::read(marks_file).map_err(|e| e.to_string())?).map_err(|e| format!("Invalid bookmarks: {e}"))?
        } else { Vec::new() };
        for m in &marks { tape.validate_mark(*m)?; }
        let target = marks.last().copied().unwrap_or(tape.mark);
        Ok(Self { window: Default::default(), tape, path, game, current: None, target, paused: true, marks, message: None, overlay: true, exit: false, dragging: false, scrub_changed: None, clock: None, sounds: None, music: None, music_db: -9.0, crowd: None, audio_running: false, rewind_reuses: 0, lens: Lens::default() })
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
        self.target = mark;
        self.paused = true;
        self.clock = None;
        self.silence();
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
    pub(crate) fn set_audio(&mut self, sounds: Option<crate::sounds::Sounds>, music: Option<crate::music::Shared>, music_db: f32) {
        self.sounds = sounds;
        self.music = music.map(|m| (m, Default::default()));
        self.music_db = music_db;
        self.silence();
    }
    pub(crate) fn take_sounds(&mut self) -> Option<crate::sounds::Sounds> {
        self.silence();
        self.sounds.take()
    }
    pub(crate) fn set_crowd(&mut self, bank: Option<(audio::sfx::SfxSender, crate::crowd_sound::Samples)>) {
        self.crowd = bank;
        self.game.vis.stadium.sound.replay_audio(self.crowd.as_ref(), false);
        if let Some((tx, _)) = &self.crowd { tx.volume(0.0); }
    }
    fn silence(&mut self) {
        self.audio_running = false;
        if let Some((tx, _)) = &self.crowd { tx.volume(0.0); }
        if let Some(s) = &mut self.sounds { s.reset_replay(); }
        if let Some((m, _)) = &self.music { crate::music::lock(m).set_volume_db(crate::music::MUSIC_DB_MIN); }
    }
    fn toggle(&mut self) {
        if self.ready() && self.target.frame + 1 == self.tape.len() { self.step(-(self.target.frame as isize)); }
        self.paused = !self.paused;
        self.clock = None;
        if self.paused { self.silence(); }
    }
    /// Recorded time is independent of how many app ticks or paints happened.
    /// A frame's alpha interpolates its previous and current snapshot.
    fn advance_clock(&mut self, now: std::time::Instant) {
        if self.paused { self.clock = None; return; }
        let Some((start, phase)) = self.clock else {
            if self.ready() { self.clock = Some((now, self.target.frame as f64 + self.target.alpha as f64)); }
            return;
        };
        let phase = phase + now.duration_since(start).as_secs_f64() * self.tape.tick_hz as f64;
        self.target = mark_at_phase(phase, self.tape.len());
        if phase >= self.tape.len() as f64 { self.paused = true; self.clock = None; }
    }
    fn play_audio(&mut self) {
        self.audio_running = true;
        if let Some(s) = &mut self.sounds { s.tick(&self.game.world, &self.game.slots, DT); }
        if let Some((m, director)) = &mut self.music {
            let mut m = crate::music::lock(m);
            m.set_volume_db(self.music_db);
            director.follow(&mut *m, &self.game.world, &self.game.slots);
            m.update(DT);
        }
    }

}
fn mark_at_phase(phase: f64, frames: usize) -> Mark {
    let phase = phase.clamp(0.0, frames as f64);
    let frame = (phase.ceil() as usize).saturating_sub(1).min(frames - 1);
    Mark { frame, alpha: (phase - frame as f64).clamp(0.0, 1.0) as f32 }
}

impl Game for Viewer {
    fn quit(&self) -> bool { self.exit }
    fn editor_shortcut(&self,key:u8)->Option<(&'static str,bool)> { (key==11).then_some(("Replay",self.overlay)) }
    fn editor_windows_open(&self) -> bool { self.overlay }
    fn set_editor_windows_open(&mut self, open: bool) { self.overlay = open; self.dragging = false; self.scrub_changed = None; self.window.cancel(); }
    fn function_key(&mut self, n: u8) -> bool { if n == 11 { self.exit = true; true } else { false } }
    fn fixed_quality(&self) -> bool { self.paused && !self.dragging }
    fn local_players(&self) -> usize { 1 }
    fn mouse_capture(&self) -> Option<bool> { Some(false) }
    fn tick(&mut self, controls: &[Controls]) {
        let c = controls.first().copied().unwrap_or_default();
        if c.pressed.has(Buttons::SOUTH | Buttons::START) { self.toggle(); }
        if c.pressed.has(Buttons::LEFT) { self.step(-1); }
        if c.pressed.has(Buttons::RIGHT) { self.step(1); }
        if c.pressed.has(Buttons::WEST) { let _ = self.bookmark(); }
        if c.pressed.has(Buttons::NORTH) { self.lens.on = !self.lens.on; }
        if c.pressed.has(Buttons::LEFT_SHOULDER) { self.lens.stop(-1); }
        if c.pressed.has(Buttons::RIGHT_SHOULDER) { self.lens.stop(1); }

    }
    fn pointer(&mut self, p: Pointer, size: [u32; 2]) {
        let frame=self.layout(size);
        // Right-click the scene (not the replay window) to focus the lens there.
        if let Some([x, y]) = p.position.filter(|_| p.right_pressed)
            && !(self.overlay && render::editor_style::EditorButton(frame.rect).contains([x, y])) {
            self.lens = Lens { on: true, focus: [x / size[0].max(1) as f32, y / size[1].max(1) as f32], focus_m: 0.0, ..self.lens };
        }
        if !self.overlay {return;}
        if !self.dragging {
            match self.window.pointer(size,frame,[720.*frame.scale,150.*frame.scale],p.position,p.pressed,p.held) {
                render::editor_window::WindowEvent::Close=>{self.exit=true;return;},
                render::editor_window::WindowEvent::Handled=>return,
                _=>{}
            }
            if !p.position.is_some_and(|xy|render::editor_style::EditorButton(frame.body()).contains(xy)){return;}
        }
        let [bx,by,w,h]=frame.body();
        let releasing = self.dragging && !p.held;
        let was_dragging = self.dragging;
        if releasing { self.dragging = false; self.scrub_changed = None; }
        let Some([x, y]) = p.position.filter(|_| p.pressed || was_dragging) else { return };
        let (x,y)=(x-bx,y-by);
        let s=frame.scale.min(w/760.).min(h/124.);
        if !was_dragging && y < h - 118.0 * s { return; }
        if was_dragging || y > h - 48.0 * s {
            self.dragging = p.held;
            let t = ((x - 32.0 * s) / (w - 64.0 * s)).clamp(0.0, 1.0);
            let mark = self.marks.iter().chain(std::iter::once(&self.tape.mark)).copied().find(|m| {
                let mx = 32.0 * s + (w - 64.0 * s) * m.frame as f32 / (self.tape.len() - 1).max(1) as f32;
                (mx - x).abs() <= 8.0 * s
            }).unwrap_or(Mark { frame: (t * (self.tape.len() - 1) as f32).round() as usize, alpha: 1.0 });
            if mark != self.target {
                if let Err(e) = self.seek(mark) { self.message = Some(e); }
                if !releasing { self.scrub_changed = Some(std::time::Instant::now()); }
            }
        } else if p.pressed {
            match (x / s) as u32 { 20..=150 => self.toggle(), 151..=245 => self.step(-1), 246..=340 => self.step(1), 341..=520 => { let _ = self.bookmark(); }, 521..=700 => { if let Err(e) = self.seek(self.tape.mark) { self.message = Some(e); } }, _ => {} }
        }
    }
    fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> {
        let now = std::time::Instant::now();
        let was_playing = !self.paused;
        let clock_running = self.clock.is_some();
        self.advance_clock(now);
        let coalescing = self.dragging && self.scrub_changed.is_some_and(|t| t.elapsed() < SCRUB_DEBOUNCE);
        let dt = frame.dt;
        let alpha = frame.alpha;
        frame.dt = 0.0;
        frame.alpha = 1.0;
        if !coalescing {
            if self.current.is_some_and(|n| self.target.frame < n) {
                let fresh = Self::game(&self.tape)?;
                // Same immutable geometry and reserved pools: retain the GPU
                // renderer instead of recompiling/rebaking it on every rewind.
                let same_layout = self.game.vis.scene.same_reserved_geometry(&fresh.vis.scene);
                self.game = fresh;
                self.game.vis.stadium.sound.replay_audio(self.crowd.as_ref(), false);
                if same_layout { self.game.replay_reuse_renderer(frame)?; self.rewind_reuses += 1; }
                self.current = None;
            }
            let started = std::time::Instant::now();
            for i in self.current.map_or(0, |n| n + 1)..=self.target.frame {
                let sample = self.tape.sample(i)?;
                let (chunk, offset) = self.tape.locate(i)?;
                let arena = (offset == 0 && (i == 0 || self.tape.chunks[chunk].reset))
                    .then(|| decode::<Arena>(&self.tape.chunks[chunk].arena)).transpose()?;
                let audible = was_playing && clock_running && self.target.frame.saturating_sub(i) < 3;
                self.game.vis.stadium.sound.replay_audio(self.crowd.as_ref(), audible);
                if let Some((tx, _)) = &self.crowd { tx.volume(if audible { 1.0 } else { 0.0 }); }
                self.game.replay_apply(sample, arena)?;
                self.game.replay_gpu_tick(frame)?;
                self.current = Some(i);
                // No seek/catch-up sound storm. Audible events belong only to
                // ticks close to the running clock, never a paused restore.
                if audible { self.play_audio(); }
                if started.elapsed() >= SEEK_BUDGET { break; }
            }
        }
        if self.paused && self.audio_running { self.silence(); }
        frame.alpha = if self.ready() { self.target.alpha } else { 1.0 };
        let mut draw = self.game.draw(frame)?;
        if let Some(renderer) = frame.renderer() { renderer.set_depth_of_field(frame.gpu, &self.lens.settings(draw.cameras.len())); }
        frame.dt = dt;
        frame.alpha = alpha;
        if self.overlay {
            use crate::hud::style::{Label, BODY};
            let window=self.layout(frame.size);
            let first_text=draw.hud.text.len();
        window.draw(&mut draw.hud,0x6767_5230,11,"Replay");
            let [bx,by,w,h]=window.body();
            let s=window.scale.min(w/760.).min(h/124.);
            let clip=window.body();
            let state = if self.dragging { "SCRUBBING" } else if !self.ready() { "RESTORING EFFECTS" } else if self.paused { "PAUSED" } else { "PLAYING" };
            let n = self.current.unwrap_or(0);
            let label = format!("REPLAY  /  {state}     frame {n} / {}     {:.2}s     tick {}", self.tape.len() - 1, n as f32 * DT, self.game.world.tick);
            Label::new(BODY, render::editor_style::font_size(draw.hud.size), [0.78, 0.86, 0.95, 1.0]).draw(&mut draw.hud, &label, bx+32.0 * s, by+h - 103.0 * s, clip);
            for (left, right, text) in [(20.0, 150.0, if self.paused { "PLAY [SPACE]" } else { "PAUSE [SPACE]" }), (151.0, 245.0, "< FRAME"), (246.0, 340.0, "FRAME >"), (341.0, 520.0, "SAVE FRAME [R]"), (521.0, 700.0, "F4 MOMENT")] {
                draw.hud.rect([bx+left * s, by+h - 88.0 * s, (right - left - 5.0) * s, 36.0 * s], [0.12, 0.18, 0.26, 1.0], clip);
                Label::new(BODY, render::editor_style::font_size(draw.hud.size), [1.0; 4]).center().draw(&mut draw.hud, text, bx+(left + right - 5.0) * 0.5 * s, by+h - 69.0 * s, clip);
            }
            Label::new(BODY, render::editor_style::font_size(draw.hud.size), [0.6, 0.72, 0.84, 1.0]).right().draw(&mut draw.hud, self.message.as_deref().unwrap_or(&format!("Drag timeline to seek   |   {}   |   F11 / Esc closes replay", self.lens.label())), bx+w - 32.0 * s, by+h - 44.0 * s, clip);
            let bar = [bx+32.0 * s, by+h - 30.0 * s, w - 64.0 * s, 5.0 * s];
            draw.hud.rect(bar, [0.18, 0.23, 0.3, 1.0], clip);
            let x = |m: Mark| bar[0] + bar[2] * m.frame as f32 / (self.tape.len() - 1).max(1) as f32;
            for m in self.marks.iter().chain(std::iter::once(&self.tape.mark)) { draw.hud.rect([x(*m) - 2.0 * s, bar[1] - 5.0 * s, 4.0 * s, 15.0 * s], [1.0, 0.72, 0.24, 1.0], clip); }
            draw.hud.rect([x(self.target) - 4.0 * s, bar[1] - 7.0 * s, 8.0 * s, 19.0 * s], [0.3, 0.78, 1.0, 1.0], clip);
        render::editor_style::style_text(&mut draw.hud, first_text);
        }
        Ok(draw)
    }
    fn split(&self) -> render::SplitLayout { self.game.split() }
    fn state(&self) -> Option<String> {
        let mut value: serde_json::Value = serde_json::from_str(&self.game.state()?).ok()?;
        value["replay"] = serde_json::json!({"frame": self.current, "target": self.target.frame, "alpha": self.target.alpha,
            "paused": self.paused, "ready": self.ready(), "dragging": self.dragging, "rewind_reuses": self.rewind_reuses, "lens": self.lens});
        Some(value.to_string())
    }
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
        .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok()).ok_or("No saved replays yet. Press F4 during play.".into())
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
    if let Some(f) = args.value("--lens") {
        let f_stop: f32 = f.parse().map_err(|_| "--lens takes an f-number, such as 2.8")?;
        let focus = match args.value("--focus") {
            Some(v) => { let (x, y) = v.split_once(',').ok_or("--focus takes X,Y fractions, such as 0.5,0.4")?;
                [x.parse().map_err(|_| "--focus X must be a number")?, y.parse().map_err(|_| "--focus Y must be a number")?] }
            None => [0.5, 0.5],
        };
        viewer.lens = Lens { on: f_stop > 0.0, f_stop, focus, focus_m: args.num("--focus-m", 0.0) };
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
        let samples = crate::sounds::find_dir(args.value("--sounds"));
        let music = crate::play::music(args, samples.as_deref(), viewer.game.world.arena.seed, true);
        let _speaker = match crate::sounds::speakers(args.value("--speakers"), music.as_ref()) {
            Ok((speaker, tx, crowd)) => {
                viewer.game.set_crowd(args.value("--crowd"), samples.as_deref(), &crowd);
                viewer.set_crowd(viewer.game.vis.stadium.sound.audio_bank());
                let mut sounds = crate::sounds::Sounds::new(tx, crate::sounds::find_patches(args.value("--sfx")), samples.as_deref());
                sounds.music = music.clone();
                viewer.set_audio(Some(sounds), music, -9.0);
                Some(speaker)
            }
            Err(e) => { eprintln!("silent replay: {e}"); None }
        };
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
    fn v2_replay_keeps_the_fields_after_the_snapshot() {
        let mut world = World::new(3, &[false, true]);
        world.release_cage();
        let mut recorder = Recorder::default();
        recorder.push(&world, &[1, 0], "noon", true);
        let mut tape = recorder.tape(1., [960, 540]).unwrap();
        let sample = tape.sample(0).unwrap();
        let mut legacy = encode(&sample.state).unwrap();
        // In V3 an empty landing_spots takes one byte at the end of the snapshot.
        assert_eq!(legacy.pop(), Some(0));
        legacy.extend(encode(&(sample.broken, sample.slots, sample.sky, sample.tilt)).unwrap());
        tape.version = 2;
        let restored = tape.decode_frame(&legacy).unwrap();
        assert_eq!(restored.slots, [1, 0]);
        assert_eq!(restored.sky, "noon");
        assert_eq!(encode(&restored.state).unwrap(), encode(&world.snapshot()).unwrap());
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

    #[test]
    fn clock_drag_and_audio_follow_the_recording() {
        let mut world = World::new(3, &[false, true]);
        let mut recorder = Recorder::default();
        for _ in 0..300 { world.step(&[]); recorder.push(&world, &[0], "godot", true); }
        let tape = recorder.tape(1.0, [1600, 900]).unwrap();
        let mut viewer = Viewer::new(tape, std::env::temp_dir().join("replay-controls-test.ggreplay")).unwrap();
        viewer.current = Some(20);
        viewer.target = Mark { frame: 20, alpha: 0.25 };
        viewer.toggle();
        assert!(!viewer.fixed_quality(), "normal adaptive quality during playback");
        let now = std::time::Instant::now();
        viewer.advance_clock(now);
        viewer.advance_clock(now + std::time::Duration::from_millis(250));
        assert_eq!(viewer.target, Mark { frame: 35, alpha: 0.25 });
        // Still only frame 20 rendered: time must nevertheless keep advancing.
        for _ in 0..10 { viewer.tick(&[Controls::default()]); }
        viewer.advance_clock(now + std::time::Duration::from_millis(500));
        assert_eq!(viewer.target, Mark { frame: 50, alpha: 0.25 });
        viewer.pointer(Pointer { position: Some([300.0, 880.0]), pressed: true, held: true, ..Default::default() }, [1600, 900]);
        let first = viewer.target;
        viewer.pointer(Pointer { position: Some([900.0, 880.0]), moved: true, held: true, ..Default::default() }, [1600, 900]);
        assert!(viewer.target.frame > first.frame, "held drag moves without another press");
        assert!(viewer.paused && viewer.dragging);
        viewer.pointer(Pointer { position: Some([2000.0, 100.0]), ..Default::default() }, [1600, 900]);
        assert_eq!(viewer.target.frame, 299, "drag keeps capture outside timeline");
        assert!(!viewer.dragging);
        assert_eq!(viewer.current, Some(20), "input only queues seeks; no expensive scene build");
        assert_eq!(mark_at_phase(45.0, 300), Mark { frame: 44, alpha: 1.0 });
        assert_eq!(mark_at_phase(999.0, 300), Mark { frame: 299, alpha: 1.0 });

        let (tx, mut mix) = audio::synth::mixer(audio::synth::Layout::stereo(), 160, audio::SAMPLE_RATE);
        viewer.set_audio(Some(crate::sounds::Sounds::new(tx, crate::sounds::find_patches(None), None)), None, -9.0);
        let player = &viewer.game.world.players[0];
        viewer.game.world.events = vec![growing_guns::Event::Shot { player: 0, from: player.pos, dir: glam::Vec3::NEG_Z }];
        viewer.play_audio();
        let mut pcm = vec![0.0; 4800 * 2];
        mix.render(&mut pcm, 2);
        assert!(pcm.iter().any(|x| x.abs() > 0.001), "recorded shot produces real PCM");
        viewer.seek(Mark { frame: 0, alpha: 1.0 }).unwrap();
        mix.render(&mut pcm, 2);
        assert_eq!(mix.active(), 0, "seeking stops old voices, including loops");
        assert!(viewer.take_sounds().is_some(), "live game gets its mixer controller back");
    }

    // Permit tiny GPU rasterization differences on isolated spectator pixels,
    // not moving effects. Shard transforms are compared exactly below.
    fn assert_same_image(a: &[u8], b: &[u8], label: &str) {
        assert_eq!(a.len(), b.len());
        let pixels = a.chunks_exact(4).zip(b.chunks_exact(4)).filter(|(a, b)| a != b).count();
        let error: usize = a.iter().zip(b).map(|(a, b)| a.abs_diff(*b) as usize).sum();
        assert!(pixels <= 24 && error <= 256, "{label}: {pixels} pixels differ, total channel error {error}");
    }

    #[test]
    fn lens_stops_and_settings() {
        let mut lens = Lens::default();
        assert!(lens.settings(1).iter().all(|s| !s.active()), "off by default");
        lens.on = true;
        lens.stop(-1); assert_eq!(lens.f_stop, 2.0);
        for _ in 0..20 { lens.stop(1); }
        assert_eq!(lens.f_stop, 22.0);
        lens.focus = [0.25, 0.75];
        assert_eq!(lens.settings(1)[0].focus_point, [0.25, 0.75]);
        assert_eq!(lens.settings(2)[1].focus_point, [0.5, 0.5]);
    }

    /// Review lens on a recorded frame: off is bit-identical before and after,
    /// on blurs away from focus. Writes build/depth-of-field/*.png.
    #[test]
    #[ignore = "GPU; run serially without another game rendering"]
    fn lens_gpu_review() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../build/depth-of-field");
        std::fs::create_dir_all(&root).unwrap();
        let mut game = Gg::new(World::new(3, &[false, true, true, true]), Session::Local, vec![0]);
        for cmd in ["bots pause", "health 0 50000"] { game.command(cmd).unwrap(); }
        for _ in 0..90 { game.tick(&[Controls::default()]); }
        let size = [1280, 720];
        let tape = game.recorder.tape(1.0, size).unwrap();
        let mut viewer = Viewer::new(tape, root.join("lens.ggreplay")).unwrap();
        viewer.overlay = false;
        let gpu = render::Gpu::new().unwrap();
        let mut headless = app::Headless::new(&gpu, size, TICK_HZ as f32).unwrap();
        let mut ready = |v: &mut Viewer| loop { let rgba = headless.frame(v).unwrap(); if v.ready() { break rgba; } };
        // Warm up history-based lighting before comparing paused frames.
        for _ in 0..8 { ready(&mut viewer); }
        let sharp = ready(&mut viewer);
        assert_same_image(&sharp, &ready(&mut viewer), "paused frame animated");
        render::write_png(&root.join("off.png"), size[0], size[1], &sharp).unwrap();
        let focus = std::env::var("GG_LENS_FOCUS").ok().and_then(|v| { let (x, y) = v.split_once(',')?; Some([x.parse().ok()?, y.parse().ok()?]) }).unwrap_or([0.5, 0.55]);
        for f_stop in [1.4, 4.0] {
            viewer.lens = Lens { on: true, f_stop, focus, focus_m: 0.0 };
            let soft = ready(&mut viewer);
            render::write_png(&root.join(format!("f{f_stop}.png")), size[0], size[1], &soft).unwrap();
            let changed = soft.chunks(4).zip(sharp.chunks(4)).filter(|(a, b)| a.iter().zip(*b).any(|(x, y)| x.abs_diff(*y) > 8)).count();
            println!("f/{f_stop}: {changed} pixels changed");
            assert!(changed > 1000, "lens at f/{f_stop} changed only {changed} pixels");
        }
        viewer.lens.on = false;
        let off_again = ready(&mut viewer);
        assert_same_image(&sharp, &off_again, "lens off must restore the sharp frame");
    }

    /// Serial Metal fixture: exercises F4's screenshot/replay hook, rewinds,
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
        assert_same_image(&clean, &again, "paused frame animated");
        render::write_png(&root.join("clean.png"), 1600, 900, &clean).unwrap();
        viewer.seek(Mark { frame: 90, alpha: 1.0 }).unwrap();
        render_ready(&mut headless, &mut viewer);
        viewer.seek(tape.mark).unwrap();
        let restored = render_ready(&mut headless, &mut viewer);
        assert_eq!(viewer.rewind_reuses, 1, "rewind must retain the existing GPU scene");
        render::write_png(&root.join("restored.png"), 1600, 900, &restored).unwrap();
        assert_same_image(&clean, &restored, "backward/forward seek changed the marked frame");
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
        assert_same_image(&before, &paused, "GPU debris moved while paused");
        let debris_before = headless.renderer().unwrap().debris_debug_instances(&gpu).unwrap();
        impact.seek(Mark { frame: 40, alpha: 1.0 }).unwrap();
        render_ready(&mut headless, &mut impact);
        impact.seek(impact_tape.mark).unwrap();
        let after = render_ready(&mut headless, &mut impact);
        assert_same_image(&before, &after, "GPU debris changed after rewind");
        let debris_after = headless.renderer().unwrap().debris_debug_instances(&gpu).unwrap();
        assert_eq!(debris_before.len(), debris_after.len());
        for (a, b) in debris_before.iter().zip(&debris_after) {
            assert_eq!(a.rows, b.rows, "GPU shard position/orientation changed after rewind");
            assert_eq!((a.first_cluster, a.cluster_count), (b.first_cluster, b.cluster_count));
        }

        // Exercise the actual held-pointer path and repaint its selected frame.
        impact.overlay = true;
        for (x, pressed, held) in [(1000.0, true, true), (600.0, false, true), (600.0, false, false)] {
            impact.pointer(Pointer { position: Some([x, 880.0]), pressed, held, moved: true, ..Default::default() }, [1600, 900]);
        }
        let selected = impact.target;
        let scrubbed = render_ready(&mut headless, &mut impact);
        assert_eq!(impact.current, Some(selected.frame));
        assert_eq!(impact.game.world.tick, impact.tape.sample(selected.frame).unwrap().state.tick);
        render::write_png(&root.join("scrubbed.png"), 1600, 900, &scrubbed).unwrap();

        // A real wall-clock playback, including rendering/readback latency.
        impact.seek(Mark { frame: 0, alpha: 1.0 }).unwrap();
        render_ready(&mut headless, &mut impact);
        impact.toggle();
        let began = std::time::Instant::now();
        impact.clock = Some((began, 1.0));
        let mut paints = 0;
        let mut painted_at = 0.0;
        while began.elapsed().as_secs_f64() < 1.5 {
            painted_at = began.elapsed().as_secs_f64();
            headless.frame(&mut impact).unwrap();
            paints += 1;
        }
        let replay_seconds = (impact.target.frame as f64 + impact.target.alpha as f64 - 1.0) / TICK_HZ as f64;
        assert!((replay_seconds - painted_at).abs() < DT as f64, "playback drifted from real time");
        assert!(impact.target.frame.saturating_sub(impact.current.unwrap()) <= 12, "presentation fell far behind its clock");
        std::fs::write(root.join("timing.json"), serde_json::to_vec_pretty(&serde_json::json!({
            "wall_seconds": painted_at, "replay_seconds": replay_seconds, "paints": paints,
            "lag_ticks": impact.target.frame.saturating_sub(impact.current.unwrap()), "rewind_reuses": impact.rewind_reuses,
            "includes_readback": true
        })).unwrap()).unwrap();
        impact.toggle();

        // The replay's real audio mixer, rendered offline for inspection.
        let (mut source, tx, _) = crate::sounds::offline(None);
        impact.set_audio(Some(crate::sounds::Sounds::new(tx, crate::sounds::find_patches(None), crate::sounds::find_dir(None).as_deref())), None, -9.0);
        let mut pcm = Vec::new();
        for i in 0..impact.tape.len() {
            let sample = impact.tape.sample(i).unwrap();
            let (chunk, offset) = impact.tape.locate(i).unwrap();
            if offset == 0 && (i == 0 || impact.tape.chunks[chunk].reset) { impact.game.world.arena = impact.tape.arena(i).unwrap(); }
            impact.game.world.arena.apply_breakage(&sample.broken);
            impact.game.world.apply(sample.state);
            impact.game.slots = sample.slots;
            impact.play_audio();
            let mut tick = vec![0.0; (audio::SAMPLE_RATE as usize / TICK_HZ as usize) * 2];
            source.render(&mut tick, 2);
            pcm.extend(tick);
        }
        assert!(pcm.iter().any(|v| v.abs() > 0.01));
        audio::analysis::write_wav(&root.join("replay-audio.wav"), audio::SAMPLE_RATE as u32, 2, &pcm).unwrap();
        impact.silence();
        assert!(game.function_key(11));
        assert_eq!(game.mouse_capture(), Some(false));
        assert!(game.fixed_quality());
        assert!(game.escape());
        assert!(!game.fixed_quality());
        game.recorder.last = None; // A fresh process has no in-memory capture.
        assert!(game.function_key(11));
        assert!(game.fixed_quality(), "F11 did not reopen the saved file");
        assert!(game.escape());
        println!("GPU review saved to {}", root.display());
    }
}
