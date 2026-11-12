//! The match's sky, rendering only (the simulation never sees it):
//!
//! - `random` (default): seeded random time and weather for each arena;
//! - `godot`: Godot's gradient sky and fixed sun (`view::sky`,
//!   `view::lighting`), so parity captures and golden images hold;
//! - a procedural preset from `render::sky`: `noon`, `sunset:mist`,
//!   `night:haze` (time sunrise|morning|noon|afternoon|sunset|night, weather
//!   clear|haze|mist; either part may be left out);
//! - `rounds[:WEATHER]`: the time of day steps on each round, starting at
//!   morning (a round modifier for the sky), baked in the background;
//! - `cycle[:WEATHER][:MINUTES]`: a slow day and night on the game clock,
//!   baked off the frame loop by `render::sky::LiveSky` (default 12 minutes
//!   for a whole day).
//!
//! Chosen with `--sky` (or `GG_SKY`), and at runtime with the `sky` command.
use render::atmosphere::{AtmosphereCache, BakeQuality, SolarState};
use render::sky::{bake_live, exposure_for_elevation, DayTime, LiveSettings, LiveSky, PreparedSky, WeatherLook};
use render::volumetric::Volumetric;
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SkyChoice {
    Random,
    Godot,
    Preset(LiveSettings),
    Rounds(WeatherLook),
    Cycle { atmosphere: WeatherLook, minutes: f32 },
}

impl SkyChoice {
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut parts = s.split(':');
        let head = parts.next().unwrap_or("");
        let rest: Vec<&str> = parts.collect();
        let weather = |w: Option<&&str>| w.map_or(Ok(WeatherLook::Clear), |w| WeatherLook::parse(w));
        match head {
            "random" if rest.is_empty() => Ok(Self::Random),
            "godot" if rest.is_empty() => Ok(Self::Godot),
            "rounds" if rest.len() <= 1 => Ok(Self::Rounds(weather(rest.first())?)),
            "cycle" if rest.len() <= 2 => {
                let mut atmosphere = WeatherLook::Clear;
                let mut minutes = 12.0;
                for p in rest {
                    match p.parse::<f32>() {
                        Ok(m) if m.is_finite() && m >= 0.5 => minutes = m,
                        Ok(_) => return Err("cycle minutes must be at least 0.5".into()),
                        Err(_) => atmosphere = WeatherLook::parse(p)?,
                    }
                }
                Ok(Self::Cycle { atmosphere, minutes })
            }
            _ if rest.len() <= 1 => {
                // `noon`, `mist` (noon), `sunset:mist`.
                let (time, atmosphere) = match (DayTime::parse(head), WeatherLook::parse(head)) {
                    (Ok(t), _) => (t, weather(rest.first())?),
                    (_, Ok(w)) if rest.is_empty() => (DayTime::Noon, w),
                    _ => return Err(Self::usage(s)),
                };
                Ok(Self::Preset(LiveSettings::preset(time, atmosphere)))
            }
            _ => Err(Self::usage(s)),
        }
    }

    fn usage(s: &str) -> String {
        format!("unknown sky {s}: random | godot | TIME[:WEATHER] | rounds[:WEATHER] | cycle[:WEATHER][:MINUTES]; \
            TIME sunrise|morning|noon|afternoon|sunset|night, WEATHER clear|haze|mist")
    }

    pub fn name(&self) -> String {
        match self {
            Self::Random => "random".into(),
            Self::Godot => "godot".into(),
            Self::Preset(s) => format!("{}:{}", s.time.key(), s.atmosphere.key()),
            Self::Rounds(w) => format!("rounds:{}", w.key()),
            Self::Cycle { atmosphere, minutes } => format!("cycle:{}:{minutes}", atmosphere.key()),
        }
    }

    /// The settings for a round (`round` counts from 1) at `seconds` on the
    /// game clock; `None` for Godot's sky.
    pub fn settings(&self, round: u32, seconds: f32) -> Option<LiveSettings> {
        self.sky_settings(round, seconds).map(|s| if s.exposure() < MIN_EV100 { s.with_exposure(MIN_EV100) } else { s })
    }

    /// Seeded from the arena so clients and F12 replays agree without touching
    /// gameplay RNG. Blackout overrides every selected mode, including cycles.
    pub fn for_arena(&self, round: u32, seconds: f32, seed: u64, blackout: bool) -> Option<LiveSettings> {
        let selected = if matches!(self, Self::Random) { Some(Self::random_settings(seed)) }
            else { self.settings(round,seconds) };
        let selected = if blackout {
            Some(LiveSettings::preset(DayTime::Night,selected.map_or(WeatherLook::Clear,|s|s.atmosphere)))
        } else {selected};
        selected.map(|s|if s.exposure()<MIN_EV100{s.with_exposure(MIN_EV100)}else{s})
    }

    fn random_settings(seed: u64) -> LiveSettings {
        let mut n=seed.wrapping_add(0x9e3779b97f4a7c15);
        n=(n^(n>>30)).wrapping_mul(0xbf58476d1ce4e5b9);
        n=(n^(n>>27)).wrapping_mul(0x94d049bb133111eb);n^=n>>31;
        let time=[DayTime::Sunrise,DayTime::Morning,DayTime::Noon,DayTime::Afternoon,DayTime::Sunset,DayTime::Night][(n%6) as usize];
        let weather=[WeatherLook::Clear,WeatherLook::Haze,WeatherLook::Mist][((n/6)%3) as usize];
        LiveSettings::preset(time,weather)
    }

    fn sky_settings(&self, round: u32, seconds: f32) -> Option<LiveSettings> {
        match *self {
            Self::Random => Some(Self::random_settings(round as u64)),
            Self::Godot => None,
            Self::Preset(s) => Some(s),
            Self::Rounds(w) => Some(LiveSettings::preset(DayTime::Morning.step((round.saturating_sub(1) % 6) as i8), w)),
            Self::Cycle { atmosphere, minutes } => {
                let sun = cycle_sun(seconds / (minutes * 60.0));
                Some(LiveSettings::preset(DayTime::Morning, atmosphere).with_sun(sun, exposure_for_elevation(sun.elevation_degrees)))
            }
        }
    }
}

