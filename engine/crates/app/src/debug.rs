//! The engine's debug menu (F10), for every game on the shell: switch render
//! features and light effects off, change LOD and render resolution, and
//! whatever the game adds with [`crate::Game::debug`]. Built on render's
//! [`DebugUi`]; nothing here changes the scene or the simulation.
use render::debug_settings::DebugRenderOptions;
use render::debug_ui::{DebugAction, DebugControl, DebugKey, DebugMenu, DebugModel, DebugUi, DebugValue};
use render::hud::HudDrawList;
use render::Lighting;

/// Control and menu ids below this are the engine's; games use the rest.
pub const GAME_IDS: u32 = 1000;

const RENDER_MENU: u32 = 1;
const LIGHT_MENU: u32 = 2;
const GRASS_MENU: u32 = 3;

const GRASS: u32 = 10;
const GLASS: u32 = 11;
const BLOOM: u32 = 12;
const BLOOM_STRENGTH: u32 = 13;
const FOG: u32 = 14;
const LOD: u32 = 15;
const EXPOSURE: u32 = 16;
const RESOLUTION: u32 = 17;
const TAA: u32 = 18;
const ADAPTIVE_QUALITY: u32 = 19;
const FPS_TARGET: u32 = 26;
const EMULATE: u32 = 27;
const EMULATE_GPU: u32 = 28;
const SUN: u32 = 20;
const SHADOWS: u32 = 21;
const NEAR_SHADOWS: u32 = 22;
const CONTACT: u32 = 23;
const GI: u32 = 24;
const REFLECTION: u32 = 25;
const GRASS_DISTANCE: u32 = 30;
const GRASS_SIZE: u32 = 31;
const GRASS_COUNT: u32 = 32;
const GRASS_LOD: u32 = 33;
const HDR_OUTPUT: u32 = 34;
const HDR_HEADROOM: u32 = 35;
const AUTO_EXPOSURE: u32 = 36;

/// Render heights offered, tallest first; 0 is native.
const HEIGHTS: [u32; 6] = [0, 2160, 1800, 1440, 1080, 720];

/// FPS targets offered; 0 follows display refresh rate.
const FPS_TARGETS: [u32; 6] = [0, 60, 90, 120, 144, 240];

#[derive(Clone, Debug, PartialEq)]
struct Settings {
    options: DebugRenderOptions,
    fog: bool,
    /// Temporal AA as the game set it (false switches it off).
    taa: bool,
    adaptive_quality: bool,
    fps_target: Option<u32>,
    lod_pixels: Option<f32>,
    exposure: f32,
    /// None: the game's own [`crate::Config::max_render_height`].
    render_height: Option<u32>,
    sun: f32,
    shadows: f32,
    near_shadows: bool,
    contact: bool,
    gi: bool,
    reflection: bool,
    /// None: the game's own choice.
    auto_exposure: Option<bool>,
    /// None: the game's choice or `WARP_HDR`.
    hdr: Option<bool>,
    /// None: what the display reports (or `WARP_HDR_HEADROOM`).
    hdr_headroom: Option<f32>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { options: DebugRenderOptions::default(), fog: true, taa: true, adaptive_quality: true, fps_target: None, lod_pixels: None, exposure: 0.0, render_height: None,
            sun: 1.0, shadows: 1.0, near_shadows: true, contact: true, gi: true, reflection: true, auto_exposure: None, hdr: None, hdr_headroom: None }
    }
}

