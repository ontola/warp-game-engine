//! The engine's F3 stats overlay, for every game on the shell: what a frame
//! costs while you play. A nested GPU tree (feature group, pass, authored
//! sub-scopes) like a profiler's, the CPU side of the frame, the renderer's
//! resolution and memory, and whatever the game adds with
//! [`crate::Game::debug_stats`].
//!
//! Cheap by construction:
//! - Closed, it costs nothing: GPU timestamps stay off unless something else
//!   (F10, a frame log, a probe) asked for them.
//! - Open, GPU timing is two timestamp writes per pass, read back a few
//!   frames later from a four-slot ring that drops a sample rather than
//!   wait ([`render::gpu_profile`]). The CPU never blocks on the GPU.
//! - Numbers are smoothed every frame but the text only changes four times a
//!   second, so the HUD reshapes a handful of runs per refresh, not per frame.
//! - The GPU memory report (a list of every allocation) is asked for once a
//!   second at most.
//!
//! The overlay measures its own CPU time and shows it on its last line.
//!
//! Keys: F3 opens and closes it (unless the game claims F3, see
//! [`crate::Game::function_key`]); Shift+F3 cycles the tree depth.
//! `WARP_STATS=1` opens it at start, `WARP_STATS=3` at full depth.
use render::debug_timing::pass_group;
use render::gpu_profile::{interval_union_ms, GpuProfileSample};
use render::hud::{FontFace, HudDrawList, TextAlign, TextRun};
use std::collections::{HashMap, VecDeque};
use web_time::Instant;

/// HUD text ids from here up are this overlay's.
const TEXT_ID: u64 = u64::MAX - 20_000;
/// Exponential smoothing per sample: about the last ten frames.
const SMOOTH: f64 = 0.1;
/// Seconds between text refreshes.
const REFRESH: f32 = 0.25;
/// Frames kept for the worst frame time.
const WINDOW: usize = 120;
/// Tree rows below this many milliseconds fold into one "smaller" row.
const TINY_MS: f64 = 0.02;

use render::editor_style::{FG, MUTED, ACCENT};
const WARN: [u8; 4] = [255, 190, 90, 255];
const HOT: [u8; 4] = [255, 110, 90, 255];

/// A node of the GPU tree: a feature group, a pass in it, or an authored
/// scope inside that pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Key { group: &'static str, pass: Option<&'static str>, scope: Option<&'static str> }

impl Key {
    fn depth(&self) -> usize { if self.scope.is_some() { 2 } else if self.pass.is_some() { 1 } else { 0 } }
}

