//! The engine's debug menu (F3), for every game on the shell: switch render
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

/// Render heights offered, tallest first; 0 is native.
const HEIGHTS: [u32; 6] = [0, 2160, 1800, 1440, 1080, 720];

#[derive(Clone, Debug, PartialEq)]
struct Settings {
    options: DebugRenderOptions,
    fog: bool,
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
}

impl Default for Settings {
    fn default() -> Self {
        Settings { options: DebugRenderOptions::default(), fog: true, lod_pixels: None, exposure: 0.0, render_height: None,
            sun: 1.0, shadows: 1.0, near_shadows: true, contact: true, gi: true, reflection: true }
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
    /// When a setting last changed, so the shell can re-render the scene
    /// soon after instead of every frame while the menu is used.
    pub changed: Option<std::time::Instant>,
}

impl EngineDebug {
    pub fn open(&self) -> bool { self.ui.open }

    /// Rebuild the menu from the current settings and the game's controls.
    pub fn refresh(&mut self, game: (Vec<DebugMenu>, Vec<DebugControl>), base_height: Option<u32>, fps: f32) {
        let (s, base_lod) = (&self.s, self.base_lod);
        let mut controls = Vec::new();
        let mut toggle = |id, menu, label: &str, value, help: &str| controls.push(DebugControl { id, label: label.into(), value: DebugValue::Toggle(value), enabled: true, menu: Some(menu), advanced: false, help: help.into() });
        toggle(GRASS, GRASS_MENU, "Grass", s.options.meadow_enabled, "Skips grass rendering (when the scene has grass).");
        toggle(GLASS, RENDER_MENU, "Transparency", s.options.transparency_enabled, "Skips the transparent-material pass.");
        toggle(BLOOM, RENDER_MENU, "Highlight bloom", s.options.bloom_enabled, "Screen-space halo around bright highlights.");
        toggle(FOG, RENDER_MENU, "Distance fog", s.fog, "Off: no distance extinction and no fog culling.");
        toggle(NEAR_SHADOWS, LIGHT_MENU, "Near shadows", s.near_shadows, "The detailed shadow map close to the camera.");
        toggle(CONTACT, LIGHT_MENU, "Contact occlusion", s.contact, "Small-scale darkening where surfaces meet.");
        toggle(GI, LIGHT_MENU, "Global illumination", s.gi, "Bounced light; off leaves sun and sky only.");
        toggle(REFLECTION, LIGHT_MENU, "Reflection occlusion", s.reflection, "Darkens sky reflections in creases.");
        let mut number = |id, menu, label: &str, value: f64, (min, max, step, decimals): (f64, f64, f64, u8), unit: &str, help: &str| controls.push(DebugControl {
            id, label: label.into(), value: DebugValue::Number { value, min, max, step, decimals, unit: unit.into() }, enabled: true, menu: Some(menu), advanced: false, help: help.into() });
        number(BLOOM_STRENGTH, RENDER_MENU, "Bloom strength", f64::from(s.options.bloom_strength) * 100.0, (0.0, 20.0, 1.0, 0), "%", "0% skips the bloom passes.");
        number(LOD, RENDER_MENU, "Geometry LOD error", f64::from(s.lod_pixels.unwrap_or(base_lod)), (0.25, 8.0, 0.25, 2), "px", "Larger error picks simpler cluster meshes (trees, rocks, props) sooner.");
        number(EXPOSURE, RENDER_MENU, "Exposure offset", f64::from(s.exposure), (-3.0, 3.0, 0.25, 2), "EV", "Positive darkens, negative brightens.");
        number(SUN, LIGHT_MENU, "Sun", f64::from(s.sun) * 100.0, (0.0, 200.0, 10.0, 0), "%", "Sun illuminance.");
        number(SHADOWS, LIGHT_MENU, "Shadow strength", f64::from(s.shadows) * 100.0, (0.0, 100.0, 10.0, 0), "%", "0% leaves the sun unshadowed.");
        number(GRASS_DISTANCE, GRASS_MENU, "Grass draw distance", f64::from(s.options.meadow_distance_scale) * 100.0, (25.0, 100.0, 5.0, 0), "%", "Shrinks the grass radius; density unchanged.");
        number(GRASS_SIZE, GRASS_MENU, "Grass size", f64::from(s.options.meadow_size_scale) * 100.0, (50.0, 200.0, 5.0, 0), "%", "Blade size around fixed roots.");
        number(GRASS_COUNT, GRASS_MENU, "Grass patch count", f64::from(s.options.meadow_density) * 100.0, (0.0, 100.0, 5.0, 0), "%", "Share of grass patches drawn.");
        number(GRASS_LOD, GRASS_MENU, "Grass LOD error", f64::from(s.options.meadow_lod_pixels), (0.5, 4.0, 0.25, 2), "px", "Higher picks simpler blades sooner.");
        let height = s.render_height.or(base_height).unwrap_or(0);
        let index = HEIGHTS.iter().position(|&h| h == height).unwrap_or(0);
        controls.push(DebugControl { id: RESOLUTION, label: "Render height".into(), enabled: true, menu: Some(RENDER_MENU), advanced: false,
            value: DebugValue::Choice { index, labels: HEIGHTS.iter().map(|&h| if h == 0 { "native".into() } else { format!("{h}p") }).collect() },
            help: "3D views render at most this tall and are scaled up; the HUD stays sharp. Shading cost follows the pixel count.".into() });
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
        self.model = DebugModel { controls, menus, gpu_passes, gpu_active_ms, cpu_frame_ms: (fps > 0.0).then(|| 1000.0 / f64::from(fps)),
            status: Some(format!("{fps:.0} fps  |  F3 closes, arrows change, Enter selects")), ..Default::default() };
    }