#[derive(Default)]
pub struct EngineDebug {
    ui: DebugUi,
    s: Settings,
    model: DebugModel,
    /// Renderer (by address) and settings last handed to it.
    applied: Option<(usize, DebugRenderOptions)>,
    /// The game's own LOD error, shown until the menu changes it.
    base_lod: f32,
    /// Renderer (by address) whose GPU timestamps are on.
    profiling: Option<usize>,
    /// Smoothed milliseconds per GPU pass, in first-seen order.
    passes: Vec<(&'static str, f64)>,
    /// Each profiled frame's GPU time (covered union), for a start probe.
    pub covered: Vec<f64>,
    /// The newest one, for hardware emulation.
    pub last_gpu_ms: Option<f64>,
    /// When a setting last changed, so the shell can re-render the scene
    /// soon after instead of every frame while the menu is used.
    pub changed: Option<web_time::Instant>,
    /// The game's light had automatic exposure on (before this menu).
    game_auto_exposure: bool,
    /// The newest exposure and meter reading of view 0 (EV100).
    exposure: (Option<f32>, Option<f32>),
    display: DisplayStatus,
}

/// What the window's screen offers, for the HDR controls.
#[derive(Clone, Debug, Default)]
struct DisplayStatus { supported: bool, on: bool, headroom: f32, line: String }

impl EngineDebug {
    pub fn open(&self) -> bool { self.ui.open }

