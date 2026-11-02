//! Stage-authored vegetation phenotype. Growth is a construction parameter,
//! not a simulation clock: roots, population and RNG consumption stay fixed.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Season { Spring, #[default] Summer, Autumn, Winter }

/// Visual families derived from the three retained evergreen source meshes.
/// These names describe silhouettes, not botanically distinct scanned species.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum TreeArchetype { Canopy, Conifer, Stout }

impl TreeArchetype {
    pub const ALL: [Self; 3] = [Self::Canopy, Self::Conifer, Self::Stout];
    pub fn index(self) -> usize { self as usize }
    pub fn label(self) -> &'static str {
        match self { Self::Canopy => "canopy", Self::Conifer => "tapered conifer", Self::Stout => "stout trunk" }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TreePhenotype {
    pub archetype: TreeArchetype,
    pub juvenile: bool,
    /// Relative width and height, including the stage growth parameter.
    /// Both the visible instance and the trunk collider use these values.
    pub scale: [f32; 2],
}

fn mix(mut h: u64) -> u64 {
    h = (h ^ (h >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94d049bb133111eb);
    h ^ (h >> 31)
}
fn unit(h: u64) -> f32 { (h >> 40) as f32 / 16_777_216.0 }

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
    /// The land's own grass colour relative to Sideways' (a biome's; 1 is
    /// Sideways' Finnish summer).
    pub land_tint: [f32; 3],
}

impl Default for Vegetation {
    fn default() -> Self { Self { seed: 0, growth: 1.0, season: Season::Summer, land_tint: [1.0; 3] } }
}

impl Vegetation {
    pub fn new(seed: u64, growth: f32, season: Season) -> Result<Self, String> {
        if !growth.is_finite() || !(0.0..=1.0).contains(&growth) {
            return Err("growth must be finite and in 0..=1".into());
        }
        Ok(Self { seed, growth, season, land_tint: [1.0; 3] })
    }
    pub fn growth(self) -> f32 { self.growth }
    /// Stateless identity shared by near trees, background trees and their
    /// derived representations. Changing season/growth does not change family
    /// or age, and this never consumes the stage placement RNG.
    pub fn tree(self, stage_seed: u64, x: f32, z: f32) -> TreePhenotype {
        let identity = mix(self.seed ^ stage_seed.rotate_left(23)
            ^ (x.to_bits() as u64).rotate_left(17) ^ (z.to_bits() as u64).rotate_left(41));
        let family = unit(mix(identity ^ 0x734d_f029_4ae1_62b5));
        let archetype = if family < 0.55 { TreeArchetype::Canopy }
            else if family < 0.85 { TreeArchetype::Conifer } else { TreeArchetype::Stout };
        let juvenile = unit(mix(identity ^ 0x191d_8126_9083_3fb1)) < 0.12;
        let size = unit(mix(identity ^ 0x86d9_37a8_5421_e3c5));
        let age = if juvenile { [0.46 + 0.16 * size, 0.42 + 0.18 * size] } else { [1.0; 2] };
        let family_scale = match archetype {
            TreeArchetype::Canopy => [1.0, 1.0],
            TreeArchetype::Conifer => [0.84, 1.20],
            TreeArchetype::Stout => [1.42, 0.96],
        };
        let growth = self.tree_scale(x, z);
        TreePhenotype { archetype, juvenile,
            scale: [family_scale[0] * age[0] * growth[0], family_scale[1] * age[1] * growth[1]] }
    }
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
        let t = self.season_grass_tint();
        std::array::from_fn(|i| t[i] * self.land_tint[i])
    }
    fn season_grass_tint(self) -> [f32; 3] {
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

    #[test] fn tree_families_are_stable_bounded_and_include_occasional_juveniles() {
        let mut families = [0; 3]; let mut young = 0; let mut changed = 0;
        let summer = Vegetation::new(19, 1.0, Season::Summer).unwrap();
        let winter = Vegetation::new(19, 0.3, Season::Winter).unwrap();
        for i in 0..10_000 {
            let x = i as f32 * 1.337; let z = -3.17 * i as f32;
            let a = summer.tree(77, x, z); let b = winter.tree(77, x, z);
            assert_eq!(a, summer.tree(77, x, z));
            assert_eq!((a.archetype, a.juvenile), (b.archetype, b.juvenile));
            assert!(a.scale[0] > 0.38 && a.scale[0] <= 1.42 && a.scale[1] > 0.4 && a.scale[1] <= 1.2);
            assert!(b.scale[0] < a.scale[0] && b.scale[1] < a.scale[1]);
            families[a.archetype.index()] += 1; young += usize::from(a.juvenile);
            changed += usize::from(a != summer.tree(78, x, z));
        }
        assert!((5000..6000).contains(&families[0]));
        assert!((2500..3500).contains(&families[1]));
        assert!((1000..2000).contains(&families[2]));
        assert!((900..1500).contains(&young));
        assert!(changed > 4000, "stage seed must affect visual identities");
    }
}