    /// A key while the menu may be open. Returns the game's action, if the
    /// key changed a game control, and whether the key was used.
    pub fn key(&mut self, key: DebugKey, size: [u32; 2]) -> (bool, Option<DebugAction>) {
        let r = self.ui.on_key(key, size, &self.model);
        (r.consumed, r.action.and_then(|a| self.apply(a)))
    }

    pub fn pointer_move(&mut self, cursor: [f32; 2]) { self.ui.pointer_move(cursor); }
    pub fn pointer_down(&mut self, size: [u32; 2]) -> (bool, Option<DebugAction>) {
        let r = self.ui.on_pointer_down(size, &self.model);
        (r.consumed, r.action.and_then(|a| self.apply(a)))
    }
    pub fn pointer_up(&mut self) -> Option<DebugAction> { self.ui.pointer_up().action.and_then(|a| self.apply(a)) }
    pub fn scroll(&mut self, lines: f32, size: [u32; 2]) -> bool { self.ui.on_scroll(lines, size, &self.model) }
    pub fn tick(&mut self, dt: f64, size: [u32; 2]) -> Option<DebugAction> { self.ui.tick(dt, size, &self.model).action.and_then(|a| self.apply(a)) }
    pub fn draw(&mut self, hud: &mut HudDrawList) { self.ui.draw(hud, &self.model); }

