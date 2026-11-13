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
    pub flora: Flora,
}

/// What grows: tree and shrub species (plant_species names) with their
/// shares, as the biome's placement rules (`Biome::rules`) pick them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flora {
    pub trees: [(&'static str, f32); 4],
    /// Tree height scale (stunted on the tundra).
    pub tree_scale: f32,
    /// Trees along ditches and banks instead of in stands (polder rows).
    pub tree_rows: bool,
    pub shrubs: [(&'static str, f32); 2],
    /// Maximum share of suitable cells (3 m) with a shrub.
    pub shrub_density: f32,
    /// Shrub preference from -1 (clearings only) through 0 (no preference)
    /// to 1 (under a closed canopy only).
    pub shrub_shade: f32,
}

const NONE: (&str, f32) = ("", 0.0);

impl Flora {
    const TEMPERATE: Flora = Flora { trees: [("spruce", 0.55), ("pine", 0.15), ("birch", 0.15), ("broadleaf", 0.15)], tree_scale: 1.0,
        tree_rows: false, shrubs: [("shrub", 0.6), ("juniper", 0.4)], shrub_density: 0.03, shrub_shade: -0.7 };
}

/// Every preset, for menus and tests.
pub const NAMES: [&str; 9] = ["swiss", "temperate", "alpine", "lowland", "polder", "tundra", "desert", "norway", "rainforest"];

impl Biome {
    /// An authored land from `NAMES`. "default" is temperate.
    pub fn named(name: &str) -> Option<Self> {
        let base = VegetationStyle::default();
        let terrain = Style::default();
        Some(match name {
            "temperate" | "default" => Biome { name: "temperate", terrain, vegetation: base, tree_density: 0.6, deciduous: 0.2,
                flora: Flora::TEMPERATE },
            // Swiss valleys (Joep, 9 November, a Grindelwald photo): fresh
            // green pasture on rolling low ground, dense dark spruce in
            // clear stands with loose trees between, rugged grey rock and
            // snow above, the peaks in the clouds (Biome::clouds). The grass
            // texture is brown (linear 0.16, 0.12, 0.05), so the tints lean
            // hard to green: lush pasture comes out near (0.07, 0.15, 0.03), dry patches
            // yellower, so the pasture is not one flat green.
            "swiss" => Biome { name: "swiss", terrain: Style::swiss(),
                vegetation: VegetationStyle {
                    ground: GroundStyle { grass_dry: [0.62, 1.12, 0.42, 0.3], grass_lush: [0.42, 1.25, 0.6, 0.0],
                        soil: [0.75, 0.72, 0.62, 0.35], gravel: [0.92, 0.96, 1.02, 0.35], rock: [0.96, 1.0, 1.06, 0.25], ..base.ground },
                    forest: ForestStyle { needles: [0.035, 0.075, 0.03, 0.4], far: [0.06, 0.09, 0.035, 22.0], crown: [2.6, 0.42, 1.9, 0.08], ..base.forest },
                    meadow_dry_season: 0.0 },
                tree_density: 1.0, deciduous: 0.08,
                flora: Flora { trees: [("spruce", 0.85), ("pine", 0.07), ("broadleaf", 0.08), NONE], shrubs: [("juniper", 0.5), ("shrub", 0.5)],
                    shrub_density: 0.02, ..Flora::TEMPERATE } },
            "alpine" => Biome { name: "alpine", terrain: Style::alpine(), vegetation: VegetationStyle::named("alpine")?,
                tree_density: 0.6, deciduous: 0.1,
                flora: Flora { trees: [("spruce", 0.75), ("pine", 0.25), NONE, NONE], shrubs: [("juniper", 1.0), NONE], ..Flora::TEMPERATE } },
            "lowland" => Biome { name: "lowland", terrain: Style::lowland(), vegetation: VegetationStyle::named("lowland")?,
                tree_density: 0.5, deciduous: 0.4,
                flora: Flora { trees: [("broadleaf", 0.45), ("birch", 0.2), ("spruce", 0.2), ("pine", 0.15)], shrub_density: 0.05, ..Flora::TEMPERATE } },
            // Dead-flat reclaimed land: water everywhere in channels, lush
            // grass, a few rows of trees.
            "polder" => Biome { name: "polder",
                terrain: Style { mountains: 0.0, peaks: 0.0, hills: 0.04, rivers: 1.0, streams: 0.9, rocks: 0.0, boulders: 0.0,
                    moor: 0.05, grass: 1.0, ditches: 1.0, ..terrain },
                vegetation: VegetationStyle {
                    ground: GroundStyle { grass_dry: [0.36, 0.6, 0.22, 0.3], grass_lush: [0.26, 0.7, 0.24, 0.0], ..base.ground },
                    forest: ForestStyle { far: [0.075, 0.11, 0.04, 14.0], crown: [3.2, 0.375, 1.6, 0.6], ..base.forest },
                    meadow_dry_season: 0.0 },
                tree_density: 0.15, deciduous: 0.8,
                flora: Flora { trees: [("willow", 0.55), ("broadleaf", 0.3), ("birch", 0.15), NONE], tree_rows: true,
                    shrubs: [("shrub", 1.0), NONE], shrub_density: 0.02, ..Flora::TEMPERATE } },
            // Cold open land: low rolling hills, moss and heather, ponds,
            // stunted spruce here and there, snow on any height.
            "tundra" => Biome { name: "tundra",
                terrain: Style { mountains: 0.15, peaks: 0.0, hills: 0.5, rivers: 0.6, streams: 0.7, rocks: 0.5, boulders: 1.4,
                    moor: 1.0, grass: 0.6, snow_line: 600.0, ponds: 1.0, ..terrain },
                vegetation: VegetationStyle {
                    ground: GroundStyle { grass_dry: [0.5, 0.5, 0.3, 0.5], grass_lush: [0.4, 0.55, 0.3, 0.0],
                        moor: [0.62, 0.6, 0.55, 0.7], ..base.ground },
                    forest: ForestStyle { needles: [0.04, 0.075, 0.035, 0.5], ..base.forest },
                    meadow_dry_season: 0.5 },
                tree_density: 0.12, deciduous: 0.0,
                flora: Flora { trees: [("spruce", 0.7), ("birch", 0.3), NONE, NONE], tree_scale: 0.55,
                    shrubs: [("dwarf_birch", 0.8), ("juniper", 0.2)], shrub_density: 0.35, ..Flora::TEMPERATE } },
            // A sand sea: dunes over low rolling ground, rocky outcrops, no
            // water, no grass.
            "desert" => Biome { name: "desert",
                terrain: Style { mountains: 0.15, peaks: 0.0, hills: 0.35, rivers: 0.0, streams: 0.0, rocks: 0.4, boulders: 0.6,
                    moor: 0.0, grass: 0.0, snow_line: 4000.0, dunes: 1.0, ..terrain },
                vegetation: VegetationStyle {
                    ground: GroundStyle { grass_dry: [1.25, 0.95, 0.55, 0.0], grass_lush: [1.15, 0.9, 0.55, 0.0],
                        sand: 1.0, soil: [1.92, 1.37, 0.88, 0.1], gravel: [0.80, 0.68, 0.55, 0.4], rock: [1.10, 0.78, 0.55, 0.6], ..base.ground },
                    meadow_dry_season: 1.0, ..base },
                tree_density: 0.0, deciduous: 0.0,
                flora: Flora { trees: [NONE; 4], shrubs: [("desert_shrub", 0.55), ("saltbush", 0.45)], shrub_density: 0.025, ..Flora::TEMPERATE } },
            // A snowy Norwegian upland stage: rounded fells, exposed dark
            // rock and winter snow on the wooded valley floor. Lying snow
            // is a ground cover; lowering the tree line would remove the woods.
            "norway" => Biome { name: "norway",
                terrain: Style { mountains: 0.60, peaks: 0.62, hills: 0.85, snow_line: 1050.0,
                    rivers: 0.65, streams: 0.5, ponds: 0.24, rocks: 0.75, boulders: 1.2,
                    moor: 0.65, grass: 0.06, ..terrain },
                vegetation: VegetationStyle {
                    ground: GroundStyle { snowpack: 0.88, snow: [0.83, 0.88, 0.95, 0.0], soil: [0.67, 0.63, 0.57, 0.38],
                        rock: [0.66, 0.70, 0.77, 0.22], gravel: [0.74, 0.79, 0.85, 0.26], ..base.ground },
                    forest: ForestStyle { needles: [0.032, 0.064, 0.049, 0.45], far: [0.095, 0.11, 0.088, 13.0],
                        crown: [2.7, 0.35, 1.6, 0.20], ..base.forest }, meadow_dry_season: 0.8 },
                tree_density: 0.53, deciduous: 0.20,
                flora: Flora { trees: [("upland_spruce", 0.67), ("pine", 0.13), ("mountain_birch", 0.20), NONE], tree_scale: 0.84,
                    shrubs: [("juniper", 0.6), ("dwarf_birch", 0.4)], shrub_density: 0.025, ..Flora::TEMPERATE } },
            // Central African lowland rainforest: river valleys, damp red
            // clay and broadleaf litter below a canopy of mahogany and kapok.
            "rainforest" | "congo" => Biome { name: "rainforest",
                terrain: Style { mountains: 0.12, peaks: 0.0, hills: 1.2, rivers: 1.0, streams: 0.9,
                    ponds: 0.45, rocks: 0.18, boulders: 0.35, moor: 0.0, grass: 0.25, snow_line: 5000.0, ..terrain },
                vegetation: VegetationStyle {
                    ground: GroundStyle { leaf_litter: 1.0, soil: [1.40, 0.58, 0.34, 0.42],
                        moor: [0.57, 0.43, 0.34, 0.0], grass_dry: [0.45, 0.65, 0.28, 0.0],
                        grass_lush: [0.30, 0.74, 0.31, 0.0], ..base.ground },
                    forest: ForestStyle { needles: [0.039, 0.115, 0.046, 0.40], floor: [0.035, 0.027, 0.014, 0.34],
                        far: [0.065, 0.15, 0.05, 13.0], crown: [3.3, 0.42, 0.75, 0.0] }, meadow_dry_season: 0.05 },
                tree_density: 0.6, deciduous: 0.0,
                flora: Flora { trees: [("african_mahogany", 0.78), ("kapok", 0.22), NONE, NONE], tree_scale: 0.92,
                    shrubs: [("forest_coffee", 0.62), ("raffia_palm", 0.38)], shrub_density: 0.035, shrub_shade: 0.6, tree_rows: false } },
            _ => return None,
        })
    }
}

impl Biome {
    /// The cloud layer this biome's sky holds (sky_volume::Clouds), when
    /// it is part of its look: in the swiss valleys broken cumulus round the
    /// summits.
    pub fn clouds(&self) -> Option<crate::sky_volume::Clouds> {
        match self.name {
            // Broken cumulus from the snow line up: the summits stand in
            // them and the valleys keep blue sky (a 0.55 deck at 1400 m
            // closed the sky over the valleys).
            "swiss" => Some(crate::sky_volume::Clouds { coverage: 0.35, base_m: self.terrain.snow_line, thickness_m: 1000.0,
                ..Default::default() }),
            _ => None,
        }
    }
}

/// The rule id of a biome's shrubs (placement::TREE are its trees).
pub const SHRUB: u32 = 16;

impl Flora {
    /// Share of the biome's shrub density retained at this forest cover (0..1).
    pub fn shrub_habitat(&self, forest: f32) -> f32 {
        let canopy = smooth(0.3, 0.8, forest);
        let preference = self.shrub_shade.clamp(-1.0, 1.0);
        if preference > 0.0 { 1.0 - preference * (1.0 - canopy) }
        else { 1.0 + preference * canopy }
    }

    /// The tree or shrub selected by a roll in [0, 1), indexing the
    /// tree-then-shrub catalogue returned by `Biome::species`.
    pub fn tree_variant(&self, roll: f32) -> Option<usize> {
        Self::pick(&self.trees, roll)
    }

    pub fn shrub_variant(&self, roll: f32) -> Option<usize> {
        let first = self.trees.iter().filter(|e| e.1 > 0.0).count();
        Self::pick(&self.shrubs, roll).map(|variant| first + variant)
    }

    fn pick(list: &[(&str, f32)], roll: f32) -> Option<usize> {
        let total: f32 = list.iter().map(|e| e.1).sum();
        let mut acc = 0.0;
        let mut last = None;
        for (i, &(_, weight)) in list.iter().filter(|e| e.1 > 0.0).enumerate() {
            acc += weight / total;
            if roll < acc { return Some(i); }
            last = Some(i);
        }
        last
    }
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Biome {
    /// The species that grow here (plant_species recipes): its trees, then
    /// its shrubs. A placed object's `variant` indexes this list.
    pub fn species(&self) -> Vec<(&'static str, crate::plant_recipe::WoodyRecipe)> {
        self.flora.trees.iter().chain(&self.flora.shrubs).filter(|(n, w)| *w > 0.0 && !n.is_empty())
            .map(|(n, _)| (*n, crate::plant_species::named(n).expect("biome species"))).collect()
    }

    /// The biome's placement rules: its trees (in the forest's stands, or in
    /// rows along wet banks), then shrubs on open ground. Instances carry
    /// the species index (`species`) as their variant.
    pub fn rules(&self) -> Vec<crate::placement::Rule> {
        use crate::placement::{Rule, Shape, TREE};
        let flora = self.flora;
        let mut rules = Vec::new();
        if self.tree_density > 0.0 && flora.trees.iter().any(|e| e.1 > 0.0) {
            let mut tree = crate::placement::tree_rule(self.tree_density);
            let density = self.tree_density;
            if flora.tree_rows {
                // Rows along ditches and banks: damp ground, not water.
                tree.cap = 0.7;
                tree.density = Box::new(move |s| {
                    let g = &s.ground;
                    if g.water > 0.01 || s.field("grows") == 0.0 { return 0.0; }
                    (0.7 * smooth(0.25, 0.45, g.wet) * (1.0 - smooth(0.85, 0.98, g.wet)) + s.field("forest") * density * 0.3)
                        * crate::placement::off_road(s, 0.5)
                });
            }
            let scale = flora.tree_scale;
            tree.shape = Box::new(move |s| {
                let width = (0.75 + 0.5 * s.cell.hash(4)) * scale;
                Shape { yaw: s.cell.hash(6) * std::f32::consts::TAU, scale: glam::Vec3::new(width, width * (0.9 + 0.25 * s.cell.hash(5)), width),
                    variant: flora.tree_variant(s.cell.hash(7)).unwrap() as u32, lift: -0.2 }
            });
            rules.push(tree);
        }
        if flora.shrub_density > 0.0 && flora.shrubs.iter().any(|e| e.1 > 0.0) {
            let d = flora.shrub_density;
            rules.push(Rule { id: SHRUB, name: "shrub", cell: 3.0, salt: 0x5348_5255, cap: d,
                density: Box::new(move |s| {
                    let g = &s.ground;
                    if g.water > 0.01 || g.snow > 0.3 || g.up < 0.8 { return 0.0; }
                    d * (1.0 - smooth(0.2, 0.6, g.rock)) * flora.shrub_habitat(s.field("forest")) * crate::placement::off_road(s, 0.5)
                }),
                shape: Box::new(move |s| {
                    let size = 0.6 + 0.8 * s.cell.hash(4);
                    Shape { yaw: s.cell.hash(6) * std::f32::consts::TAU, scale: glam::Vec3::new(size, size * (0.8 + 0.4 * s.cell.hash(5)), size),
                        variant: flora.shrub_variant(s.cell.hash(7)).unwrap() as u32, lift: -0.05 }
                }),
                avoid: vec![(TREE, 1.5)] });
        }
        rules
    }
}

/// Several biomes as the kinds of one world's regions (crate::regions):
/// biome k is region kind k, its terrain the regions' style k. Their
/// species share one list and their rules place each biome's flora where
/// its region holds, thinning out across a border as the other's thickens.
#[derive(Clone, Debug, PartialEq)]
pub struct Lands {
    pub biomes: Vec<Biome>,
}

impl Lands {
    /// The terrain styles, in kind order, for `regions::Regions`.
    pub fn styles(&self) -> Vec<Style> { self.biomes.iter().map(|b| b.terrain).collect() }

    /// Every species any biome grows, each once (by name), in first-seen
    /// order. Placed objects' variants index this list.
    pub fn species(&self) -> Vec<(&'static str, crate::plant_recipe::WoodyRecipe)> {
        let mut out: Vec<(&'static str, crate::plant_recipe::WoodyRecipe)> = Vec::new();
        for b in &self.biomes {
            for (name, recipe) in b.species() {
                if !out.iter().any(|(n, _)| *n == name) { out.push((name, recipe)); }
            }
        }
        out
    }

    /// Every biome's rules, scaled by its region's share (placement::regional).
    /// One biome alone gives exactly `Biome::rules`.
    pub fn rules(&self) -> Vec<crate::placement::Rule> {
        let all = self.species();
        let map: Vec<Vec<u32>> = self.biomes.iter().map(|b| b.species().iter()
            .map(|(n, _)| all.iter().position(|(m, _)| m == n).expect("species listed") as u32).collect()).collect();
        let map = std::sync::Arc::new(map);
        crate::placement::regional(self.biomes.iter().map(|b| b.rules()).collect(),
            move |kind, v| map[kind as usize].get(v as usize).copied().unwrap_or(v))
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
        // Equatorial forest stays green through the northern autumn and winter.
        if self.biome.name == "rainforest" { return v; }
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

    /// The biome's own terrain and forest, for its rules.
    struct Land(Biome);
    impl crate::placement::World for Land {
        fn seed(&self) -> u32 { 1 }
        fn ground(&self, x: f32, z: f32) -> crate::terrain_field::Sample {
            crate::terrain_field::eval(x, z, crate::forest_field::CUT, 1, &self.0.terrain)
        }
        fn field(&self, name: &str, x: f32, z: f32, g: &crate::terrain_field::Sample) -> f32 {
            let grows = crate::forest_field::grows(g, &self.0.terrain);
            match name {
                "forest" if grows => crate::forest_field::share(x, z, 1, crate::forest_field::site(g.up, g.wet, self.0.terrain.forest_slopes), 0.0),
                "grows" => f32::from(u8::from(grows)),
                _ => 0.0,
            }
        }
    }

    #[test]
    fn rainforest_shrubs_favor_shelter_and_stay_off_flooded_ground() {
        use crate::placement::{place, World};
        use crate::terrain_field::Sample;
        // Identical seeded cells and terrain for both trials. Flood the northern
        // half to check that even a fully sheltered site cannot grow underwater.
        struct Plot { forest: f32 }
        impl World for Plot {
            fn seed(&self) -> u32 { 37 }
            fn field(&self, name: &str, _: f32, _: f32, _: &Sample) -> f32 {
                if name == "forest" { self.forest } else { 0.0 }
            }
            fn ground(&self, x: f32, z: f32) -> Sample {
                Sample { pos: glam::Vec3::new(x, 0.0, z), up: 1.0, rock: 0.0, scree: 0.0, wet: 0.0,
                    water: if z > 0.0 { 1.0 } else { 0.0 }, film: 0.0, moor: 0.0, snow: 0.0, amp: 0.0, tors: 0.0 }
            }
        }
        let rainforest = Biome::named("rainforest").unwrap();
        let shrubs: Vec<_> = rainforest.rules().into_iter().filter(|r| r.id == SHRUB).collect();
        let grow = |forest| place(&shrubs, &Plot { forest }, glam::Vec2::ZERO, 180.0);
        let open = grow(0.0);
        let sheltered = grow(1.0);
        assert!(!open.is_empty() && sheltered.len() > open.len() * 2);
        assert_eq!(sheltered, grow(1.0));
        assert!(sheltered.iter().all(|plant| plant.pos.z <= 0.0));
        let temperate = Biome::named("temperate").unwrap().flora;
        for forest in [0.0, 0.35, 0.5, 0.7, 1.0] {
            assert_eq!(temperate.shrub_habitat(forest), 1.0 - 0.7 * smooth(0.3, 0.8, forest));
        }
    }

    #[test]
    fn each_biome_grows_its_own_flora() {
        for name in NAMES {
            let biome = Biome::named(name).unwrap();
            let species = biome.species();
            let placed = crate::placement::place(&biome.rules(), &Land(biome), glam::Vec2::ZERO, 300.0);
            let count = |rule| placed.iter().filter(|i| i.rule == rule).count();
            assert!(placed.iter().all(|i| (i.variant as usize) < species.len()), "{name}: variant out of range");
            eprintln!("FLORA {name}: {} trees, {} shrubs, species {:?}", count(crate::placement::TREE), count(SHRUB),
                species.iter().map(|s| s.0).collect::<Vec<_>>());
            match name {
                "desert" => assert!(count(crate::placement::TREE) == 0 && count(SHRUB) > 20),
                "tundra" => assert!(count(SHRUB) > 1000),
                "polder" => assert!(count(crate::placement::TREE) > 20),
                _ => assert!(count(crate::placement::TREE) > 100),
            }
        }
    }

    /// One biome as Lands places exactly what the biome places; two biomes
    /// over regions each grow their own flora on their own side, with
    /// distinct ids and variants from the shared species list.
    #[test]
    fn each_region_grows_its_own_biome() {
        use crate::placement::{self, TerrainWorld};
        use crate::world_constraints::Constraints;
        let temperate = Biome::named("temperate").unwrap();
        let one = Lands { biomes: vec![temperate] };
        let land = Land(temperate);
        assert_eq!(placement::place(&one.rules(), &land, glam::Vec2::ZERO, 200.0), placement::place(&temperate.rules(), &land, glam::Vec2::ZERO, 200.0));
        let tundra = Biome::named("tundra").unwrap();
        let lands = Lands { biomes: vec![temperate, tundra] };
        let regions = crate::regions::Regions::split([temperate.terrain, tundra.terrain], glam::Vec2::ZERO, 6000.0, glam::Vec2::X, 400.0, 0.0, 1);
        let world = Constraints { regions: Some(regions), ..Default::default() }.bake(1, &temperate.terrain);
        let tw = TerrainWorld { seed: 1, style: &temperate.terrain, constraints: Some(&world) };
        let species = lands.species();
        let rules = lands.rules();
        let west = placement::place(&rules, &tw, glam::Vec2::new(-1500.0, 0.0), 250.0);
        let east = placement::place(&rules, &tw, glam::Vec2::new(1500.0, 0.0), 250.0);
        assert!(west.iter().all(|i| placement::rule_kind(i.rule) == 0) && east.iter().all(|i| placement::rule_kind(i.rule) == 1));
        assert!(!west.is_empty() && !east.is_empty());
        let named = |i: &placement::Instance| species[i.variant as usize].0;
        assert!(east.iter().any(|i| named(i) == "dwarf_birch"), "tundra shrubs east");
        assert!(west.iter().all(|i| named(i) != "dwarf_birch"), "no tundra shrubs west");
        let mut ids: Vec<u64> = west.iter().chain(&east).map(|i| i.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), west.len() + east.len());
    }

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