/// The darkest exposure (EV100) Growing Guns uses: a floodlit stadium is
/// exposed for its lamps, and the sky goes black. The emissive surfaces
/// (lava, beacons, tracers) are tuned for Godot's day at EV 13.6 and are
/// shown as at that exposure under every sky (view::EMISSIVE_COMPENSATION),
/// so they keep their colour at night instead of clipping to white.
pub const MIN_EV100: f32 = 11.0;

/// Floodlight strength against Godot's (day) for the sun's elevation: 1 in
/// daylight, rising through dusk to `NIGHT_FLOODLIGHTS` below -4 deg, so the
/// lamps carry the arena at the night exposure.
pub fn floodlights(elevation_degrees: f64) -> f32 {
    1.0 + (NIGHT_FLOODLIGHTS - 1.0) * night_weight(elevation_degrees)
}

/// Full-bowl coverage needs less power than the old short-range lamps.
/// At EV 11 this keeps exposed stone readable without washing out the night.
pub const NIGHT_FLOODLIGHTS: f32 = 4.0;

/// Volumetric light (the floodlight beams, haze around the lamps) for a sky:
/// only once the sun is below the horizon (fading in to -6 deg), so day,
/// sunrise and sunset look exactly as before; thicker air in haze and mist.
/// The default `godot` sky never gets it.
pub fn volumetric(settings: &LiveSettings) -> Option<Volumetric> {
    let elevation = settings.solar().elevation_degrees;
    if elevation >= 0.0 { return None; }
    let t = (-elevation / 6.0).clamp(0.0, 1.0) as f32;
    let night = t * t * (3.0 - 2.0 * t);
    let density = match settings.atmosphere { WeatherLook::Clear => NIGHT_AIR_DENSITY, WeatherLook::Haze => 2.5 * NIGHT_AIR_DENSITY, WeatherLook::Mist => 5.0 * NIGHT_AIR_DENSITY };
    Some(Volumetric { density: density * night, ..NIGHT_AIR })
}

/// Modest stadium bounce approximation, fading in after sunset.
/// Surface AO/contact shadows still occlude it; blackout explicitly disables it.
pub fn indirect_fill(elevation_degrees: f64) -> f32 {
    let t = (-elevation_degrees / 6.0).clamp(0.0, 1.0) as f32;
    100.0 * t*t*(3.0-2.0*t)
}

/// Scattering per metre of clear night air over the arena (`volumetric`).
pub const NIGHT_AIR_DENSITY: f32 = 0.0003;
/// The night medium apart from its density: slightly warm, forward scattering.
pub const NIGHT_AIR: Volumetric = Volumetric { density: NIGHT_AIR_DENSITY, color: glam::Vec3::new(1.0, 0.97, 0.92), anisotropy: 0.45, intensity: 1.0, extinction: 0.0, ambient: glam::Vec3::ZERO, length: 0.0 };

