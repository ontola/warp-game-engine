//! Biomes, seasons and the time of day as data: what a procedural world
//! looks like, chosen by a game or artist instead of written in code. A
//! `Biome` holds the terrain's shape (terrain_field::Style), its palette
//! (VegetationStyle) and how much grows; an `Environment` adds the season
//! and the hour, which shift the palette, lay seasonal snow and place the
//! sun. The same track or map can then be a summer polder at noon or a
//! winter tundra at dusk.
//!
//! ```ignore
//! let env = Environment { biome: Biome::named("polder").unwrap(), season: Season::AUTUMN, hour: 17.5, latitude_deg: 52.0 };
//! let terrain = env.biome.terrain;                    // terrain_field::eval, procedural_terrain::reserve
//! let palette = env.vegetation();                     // procedural_terrain::vegetation_style, far_forest
//! let (elevation, azimuth) = env.sun_degrees();       // SolarState::from_degrees
//! ```
use crate::forest_field::ForestStyle;
use crate::terrain_field::Style;
use crate::vegetation_style::{GroundStyle, VegetationStyle};

/// One kind of land: its terrain, palette and vegetation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Biome {
    pub name: &'static str,
    pub terrain: Style,
    /// The summer palette; `Environment::vegetation` turns it with the season.
    pub vegetation: VegetationStyle,
    /// Share of forest ground that grows a tree (forest_field's density).
    pub tree_density: f32,
    /// Share of the trees that drop their leaves (broadleaf): they colour in
    /// autumn and stand bare in winter. The rest are evergreen.
    pub deciduous: f32,
}

/// Every preset, for menus and tests.
pub const NAMES: [&str; 6] = ["temperate", "alpine", "lowland", "polder", "tundra", "desert"];

impl Biome {
    /// A preset: temperate (the engine's default), alpine, lowland, polder,
    /// tundra, desert. "default" is temperate.
    pub fn named(name: &str) -> Option<Self> {
        let base = VegetationStyle::default();
        let terrain = Style::default();
        Some(match name {
            "temperate" | "default" => Biome { name: "temperate", terrain, vegetation: base, tree_density: 0.6, deciduous: 0.2 },
            "alpine" => Biome { name: "alpine", terrain: Style::alpine(), vegetation: VegetationStyle::named("alpine")?,
                tree_density: 0.6, deciduous: 0.1 },
            "lowland" => Biome { name: "lowland", terrain: Style::lowland(), vegetation: VegetationStyle::named("lowland")?,
                tree_density: 0.5, deciduous: 0.4 },
            // Dead-flat reclaimed land: water everywhere in channels, lush
            // grass, a few rows of trees.
            "polder" => Biome { name: "polder",
                terrain: Style { mountains: 0.0, peaks: 0.0, hills: 0.04, rivers: 1.0, streams: 0.9, rocks: 0.0, boulders: 0.0,
                    moor: 0.05, grass: 1.0, ..terrain },
                vegetation: VegetationStyle {
                    ground: GroundStyle { grass_dry: [0.36, 0.6, 0.22, 0.3], grass_lush: [0.26, 0.7, 0.24, 0.0], ..base.ground },
                    forest: ForestStyle { far: [0.075, 0.11, 0.04, 14.0], crown: [3.2, 0.375, 1.6, 0.6], ..base.forest },
                    meadow_dry_season: 0.0 },
                tree_density: 0.15, deciduous: 0.8 },
            // Cold open land: low rolling hills, moss and heather, ponds,
            // stunted spruce here and there, snow on any height.
            "tundra" => Biome { name: "tundra",
                terrain: Style { mountains: 0.15, peaks: 0.0, hills: 0.5, rivers: 0.6, streams: 0.7, rocks: 0.5, boulders: 1.4,
                    moor: 1.0, grass: 0.6, snow_line: 600.0, ..terrain },
                vegetation: VegetationStyle {
                    ground: GroundStyle { grass_dry: [0.5, 0.5, 0.3, 0.5], grass_lush: [0.4, 0.55, 0.3, 0.0],
                        moor: [0.62, 0.6, 0.55, 0.7], ..base.ground },
                    forest: ForestStyle { needles: [0.04, 0.075, 0.035, 0.5], ..base.forest },
                    meadow_dry_season: 0.5 },
                tree_density: 0.12, deciduous: 0.0 },
            // Dry rocky land: no water, no grass, sand-coloured ground.
            "desert" => Biome { name: "desert",
                terrain: Style { mountains: 0.35, peaks: 0.1, hills: 1.3, rivers: 0.0, streams: 0.0, rocks: 0.7, boulders: 1.2,
                    moor: 0.0, grass: 0.0, snow_line: 4000.0, ..terrain },
                vegetation: VegetationStyle {
                    ground: GroundStyle { grass_dry: [1.25, 0.95, 0.55, 0.0], grass_lush: [1.15, 0.9, 0.55, 0.0],
                        soil: [1.1, 0.85, 0.6, 0.1], gravel: [1.1, 0.95, 0.8, 0.6], rock: [1.15, 0.95, 0.8, 0.6], ..base.ground },
                    meadow_dry_season: 1.0, ..base },
                tree_density: 0.0, deciduous: 0.0 },
            _ => return None,
        })
    }
}

