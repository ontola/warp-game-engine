//! Caller-supplied engine-voice presets: "base" merged under each model.
//! Games own their configuration, model identifiers, and fallback policy.

use crate::engine::EngineSound;
use std::path::Path;

pub struct Sounds {
    table: toml::Table,
}

#[cfg(test)]
mod tests {
    use super::*;
    const VOICES: &str = include_str!("../tests/fixtures/engine_voices.toml");

    #[test]
    fn caller_text_controls_models_defaults_and_overrides() {
        let mut sounds=Sounds::parse(VOICES).unwrap();
        assert_eq!(sounds.ids(),["boxer_four","inline_five","inline_four"]);
        assert_eq!(sounds.for_car("inline_five").unwrap().cylinders,5);
        assert_eq!(sounds.get("inline_four").unwrap().headers,[0.55,0.58,0.58,0.55]);
        sounds.set("pipe=4.25").unwrap();
        for id in sounds.ids() {assert_eq!(sounds.get(&id).unwrap().pipe,4.25);}
        let sounds=Sounds::parse(&format!("every_car='inline_four'\n{VOICES}")).unwrap();
        assert_eq!(sounds.for_car("inline_five").unwrap().cylinders,4);
    }

    #[test]
    fn explicit_file_is_loaded_and_missing_file_does_not_use_game_defaults() {
        let path=std::env::temp_dir().join(format!("warp-audio-presets-{}-{}.toml",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::write(&path,VOICES).unwrap();
        assert_eq!(Sounds::load(&path).unwrap().get("inline_five").unwrap().cylinders,5);
        std::fs::remove_file(&path).unwrap();
        let error=Sounds::load(&path).err().expect("missing explicit file must fail");
        assert!(error.contains(path.to_str().unwrap()));
        assert!(Sounds::parse("[not valid TOML").is_err());
    }
}

impl Sounds {
    /// Parse configuration supplied by the caller, including embedded data.
    pub fn parse(text: &str) -> Result<Sounds, String> {
        Ok(Sounds { table: text.parse().map_err(|e| format!("sound configuration: {e}"))? })
    }

    /// Read one explicit configuration file. No implicit game-data lookup or
    /// fallback: the application decides whether a missing file is optional.
    pub fn load(path: impl AsRef<Path>) -> Result<Sounds, String> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
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

    /// Model identifiers supplied by the configuration.
    pub fn ids(&self) -> Vec<String> {
        self.table.get("models").and_then(|m| m.as_table()).map(|m| m.keys().cloned().collect()).unwrap_or_default()
    }

    /// The sound for a car of model `id`: the `every_car` entry when the
    /// file sets one, else the model's own.
    pub fn for_car(&self, id: &str) -> Result<EngineSound, String> {
        match self.table.get("every_car").and_then(|v| v.as_str()) {
            Some(all) if !all.is_empty() => self.get(all),
            _ => self.get(id),
        }
    }

    pub fn get(&self, id: &str) -> Result<EngineSound, String> {
        let mut merged = self.table.get("base").and_then(|b| b.as_table()).cloned().ok_or("sound configuration has no [base]")?;
        if id != "base" {
            let model = self.table.get("models").and_then(|m| m.get(id)).and_then(|m| m.as_table()).ok_or_else(|| format!("no engine sound '{id}'"))?;
            for (k, v) in model {
                merged.insert(k.clone(), v.clone());
            }
        }
        toml::Value::Table(merged).try_into().map_err(|e| format!("engine sound '{id}': {e}"))
    }
}