    /// Rebuild the menu from the current settings and the game's controls.
    pub fn refresh(&mut self, game: (Vec<DebugMenu>, Vec<DebugControl>), base_height: Option<u32>, fps: f32, quality_label: Option<&str>) {
        let (s, base_lod) = (&self.s, self.base_lod);
        let mut controls = Vec::new();
        let mut toggle = |id, menu, label: &str, value, help: &str| controls.push(DebugControl { id, label: label.into(), value: DebugValue::Toggle(value), enabled: true, menu: Some(menu), advanced: false, help: help.into() });
        toggle(ADAPTIVE_QUALITY, RENDER_MENU, "Dynamic quality", s.adaptive_quality, "Dynamically adjust graphics settings and resolution to hit target framerate.");
        toggle(GRASS, GRASS_MENU, "Grass", s.options.meadow_enabled, "Skips grass rendering (when the scene has grass).");
        toggle(GLASS, RENDER_MENU, "Transparency", s.options.transparency_enabled, "Skips the transparent-material pass.");
        toggle(BLOOM, RENDER_MENU, "Highlight bloom", s.options.bloom_enabled, "Screen-space halo around bright highlights.");
        toggle(FOG, RENDER_MENU, "Distance fog", s.fog, "Off: no distance extinction and no fog culling.");
        toggle(TAA, RENDER_MENU, "Temporal AA", s.taa, "Off disables temporal anti-aliasing if the game uses it (it cannot switch it on).");
        toggle(NEAR_SHADOWS, LIGHT_MENU, "Near shadows", s.near_shadows, "The detailed shadow map close to the camera.");
        toggle(CONTACT, LIGHT_MENU, "Contact occlusion", s.contact, "Small-scale darkening where surfaces meet.");
        toggle(GI, LIGHT_MENU, "Global illumination", s.gi, "Bounced light; off leaves sun and sky only.");
        toggle(REFLECTION, LIGHT_MENU, "Reflection occlusion", s.reflection, "Darkens sky reflections in creases.");
        let auto_help = match self.exposure {
            (Some(ev), Some(metered)) => format!("Meters each view and adapts like an eye. Now EV {ev:.1}; the scene meters EV {metered:.1}."),
            _ => "Meters each view and adapts like an eye. Off: the game's fixed exposure.".into(),
        };
        toggle(AUTO_EXPOSURE, RENDER_MENU, "Auto exposure", s.auto_exposure.unwrap_or(self.game_auto_exposure), &auto_help);
        let d = &self.display;
        controls.push(DebugControl { id: HDR_OUTPUT, label: "HDR output".into(), value: DebugValue::Toggle(d.on), enabled: d.supported || d.on, menu: Some(RENDER_MENU), advanced: false,
            help: if d.supported || d.on { format!("Brighter highlights on an HDR screen; screenshots stay SDR. {}.", d.line) }
                else { "This screen and driver offer no HDR surface (on Windows: turn on HDR in the display settings).".into() } });
        let mut number = |id, menu, label: &str, value: f64, (min, max, step, decimals): (f64, f64, f64, u8), unit: &str, help: &str| controls.push(DebugControl {
            id, label: label.into(), value: DebugValue::Number { value, min, max, step, decimals, unit: unit.into() }, enabled: true, menu: Some(menu), advanced: false, help: help.into() });
        number(BLOOM_STRENGTH, RENDER_MENU, "Bloom strength", f64::from(s.options.bloom_strength) * 100.0, (0.0, 20.0, 1.0, 0), "%", "0% skips the bloom passes.");
        number(LOD, RENDER_MENU, "Geometry LOD error", f64::from(s.lod_pixels.unwrap_or(base_lod)), (0.25, 8.0, 0.25, 2), "px", "Larger error picks simpler cluster meshes (trees, rocks, props) sooner.");
        number(EXPOSURE, RENDER_MENU, "Exposure offset", f64::from(s.exposure), (-3.0, 3.0, 0.25, 2), "EV", "Positive darkens, negative brightens (also with auto exposure).");
        number(SUN, LIGHT_MENU, "Sun", f64::from(s.sun) * 100.0, (0.0, 200.0, 10.0, 0), "%", "Sun illuminance.");
        number(SHADOWS, LIGHT_MENU, "Shadow strength", f64::from(s.shadows) * 100.0, (0.0, 100.0, 10.0, 0), "%", "0% leaves the sun unshadowed.");
        number(GRASS_DISTANCE, GRASS_MENU, "Grass draw distance", f64::from(s.options.meadow_distance_scale) * 100.0, (10.0, 100.0, 5.0, 0), "%", "Shrinks the blade radius; past it the ground is shaded as grass. Density unchanged.");
        number(GRASS_SIZE, GRASS_MENU, "Grass size", f64::from(s.options.meadow_size_scale) * 100.0, (50.0, 200.0, 5.0, 0), "%", "Blade size around fixed roots.");
        number(GRASS_COUNT, GRASS_MENU, "Grass patch count", f64::from(s.options.meadow_density) * 100.0, (0.0, 100.0, 5.0, 0), "%", "Share of grass patches drawn.");
        number(GRASS_LOD, GRASS_MENU, "Grass LOD error", f64::from(s.options.meadow_lod_pixels), (0.5, 4.0, 0.25, 2), "px", "Higher picks simpler blades sooner.");
        let headroom_help = format!("How many times brighter than SDR white the brightest highlights get. The display reports {}.", d.line);
        controls.push(DebugControl { id: HDR_HEADROOM, label: "HDR peak".into(), enabled: d.on, menu: Some(RENDER_MENU), advanced: false, help: headroom_help,
            value: DebugValue::Number { value: f64::from(s.hdr_headroom.unwrap_or(d.headroom.max(1.0))), min: 1.0, max: 16.0, step: 0.5, decimals: 1, unit: "x".into() } });
        let height = s.render_height.or(base_height).unwrap_or(0);
        let index = HEIGHTS.iter().position(|&h| h == height).unwrap_or(0);
        controls.push(DebugControl { id: EMULATE, label: "Emulate hardware".into(), enabled: true, menu: Some(RENDER_MENU), advanced: false,
            help: "Feel this machine here: its GPU and CPU speed, refresh rate and screen height. A good guess, not a measurement.".into(),
            value: DebugValue::Choice { index: crate::emulate::choice(), labels: crate::emulate::choices() } });
        controls.push(DebugControl { id: EMULATE_GPU, label: "Emulated GPU slowdown".into(), enabled: true, menu: Some(RENDER_MENU), advanced: false,
            help: "Any GPU slowdown: 1 is the real speed.".into(),
            value: DebugValue::Number { value: f64::from(crate::emulate::gpu_factor()), min: 1.0, max: 8.0, step: 0.1, decimals: 1, unit: "x".into() } });
        controls.push(DebugControl { id: RESOLUTION, label: "Render height".into(), enabled: true, menu: Some(RENDER_MENU), advanced: false,
            value: DebugValue::Choice { index, labels: HEIGHTS.iter().map(|&h| if h == 0 { "native".into() } else { format!("{h}p") }).collect() },
            help: "3D views render at most this tall and are scaled up; the HUD stays sharp. Shading cost follows the pixel count.".into() });
        let fps_target = s.fps_target.unwrap_or(0);
        let fps_index = FPS_TARGETS.iter().position(|&f| f == fps_target).unwrap_or(0);
        controls.push(DebugControl {
            id: FPS_TARGET,
            label: "Target FPS".into(),
            enabled: s.adaptive_quality,
            menu: Some(RENDER_MENU),
            advanced: false,
            value: DebugValue::Choice {
                index: fps_index,
                labels: FPS_TARGETS.iter().map(|&f| if f == 0 { "display (auto)".into() } else { format!("{f} fps") }).collect(),
            },
            help: "Target frame rate for adaptive quality. 0 follows the screen refresh rate.".into(),
        });
        let (game_menus, game_controls) = game;
        controls.extend(game_controls);
        let mut menus = vec![
            DebugMenu { id: RENDER_MENU, parent: None, label: "Rendering".into(), help: "Features, LOD and resolution.".into() },
            DebugMenu { id: LIGHT_MENU, parent: None, label: "Light effects".into(), help: "Sun, shadows, GI and occlusion.".into() },
            DebugMenu { id: GRASS_MENU, parent: None, label: "Grass".into(), help: "Live grass controls.".into() },
        ];
        menus.extend(game_menus);
        let gpu_passes = self.passes.iter().map(|&(label, ms)| render::debug_ui::DebugPass { label: label.into(), ms }).collect();
        let gpu_active_ms = (!self.passes.is_empty()).then(|| self.passes.iter().map(|p| p.1).sum());
        let status = match quality_label {
            Some(lbl) => format!("{fps:.0} fps  |  {lbl}  |  F10 closes, arrows change, Enter selects"),
            None => format!("{fps:.0} fps  |  F10 closes, arrows change, Enter selects"),
        };
        self.model = DebugModel { controls, menus, gpu_passes, gpu_active_ms, cpu_frame_ms: (fps > 0.0).then(|| 1000.0 / f64::from(fps)),
            status: Some(status), ..Default::default() };
    }