/// The time of year, 0..1: 0 spring equinox, 0.25 midsummer, 0.5 autumn
/// equinox, 0.75 midwinter (northern hemisphere).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Season(pub f32);

impl Season {
    pub const SPRING: Season = Season(0.1);
    pub const SUMMER: Season = Season(0.3);
    pub const AUTUMN: Season = Season(0.55);
    pub const WINTER: Season = Season(0.8);

    /// "spring", "summer", "autumn", "winter" or a number 0..1.
    pub fn named(name: &str) -> Option<Self> {
        Some(match name {
            "spring" => Self::SPRING, "summer" => Self::SUMMER, "autumn" | "fall" => Self::AUTUMN, "winter" => Self::WINTER,
            n => Season(n.parse::<f32>().ok()?.rem_euclid(1.0)),
        })
    }

    /// How far from summer, 0 at midsummer to 1 at midwinter.
    /// Zero through high summer, so summer is the biome's own palette.
    fn cold(self) -> f32 { ((0.5 - 0.5 * ((self.0 - 0.25) * std::f32::consts::TAU).cos() - 0.05) / 0.95).clamp(0.0, 1.0) }
    /// Autumn colour, peaking just after the autumn equinox.
    fn fall(self) -> f32 { (1.0 - ((self.0 - 0.58) / 0.12).powi(2)).max(0.0) }
    /// Fresh spring growth.
    fn spring(self) -> f32 { (1.0 - ((self.0 - 0.1) / 0.12).powi(2)).max(0.0) }
    /// Seasonal snow lying on open ground, by how cold the biome gets.
    fn snow(self, biome: &Biome) -> f32 {
        let winter = ((self.cold() - 0.7) / 0.3).clamp(0.0, 1.0);
        // Cold biomes keep their snow longer; deserts never get it.
        let climate = (1.6 - biome.terrain.snow_line / 1500.0).clamp(0.0, 1.0);
        winter * climate
    }
}

/// A biome at a time of year and day.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Environment {
    pub biome: Biome,
    pub season: Season,
    /// Local solar time, hours 0..24 (12 is noon).
    pub hour: f32,
    /// For the sun's path; 52 is the Netherlands.
    pub latitude_deg: f32,
}

fn mix3(a: [f32; 4], b: [f32; 3], t: f32) -> [f32; 4] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t, a[3]]
}

impl Environment {
    pub fn new(biome: Biome) -> Self { Environment { biome, season: Season::SUMMER, hour: 14.0, latitude_deg: 52.0 } }

