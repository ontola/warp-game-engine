//! Process-local weather and lighting controls for the shared debug panel.
//! Sky requests use the existing bounded LiveSky worker; this adapter does not
//! bake, touch GPU resources, change road grip, or accumulate light overrides.
use crate::procedural_sky::{DayTime, LiveSettings, WeatherLook};
use render::{debug_ui::{DebugAction, DebugControl, DebugMenu, DebugValue}, Lighting};
use serde_json::{json, Value};
use std::collections::VecDeque;

pub const WEATHER_MENU: u32 = 104;
pub const LIGHTING_MENU: u32 = 105;
pub const DAYTIME: u32 = 30;
pub const WEATHER: u32 = 31;
pub const SUN: u32 = 32;
pub const SHADOW: u32 = 33;
pub const NEAR_SHADOWS: u32 = 34;
pub const CONTACT: u32 = 35;
pub const GI: u32 = 36;
pub const REFLECTION: u32 = 37;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change { Unhandled, Sky, Lighting }

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightingEdit {
    pub sun_scale: f32,
    pub shadow_scale: f32,
    pub near_shadows: bool,
    pub contact: bool,
    pub gi: bool,
    pub reflection: bool,
}
impl Default for LightingEdit {
    fn default() -> Self {
        Self { sun_scale: 1.0, shadow_scale: 1.0, near_shadows: true,
            contact: true, gi: true, reflection: true }
    }
}
impl LightingEdit {
    fn effective_light(self, base: &Lighting) -> Lighting {
        let mut light = base.clone();
        light.sun_lux *= self.sun_scale;
        light.shadow_strength *= self.shadow_scale;
        if !self.near_shadows { light.near_shadows = 0.0; }
        if !self.contact { light.contact_occlusion = 0.0; }
        if !self.gi { light.gi = 0.0; }
        if !self.reflection { light.reflection_occlusion = 0.0; }
        light
    }
}

pub struct EnvironmentDebug {
    desired: LiveSettings,
    status: String,
    lighting: LightingEdit,
    // At most one pending value for each of the two existing host keys.
    requests: VecDeque<(&'static str, Value)>,
}
impl EnvironmentDebug {
    pub fn new(initial: LiveSettings) -> Self {
        Self { desired: initial, status: "Current sky".into(),
            lighting: Default::default(), requests: VecDeque::new() }
    }

    /// Synchronize with requests made by the pause/host menu as well. Local
    /// edits not yet consumed by the host take precedence over that snapshot.
    pub fn sync_sky(&mut self, mut settings: LiveSettings, status: &str) -> bool {
        for (key, value) in &self.requests {
            settings = settings.changed(key, value).expect("validated debug sky request");
        }
        let changed = self.desired != settings || self.status != status;
        self.desired = settings;
        self.status = status.into();
        changed
    }

    pub fn menus() -> Vec<DebugMenu> {
        vec![
            DebugMenu { id: WEATHER_MENU, parent: None, label: "Weather / daytime".into(),
                help: "Live atmosphere presets update sunlight, sky, fog and exposure together. Road grip is unchanged.".into() },
            DebugMenu { id: LIGHTING_MENU, parent: None, label: "Lighting / shadows".into(),
                help: "Live diagnostic overrides relative to the current sky. Reset restores these overrides and keeps the selected weather/time.".into() },
        ]
    }

    fn time_choices(&self) -> Vec<(&'static str, &'static str)> {
        let mut choices = Vec::with_capacity(7);
        if self.desired.time_key() == "current" { choices.push(("current", "Current")); }
        choices.extend(DayTime::ALL.into_iter().map(|time| (time.key(), time.label())));
        choices
    }