/// 0 in daylight, 1 below -4 deg: the dusk ramp of `floodlights`.
fn night_weight(elevation_degrees: f64) -> f32 {
    let t = ((8.0 - elevation_degrees) / 12.0).clamp(0.0, 1.0) as f32;
    t * t * (3.0 - 2.0 * t)
}

/// The sun over one cycle (`day` in days, 0 = morning): from -12 deg at
/// midnight to the noon preset's 60 deg, turning once around the sky. The
/// phase is quantised to 1/720 day (2 minutes of the day) so repeated draws
/// request the same bake.
pub fn cycle_sun(day: f32) -> SolarState {
    let step = (day as f64 * 720.0).floor() / 720.0;
    let theta = (step.fract() + 0.25) * std::f64::consts::TAU;
    SolarState::from_degrees(24.0 - 36.0 * theta.cos(), 125.0 - 180.0 + theta.to_degrees()).unwrap()
}

// All eighteen possible round skies fit in about 9 MiB. The pixels are shared by Arc.
static ROUND_SKIES: Mutex<Vec<PreparedSky>> = Mutex::new(Vec::new());

fn round_preset(time: DayTime, weather: WeatherLook) -> LiveSettings {
    let s = LiveSettings::preset(time, weather);
    if s.exposure() < MIN_EV100 { s.with_exposure(MIN_EV100) } else { s }
}

fn cached_sky(settings: LiveSettings) -> Option<PreparedSky> {
    ROUND_SKIES.lock().unwrap().iter().find(|s| s.settings == settings).cloned()
}

fn remember_round_sky(sky: &PreparedSky) {
    let mut skies = ROUND_SKIES.lock().unwrap();
    if !skies.iter().any(|s| s.settings == sky.settings) { skies.push(sky.clone()); }
}

/// Prepare the finite set used by random and round skies while the current arena is playable.
#[cfg(not(target_arch = "wasm32"))]
pub fn prepare_round_skies() {
    static START: std::sync::Once = std::sync::Once::new();
    START.call_once(|| {
        std::thread::spawn(|| {
            let mut cache = AtmosphereCache::default();
            for weather in WeatherLook::ALL {
                for time in DayTime::ALL {
                    let s = round_preset(time, weather);
                    if cached_sky(s).is_some() { continue; }
                    if let Ok(sky) = bake_live(0, s, &mut cache, BakeQuality::default()) {
                        remember_round_sky(&sky);
                    }
                }
            }
        });
    });
}

pub fn bake(settings: LiveSettings) -> Result<PreparedSky, String> {
    if let Some(sky) = cached_sky(settings) { return Ok(sky); }
    static CACHE: OnceLock<Mutex<AtmosphereCache>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().map_err(|_| "sky cache lock poisoned")?;
    let sky = bake_live(0, settings, &mut cache, BakeQuality::default())?;
    if settings == round_preset(settings.time, settings.atmosphere) { remember_round_sky(&sky); }
    Ok(sky)
}

/// The match's sky state: the choice, what is applied, and the live worker
/// for the cycle.
pub struct Sky {
    pub choice: SkyChoice,
    pub applied: Option<LiveSettings>,
    pub live: Option<LiveSky>,
}

impl Sky {
    pub fn new(choice: SkyChoice) -> Self { Self { choice, applied: None, live: None } }

