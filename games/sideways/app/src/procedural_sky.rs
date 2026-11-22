//! Explicit developer opt-in for the atmosphere foundation. A single cached
//! world sky feeds all views; this never runs inside the frame loop.
//! Presets, live settings and the bake worker live in `render::sky`; this
//! file keeps the SIDEWAYS_SKY environment path and the pause menu.
use render::atmosphere::{Atmosphere, AtmosphereCache, BakeQuality, SolarState};
use render::sky::night_metadata;
pub use render::sky::{bake_live, DayTime, LiveSettings, LiveSky, PreparedSky, WeatherLook};
use render::{Lighting, Scene};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq)]
struct Config {
    solar: SolarState,
}

fn parse(mut get: impl FnMut(&str) -> Option<String>) -> Result<Option<Config>, String> {
    match get("SIDEWAYS_SKY").as_deref() {
        None | Some("hdri") => return Ok(None),
        Some("procedural") => {},
        Some(_) => return Err("SIDEWAYS_SKY must be hdri or procedural".into()),
    }
    let mut number = |name, default| -> Result<f64, String> {
        get(name).map(|value| value.parse::<f64>().map_err(|_| format!("{name} must be a finite number")))
            .transpose().map(|value| value.unwrap_or(default))
    };
    let elevation = number("SIDEWAYS_SUN_ELEVATION", 60.0)?;
    let azimuth = number("SIDEWAYS_SUN_AZIMUTH", 125.0)?;
    Ok(Some(Config { solar: SolarState::from_degrees(elevation, azimuth)? }))
}

/// Apply before Renderer::new, once for the world. Stage rebuilds with the
/// same physical parameters reuse the bake. `--set` light overrides in a shot
/// may deliberately run afterwards; metadata records both the bake and light.
pub fn apply_environment(scene: &mut Scene, light: &mut Lighting) -> Result<serde_json::Value, String> {
    let Some(config) = parse(|name| std::env::var(name).ok())? else {
        return Ok(serde_json::json!({"mode":"hdri"}));
    };
    static CACHE: OnceLock<Mutex<AtmosphereCache>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(|| Mutex::new(AtmosphereCache::default())).lock()
        .map_err(|_| "procedural sky cache lock poisoned".to_string())?;
    apply(config, &mut cache, BakeQuality::default(), scene, light)
}

fn apply(config: Config, cache: &mut AtmosphereCache, quality: BakeQuality, scene: &mut Scene, light: &mut Lighting)
    -> Result<serde_json::Value, String>
{
    let before = cache.sky_bakes;
    let sky = cache.get_or_bake(&Atmosphere::default(), config.solar, quality)?;
    sky.apply(scene, light);
    let reused = cache.sky_bakes == before;
    eprintln!("procedural sky: elevation {} deg, azimuth {} deg, cached {reused}; single scattering, no clouds or live time progression",
        config.solar.elevation_degrees, config.solar.azimuth_degrees);
    Ok(serde_json::json!({
        "mode":"procedural_single_scattering", "cached":reused,
        "solar_elevation_degrees":config.solar.elevation_degrees,
        "solar_azimuth_degrees":config.solar.azimuth_degrees,
        "sun_rgb_lux":sky.sun_rgb_lux.to_array(),
        "sky_horizontal_rgb_lux":sky.sky_horizontal_rgb_lux.to_array(),
        "night_environment":night_metadata(&sky),
        "sky_lut":[quality.sky_width,quality.sky_height],
        "transmittance_lut":[quality.transmittance_width,quality.transmittance_height],
        "fog_medium":{"height_m":sky.fog_height_m,"phase_g":sky.fog_phase_g,
            "scattering_albedo":sky.fog_scattering_albedo.to_array(),"density_per_m":light.fog_density,
            "mode":"local_exponential_height_single_scattering","hard_end":false},
        "limitations":["no volumetric clouds or cloud shadows", "no atmospheric multiple scattering",
            "local height aerosol: camera-terrain sun gate; no along-ray scene occlusion or aerial perspective LUT",
            "no visible solar disc, moon or automatic time progression",
            "stars use angular point filtering, not an astronomical catalogue; no stellar reflections or integrated stellar GI"]
    }))
}

