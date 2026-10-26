//! Sideways-owned presentation preset. Warp's neutral defaults stay reusable.
//! Read once per play invocation; live edits are not silently written to disk.
use render::debug_settings::DebugRenderOptions;
use crate::tree_tuning;

const BUILT_IN: &str = include_str!("../../data/render-baseline.toml");

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Origin { Saved, Authored }
impl Origin {
    pub fn label(self) -> &'static str { match self { Self::Saved=>"data/render-baseline.toml", Self::Authored=>"SIDEWAYS_RENDER_BASELINE=authored" } }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Preset {
    pub grass: DebugRenderOptions,
    pub trees: tree_tuning::Settings,
    pub origin: Origin,
}

impl Preset {
    pub fn load() -> Result<Self, String> {
        let mode=match std::env::var("SIDEWAYS_RENDER_BASELINE") {
            Ok(value)=>Some(value),
            Err(std::env::VarError::NotPresent)=>None,
            Err(_)=>return Err("SIDEWAYS_RENDER_BASELINE is not valid Unicode".into()),
        };
        if Self::authored_mode(mode.as_deref())? {return Ok(Self::authored());}
        let text = sideways::game_assets::read_data("render-baseline.toml", BUILT_IN)?;
        Self::parse(&text)
    }

    fn authored_mode(value:Option<&str>)->Result<bool,String> {
        match value {
            None|Some("")|Some("saved")=>Ok(false),
            Some("authored")=>Ok(true),
            Some(_)=>Err("SIDEWAYS_RENDER_BASELINE must be saved or authored".into()),
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let value: toml::Value = toml::from_str(text).map_err(|e| format!("render baseline: {e}"))?;
        if value.get("schema").and_then(toml::Value::as_integer) != Some(1) {
            return Err("render baseline requires schema = 1".into());
        }
        let number = |table: &str, key: &str| -> Result<f32, String> {
            let value = value.get(table).and_then(|v| v.get(key)).ok_or_else(|| format!("render baseline missing {table}.{key}"))?;
            value.as_float().or_else(|| value.as_integer().map(|n| n as f64))
                .map(|v| v as f32).filter(|v| v.is_finite())
                .ok_or_else(|| format!("render baseline {table}.{key} must be a finite number"))
        };
        let rgb = |table: &str| -> Result<[f32; 3], String> {
            let values = value.get(table).and_then(|v| v.get("color_multiplier")).and_then(toml::Value::as_array)
                .filter(|v| v.len() == 3).ok_or_else(|| format!("render baseline {table}.color_multiplier needs three numbers"))?;
            let mut result = [0.0; 3];
            for (dst, src) in result.iter_mut().zip(values) {
                *dst = src.as_float().or_else(|| src.as_integer().map(|n| n as f64))
                    .ok_or_else(|| format!("render baseline {table}.color_multiplier must be numeric"))? as f32;
            }
            Ok(result)
        };
        let trees = tree_tuning::Settings { size_scale: number("trees", "size_scale")?,
            density: number("trees", "density")?, color_multiplier: rgb("trees")? };
        let grass = DebugRenderOptions { meadow_size_scale: number("grass", "size_scale")?,
            meadow_density: number("grass", "density")?, meadow_color_multiplier: rgb("grass")?,
            meadow_lod_pixels: number("grass", "lod_pixels")?, meadow_distance_scale: number("grass", "distance_scale")?,
            ..Default::default() };
        trees.validate()?; grass.validate()?;
        Ok(Self { grass, trees, origin:Origin::Saved })
    }

    pub fn authored() -> Self { Self { grass: Default::default(), trees: Default::default(), origin:Origin::Authored } }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_baseline_is_the_applied_playtest_state_not_the_sparse_reference() {
        let preset = Preset::parse(BUILT_IN).unwrap();
        assert_eq!(preset.trees, tree_tuning::Settings { size_scale: 1.5, density: 0.45, color_multiplier: [1.0, 1.15, 0.9] });
        assert_eq!(preset.grass.meadow_size_scale, 2.0);
        assert_eq!(preset.grass.meadow_density, 0.55);
        assert_eq!(preset.grass.meadow_color_multiplier, [1.3, 0.9, 1.3]);
        assert_eq!(preset.grass.meadow_lod_pixels, 4.0);
        assert_eq!(preset.grass.meadow_distance_scale, 1.0);
        let reference: toml::Value = toml::from_str(BUILT_IN).unwrap();
        assert_eq!(reference["references"]["grass_sparse"]["density"].as_float(), Some(0.15));
        let neutral = DebugRenderOptions::default();
        assert_eq!(neutral.meadow_density, 1.0); assert_eq!(neutral.meadow_size_scale, 1.0);
        assert_eq!(neutral.meadow_color_multiplier, [1.0; 3]); assert_eq!(neutral.meadow_lod_pixels, 1.0);
    }
    #[test]
    fn invalid_baseline_is_rejected_instead_of_silently_changing_the_scene() {
        for text in [BUILT_IN.replace("schema = 1", "schema = 2"),
            BUILT_IN.replacen("size_scale = 1.5", "size_scale = nan", 1),
            BUILT_IN.replacen("density = 0.45", "density = 1.1", 1),
            BUILT_IN.replacen("lod_pixels = 4.0", "lod_pixels = 5.0", 1),
            BUILT_IN.replacen("color_multiplier = [1.0, 1.15, 0.9]", "color_multiplier = [1.0, 0.9]", 1)] {
            assert!(Preset::parse(&text).is_err());
        }
    }
    #[test]
    fn authored_mode_is_explicit_and_keeps_engine_defaults() {
        assert!(!Preset::authored_mode(None).unwrap());
        assert!(!Preset::authored_mode(Some("saved")).unwrap());
        assert!(Preset::authored_mode(Some("authored")).unwrap());
        assert!(Preset::authored_mode(Some("auto")).is_err());
        let authored=Preset::authored();
        assert_eq!(authored.grass,DebugRenderOptions::default());
        assert_eq!(authored.trees,tree_tuning::Settings::default());
        assert_eq!(authored.origin.label(),"SIDEWAYS_RENDER_BASELINE=authored");
    }
}