    /// The biome's palette at this season: fresh grass in spring, dry and
    /// yellow in late summer and autumn, brown under snow in winter; the
    /// deciduous canopy colours in autumn and stands grey-brown in winter.
    pub fn vegetation(&self) -> VegetationStyle {
        let mut v = self.biome.vegetation;
        let (cold, fall, spring) = (self.season.cold(), self.season.fall(), self.season.spring());
        let g = &mut v.ground;
        g.grass_dry = mix3(g.grass_dry, [0.32, 0.62, 0.2], 0.5 * spring);
        g.grass_lush = mix3(g.grass_lush, [0.24, 0.72, 0.2], 0.5 * spring);
        g.grass_dry = mix3(g.grass_dry, [0.6, 0.52, 0.26], 0.6 * fall);
        g.grass_dry = mix3(g.grass_dry, [0.5, 0.44, 0.3], 0.7 * cold * cold);
        g.grass_lush = mix3(g.grass_lush, [0.42, 0.46, 0.26], 0.6 * cold * cold);
        g.moor = mix3(g.moor, [0.58, 0.45, 0.38], 0.4 * fall);
        // The terrain shader reads the seasonal snow cover from snow.w.
        g.snow[3] = self.season.snow(&self.biome);
        let f = &mut v.forest;
        // The second canopy colour (far.rgb, crown.w its share) stands for
        // the deciduous trees: autumn colour, then bare grey-brown twigs.
        f.crown[3] = self.biome.deciduous;
        let leaves = [f.far[0], f.far[1], f.far[2]];
        let autumn = [0.2, 0.09, 0.03];
        let bare = [0.06, 0.05, 0.04];
        let c = mix3([leaves[0], leaves[1], leaves[2], f.far[3]], autumn, fall);
        f.far = mix3(c, bare, (cold - 0.6).clamp(0.0, 0.4) / 0.4);
        v.meadow_dry_season = (v.meadow_dry_season + 0.6 * fall + 0.8 * cold * cold - 0.4 * spring).clamp(0.0, 1.0);
        v
    }

    /// The sun's elevation and azimuth (degrees, azimuth from north towards
    /// east, as atmosphere::SolarState::from_degrees) at this hour, season
    /// and latitude. Below the horizon at night.
    pub fn sun_degrees(&self) -> (f32, f32) {
        let lat = self.latitude_deg.to_radians();
        // Declination: 0 at the equinoxes, +23.44 at midsummer.
        let decl = (23.44f32).to_radians() * ((self.season.0) * std::f32::consts::TAU).sin();
        let hour_angle = ((self.hour - 12.0) * 15.0).to_radians();
        let sin_el = lat.sin() * decl.sin() + lat.cos() * decl.cos() * hour_angle.cos();
        let el = sin_el.clamp(-1.0, 1.0).asin();
        let az = (-hour_angle.sin()).atan2(lat.cos() * decl.tan() - lat.sin() * hour_angle.cos());
        (el.to_degrees(), az.to_degrees().rem_euclid(360.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_seasons_and_the_sun() {
        for n in NAMES { assert!(Biome::named(n).is_some(), "{n}"); }
        assert!(Biome::named("mars").is_none());
        // Temperate summer is the engine's default look.
        let summer = Environment::new(Biome::named("temperate").unwrap());
        assert_eq!(summer.vegetation(), Biome::named("temperate").unwrap().vegetation, "summer is the biome's palette");
        // Winter lays snow in the cold, none in the desert.
        let winter = |b: &str| Environment { season: Season::WINTER, ..Environment::new(Biome::named(b).unwrap()) }.vegetation();
        assert!(winter("tundra").ground.snow[3] > 0.5 && winter("temperate").ground.snow[3] > 0.0);
        assert_eq!(winter("desert").ground.snow[3], 0.0);
        // Noon is the day's highest sun, due south; midnight is below the horizon.
        let at = |hour| Environment { hour, ..summer }.sun_degrees();
        let (noon, az) = at(12.0);
        assert!(noon > 55.0 && noon < 62.0 && (az - 180.0).abs() < 1.0, "{noon} {az}");
        assert!(at(0.0).0 < 0.0 && at(9.0).1 < 180.0 && at(15.0).1 > 180.0);
        assert!(Environment { season: Season::WINTER, hour: 12.0, ..summer }.sun_degrees().0 < 20.0);
    }
}