    /// A key while the menu may be open. Returns the game's action, if the
    /// key changed a game control, and whether the key was used.
    pub fn key(&mut self, key: DebugKey, size: [u32; 2]) -> (bool, Option<DebugAction>) {
        let r = self.ui.on_key(key, size, &self.model);
        (r.consumed, r.action.and_then(|a| self.apply(a)))
    }

    pub fn set_visible(&mut self,open:bool) { self.ui.open=open; self.ui.cancel_interaction(); }
    pub fn pointer_move(&mut self, cursor: [f32; 2]) { self.ui.pointer_move(cursor); }
    pub fn pointer_down(&mut self, size: [u32; 2]) -> (bool, Option<DebugAction>) {
        let r = self.ui.on_pointer_down(size, &self.model);
        (r.consumed, r.action.and_then(|a| self.apply(a)))
    }
    pub fn cancel_interaction(&mut self) { self.ui.cancel_interaction(); }
    pub fn pointer_up(&mut self) -> Option<DebugAction> { self.ui.pointer_up().action.and_then(|a| self.apply(a)) }
    pub fn scroll(&mut self, lines: f32, size: [u32; 2]) -> bool { self.ui.on_scroll(lines, size, &self.model) }
    pub fn tick(&mut self, dt: f64, size: [u32; 2]) -> Option<DebugAction> { self.ui.tick(dt, size, &self.model).action.and_then(|a| self.apply(a)) }
    pub fn draw(&mut self, hud: &mut HudDrawList) { self.ui.draw(hud, &self.model); }