    /// Apply an engine action; a game's action (or a reset, which the game
    /// should also see) is returned for [`crate::Game::debug_action`].
    fn apply(&mut self, action: DebugAction) -> Option<DebugAction> {
        self.changed = Some(std::time::Instant::now());
        let s = &mut self.s;
        let pct = |v: f64, lo: f64, hi: f64| (v / 100.0).clamp(lo, hi) as f32;
        match action {
            DebugAction::ResetDefaults => { *s = Settings::default(); return Some(action); }
            DebugAction::SetToggle { id, value } if id < GAME_IDS => match id {
                GRASS => s.options.meadow_enabled = value,
                GLASS => s.options.transparency_enabled = value,
                BLOOM => s.options.bloom_enabled = value,
                FOG => s.fog = value,
                NEAR_SHADOWS => s.near_shadows = value,
                CONTACT => s.contact = value,
                GI => s.gi = value,
                REFLECTION => s.reflection = value,
                _ => {}
            },
            DebugAction::SetNumber { id, value } if id < GAME_IDS && value.is_finite() => match id {
                BLOOM_STRENGTH => s.options.bloom_strength = pct(value, 0.0, 0.2),
                LOD => s.lod_pixels = Some(value.clamp(0.25, 8.0) as f32),
                EXPOSURE => s.exposure = value.clamp(-3.0, 3.0) as f32,
                SUN => s.sun = pct(value, 0.0, 2.0),
                SHADOWS => s.shadows = pct(value, 0.0, 1.0),
                GRASS_DISTANCE => s.options.meadow_distance_scale = pct(value, 0.25, 1.0),
                GRASS_SIZE => s.options.meadow_size_scale = pct(value, 0.5, 2.0),
                GRASS_COUNT => s.options.meadow_density = pct(value, 0.0, 1.0),
                GRASS_LOD => s.options.meadow_lod_pixels = value.clamp(0.5, 4.0) as f32,
                _ => {}
            },
            DebugAction::SetChoice { id: RESOLUTION, index } => s.render_height = HEIGHTS.get(index).copied(),
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
        l.ev100 += s.exposure;
        if !s.fog { l.fog_density = 0.0; l.fog_culling = false; }
        l.sun_lux *= s.sun;
        l.shadow_strength *= s.shadows;
        if !s.near_shadows { l.near_shadows = 0.0; }
        if !s.contact { l.contact_occlusion = 0.0; }
        if !s.gi { l.gi = 0.0; }
        if !s.reflection { l.reflection_occlusion = 0.0; }
        l
    }

    /// The render height to use instead of the game's, if changed here.
    pub fn render_height(&self) -> Option<Option<u32>> { self.s.render_height.map(|h| (h > 0).then_some(h)) }

    /// Hand the render options to `renderer` when they or it changed.
    pub fn sync(&mut self, renderer: &render::Renderer) {
        let key = (std::ptr::from_ref(renderer) as usize, self.s.options);
        if self.applied != Some(key) {
            if let Err(e) = renderer.set_debug_options(self.s.options) { eprintln!("debug menu: {e}"); }
            self.applied = Some(key);
        }
    }

    /// Collect GPU pass times (when `on`; switching on is per renderer).
    pub fn profile(&mut self, gpu: &render::Gpu, renderer: &render::Renderer, on: bool) {
        let id = std::ptr::from_ref(renderer) as usize;
        if !on {
            if self.profiling.take() == Some(id) { let _ = renderer.set_async_gpu_profiling(gpu, false); }
            return;
        }
        if self.profiling != Some(id) {
            self.profiling = renderer.set_async_gpu_profiling(gpu, true).is_ok().then_some(id);
            self.passes.clear();
        }
        for sample in renderer.drain_gpu_profiles(gpu).samples {
            for (name, ms) in sample.gpu_ms {
                match self.passes.iter_mut().find(|p| p.0 == name) {
                    Some(p) => p.1 += (ms - p.1) * 0.1,
                    None => self.passes.push((name, ms)),
                }
            }
        }
    }

    /// Smoothed GPU milliseconds per pass, as measured by [`Self::profile`].
    pub fn passes(&self) -> &[(&'static str, f64)] { &self.passes }

    /// What differs from the defaults, for screenshot metadata.
    pub fn metadata(&self) -> serde_json::Value {
        let s = &self.s;
        let o = &s.options;
        serde_json::json!({
            "grass": o.meadow_enabled, "transparency": o.transparency_enabled, "bloom": o.bloom_enabled, "bloom_strength": o.bloom_strength,
            "fog": s.fog, "lod_pixels": s.lod_pixels, "exposure_offset_ev": s.exposure, "render_height": s.render_height,
            "sun_scale": s.sun, "shadow_scale": s.shadows, "near_shadows": s.near_shadows, "contact_occlusion": s.contact, "gi": s.gi,
            "reflection_occlusion": s.reflection, "grass_distance": o.meadow_distance_scale, "grass_size": o.meadow_size_scale,
            "grass_count": o.meadow_density, "grass_lod_pixels": o.meadow_lod_pixels,
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
