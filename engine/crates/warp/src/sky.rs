//! Sky names for `sky.set`, `--sky` and `game.toml`: `gradient` (a flat
//! blue gradient and a fixed sun, cheap) or a `render::sky` preset,
//! `TIME[:WEATHER]` with TIME sunrise|morning|noon|afternoon|sunset|night and
//! WEATHER clear|haze|mist (a bare weather means noon).
use render::sky::{DayTime, LiveSettings, WeatherLook};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SkyChoice {
    Gradient,
    Preset(LiveSettings),
}

impl SkyChoice {
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim().to_ascii_lowercase();
        if s == "gradient" || s.is_empty() { return Ok(Self::Gradient); }
        let (head, rest) = match s.split_once(':') { Some((a, b)) => (a, Some(b)), None => (s.as_str(), None) };
        let usage = || format!("unknown sky {s:?}: gradient, or TIME[:WEATHER] with TIME sunrise|morning|noon|afternoon|sunset|night and WEATHER clear|haze|mist");
        let (time, weather) = match (DayTime::parse(head), WeatherLook::parse(head)) {
            (Ok(t), _) => (t, match rest { Some(w) => WeatherLook::parse(w).map_err(|_| usage())?, None => WeatherLook::Clear }),
            (_, Ok(w)) if rest.is_none() => (DayTime::Noon, w),
            _ => return Err(usage()),
        };
        Ok(Self::Preset(LiveSettings::preset(time, weather)))
    }

    pub fn name(&self) -> String {
        match self {
            Self::Gradient => "gradient".into(),
            Self::Preset(s) => format!("{}:{}", s.time.key(), s.atmosphere.key()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_parse() {
        assert_eq!(SkyChoice::parse("gradient").unwrap(), SkyChoice::Gradient);
        assert_eq!(SkyChoice::parse("sunset:haze").unwrap().name(), "sunset:haze");
        assert_eq!(SkyChoice::parse("Night").unwrap().name(), "night:clear");
        assert_eq!(SkyChoice::parse("mist").unwrap().name(), "noon:mist");
        for bad in ["dusk", "noon:rain", "mist:noon"] { assert!(SkyChoice::parse(bad).is_err(), "{bad}"); }
    }
}