    /// Apply an engine action; a game's action (or a reset, which the game
    /// should also see) is returned for [`crate::Game::debug_action`].
    fn apply(&mut self, action: DebugAction) -> Option<DebugAction> {
        self.changed = Some(web_time::Instant::now());
        let s = &mut self.s;
        let pct = |v: f64, lo: f64, hi: f64| (v / 100.0).clamp(lo, hi) as f32;
        match action {
            DebugAction::ResetDefaults => { *s = Settings::default(); return Some(action); }
            DebugAction::SetToggle { id, value } if id < GAME_IDS => match id {
                GRASS => s.options.meadow_enabled = value,
                GLASS => s.options.transparency_enabled = value,
                BLOOM => s.options.bloom_enabled = value,
                FOG => s.fog = value,
                TAA => s.taa = value,
                ADAPTIVE_QUALITY => s.adaptive_quality = value,
                NEAR_SHADOWS => s.near_shadows = value,
                CONTACT => s.contact = value,
                GI => s.gi = value,
                REFLECTION => s.reflection = value,
                AUTO_EXPOSURE => s.auto_exposure = Some(value),
                HDR_OUTPUT => s.hdr = Some(value),
                _ => {}
            },
            DebugAction::SetNumber { id, value } if id < GAME_IDS && value.is_finite() => match id {
                BLOOM_STRENGTH => s.options.bloom_strength = pct(value, 0.0, 0.2),
                LOD => s.lod_pixels = Some(value.clamp(0.25, 8.0) as f32),
                EXPOSURE => s.exposure = value.clamp(-3.0, 3.0) as f32,
                SUN => s.sun = pct(value, 0.0, 2.0),
                SHADOWS => s.shadows = pct(value, 0.0, 1.0),
                GRASS_DISTANCE => s.options.meadow_distance_scale = pct(value, f64::from(render::debug_settings::MEADOW_DISTANCE_SCALE_MIN), 1.0),
                GRASS_SIZE => s.options.meadow_size_scale = pct(value, 0.5, 2.0),
                GRASS_COUNT => s.options.meadow_density = pct(value, 0.0, 1.0),
                GRASS_LOD => s.options.meadow_lod_pixels = value.clamp(0.5, 4.0) as f32,
                EMULATE_GPU => crate::emulate::set_gpu_factor(value.clamp(1.0, 8.0) as f32),
                HDR_HEADROOM => s.hdr_headroom = Some(value.clamp(1.0, 16.0) as f32),
                _ => {}
            },
            DebugAction::SetChoice { id: RESOLUTION, index } => s.render_height = HEIGHTS.get(index).copied(),
            DebugAction::SetChoice { id: FPS_TARGET, index } => s.fps_target = FPS_TARGETS.get(index).copied(),
            DebugAction::SetChoice { id: EMULATE, index } => crate::emulate::choose(index),
            DebugAction::SetToggle { .. } | DebugAction::SetNumber { .. } | DebugAction::SetChoice { .. } | DebugAction::Activate { .. } => {
                return (action_id(action) >= GAME_IDS).then_some(action);
            }
        }
        None
    }

    /// The game's lighting with the menu's changes.
    pub fn light(&mut self, mut l: Lighting) -> Lighting {
        self.base_lod = l.lod_pixels;
        let s = &self.s;
        if let Some(px) = s.lod_pixels { l.lod_pixels = px; }
        self.game_auto_exposure = l.auto_exposure.is_some();
        match s.auto_exposure {
            Some(true) => { l.auto_exposure.get_or_insert_with(Default::default); }
            Some(false) => l.auto_exposure = None,
            None => {}
        }
        l.ev100 += s.exposure;
        // Positive darkens: one stop less compensation per EV of offset.
        if let Some(a) = &mut l.auto_exposure { a.compensation -= s.exposure; }
        if !s.fog { l.fog_density = 0.0; l.fog_culling = false; }
        if !s.taa { l.taa = 0.0; }
        l.sun_lux *= s.sun;
        l.shadow_strength *= s.shadows;
        if !s.near_shadows { l.near_shadows = 0.0; }
        if !s.contact { l.contact_occlusion = 0.0; }
        if !s.gi { l.gi = 0.0; }
        if !s.reflection { l.reflection_occlusion = 0.0; }
        l
    }