/// One GPU sample as tree nodes with this frame's milliseconds. Groups are
/// the union of their passes' intervals (passes on one queue can overlap,
/// so a sum could exceed the frame); a pass is the sum of its same-named
/// intervals across views; a scope sits under the root pass that holds it.
fn sample_nodes(sample: &GpuProfileSample) -> Vec<(Key, f64)> {
    let roots: Vec<usize> = (0..sample.intervals.len()).map(|mut i| {
        // Authored scopes can nest; everything below a root folds into it.
        for _ in 0..sample.intervals.len() {
            match sample.intervals[i].parent { Some(p) if p < sample.intervals.len() && p != i => i = p, _ => break }
        }
        i
    }).collect();
    let mut nodes: Vec<(Key, f64)> = Vec::new();
    let mut add = |key: Key, ms: f64| match nodes.iter_mut().find(|n| n.0 == key) {
        Some(n) => n.1 += ms,
        None => nodes.push((key, ms)),
    };
    let mut groups: Vec<(&'static str, Vec<(f64, f64)>)> = Vec::new();
    for (i, interval) in sample.intervals.iter().enumerate() {
        let ms = (interval.end_ms - interval.start_ms).max(0.0);
        if !ms.is_finite() { continue; }
        let root = &sample.intervals[roots[i]];
        let group = pass_group(root.name);
        if roots[i] == i {
            add(Key { group, pass: Some(interval.name), scope: None }, ms);
            match groups.iter_mut().find(|g| g.0 == group) {
                Some(g) => g.1.push((interval.start_ms, interval.end_ms)),
                None => groups.push((group, vec![(interval.start_ms, interval.end_ms)])),
            }
        } else {
            add(Key { group, pass: Some(root.name), scope: Some(interval.name) }, ms);
        }
    }
    for (group, spans) in groups {
        if let Some(ms) = interval_union_ms(spans) { nodes.push((Key { group, pass: None, scope: None }, ms)); }
    }
    nodes
}

/// A line of the overlay as it is shown until the next refresh.
#[derive(Clone, Debug, PartialEq)]
struct Line { depth: usize, label: String, value: String, color: [u8; 4], bar: f32, heading: bool }

impl Line {
    fn heading(label: &str) -> Self { Line { depth: 0, label: label.into(), value: String::new(), color: ACCENT, bar: 0.0, heading: true } }
    fn pair(label: impl Into<String>, value: impl Into<String>) -> Self { Line { depth: 0, label: label.into(), value: value.into(), color: FG, bar: 0.0, heading: false } }
}

/// What the renderer looks like this frame, asked for only while open.
#[derive(Clone, Debug, Default)]
pub struct RenderInfo {
    pub adapter: String,
    pub backend: String,
    pub output: [u32; 2],
    /// Each view's rendered size and the size it is shown at.
    pub views: Vec<([u32; 2], [u32; 2])>,
    pub upscale: String,
    /// Voxel forest pages: resident, pending, bytes.
    pub foliage_pages: Option<(u32, u32, u64)>,
}

#[derive(Default)]
pub struct Stats {
    window: render::editor_window::EditorWindow,
    scroll: usize,
    open: bool,
    /// Tree depth shown: 1 groups, 2 passes, 3 authored scopes.
    detail: usize,
    gpu: HashMap<Key, f64>,
    gpu_total: Option<f64>,
    gpu_worst: VecDeque<f64>,
    gpu_frame: Option<u64>,
    gpu_samples: u64,
    timestamps: usize,
    cpu: Vec<(&'static str, f64)>,
    frame_ms: Option<f64>,
    frames: VecDeque<f64>,
    info: RenderInfo,
    memory: Option<(u64, u64, usize)>,
    memory_at: Option<Instant>,
    /// The overlay's own CPU milliseconds: this frame so far, and smoothed.
    own_frame_ms: f64,
    own_ms: f64,
    lines: Vec<Line>,
    refreshed: Option<Instant>,
}

fn smooth(old: &mut f64, new: f64) { *old += (new - *old) * SMOOTH; }

fn ms(v: f64) -> String { if v >= 10.0 { format!("{v:.1}") } else { format!("{v:.2}") } }

fn bytes(b: u64) -> String {
    let b = b as f64;
    if b >= 1e9 { format!("{:.2} GB", b / 1e9) } else { format!("{:.0} MB", b / 1e6) }
}

impl Stats {
    /// Closed, or open as `WARP_STATS` says (1..=3 is the depth).
    pub fn from_env() -> Self {
        let mut stats = Stats { detail: 2, ..Default::default() };
        if let Some(depth) = std::env::var("WARP_STATS").ok().and_then(|v| v.parse::<usize>().ok()).filter(|d| *d > 0) {
            stats.open = true;
            stats.detail = depth.clamp(1, 3);
        }
        stats
    }

    pub fn open(&self) -> bool { self.open }

    /// F3: open or close. Closing forgets the measurements, so reopening
    /// never shows a stale frame.
    pub fn toggle(&mut self) {
        self.window.cancel();
        let detail = self.detail.max(1);
        *self = Stats { open: !self.open, detail, window: self.window, ..Default::default() };
    }

    /// Shift+F3: one level deeper, back to groups after scopes. Opens a
    /// closed overlay.
    pub fn cycle_detail(&mut self) {
        if !self.open { self.toggle(); return; }
        self.detail = self.detail % 3 + 1;
        self.refreshed = None;
    }

    /// GPU samples drained this frame (they are a few frames old).
    pub fn gpu(&mut self, samples: &[GpuProfileSample]) {
        if !self.open { return; }
        let started = Instant::now();
        for sample in samples {
            let nodes = sample_nodes(sample);
            for value in self.gpu.values_mut() { *value *= 1.0 - SMOOTH; }
            for (key, ms) in nodes {
                match self.gpu.get_mut(&key) {
                    Some(value) => *value += ms * SMOOTH,
                    None => { self.gpu.insert(key, ms); }
                }
            }
            self.gpu.retain(|_, v| *v >= 0.0005);
            let total = sample.covered_union_ms;
            match &mut self.gpu_total { Some(t) => smooth(t, total), None => self.gpu_total = Some(total) }
            self.gpu_worst.push_back(total);
            if self.gpu_worst.len() > WINDOW { self.gpu_worst.pop_front(); }
            self.gpu_frame = Some(sample.frame_id);
            self.gpu_samples += 1;
            self.timestamps = sample.intervals.iter().filter(|i| i.parent.is_none()).count() * 2;
        }
        self.own_frame_ms += started.elapsed().as_secs_f64() * 1000.0;
    }

    /// One frame's CPU side: the time since the previous frame and where the
    /// shell spent it.
    pub fn cpu(&mut self, frame_ms: f64, parts: &[(&'static str, f64)]) {
        if !self.open || !frame_ms.is_finite() { return; }
        match &mut self.frame_ms { Some(f) => smooth(f, frame_ms), None => self.frame_ms = Some(frame_ms) }
        self.frames.push_back(frame_ms);
        if self.frames.len() > WINDOW { self.frames.pop_front(); }
        for &(name, value) in parts {
            if !value.is_finite() { continue; }
            match self.cpu.iter_mut().find(|p| p.0 == name) {
                Some(p) => smooth(&mut p.1, value),
                None => self.cpu.push((name, value)),
            }
        }
    }

    /// Whether [`Self::set_info`] would be used now: only right before a refresh.
    pub fn wants_info(&self) -> bool { self.open && self.refreshed.is_none_or(|t| t.elapsed().as_secs_f32() >= REFRESH) }
    pub fn set_info(&mut self, info: RenderInfo) { self.info = info; }

    /// GPU memory from the allocator report, once a second while open.
    /// Backends without sub-allocation (Metal, WebGPU) have no report.
    pub fn memory(&mut self, gpu: &render::Gpu) {
        if !self.open || self.memory_at.is_some_and(|t| t.elapsed().as_secs_f32() < 1.0) { return; }
        let started = Instant::now();
        self.memory_at = Some(started);
        self.memory = gpu.device.generate_allocator_report().map(|r| (r.total_allocated_bytes, r.total_reserved_bytes, r.allocations.len()));
        self.own_frame_ms += started.elapsed().as_secs_f64() * 1000.0;
    }

    fn refresh(&mut self, game: &[(String, String)]) {
        let mut lines = Vec::new();
        let frame = self.frame_ms.unwrap_or(0.0);
        let worst = self.frames.iter().copied().fold(0.0, f64::max);
        let heat = |v: f64, budget: f64| if v > budget * 1.5 { HOT } else if v > budget { WARN } else { FG };
        let budget = 1000.0 / 60.0;
        lines.push(Line { color: heat(frame, budget), ..Line::pair("Frame", if frame > 0.0 { format!("{} ms  {:.0} fps  worst {} ms", ms(frame), 1000.0 / frame, ms(worst)) } else { "--".into() }) });
        let cpu_sum: f64 = self.cpu.iter().map(|p| p.1).sum();
        lines.push(Line::pair("CPU", format!("{} ms on the main thread", ms(cpu_sum))));
        let gpu = match self.gpu_total {
            Some(t) => format!("{} ms  worst {} ms", ms(t), ms(self.gpu_worst.iter().copied().fold(0.0, f64::max))),
            None => "waiting for timestamps (none on this GPU?)".into(),
        };
        lines.push(Line { color: self.gpu_total.map_or(MUTED, |t| heat(t, budget)), ..Line::pair("GPU", gpu) });
        let i = &self.info;
        if !i.adapter.is_empty() { lines.push(Line { color: MUTED, ..Line::pair("Device", format!("{} ({})", i.adapter, i.backend)) }); }
        for (n, (input, output)) in i.views.iter().enumerate() {
            let label = if i.views.len() > 1 { format!("View {}", n + 1) } else { "Render".into() };
            let scale = if input == output { "native".into() } else { format!("{} to {}x{}", i.upscale, output[0], output[1]) };
            lines.push(Line::pair(label, format!("{}x{} {scale}", input[0], input[1])));
        }
        if i.output != [0, 0] && i.views.iter().all(|(_, o)| o[1] < i.output[1]) {
            lines.push(Line::pair("Window", format!("{}x{}, the picture scaled up", i.output[0], i.output[1])));
        }
        if let Some((allocated, reserved, count)) = self.memory {
            lines.push(Line::pair("GPU memory", format!("{} in {count} allocations, {} reserved", bytes(allocated), bytes(reserved))));
        }
        if let Some((resident, pending, b)) = i.foliage_pages {
            lines.push(Line::pair("Forest pages", format!("{resident} resident, {pending} pending, {}", bytes(b))));
        }

        lines.push(Line::heading(&format!("GPU  (depth {} of 3, Shift+F3)", self.detail)));
        let total = self.gpu_total.unwrap_or(0.0).max(0.01);
        let mut keys: Vec<(Key, f64)> = self.gpu.iter().map(|(k, v)| (*k, *v)).collect();
        let cost = |k: Key| self.gpu.get(&k).copied().unwrap_or(0.0);
        // Sort by the group's cost, then the pass's, then the scope's, so
        // children follow their parent in descending order.
        keys.sort_by(|(a, va), (b, vb)| {
            let ga = cost(Key { group: a.group, pass: None, scope: None });
            let gb = cost(Key { group: b.group, pass: None, scope: None });
            let pa = a.pass.map_or(f64::INFINITY, |p| cost(Key { group: a.group, pass: Some(p), scope: None }));
            let pb = b.pass.map_or(f64::INFINITY, |p| cost(Key { group: b.group, pass: Some(p), scope: None }));
            gb.total_cmp(&ga).then(a.group.cmp(b.group))
                .then(pb.total_cmp(&pa)).then(a.pass.cmp(&b.pass))
                .then(a.scope.is_some().cmp(&b.scope.is_some())).then(vb.total_cmp(va)).then(a.scope.cmp(&b.scope))
        });
        let mut tiny = (0usize, 0.0);
        for (key, value) in keys {
            if key.depth() >= self.detail { continue; }
            if key.depth() > 0 && value < TINY_MS { tiny.0 += 1; tiny.1 += value; continue; }
            let label = key.scope.or(key.pass).unwrap_or(key.group);
            let color = if key.depth() == 0 { FG } else if value >= 1.0 { FG } else { MUTED };
            lines.push(Line { depth: key.depth(), label: label.into(), value: ms(value), color, bar: (value / total).clamp(0.0, 1.0) as f32, heading: false });
        }
        if tiny.0 > 0 { lines.push(Line { depth: 1, label: format!("{} smaller passes", tiny.0), value: ms(tiny.1), color: MUTED, bar: 0.0, heading: false }); }
        if self.gpu_samples > 0 {
            lines.push(Line { color: MUTED, ..Line::pair(format!("{} timestamps a frame, read back without waiting (frame {})", self.timestamps, self.gpu_frame.unwrap_or(0)), "") });
        }

        lines.push(Line::heading("CPU"));
        let mut cpu = self.cpu.clone();
        cpu.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (name, value) in cpu {
            lines.push(Line { depth: 1, label: name.into(), value: ms(value), color: if value >= 1.0 { FG } else { MUTED }, bar: (value / frame.max(0.01)).clamp(0.0, 1.0) as f32, heading: false });
        }
        let waited = frame - cpu_sum;
        if frame > 0.0 && waited > 0.0 { lines.push(Line { depth: 1, label: "waiting (vsync, GPU, OS)".into(), value: ms(waited), color: MUTED, bar: 0.0, heading: false }); }

        if !game.is_empty() {
            lines.push(Line::heading("Game"));
            for (label, value) in game { lines.push(Line::pair(label.clone(), value.clone())); }
        }
        lines.push(Line { color: MUTED, ..Line::pair("F3 stats overlay", format!("{} ms CPU", ms(self.own_ms))) });
        self.lines = lines;
        self.refreshed = Some(Instant::now());
    }

    fn layout(&self,size:[u32;2]) -> render::editor_window::WindowFrame {
        let [w,h]=size.map(|n|n as f32);
        let s=(h/1080.).clamp(0.6,2.).min(w/420.).max(0.01);
        self.window.frame(size,[12.*s,12.*s,560.*s,(h-24.*s).min((80.+21.*self.lines.len().max(6) as f32)*s)], [330.*s,135.*s],s)
    }
    pub fn pointer(&mut self,p:crate::Pointer,size:[u32;2])->bool {
        let f=self.layout(size);
        match self.window.pointer(size,f,[330.*f.scale,135.*f.scale],p.position,p.pressed,p.held) {
            render::editor_window::WindowEvent::Close=>{self.toggle();return true;},
            render::editor_window::WindowEvent::Handled=>return true,
            _=>{}
        }
        if p.position.is_some_and(|xy|render::editor_style::EditorButton(f.rect).contains(xy)) {
            self.scroll=self.scroll.saturating_add_signed((-p.wheel*3.).round() as isize);
            true
        } else {false}
    }
    /// Draw over everything else in `hud`. `game` is
    /// [`crate::Game::debug_stats`], asked for only at a refresh.
    pub fn draw(&mut self, hud: &mut HudDrawList, game: impl FnOnce() -> Vec<(String, String)>) {
        if !self.open || hud.size.contains(&0) { return; }
        let started = Instant::now();
        if self.refreshed.is_none_or(|t| t.elapsed().as_secs_f32() >= REFRESH) { self.refresh(&game()); }
        let frame=self.layout(hud.size);
        let s=frame.scale;
        let (row,font)=(21.*s,16.*s);
        frame.draw(hud,TEXT_ID+12_000,3,"Profiler");
        let [x,top,width,height]=frame.body();
        let fits=((height/row).floor() as usize).saturating_sub(1);
        let shown=self.lines.len().min(fits);
        self.scroll=self.scroll.min(self.lines.len()-shown);
        let clip=frame.body();
        let inner=x+8.*s;
        let right=x+width-8.*s;
        for (n,line) in self.lines.iter().skip(self.scroll).take(shown).enumerate() {
            let y = top + 0.3 * row + n as f32 * row;
            let indent = line.depth as f32 * 14.0 * s;
            if line.bar > 0.0 {
                hud.rect([inner + indent, y + 2.0 * s, (right - inner - indent) * line.bar, row - 4.0 * s], [0.08, 0.22, 0.26, 0.75], clip);
            }
            // Tree rows: name left, milliseconds right. Header rows: a
            // label column, then the value.
            let tree = line.depth > 0 || line.heading || line.bar > 0.0;
            let label_w = if tree { right - inner - indent - 80.0 * s } else if line.value.is_empty() { right - inner } else { 110.0 * s };
            let face = if line.heading || line.depth == 0 && line.bar > 0.0 { FontFace::Title } else { FontFace::Body };
            let id = TEXT_ID + n as u64 * 2;
            let at = [inner + indent + 3.0 * s, y];
            hud.text.push(TextRun { id, text: line.label.clone(), position: at, bounds: [at[0], y, label_w, row],
                font_size: font, color: if line.heading { ACCENT } else { line.color }, font: face, align: TextAlign::Left, width: label_w });
            if !line.value.is_empty() {
                let (align, vx, vw) = if tree { (TextAlign::Right, right - 80.0 * s, 80.0 * s) } else { (TextAlign::Left, inner + 110.0 * s, right - inner - 110.0 * s) };
                hud.text.push(TextRun { id: id + 1, text: line.value.clone(), position: [vx, y], bounds: [vx, y, vw, row],
                    font_size: font, color: line.color, font: FontFace::Body, align, width: vw });
            }
        }
        if shown < self.lines.len() {
            let y = top + 0.3 * row + shown as f32 * row;
            hud.text.push(TextRun { id: TEXT_ID + 10_000, text: format!("Rows {}-{} of {}   |   Wheel to scroll", self.scroll+1,self.scroll+shown,self.lines.len()), position: [inner, y],
                bounds: [inner, y, right - inner, row], font_size: font * 0.9, color: MUTED, font: FontFace::Body, align: TextAlign::Left, width: right - inner });
        }
        // Own cost: this draw plus the sampling earlier this frame.
        let own = std::mem::take(&mut self.own_frame_ms) + started.elapsed().as_secs_f64() * 1000.0;
        smooth(&mut self.own_ms, own);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use render::gpu_profile::GpuPassInterval;

    fn interval(name: &'static str, start_ms: f64, end_ms: f64, parent: Option<usize>, view: Option<u32>) -> GpuPassInterval {
        GpuPassInterval { name, start_ms, end_ms, parent, view_index: view }
    }
    fn sample(intervals: Vec<GpuPassInterval>) -> GpuProfileSample {
        let union = interval_union_ms(intervals.iter().map(|i| (i.start_ms, i.end_ms))).unwrap_or(0.0);
        GpuProfileSample { frame_id: 7, render_sequence: 7, gpu_ms: Vec::new(), instrumented_gpu_ms: 0.0, covered_interval_ms: union,
            covered_union_ms: union, intervals, output_size: [1920, 1080], views: 2 }
    }
    fn node(nodes: &[(Key, f64)], group: &'static str, pass: Option<&'static str>, scope: Option<&'static str>) -> f64 {
        nodes.iter().find(|n| n.0 == Key { group, pass, scope }).map(|n| n.1).unwrap_or_else(|| panic!("no {group}/{pass:?}/{scope:?}"))
    }

    #[test]
    fn passes_nest_under_groups_and_scopes_under_their_root_pass() {
        let nodes = sample_nodes(&sample(vec![
            interval("near shadow raster", 0.0, 1.0, None, Some(0)),
            interval("shadow tile damage and clear", 1.0, 1.5, None, None),
            interval("raster", 2.0, 3.0, None, Some(0)),
            interval("raster", 5.0, 6.5, None, Some(1)),
            interval("meadow aggregate render", 3.0, 5.0, None, Some(0)),
            interval("meadow aggregate depth", 3.0, 3.5, Some(4), Some(0)),
            interval("meadow aggregate material", 3.5, 5.0, Some(4), Some(0)),
            interval("procedural terrain", 6.5, 6.75, None, None),
        ]));
        // Same-named passes across views add up; groups are the union.
        assert_eq!(node(&nodes, "Geometry", Some("raster"), None), 2.5);
        assert_eq!(node(&nodes, "Geometry", None, None), 2.5);
        assert_eq!(node(&nodes, "Shadows", None, None), 1.5);
        assert_eq!(node(&nodes, "Terrain", Some("procedural terrain"), None), 0.25);
        // Children sit under their root and never count in the group twice.
        assert_eq!(node(&nodes, "Vegetation", Some("meadow aggregate render"), Some("meadow aggregate material")), 1.5);
        assert_eq!(node(&nodes, "Vegetation", None, None), 2.0);
        assert!(nodes.iter().all(|(k, _)| k.pass != Some("meadow aggregate depth")));
    }

    #[test]
    fn overlapping_passes_never_make_a_group_longer_than_its_span() {
        let nodes = sample_nodes(&sample(vec![interval("shade", 0.0, 4.0, None, None), interval("shade terrain", 1.0, 3.0, None, None)]));
        assert_eq!(node(&nodes, "Lighting", None, None), 4.0);
    }

    #[test]
    fn closed_overlay_ignores_samples_and_draws_nothing() {
        let mut stats = Stats::default();
        stats.gpu(&[sample(vec![interval("shade", 0.0, 4.0, None, None)])]);
        stats.cpu(16.0, &[("game draw", 2.0)]);
        assert!(stats.gpu.is_empty() && stats.frames.is_empty());
        let mut hud = HudDrawList::new([1920, 1080]);
        stats.draw(&mut hud, || panic!("asked the game while closed"));
        assert!(hud.text.is_empty() && hud.vertices.is_empty());
    }

    #[test]
    fn open_overlay_shows_the_tree_by_depth_and_keeps_text_ids_unique() {
        let mut stats = Stats::default();
        stats.toggle();
        let s = sample(vec![interval("near shadow raster", 0.0, 1.0, None, None), interval("raster", 1.0, 3.0, None, None),
            interval("meadow aggregate render", 3.0, 5.0, None, None), interval("meadow aggregate depth", 3.0, 4.0, Some(2), None)]);
        for _ in 0..30 { stats.gpu(std::slice::from_ref(&s)); stats.cpu(16.0, &[("game draw", 2.0), ("HUD", 0.3)]); }
        let labels = |stats: &mut Stats| {
            stats.refreshed = None;
            let mut hud = HudDrawList::new([1920, 1080]);
            stats.draw(&mut hud, || vec![("Terrain".into(), "12 patches".into())]);
            let mut ids: Vec<u64> = hud.text.iter().map(|t| t.id).collect();
            ids.sort_unstable(); ids.dedup();
            assert_eq!(ids.len(), hud.text.len());
            hud.text.into_iter().map(|t| t.text).collect::<Vec<_>>()
        };
        stats.detail = 1;
        let shallow = labels(&mut stats);
        assert!(shallow.contains(&"Geometry".to_string()) && !shallow.contains(&"raster".to_string()));
        stats.detail = 2;
        let passes = labels(&mut stats);
        assert!(passes.contains(&"raster".to_string()) && !passes.contains(&"meadow aggregate depth".to_string()));
        assert!(passes.contains(&"12 patches".to_string()));
        stats.detail = 3;
        let scopes = labels(&mut stats);
        let at = |name: &str| scopes.iter().position(|t| t == name).unwrap();
        assert!(at("Vegetation") < at("meadow aggregate render") && at("meadow aggregate render") < at("meadow aggregate depth"));
        // Groups by cost: Geometry and Vegetation (2 ms) before Shadows (1 ms).
        assert!(at("Geometry") < at("Shadows") && at("Vegetation") < at("Shadows"));
    }
}
