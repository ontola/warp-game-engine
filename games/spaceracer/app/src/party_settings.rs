use script::{
    Host, Limits,
    mlua::{LuaSerdeExt, Value as LuaValue},
};
use serde_json::{Value, json};
use std::path::Path;
#[derive(serde::Deserialize)]
pub struct Live {
    pub cost: f64,
    pub duration: f64,
    pub refill: f64,
    pub recovery: f64,
    pub lod: f32,
    pub blur: bool,
    pub near_shadows: bool,
}
pub struct Settings {
    host: Host,
    pub values: Value,
    pub specs: Value,
}
impl Settings {
    pub fn new(root: &Path) -> Result<Self, String> {
        let mut host = Host::new(Limits::default(), |_, _| Ok(()))?;
        host.add_source(
            "party.luau",
            &vfs::read_to_string(root.join("rules/party.luau")).map_err(|e| e.to_string())?,
        )?;
        let specs: Value = host
            .lua()
            .from_value(
                host.modules()[0]
                    .exports
                    .as_ref()
                    .unwrap()
                    .get::<LuaValue>("specs")
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        let values = Value::Object(
            specs
                .as_array()
                .ok_or("settings array")?
                .iter()
                .map(|s| (s["key"].as_str().unwrap().into(), s["default"].clone()))
                .collect(),
        );
        Ok(Self {
            host,
            values,
            specs,
        })
    }
    pub fn set(&mut self, key: &str, value: Value) -> Result<bool, String> {
        let valid: bool = self.call("validate", json!({"key":key,"value":value}))?;
        if valid {
            self.values[key] = value;
        }
        Ok(valid)
    }
    fn call<T: serde::de::DeserializeOwned>(&self, name: &str, arg: Value) -> Result<T, String> {
        let f: script::mlua::Function = self.host.modules()[0]
            .exports
            .as_ref()
            .unwrap()
            .get(name)
            .map_err(|e| e.to_string())?;
        let out = f
            .call(self.host.lua().to_value(&arg).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        self.host.lua().from_value(out).map_err(|e| e.to_string())
    }
    pub fn live(&self) -> Result<Live, String> {
        self.call("live", self.values.clone())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_validate_and_restore_without_changing_other_values() {
        let mut s = Settings::new(Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()).unwrap();
        assert_eq!(s.specs.as_array().unwrap().len(), 11);
        for (k, v) in [
            ("boost_cost", json!(35)),
            ("boost_duration", json!(150)),
            ("energy_refill", json!(200)),
            ("respawn_seconds", json!(4)),
            ("weapon_pickups", json!(false)),
        ] {
            assert!(s.set(k, v).unwrap());
        }
        let live = s.live().unwrap();
        assert_eq!(
            (live.cost, live.duration, live.refill, live.recovery),
            (35., 1.875, 2., 4.)
        );
        let old = s.values.clone();
        for (k, v) in [
            ("seed", json!(-1)),
            ("laps", json!(2.5)),
            ("boost_cost", json!("22")),
            ("biome", json!("bad")),
            ("weapon_pickups", json!(1)),
            ("players", json!(2)),
        ] {
            assert!(!s.set(k, v).unwrap());
        }
        assert_eq!(old, s.values);
        assert!(s.set("boost_cost", json!(22)).unwrap());
        assert_eq!(s.live().unwrap().cost, 22.);
    }
}