    pub fn controls(&self) -> Vec<DebugControl> {
        let times = self.time_choices();
        let mut controls = vec![
            DebugControl { id: DAYTIME, label: "Time of day".into(),
                value: DebugValue::Choice { index: times.iter().position(|v| v.0 == self.desired.time_key()).unwrap(),
                    labels: times.iter().map(|v| v.1.into()).collect() },
                enabled: true, menu: Some(WEATHER_MENU), advanced: false,
                help: format!("{} | Six artistic sun/time presets; the shared sky bake runs in the background.", self.status) },
            DebugControl { id: WEATHER, label: "Atmosphere".into(),
                value: DebugValue::Choice { index: WeatherLook::ALL.iter().position(|v| *v == self.desired.atmosphere).unwrap(),
                    labels: WeatherLook::ALL.iter().map(|v| v.label().into()).collect() },
                enabled: true, menu: Some(WEATHER_MENU), advanced: false,
                help: format!("{} | Clear, haze and mist change aerosol scattering and distance fog. No rain or clouds are simulated.", self.status) },
        ];
        let mut number = |id, label: &str, value: f32, max, help: &str| controls.push(DebugControl {
            id, label: label.into(), value: DebugValue::Number { value: f64::from(value) * 100.0,
                min: 0.0, max, step: 5.0, decimals: 0, unit: "%".into() },
            enabled: true, menu: Some(LIGHTING_MENU), advanced: false, help: help.into(),
        });
        number(SUN, "Sun intensity", self.lighting.sun_scale, 200.0,
            "Multiplier of the selected sky's direct sunlight. 100% preserves its physical illuminance; night remains night.");
        number(SHADOW, "Sun shadow strength", self.lighting.shadow_scale, 100.0,
            "Multiplier of the current sunlight shadow strength. Lower values are an artistic diagnostic, not added ambient light.");
        let mut toggle = |id, label: &str, value, help: &str| controls.push(DebugControl {
            id, label: label.into(), value: DebugValue::Toggle(value), enabled: true,
            menu: Some(LIGHTING_MENU), advanced: false, help: help.into(),
        });
        toggle(NEAR_SHADOWS, "Near sun shadow detail", self.lighting.near_shadows,
            "Enables the existing high-resolution per-view sun maps. Outer shadows keep their configured policy.");
        toggle(GI, "Environment visibility", self.lighting.gi,
            "Uses the existing canopy/terrain visibility estimates for sky, sun fallback and reflections. This is not full bounce GI.");
        toggle(CONTACT, "Car contact occlusion", self.lighting.contact,
            "Uses moving opaque car geometry for local occlusion. Requires Environment visibility; its enabled setting is retained when that is off.");
        toggle(REFLECTION, "Reflection occlusion", self.lighting.reflection,
            "Tests reflected directions against terrain and foliage. Requires Environment visibility; this is not a ray-traced reflection toggle.");
        controls
    }

    pub fn apply(&mut self, action: DebugAction) -> Result<Change, String> {
        match action {
            DebugAction::SetChoice { id, index } if id == DAYTIME || id == WEATHER => {
                let (key, value) = if id == DAYTIME {
                    let choices = self.time_choices();
                    ("sky_time", choices.get(index).ok_or("invalid time-of-day choice")?.0)
                } else {
                    ("sky_weather", WeatherLook::ALL.get(index).ok_or("invalid atmosphere choice")?.key())
                };
                let value = Value::from(value);
                let next = self.desired.changed(key, &value)?;
                if next != self.desired {
                    self.desired = next;
                    self.requests.retain(|v| v.0 != key);
                    self.requests.push_back((key, value));
                    self.status = "Sky request queued".into();
                }
                Ok(Change::Sky)
            }
            DebugAction::SetNumber { id, value } if id == SUN || id == SHADOW => {
                let maximum = if id == SUN { 200.0 } else { 100.0 };
                if !value.is_finite() || !(0.0..=maximum).contains(&value) {
                    return Err("lighting percentage outside its finite control range".into());
                }
                if id == SUN { self.lighting.sun_scale = (value / 100.0) as f32; }
                else { self.lighting.shadow_scale = (value / 100.0) as f32; }
                Ok(Change::Lighting)
            }
            DebugAction::SetToggle { id, value } => {
                match id {
                    NEAR_SHADOWS => self.lighting.near_shadows = value,
                    CONTACT => self.lighting.contact = value,
                    GI => self.lighting.gi = value,
                    REFLECTION => self.lighting.reflection = value,
                    _ => return Ok(Change::Unhandled),
                }
                Ok(Change::Lighting)
            }
            _ => Ok(Change::Unhandled),
        }
    }