    pub fn adaptive_quality(&self) -> bool { self.s.adaptive_quality }

    /// HDR output as set here (None: not touched).
    pub fn hdr(&self) -> Option<bool> { self.s.hdr }
    /// The HDR highlight headroom set here (None: the display's).
    pub fn hdr_headroom(&self) -> Option<f32> { self.s.hdr_headroom }

    /// What the window's screen offers, shown by the HDR controls.
    pub fn set_display(&mut self, supported: bool, on: bool, headroom: f32, display: &render::hdr::HdrDisplay) {
        let line = display.describe();
        if self.display.line != line || (self.display.supported, self.display.on, self.display.headroom) != (supported, on, headroom) {
            self.display = DisplayStatus { supported, on, headroom, line };
        }
    }

    /// The newest automatic exposure of the first view, shown by its control.
    pub fn set_exposure(&mut self, ev100: Option<f32>, metered: Option<f32>) { self.exposure = (ev100, metered); }

    /// Target framerate from the debug menu.
    /// `None`: follows game / display default.
    /// `Some(None)`: explicit "display (auto)".
    /// `Some(Some(fps))`: explicit fixed target.
    pub fn target_fps(&self) -> Option<Option<f32>> {
        self.s.fps_target.map(|f| if f > 0 { Some(f as f32) } else { None })
    }

    /// Set the FPS target shown in the menu (e.g. from game config).
    pub fn set_fps_target(&mut self, target: Option<u32>) {
        self.s.fps_target = target;
    }

    pub fn options(&self) -> DebugRenderOptions { self.s.options }

    /// The render height to use instead of the game's, if changed here.
    pub fn render_height(&self) -> Option<Option<u32>> { self.s.render_height.map(|h| (h > 0).then_some(h)) }

    /// Hand the render options to `renderer` when they or it changed.
    pub fn sync(&mut self, renderer: &render::Renderer) {
        self.sync_options(renderer, self.s.options);
    }

    /// Hand specific render options to `renderer` (e.g. after adaptive quality scaling).
    pub fn sync_options(&mut self, renderer: &render::Renderer, options: DebugRenderOptions) {
        let key = (std::ptr::from_ref(renderer) as usize, options);
        if self.applied != Some(key) {
            if let Err(e) = renderer.set_debug_options(options) { eprintln!("debug menu: {e}"); }
            self.applied = Some(key);
        }
    }

    /// Collect GPU pass times (when `on`; switching on is per renderer).
    /// Turn async GPU timestamps on or off and fold in what arrived. The
    /// samples are returned for other readers (the F3 stats overlay).
    pub fn profile(&mut self, gpu: &render::Gpu, renderer: &render::Renderer, on: bool) -> Vec<render::gpu_profile::GpuProfileSample> {
        let id = std::ptr::from_ref(renderer) as usize;
        if !on {
            if self.profiling.take() == Some(id) { let _ = renderer.set_async_gpu_profiling(gpu, false); }
            return Vec::new();
        }
        if self.profiling != Some(id) {
            self.profiling = renderer.set_async_gpu_profiling(gpu, true).is_ok().then_some(id);
            self.passes.clear();
        }
        let samples = renderer.drain_gpu_profiles(gpu).samples;
        for sample in &samples {
            crate::frame_log::gpu(sample.frame_id,sample.covered_union_ms,&sample.gpu_ms);
            if self.covered.len() < 4096 { self.covered.push(sample.covered_union_ms); }
            self.last_gpu_ms = Some(sample.covered_union_ms);
            for &(name, ms) in &sample.gpu_ms {
                match self.passes.iter_mut().find(|p| p.0 == name) {
                    Some(p) => p.1 += (ms - p.1) * 0.1,
                    None => self.passes.push((name, ms)),
                }
            }
        }
        samples
    }

