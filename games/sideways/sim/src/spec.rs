//! Car numbers, read from data/cars.luau at run time so tuning needs no
//! rebuild: the game reads it again for every new stage (never inside one,
//! so a stage and its replay run on one set of numbers). Each car in the
//! file is the base car with its own numbers on top; every car is checked
//! when the file loads, so a bad edit fails there and the cars before stay.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CarSpec {
    pub name: String,
    #[serde(default)]
    pub model: String,
    pub mass: f64,
    pub inertia: [f64; 3],
    pub cg: [f64; 3],
    pub wheelbase: f64,
    pub track: f64,
    #[serde(default)]
    pub track_rear: Option<f64>,
    pub wheel_radius: f64,
    pub wheel_inertia: f64,
    pub droop_y: f64,
    pub travel: f64,
    pub spring: [f64; 2],
    pub bump: [f64; 2],
    pub rebound: [f64; 2],
    pub arb: [f64; 2],
    pub max_steer_deg: f64,
    /// Multiplies each surface's tyre friction, retaining weather differences.
    /// Old car files retain their original handling with an implicit 1.0.
    #[serde(default = "unit_scale", deserialize_with = "tyre_scale")]
    pub tyre_grip_scale: f64,
    /// Multiplies the lateral slip at peak force; lower values bite earlier.
    #[serde(default = "unit_scale", deserialize_with = "tyre_scale")]
    pub lateral_peak_scale: f64,
    pub brake: [f64; 2],
    pub handbrake: f64,
    pub drive: [f64; 2],
    pub centre_lock: f64,
    pub diff_preload: [f64; 2],
    pub diff_ramp: [f64; 2],
    pub idle_rpm: f64,
    pub limit_rpm: f64,
    pub engine_inertia: f64,
    pub torque: Vec<[f64; 2]>,
    pub friction: [f64; 2],
    pub gears: Vec<f64>,
    #[serde(rename = "final")]
    pub final_drive: f64,
    pub shift_time: f64,
    pub clutch: f64,
    pub upshift_rpm: f64,
    pub downshift_rpm: f64,
    pub drag: f64,
    pub downforce: f64,
    #[serde(default)]
    pub ride: f64,
    /// [size x, y, z, centre x, y, z]
    pub boxes: Vec<[f64; 6]>,
}

fn unit_scale() -> f64 { 1.0 }

fn tyre_scale<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
    let value = f64::deserialize(deserializer)?;
    if !value.is_finite() || !(0.5..=1.5).contains(&value) {
        return Err(serde::de::Error::custom("tyre handling scale must be finite and within 0.5..1.5"));
    }
    Ok(value)
}

impl CarSpec {
    pub fn max_steer(&self) -> f64 {
        self.max_steer_deg * (std::f64::consts::PI / 180.0)
    }
    pub fn track_rear(&self) -> f64 {
        self.track_rear.unwrap_or(self.track)
    }
}

pub const GRID: [&str; 3] = ["asti", "mv6", "toyama"];

/// All cars: "base" (also "rally2") and the grid models.
#[derive(Clone)]
pub struct Cars {
    cars: std::collections::BTreeMap<String, CarSpec>,
}

pub const DEFAULT_CARS: &str = include_str!("../../data/cars.luau");

impl Cars {
    /// The game's data file (bundle or checkout), or the copy built in.
    pub fn load(path: Option<&str>) -> Result<Cars, String> {
        let text = match path {
            Some(p) => std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?,
            None => crate::game_assets::read_data("cars.luau", DEFAULT_CARS)?,
        };
        Cars::parse(&text)
    }

    /// Cars from the text of a cars.luau.
    pub fn parse(text: &str) -> Result<Cars, String> {
        let cars: std::collections::BTreeMap<String, CarSpec> = script::data::read("cars.luau", text, "cars")?;
        if !cars.contains_key("base") {
            return Err("cars.luau: cars has no base".into());
        }
        Ok(Cars { cars })
    }

    pub fn get(&self, id: &str) -> Result<CarSpec, String> {
        let id = if id == "rally2" { "base" } else { id };
        self.cars.get(id).cloned().ok_or_else(|| format!("no car '{id}'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped cars with base's two tyre scales set (None: left out).
    fn configured(grip: Option<&str>, lateral: Option<&str>) -> Result<Cars, String> {
        let mut text = DEFAULT_CARS.to_string();
        for (line, value) in [("tyre_grip_scale = 1.12,", grip), ("lateral_peak_scale = 0.80,", lateral)] {
            assert!(text.contains(line));
            let key = line.split(' ').next().unwrap();
            text = text.replace(line, &value.map(|v| format!("{key} = {v},")).unwrap_or_default());
        }
        Cars::parse(&text)
    }

    #[test]
    fn old_car_files_keep_original_tyre_scales_and_models_inherit_tuning() {
        for id in ["base", "asti", "mv6", "toyama"] {
            let original = configured(None, None).unwrap().get(id).unwrap();
            assert_eq!((original.tyre_grip_scale, original.lateral_peak_scale), (1.0, 1.0));
            let candidate = configured(Some("1.12"), Some("0.8")).unwrap().get(id).unwrap();
            assert_eq!((candidate.tyre_grip_scale, candidate.lateral_peak_scale), (1.12, 0.8));
            assert_eq!(candidate.inertia, original.inertia);
            assert_eq!(candidate.max_steer_deg, original.max_steer_deg);
        }
    }

    #[test]
    fn invalid_tyre_scales_fail_when_the_file_loads() {
        for invalid in ["0/0", "math.huge", "-math.huge", "-1.0", "0.0", "0.49", "1.51"] {
            assert!(configured(Some(invalid), None).is_err());
            assert!(configured(None, Some(invalid)).is_err());
        }
        for bound in ["0.5", "1.5"] {
            assert!(configured(Some(bound), Some(bound)).unwrap().get("base").is_ok());
        }
    }
}

