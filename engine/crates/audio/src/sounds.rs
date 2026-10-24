//! data/sounds.toml: an engine per car, "base" merged under each model.

use crate::engine::EngineSound;

pub const DEFAULT_SOUNDS: &str = include_str!("../../../data/sounds.toml");

pub struct Sounds {
    table: toml::Table,
}

impl Sounds {
    /// The data file beside the engine (read every run: retune without a
    /// rebuild), else the built-in copy, or `path`.
    pub fn load(path: Option<&str>) -> Result<Sounds, String> {
        let text = match path {
            Some(p) => std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?,
            None => {
                let local = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/sounds.toml");
                std::fs::read_to_string(local).unwrap_or_else(|_| DEFAULT_SOUNDS.to_string())
            }
        };
        Ok(Sounds { table: text.parse().map_err(|e| format!("sounds.toml: {e}"))? })
    }

    /// Override `key=value` (TOML value) in [base] and every model: try a
    /// number without editing the file (`--set whine=0`).
    pub fn set(&mut self, kv: &str) -> Result<(), String> {
        let (k, v) = kv.split_once('=').ok_or_else(|| format!("expected key=value, got {kv}"))?;
        let value: toml::Table = format!("v = {v}").parse().map_err(|e| format!("{kv}: {e}"))?;
        let value = value["v"].clone();
        if let Some(b) = self.table.get_mut("base").and_then(|b| b.as_table_mut()) {
            b.insert(k.into(), value.clone());
        }
        if let Some(m) = self.table.get_mut("models").and_then(|m| m.as_table_mut()) {
            for (_, t) in m.iter_mut() {
                if let Some(t) = t.as_table_mut() {
                    if t.contains_key(k) {
                        t.insert(k.into(), value.clone());
                    }
                }
            }
        }
        Ok(())
    }

    /// Model ids, as in cars.toml.
    pub fn ids(&self) -> Vec<String> {
        self.table.get("models").and_then(|m| m.as_table()).map(|m| m.keys().cloned().collect()).unwrap_or_default()
    }

    pub fn get(&self, id: &str) -> Result<EngineSound, String> {
        let mut merged = self.table.get("base").and_then(|b| b.as_table()).cloned().ok_or("sounds.toml has no [base]")?;
        if id != "base" {
            let model = self.table.get("models").and_then(|m| m.get(id)).and_then(|m| m.as_table()).ok_or_else(|| format!("no engine sound '{id}'"))?;
            for (k, v) in model {
                merged.insert(k.clone(), v.clone());
            }
        }
        toml::Value::Table(merged).try_into().map_err(|e| format!("engine sound '{id}': {e}"))
    }
}