/// Pure menu navigation state. One shared menu, never one per viewport.
#[derive(Default)]
pub struct PauseMenu {pub selected:usize,axis:[i8;2],next_repeat:Option<Instant>,confirm_held:bool}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum MenuAction {Resume,NewStage,Lobby,Time(i8),Atmosphere(i8)}
/// The pause menu's rows: Resume, then a new stage right under it, then
/// Back to lobby under a Splitscreen Club host, then the sky settings.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum MenuRow {Resume,NewStage,Lobby,Time,Atmosphere}
pub fn menu_rows(lobby:bool)->Vec<MenuRow> {
    let mut rows=vec![MenuRow::Resume,MenuRow::NewStage];
    if lobby {rows.push(MenuRow::Lobby);}
    rows.extend([MenuRow::Time,MenuRow::Atmosphere]);rows
}
impl PauseMenu {
    pub fn opened(&mut self,axis:[i8;2],confirm:bool) {self.selected=0;self.axis=axis;self.confirm_held=confirm;self.next_repeat=None;}
    /// Consume held input while an overlay owns focus, retaining selection.
    pub fn suspend(&mut self,axis:[i8;2],confirm:bool) {self.axis=axis;self.confirm_held=confirm;self.next_repeat=None;}
    /// `lobby`: a Splitscreen Club host can take the party back to its room.
    pub fn navigate(&mut self,axis:[i8;2],confirm:bool,now:Instant,lobby:bool)->Option<MenuAction> {
        let rows=menu_rows(lobby);
        let fire=axis!=[0,0]&&(axis!=self.axis||self.next_repeat.is_some_and(|time|now>=time));
        let confirm_edge=confirm&&!self.confirm_held;self.confirm_held=confirm;
        if fire {
            self.next_repeat=Some(now+Duration::from_millis(if axis==self.axis {200}else{380}));
            if axis[1]!=0 {self.selected=(self.selected as i32+axis[1] as i32).rem_euclid(rows.len() as i32) as usize;}
        } else if axis==[0,0] {self.next_repeat=None;}
        self.axis=axis;self.selected=self.selected.min(rows.len()-1);
        let delta=if fire {axis[0]}else{0};
        match rows[self.selected] {
            MenuRow::Resume if confirm_edge=>Some(MenuAction::Resume),
            MenuRow::NewStage if confirm_edge=>Some(MenuAction::NewStage),
            MenuRow::Lobby if confirm_edge=>Some(MenuAction::Lobby),
            MenuRow::Time if delta!=0||confirm_edge=>Some(MenuAction::Time(if delta!=0 {delta}else{1})),
            MenuRow::Atmosphere if delta!=0||confirm_edge=>Some(MenuAction::Atmosphere(if delta!=0 {delta}else{1})),_=>None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_suspension_preserves_menu_selection_and_consumes_held_input() {
        let now=Instant::now();let mut menu=PauseMenu::default();menu.selected=2;
        assert_eq!(menu.navigate([1,0],false,now,false),Some(MenuAction::Time(1)));
        menu.suspend([1,0],true);
        assert_eq!(menu.navigate([1,0],true,now+Duration::from_secs(5),false),None);
        assert_eq!(menu.selected,2);
        assert_eq!(menu.navigate([0,0],false,now+Duration::from_secs(6),false),None);
        assert_eq!(menu.navigate([0,0],true,now+Duration::from_secs(7),false),Some(MenuAction::Time(1)));
    }

    #[test]
    fn new_stage_sits_under_resume_and_a_host_adds_back_to_lobby() {
        assert_eq!(menu_rows(false),[MenuRow::Resume,MenuRow::NewStage,MenuRow::Time,MenuRow::Atmosphere]);
        assert_eq!(menu_rows(true)[..3],[MenuRow::Resume,MenuRow::NewStage,MenuRow::Lobby]);
        let now=Instant::now();let mut menu=PauseMenu::default();menu.opened([0,0],false);
        assert_eq!(menu.navigate([0,1],false,now,true),None);
        assert_eq!(menu.navigate([0,0],false,now,true),None);
        assert_eq!(menu.navigate([0,1],false,now,true),None);
        assert_eq!(menu.navigate([0,0],true,now,true),Some(MenuAction::Lobby));
    }

    fn config(entries: &[(&str, &str)]) -> Result<Option<Config>, String> {
        parse(|name| entries.iter().find(|(key, _)| *key == name).map(|(_, value)| value.to_string()))
    }

    #[test]
    fn defaults_preserve_hdri_and_explicit_solar_state_is_validated() {
        assert_eq!(config(&[]).unwrap(), None);
        assert_eq!(config(&[("SIDEWAYS_SKY", "hdri")]).unwrap(), None);
        let noon = config(&[("SIDEWAYS_SKY", "procedural")]).unwrap().unwrap();
        assert_eq!(noon.solar, SolarState::from_degrees(60.0, 125.0).unwrap());
        let sunset = config(&[("SIDEWAYS_SKY", "procedural"), ("SIDEWAYS_SUN_ELEVATION", "-3"), ("SIDEWAYS_SUN_AZIMUTH", "-15")]).unwrap().unwrap();
        assert_eq!(sunset.solar, SolarState::from_degrees(-3.0, 345.0).unwrap());
        for entries in [vec![("SIDEWAYS_SKY", "clouds")],
            vec![("SIDEWAYS_SKY", "procedural"), ("SIDEWAYS_SUN_ELEVATION", "NaN")],
            vec![("SIDEWAYS_SKY", "procedural"), ("SIDEWAYS_SUN_ELEVATION", "91")],
            vec![("SIDEWAYS_SKY", "procedural"), ("SIDEWAYS_SUN_AZIMUTH", "infinity")],
            vec![("SIDEWAYS_SKY", "procedural"), ("SIDEWAYS_SUN_AZIMUTH", "bad")]] {
            assert!(config(&entries).is_err());
        }
    }

    #[test]
    fn metadata_describes_applied_light_and_repeated_world_reuses_bake() {
        let config = Config { solar: SolarState::from_degrees(12.0, 345.0).unwrap() };
        let quality = BakeQuality { sky_width:32, sky_height:16, sky_steps:16,
            transmittance_width:32, transmittance_height:16, transmittance_steps:32 };
        let mut cache = AtmosphereCache::default();
        let mut first_scene = Scene::default();
        let mut first_light = Lighting::default();
        let first = apply(config, &mut cache, quality, &mut first_scene, &mut first_light).unwrap();
        let mut second_scene = Scene::default();
        let mut second_light = Lighting::default();
        let second = apply(config, &mut cache, quality, &mut second_scene, &mut second_light).unwrap();
        assert_eq!(first["cached"], false);
        assert_eq!(second["cached"], true);
        assert_eq!(first["solar_elevation_degrees"], 12.0);
        assert_eq!(first["solar_azimuth_degrees"], 345.0);
        assert_eq!(first["sky_lut"], serde_json::json!([32,16]));
        let recorded: Vec<f32> = serde_json::from_value(first["sun_rgb_lux"].clone()).unwrap();
        let actual = first_light.sun_color * first_light.sun_lux;
        for channel in 0..3 { assert!((recorded[channel] - actual[channel]).abs() < 0.02); }
        assert!(first_scene.sky_physical && second_scene.sky_physical);
        assert_eq!(first_scene.sky, second_scene.sky);
        assert_eq!(first_light.sun_lux, second_light.sun_lux);
        assert_eq!(cache.sky_bakes, 1);
        assert_eq!(cache.transmittance_bakes, 1);
    }

    #[test]
    fn pause_menu_edges_repeat_and_start_release_are_bounded() {
        let now=Instant::now();let mut menu=PauseMenu::default();
        menu.opened([0,0],true);assert_eq!(menu.navigate([0,0],true,now,false),None,"held handbrake must not instantly resume");
        menu.navigate([0,0],false,now,false);
        assert_eq!(menu.navigate([0,1],false,now,false),None);assert_eq!(menu.selected,1);
        menu.navigate([0,0],false,now,false);
        assert_eq!(menu.navigate([0,1],false,now,false),None);assert_eq!(menu.selected,2);
        assert_eq!(menu.navigate([1,0],false,now,false),Some(MenuAction::Time(1)));
        assert_eq!(menu.navigate([1,0],false,now+Duration::from_millis(100),false),None);
        assert_eq!(menu.navigate([1,0],false,now+Duration::from_millis(380),false),Some(MenuAction::Time(1)));
        menu.navigate([0,1],false,now+Duration::from_secs(1),false);assert_eq!(menu.selected,3);
        assert_eq!(menu.navigate([-1,0],false,now+Duration::from_secs(1),false),Some(MenuAction::Atmosphere(-1)));
        assert_eq!(DayTime::Sunrise.step(-1),DayTime::Night);assert_eq!(WeatherLook::Clear.step(-1),WeatherLook::Mist);
    }
}