    /// Feed each pair to the existing host request_sky_setting method. Its
    /// LiveSky scheduler debounces and rejects stale background completions.
    pub fn take_sky_request(&mut self) -> Option<(&'static str, Value)> { self.requests.pop_front() }
    pub fn effective_light(&self, base: &Lighting) -> Lighting { self.lighting.effective_light(base) }
    /// Global debug Reset keeps the selected/queued atmosphere and time.
    pub fn reset_lighting(&mut self) { self.lighting = Default::default(); }
    pub fn metadata(&self) -> Value {
        json!({"time_of_day":self.desired.time_key(),"atmosphere":self.desired.atmosphere.key(),
            "sky_status":self.status,"queued_sky_requests":self.requests.len(),
            "sun_intensity_multiplier":self.lighting.sun_scale,"shadow_strength_multiplier":self.lighting.shadow_scale,
            "near_sun_shadows":self.lighting.near_shadows,"environment_visibility":self.lighting.gi,
            "contact_occlusion":self.lighting.contact,"reflection_occlusion":self.lighting.reflection,
            "scope":"Live sky presets plus frame-local light multipliers. No change to road grip, ambient SH intensity, shadow resolution or distance; reset retains chosen time/weather."})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_preset_choices_coalesce_and_keep_host_changes() {
        let mut state = EnvironmentDebug::new(LiveSettings::default());
        for index in [0, 4, 5] { state.apply(DebugAction::SetChoice { id: DAYTIME, index }).unwrap(); }
        state.apply(DebugAction::SetChoice { id: WEATHER, index: 2 }).unwrap();
        assert_eq!(state.requests.len(), 2);
        state.sync_sky(LiveSettings::default(), "Pending");
        assert_eq!(state.desired.time, DayTime::Night);
        assert_eq!(state.desired.atmosphere, WeatherLook::Mist);
        let mut host = LiveSettings::default();
        while let Some((key, value)) = state.take_sky_request() { host = host.changed(key, &value).unwrap(); }
        assert_eq!(host, state.desired);
        let host = host.changed("sky_time", &json!("morning")).unwrap();
        assert!(state.sync_sky(host, "Applied"));
        assert_eq!(state.desired.time, DayTime::Morning);
        assert_eq!(state.desired.atmosphere, WeatherLook::Mist);
    }
    #[test]
    fn overrides_are_relative_do_not_accumulate_and_reset_keeps_sky() {
        let mut state = EnvironmentDebug::new(LiveSettings::default());
        state.apply(DebugAction::SetNumber { id: SUN, value: 150.0 }).unwrap();
        state.apply(DebugAction::SetNumber { id: SHADOW, value: 25.0 }).unwrap();
        for id in [NEAR_SHADOWS, CONTACT, GI, REFLECTION] {
            state.apply(DebugAction::SetToggle { id, value: false }).unwrap();
        }
        let base = Lighting { sun_lux: 800.0, shadow_strength: 0.8, ..Lighting::default() };
        for _ in 0..3 {
            let light = state.effective_light(&base);
            assert_eq!(light.sun_lux, 1200.0); assert_eq!(light.shadow_strength, 0.2);
            assert_eq!([light.near_shadows, light.contact_occlusion, light.gi, light.reflection_occlusion], [0.0; 4]);
            assert_eq!(light.ev100, base.ev100); assert_eq!(light.sky_lux, base.sky_lux);
            assert_eq!(light.sun_dir, base.sun_dir); assert_eq!(light.fog_density, base.fog_density);
        }
        let night = Lighting { sun_lux: 0.0, ..base.clone() };
        assert_eq!(state.effective_light(&night).sun_lux, 0.0);
        state.apply(DebugAction::SetChoice { id: WEATHER, index: 1 }).unwrap();
        state.reset_lighting();
        assert_eq!(state.lighting, LightingEdit::default());
        assert_eq!(state.effective_light(&base).sun_lux, base.sun_lux);
        assert_eq!(state.desired.atmosphere, WeatherLook::Haze);
        assert_eq!(state.requests.len(), 1);
    }
    #[test]
    fn invalid_inputs_are_atomic_and_current_sky_is_not_mislabeled_noon() {
        let initial = crate::procedural_sky::LiveSky::new(&json!({}), &Lighting::default()).desired();
        assert_eq!(initial.time_key(), "current");
        let mut state = EnvironmentDebug::new(initial);
        match &state.controls()[0].value {
            DebugValue::Choice { index, labels } => { assert_eq!(*index, 0); assert_eq!(labels[0], "Current"); }
            _ => panic!("daytime must be a named preset choice"),
        }
        let before = state.metadata();
        for action in [DebugAction::SetChoice { id: DAYTIME, index: 99 },
            DebugAction::SetChoice { id: WEATHER, index: 3 },
            DebugAction::SetNumber { id: SUN, value: f64::NAN },
            DebugAction::SetNumber { id: SHADOW, value: 101.0 }] {
            assert!(state.apply(action).is_err()); assert_eq!(state.metadata(), before);
        }
        assert_eq!(state.apply(DebugAction::Activate { id: 25 }).unwrap(), Change::Unhandled);
        state.apply(DebugAction::SetChoice { id: DAYTIME, index: 3 }).unwrap();
        assert_eq!(state.desired.time_key(), "noon");
        match &state.controls()[0].value {
            DebugValue::Choice { index, labels } => { assert_eq!(*index, 2); assert_eq!(labels.len(), 6); }
            _ => panic!("daytime must remain a named preset choice"),
        }
    }
}
