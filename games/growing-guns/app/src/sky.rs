//! The match's sky, rendering only (the simulation never sees it):
//!
//! - `godot` (default): Godot's gradient sky and fixed sun (`view::sky`,
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
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SkyChoice {
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
        format!("unknown sky {s}: godot | TIME[:WEATHER] | rounds[:WEATHER] | cycle[:WEATHER][:MINUTES]; \
            TIME sunrise|morning|noon|afternoon|sunset|night, WEATHER clear|haze|mist")
    }

    pub fn name(&self) -> String {
        match self {
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

    fn sky_settings(&self, round: u32, seconds: f32) -> Option<LiveSettings> {
        match *self {
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
    let t = ((8.0 - elevation_degrees) / 12.0).clamp(0.0, 1.0) as f32;
    1.0 + (NIGHT_FLOODLIGHTS - 1.0) * t * t * (3.0 - 2.0 * t)
}

/// Tuned on the night captures (scripts/sky.json): at EV 11 and 12x the floor,
/// walls and players read clearly. The banks point
/// at the centre, so the crowd in the stands falls dark.
pub const NIGHT_FLOODLIGHTS: f32 = 12.0;

/// The sun over one cycle (`day` in days, 0 = morning): from -12 deg at
/// midnight to the noon preset's 60 deg, turning once around the sky. The
/// phase is quantised to 1/720 day (2 minutes of the day) so repeated draws
/// request the same bake.
pub fn cycle_sun(day: f32) -> SolarState {
    let step = (day as f64 * 720.0).floor() / 720.0;
    let theta = (step.fract() + 0.25) * std::f64::consts::TAU;
    SolarState::from_degrees(24.0 - 36.0 * theta.cos(), 125.0 - 180.0 + theta.to_degrees()).unwrap()
}

/// A synchronous bake, shared by every match in the process (one atmosphere
/// cache: the transmittance table survives sun changes of one weather).
pub fn bake(settings: LiveSettings) -> Result<PreparedSky, String> {
    static CACHE: OnceLock<Mutex<AtmosphereCache>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().map_err(|_| "sky cache lock poisoned")?;
    bake_live(0, settings, &mut cache, BakeQuality::default())
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

    /// `--sky`, else `GG_SKY`, else Godot's.
    pub fn from_args(arg: Option<&str>) -> Result<SkyChoice, String> {
        match arg.map(str::to_string).or_else(|| std::env::var("GG_SKY").ok()) {
            Some(s) => SkyChoice::parse(&s),
            None => Ok(SkyChoice::Godot),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