    /// Smoothed GPU milliseconds per pass, as measured by [`Self::profile`].
    pub fn passes(&self) -> &[(&'static str, f64)] { &self.passes }

    /// What differs from the defaults, for screenshot metadata.
    pub fn metadata(&self) -> serde_json::Value {
        let s = &self.s;
        let o = &s.options;
        serde_json::json!({
            "dynamic_quality": s.adaptive_quality,
            "fps_target": s.fps_target,
            "grass": o.meadow_enabled, "transparency": o.transparency_enabled, "bloom": o.bloom_enabled, "bloom_strength": o.bloom_strength,
            "fog": s.fog, "taa": s.taa, "lod_pixels": s.lod_pixels, "exposure_offset_ev": s.exposure, "render_height": s.render_height,
            "sun_scale": s.sun, "shadow_scale": s.shadows, "near_shadows": s.near_shadows, "contact_occlusion": s.contact, "gi": s.gi,
            "reflection_occlusion": s.reflection, "grass_distance": o.meadow_distance_scale, "grass_size": o.meadow_size_scale,
            "grass_count": o.meadow_density, "grass_lod_pixels": o.meadow_lod_pixels,
            "auto_exposure": s.auto_exposure.unwrap_or(self.game_auto_exposure), "exposure_ev100": self.exposure.0, "metered_ev100": self.exposure.1,
            "hdr_output": self.display.on, "hdr_headroom": self.display.headroom, "hdr_display": self.display.line,
            "gpu_pass_ms": self.passes.iter().map(|(n, ms)| (n.to_string(), serde_json::json!(ms))).collect::<serde_json::Map<_, _>>(),
        })
    }
}

fn action_id(a: DebugAction) -> u32 {
    match a {
        DebugAction::SetToggle { id, .. } | DebugAction::SetNumber { id, .. } | DebugAction::SetChoice { id, .. } | DebugAction::Activate { id } => id,
        DebugAction::ResetDefaults => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_exposure_and_hdr_controls() {
        let mut d = EngineDebug::default();
        let manual = Lighting::default();
        assert!(d.light(manual.clone()).auto_exposure.is_none(), "the game's choice by default");
        d.apply(DebugAction::SetToggle { id: AUTO_EXPOSURE, value: true });
        d.apply(DebugAction::SetNumber { id: EXPOSURE, value: 1.0 });
        let l = d.light(manual.clone());
        assert_eq!(l.auto_exposure.unwrap().compensation, -1.0, "a darker offset is less compensation");
        d.apply(DebugAction::SetToggle { id: AUTO_EXPOSURE, value: false });
        let auto = Lighting { auto_exposure: Some(Default::default()), ..manual };
        assert!(d.light(auto).auto_exposure.is_none(), "the menu can switch the game's off");
        assert_eq!((d.hdr(), d.hdr_headroom()), (None, None));
        d.apply(DebugAction::SetToggle { id: HDR_OUTPUT, value: true });
        d.apply(DebugAction::SetNumber { id: HDR_HEADROOM, value: 40.0 });
        assert_eq!((d.hdr(), d.hdr_headroom()), (Some(true), Some(16.0)));
        d.apply(DebugAction::ResetDefaults);
        assert_eq!(d.hdr(), None);
    }

    #[test]
    fn fps_target_choice_updates_settings() {
        let mut d = EngineDebug::default();
        assert_eq!(d.target_fps(), None);
        d.apply(DebugAction::SetChoice { id: FPS_TARGET, index: 3 }); // 120 fps
        assert_eq!(d.target_fps(), Some(Some(120.0)));
        d.apply(DebugAction::SetChoice { id: FPS_TARGET, index: 0 }); // display (auto)
        assert_eq!(d.target_fps(), Some(None));
    }
}
