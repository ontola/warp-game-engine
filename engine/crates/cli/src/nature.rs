//! Shared explicit stage phenotype and animation options for play and shots.
#[derive(Clone, Copy, Debug)]
pub struct Nature {
    pub vegetation: sideways::vegetation::Vegetation,
    pub wind_strength: f32,
    pub wind_time: Option<f64>,
}

impl Default for Nature {
    fn default() -> Self { Self { vegetation: Default::default(), wind_strength: 0.3, wind_time: None } }
}

impl Nature {
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let value = |key: &str| -> Result<Option<&str>, String> {
            args.iter().position(|s| s == key).map(|i| args.get(i+1).map(String::as_str)
                .filter(|s| !s.starts_with("--")).ok_or_else(|| format!("{key} requires a value"))).transpose()
        };
        let growth=value("--growth")?.unwrap_or("1").parse().map_err(|_|"invalid growth")?;
        let seed=value("--vegetation-seed")?.unwrap_or("0").parse().map_err(|_|"invalid vegetation seed")?;
        let season=value("--season")?.unwrap_or("summer").parse()?;
        let wind_strength=value("--wind")?.unwrap_or("0.3").parse::<f32>().map_err(|_|"invalid wind")?;
        if !wind_strength.is_finite() || !(0.0..=1.0).contains(&wind_strength) { return Err("wind must be finite and in 0..=1".into()); }
        let wind_time=value("--wind-time")?.map(|s|s.parse::<f64>().map_err(|_|"invalid wind time")).transpose()?;
        if wind_time.is_some_and(|t| !t.is_finite() || !(0.0..=1.0e9).contains(&t)) { return Err("wind time must be finite and in 0..=1e9 seconds".into()); }
        Ok(Self { vegetation: sideways::vegetation::Vegetation::new(seed,growth,season)?, wind_strength, wind_time })
    }
    pub fn wind(self, simulation_seconds: f64) -> render::wind::WindState {
        render::wind::WindState { time_seconds:self.wind_time.unwrap_or(simulation_seconds), strength:self.wind_strength, ..Default::default() }
    }
    pub fn metadata(self, simulation_seconds: f64) -> serde_json::Value {
        let w=self.wind(simulation_seconds);
        serde_json::json!({"vegetation":self.vegetation,"growth_mode":"stage_construction",
            "pine_tint_linear":self.vegetation.pine_tint(),"grass_tint_linear":self.vegetation.grass_tint(),"grass_scale":self.vegetation.grass_scale(),
            "wind":{"time_seconds":w.time_seconds,"time_source":if self.wind_time.is_some(){"fixed"}else{"simulation_ticks_120hz"},
                "strength":w.strength,"direction_xz":w.direction.to_array(),"tree_shear":w.tree_shear,"grass_shear":w.grass_shear},
            "limits":["Growth is authored size at stage construction, not branch growth or a biological simulation.","Pines remain evergreen; winter grass is dormant, without snow accumulation.","Packed experimental voxel tree owners remain wind-disabled; canopy extinction uses mean rest poses."]})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(s:&str)->Result<Nature,String>{Nature::parse(&s.split_whitespace().map(str::to_owned).collect::<Vec<_>>())}
    #[test] fn validates_explicit_nature_options_and_fixed_phase() {
        let n=parse("--season winter --growth 0.4 --vegetation-seed 19 --wind 0.8 --wind-time 2.5").unwrap();
        assert_eq!(n.vegetation.season,sideways::vegetation::Season::Winter);
        assert_eq!(n.vegetation.seed,19);assert_eq!(n.wind(99.0).time_seconds,2.5);
        assert_eq!(parse("").unwrap().vegetation,Default::default());
        for bad in ["--growth NaN","--growth 2","--wind -1","--wind-time inf","--season fall","--wind","--growth --season winter"] {assert!(parse(bad).is_err(),"{bad}");}
    }
}
