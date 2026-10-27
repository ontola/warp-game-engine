//! Car numbers, read from data/cars.toml at run time so tuning needs no
//! rebuild. A model's table is merged over the base table.

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

/// All cars: "base" plus every model merged over it.
#[derive(Clone)]
pub struct Cars {
    table: toml::Table,
}

pub const DEFAULT_CARS: &str = include_str!("../../data/cars.toml");

impl Cars {
    /// The game's data file (bundle or checkout), or the copy built in.
    pub fn load(path: Option<&str>) -> Result<Cars, String> {
        let text = match path {
            Some(p) => std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?,
            None => crate::game_assets::read_data("cars.toml", DEFAULT_CARS)?,
        };
        let table: toml::Table = text.parse().map_err(|e| format!("cars.toml: {e}"))?;
        Ok(Cars { table })
    }

    pub fn get(&self, id: &str) -> Result<CarSpec, String> {
        let mut merged = self.table.get("base").and_then(|b| b.as_table()).cloned().ok_or("cars.toml has no [base]")?;
        if id != "rally2" && id != "base" {
            let model = self
                .table
                .get("models")
                .and_then(|m| m.get(id))
                .and_then(|m| m.as_table())
                .ok_or_else(|| format!("no car '{id}'"))?;
            for (k, v) in model {
                merged.insert(k.clone(), v.clone());
            }
        }
        toml::Value::Table(merged).try_into().map_err(|e| format!("car '{id}': {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured(grip: Option<f64>, lateral: Option<f64>) -> Cars {
        let mut table: toml::Table = DEFAULT_CARS.parse().unwrap();
        let base = table.get_mut("base").unwrap().as_table_mut().unwrap();
        for (key, value) in [("tyre_grip_scale", grip), ("lateral_peak_scale", lateral)] {
            match value { Some(value) => { base.insert(key.into(), toml::Value::Float(value)); }, None => { base.remove(key); } }
        }
        Cars { table }
    }

    #[test]
    fn old_car_files_keep_original_tyre_scales_and_models_inherit_tuning() {
        for id in ["base", "asti", "mv6", "toyama"] {
            let original = configured(None, None).get(id).unwrap();
            assert_eq!((original.tyre_grip_scale, original.lateral_peak_scale), (1.0, 1.0));
            let candidate = configured(Some(1.12), Some(0.8)).get(id).unwrap();
            assert_eq!((candidate.tyre_grip_scale, candidate.lateral_peak_scale), (1.12, 0.8));
            assert_eq!(candidate.inertia, original.inertia);
            assert_eq!(candidate.max_steer_deg, original.max_steer_deg);
        }
    }

    #[test]
    fn invalid_tyre_scales_fail_before_car_construction() {
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0, 0.0, 0.49, 1.51] {
            assert!(configured(Some(invalid), None).get("asti").is_err());
            assert!(configured(None, Some(invalid)).get("mv6").is_err());
        }
        for bound in [0.5, 1.5] {
            assert!(configured(Some(bound), Some(bound)).get("base").is_ok());
        }
    }
}