    /// `--sky`, else `GG_SKY`, else randomized per arena.
    pub fn from_args(arg: Option<&str>) -> Result<SkyChoice, String> {
        match arg.map(str::to_string).or_else(|| std::env::var("GG_SKY").ok()) {
            Some(s) => SkyChoice::parse(&s),
            None => Ok(SkyChoice::Random),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_skies_vary_reproducibly_and_blackouts_are_always_night() {
        let random=SkyChoice::parse("random").unwrap();let mut seen=std::collections::BTreeSet::new();
        for seed in 0..200 {
            let sky=random.for_arena(1,0.0,seed,false).unwrap();
            assert_eq!(Some(sky),random.for_arena(1,0.0,seed,false));
            seen.insert(format!("{}:{}",sky.time.key(),sky.atmosphere.key()));
            for mode in [random,SkyChoice::Godot,SkyChoice::parse("noon:mist").unwrap(),SkyChoice::parse("cycle:haze").unwrap()] {
                let night=mode.for_arena(3,seed as f32,seed,true).unwrap();
                assert_eq!(night.time,DayTime::Night);assert!(night.solar().elevation_degrees<0.0);
            }
        }
        assert_eq!(seen.len(),18);
        let choice=SkyChoice::parse("noon:mist").unwrap();
        assert_eq!(choice.for_arena(2,0.0,1,true).unwrap().atmosphere,WeatherLook::Mist);
        assert_eq!(choice.for_arena(3,0.0,2,false).unwrap().time,DayTime::Noon);
    }

    #[test]
    fn choices_parse_and_name() {
        assert_eq!(SkyChoice::parse("godot").unwrap(), SkyChoice::Godot);
        assert_eq!(SkyChoice::parse("night").unwrap(), SkyChoice::Preset(LiveSettings::preset(DayTime::Night, WeatherLook::Clear)));
        assert_eq!(SkyChoice::parse("mist").unwrap(), SkyChoice::Preset(LiveSettings::preset(DayTime::Noon, WeatherLook::Mist)));
        assert_eq!(SkyChoice::parse("sunset:haze").unwrap().name(), "sunset:haze");
        assert_eq!(SkyChoice::parse("rounds:mist").unwrap(), SkyChoice::Rounds(WeatherLook::Mist));
        assert_eq!(SkyChoice::parse("cycle:3").unwrap(), SkyChoice::Cycle { atmosphere: WeatherLook::Clear, minutes: 3.0 });
        assert_eq!(SkyChoice::parse("cycle:haze:6").unwrap(), SkyChoice::Cycle { atmosphere: WeatherLook::Haze, minutes: 6.0 });
        for bad in ["dusk", "noon:rain", "godot:noon", "cycle:0", "noon:mist:2", "mist:noon"] { assert!(SkyChoice::parse(bad).is_err(), "{bad}"); }
    }

    #[test]
    fn rounds_step_through_the_day_and_cycle_reaches_noon_and_night() {
        let r = SkyChoice::Rounds(WeatherLook::Clear);
        let times: Vec<_> = (1..=7).map(|n| r.settings(n, 0.0).unwrap().time).collect();
        assert_eq!(times, [DayTime::Morning, DayTime::Noon, DayTime::Afternoon, DayTime::Sunset, DayTime::Night, DayTime::Sunrise, DayTime::Morning]);
        assert_eq!(SkyChoice::Godot.settings(1, 0.0), None);
        assert!((cycle_sun(0.0).elevation_degrees - 24.0).abs() < 1e-9);
        assert!((cycle_sun(0.25).elevation_degrees - 60.0).abs() < 1e-9);
        assert!((cycle_sun(0.25).azimuth_degrees - 125.0).abs() < 1e-9);
        assert!((cycle_sun(0.75).elevation_degrees + 12.0).abs() < 1e-9);
        assert_eq!(cycle_sun(0.3001), cycle_sun(0.3005), "quantised: no new bake every frame");
        let c = SkyChoice::Cycle { atmosphere: WeatherLook::Clear, minutes: 1.0 };
        let night = c.settings(1, 45.0).unwrap();
        assert_eq!((night.exposure(), night.time_key()), (MIN_EV100, "current"));
        let preset = SkyChoice::parse("night").unwrap().settings(1, 0.0).unwrap();
        assert_eq!((preset.exposure(), preset.time_key()), (MIN_EV100, "night"));
        assert_eq!(SkyChoice::parse("noon").unwrap().settings(1, 0.0).unwrap().exposure(), 14.0);
        assert_eq!((floodlights(60.0), floodlights(-12.0)), (1.0, NIGHT_FLOODLIGHTS));
    }

    #[test]
    fn volumetric_light_only_at_night_and_thicker_in_mist() {
        let at = |s: &str| volumetric(&SkyChoice::parse(s).unwrap().settings(1, 0.0).unwrap());
        for day in ["noon", "morning", "afternoon", "noon:mist", "sunrise", "sunset:haze"] { assert_eq!(at(day), None, "{day}"); }
        let (clear, haze, mist) = (at("night").unwrap(), at("night:haze").unwrap(), at("night:mist").unwrap());
        assert_eq!(clear.density, NIGHT_AIR_DENSITY);
        assert!(clear.density < haze.density && haze.density < mist.density);
        assert!(clear.validated().is_ok() && mist.validated().is_ok());
    }
}
