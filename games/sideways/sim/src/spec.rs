//! Car numbers, read from data/cars.luau at run time so tuning needs no
//! rebuild: the game reads it again for every new stage (never inside one,
//! so a stage and its replay run on one set of numbers). Each car in the
//! file is the base car with its own numbers on top; every car is checked
//! when the file loads, so a bad edit fails there and the cars before stay.

pub use physics::vehicle::CarSpec;

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

