//! Stage-authored vegetation phenotype. Growth is a construction parameter,
//! not a simulation clock: roots, population and RNG consumption stay fixed.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Season { Spring, #[default] Summer, Autumn, Winter }

impl std::str::FromStr for Season {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s { "spring"=>Ok(Self::Spring), "summer"=>Ok(Self::Summer),
            "autumn"=>Ok(Self::Autumn), "winter"=>Ok(Self::Winter),
            _=>Err("season must be spring, summer, autumn or winter".into()) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Vegetation {
    /// Independent phenotype seed; never advances the stage layout RNG.
    pub seed: u64,
    growth: f32,
    pub season: Season,
}

impl Default for Vegetation {
    fn default() -> Self { Self { seed: 0, growth: 1.0, season: Season::Summer } }
}

impl Vegetation {
    pub fn new(seed: u64, growth: f32, season: Season) -> Result<Self, String> {
        if !growth.is_finite() || !(0.0..=1.0).contains(&growth) {
            return Err("growth must be finite and in 0..=1".into());
        }
        Ok(Self { seed, growth, season })
    }
    pub fn growth(self) -> f32 { self.growth }
    /// Relative width/height of an established juvenile through mature pine.
    /// Mature/default is exactly one, preserving existing authored geometry.
    pub fn tree_scale(self, x: f32, z: f32) -> [f32; 2] {
        if self.growth == 1.0 { return [1.0; 2]; }
        let mut h = self.seed ^ (x.to_bits() as u64).rotate_left(17) ^ (z.to_bits() as u64).rotate_left(41);
        h = (h ^ (h >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        h = (h ^ (h >> 27)).wrapping_mul(0x94d049bb133111eb);
        h ^= h >> 31;
        let r = (h >> 40) as f32 / 16_777_216.0;
        let width = 0.50 + 0.15 * r;
        let height = 0.40 + 0.15 * r;
        [width + (1.0 - width) * self.growth, height + (1.0 - height) * self.growth]
    }
    /// Evergreen needles remain green in autumn and winter. These are linear
    /// material multipliers, not deciduous leaf-loss or snow simulation.
    pub fn pine_tint(self) -> [f32; 3] {
        match self.season { Season::Spring=>[0.96,1.06,0.94], Season::Summer=>[1.0;3],
            Season::Autumn=>[0.98,0.96,0.91], Season::Winter=>[0.84,0.92,0.96] }
    }
    pub fn grass_tint(self) -> [f32; 3] {
        match self.season { Season::Spring=>[0.90,1.08,0.83], Season::Summer=>[1.0;3],
            Season::Autumn=>[1.12,0.91,0.65], Season::Winter=>[0.88,0.82,0.66] }
    }
    pub fn grass_scale(self) -> f32 {
        let season = match self.season { Season::Spring=>0.85, Season::Summer=>1.0,
            Season::Autumn=>0.80, Season::Winter=>0.55 };
        (0.35 + 0.65 * self.growth) * season
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn default_is_exact_mature_identity() {
        let v=Vegetation::default();
        assert_eq!(v.tree_scale(-17.3, 891.5),[1.0;2]);
        assert_eq!(v.grass_scale(),1.0); assert_eq!(v.pine_tint(),[1.0;3]); assert_eq!(v.grass_tint(),[1.0;3]);
    }
    #[test] fn identity_is_stable_and_growth_is_monotonic() {
        for i in 0..32 {
            let young=Vegetation::new(19,0.0,Season::Summer).unwrap().tree_scale(i as f32, -3.0);
            let mid=Vegetation::new(19,0.5,Season::Winter).unwrap().tree_scale(i as f32, -3.0);
            assert_eq!(young,Vegetation::new(19,0.0,Season::Autumn).unwrap().tree_scale(i as f32,-3.0));
            assert!(young[0]>=0.5 && young[1]>=0.4 && young[0]<mid[0] && mid[1]<1.0);
        }
        assert_ne!(Vegetation::new(19,0.0,Season::Summer).unwrap().tree_scale(8.0,3.0),Vegetation::new(20,0.0,Season::Summer).unwrap().tree_scale(8.0,3.0));
        for g in [f32::NAN,f32::INFINITY,-0.1,1.1] { assert!(Vegetation::new(0,g,Season::Summer).is_err()); }
    }
}
