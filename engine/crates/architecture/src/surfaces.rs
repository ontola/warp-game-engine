//! Surfaces of a home, drawn in code: tileable albedo and normal maps for
//! plaster, brick, oak planks, floor tiles, stone, terrazzo and a flat roof,
//! at real sizes. No image files: every texture is a function of its pixel,
//! so the look is tuned here and nothing has to be downloaded or licensed.
//!
//! A [`Surface`] is the material family with the fixed look IFC files get.
//! A [`Style`] is a surface laid out as a design asks (AtomicBuildings'
//! `pattern`, `moduleSize` and `jointWidth`): boards of a given width and
//! length, bricks in running or stack bond, random or coursed stone. Its
//! texture is drawn neutral at that module size and the material tints it
//! to the design's colour.
use image::{Rgba, RgbaImage};
use render::scene::{Material, Scene, TRIPLANAR, TWO_SIDED};
use std::collections::HashMap;

/// What a surface is made of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Surface {
    /// Painted plaster, inside walls and ceilings.
    Plaster,
    /// Facing brick, outside walls.
    Brick,
    /// Oak planks, living rooms and bedrooms.
    Oak,
    /// Large stone-look tiles, halls, kitchens and wet rooms.
    Tile,
    /// Gravel on a flat roof.
    Roof,
    /// Painted window and door frames.
    Frame,
    /// Upholstery and cabinets.
    Furniture,
    /// Bare or screeded concrete, concrete block.
    Concrete,
    /// Aluminium and steel: curtain wall mullions, frames, railings.
    Metal,
    /// A planted (green) roof.
    Grass,
    /// Textile and upholstery fabric.
    Fabric,
    /// Natural stone: random fieldstone or coursed ashlar. Only drawn from a
    /// design's pattern (IFC names do not pick it).
    Stone,
    /// Terrazzo: chips in a cement matrix.
    Terrazzo,
}

/// Texture size in pixels (square, tileable).
const SIZE: u32 = 512;

impl Surface {
    /// Metres covered by one repeat of the texture.
    pub fn tile_metres(self) -> f32 {
        match self { Surface::Plaster => 2.0, Surface::Brick => 1.0, Surface::Oak => 2.4, Surface::Tile => 1.2, Surface::Roof => 1.0, Surface::Concrete => 2.0, Surface::Grass => 1.0,
            Surface::Frame | Surface::Furniture | Surface::Metal | Surface::Fabric => 1.0,
            Surface::Stone | Surface::Terrazzo => Style::of(self).tile()[0] }
    }

    /// Add this surface's texture (if any) and material to `scene`, mapped
    /// by the mesh's uv.
    pub fn add(self, scene: &mut Scene) -> u32 { self.add_mapped(scene, false, false) }

    /// Does it have a texture (Frame and Furniture are plain)?
    pub fn textured(self) -> bool { !matches!(self, Surface::Frame | Surface::Furniture | Surface::Metal | Surface::Fabric) }

    /// The surface an IFC material (or surface style) name describes, in
    /// English, Dutch or German, as BIM tools write them ("Brick, Common",
    /// "Masonry - Brick", "Leichtbeton", "Holz", "Aluminium", "Wood -
    /// Flooring"). `None` when the name says nothing about the finish
    /// (insulation, air, studs, "Solid", "Default").
    pub fn from_material_name(name: &str) -> Option<Surface> {
        let n = name.to_lowercase();
        let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
        // Layers inside a build-up are never seen.
        if has(&["stud", "insul", "isolat", "dämm", "air space", "air layer", "airspace", "luchtspouw", "spouw", "vapor", "vapour", "damp", "barrier", "sheathing", "lumber", "batt"]) || n.trim() == "air" { return None; }
        Some(if has(&["glass", "glas", "glazing"]) { return None }
            else if has(&["brick", "baksteen", "ziegel", "klinker", "metselwerk"]) { Surface::Brick }
            else if has(&["plaster", "gypsum", "drywall", "stuc", "putz", "gips", "gipskarton"]) { Surface::Plaster }
            else if has(&["tile", "tegel", "fliese", "ceramic", "keramik", "porcelain"]) { Surface::Tile }
            else if has(&["grass", "gras", "sedum", "vegetat"]) { Surface::Grass }
            else if has(&["alumin", "steel", "stahl", "staal", "metal", "chrome", "zink", "zinc"]) { Surface::Metal }
            else if has(&["fabric", "textile", "stof", "stoff", "leather", "leder"]) { Surface::Fabric }
            else if has(&["wood", "holz", "hout", "timber", "oak", "eiken", "eiche", "birch", "cherry", "mahogany", "lumber", "plywood", "laminate", "parquet", "parket"]) { Surface::Oak }
            else if has(&["concrete", "beton", "screed", "cement", "masonry", "block", "kalkzandsteen", "porenbeton"]) { Surface::Concrete }
            else if has(&["epdm", "bitumen", "roofing", "membrane", "dakbedekking", "gravel", "grind", "kies"]) { Surface::Roof }
            else { return None })
    }

    /// As `add`; `triplanar` maps the texture by world position instead of
    /// uv, for curved faces (a round window's reveal) where per-face uvs
    /// would break the pattern at every facet. `two_sided` as the source
    /// material says: a window frame from IFC is a closed solid whose outer
    /// face lies on the wall's reveal, and drawing its back fights the reveal.
    pub fn add_mapped(self, scene: &mut Scene, triplanar: bool, two_sided: bool) -> u32 {
        let (tint, roughness, layer) = match self {
            Surface::Frame => ([0.08, 0.085, 0.09], 0.45, -1),
            Surface::Metal => ([0.78, 0.79, 0.8], 0.35, -1),
            Surface::Fabric => ([0.5, 0.47, 0.42], 0.95, -1),
            Surface::Furniture => ([0.55, 0.52, 0.48], 0.85, -1),
            _ => {
                // One texture per surface per scene, however many houses.
                let name = format!("architecture/{self:?}");
                let layer = match scene.textures.iter().position(|t| t.name == name) {
                    Some(l) => l as i32,
                    None => {
                        let (albedo, normal, _) = Style::of(self).draw();
                        let layer = scene.add_image(&name, albedo);
                        scene.textures[layer as usize].normal = Some(normal);
                        layer
                    }
                };
                let rough = self.default_roughness();
                ([1.0; 3], rough, layer)
            }
        };
        let flags = if two_sided { TWO_SIDED } else { 0 } | if triplanar && layer >= 0 { TRIPLANAR } else { 0 };
        let scale = if flags & TRIPLANAR != 0 { self.tile_metres() } else { 1.0 };
        let metal = if self == Surface::Metal { 1.0 } else { 0.0 };
        scene.add_material(Material { tint, layer, roughness, flags, scale, metal })
    }

    fn default_roughness(self) -> f32 {
        match self { Surface::Oak => 0.45, Surface::Tile => 0.3, Surface::Plaster => 0.9, Surface::Grass => 0.95, Surface::Terrazzo => 0.35, _ => 0.85 }
    }

    /// Linear colour and height in metres at (u, v) in 0..1 of one repeat.
    fn texel(self, u: f32, v: f32) -> ([f32; 3], f32) {
        let m = self.tile_metres();
        let (px, py) = (u * m, v * m);
        match self {
            Surface::Plaster => {
                let a = fbm(u, v, 8, 4, 1) * 0.04 + fbm(u, v, 64, 2, 2) * 0.015;
                let c = 0.80 + a;
                ([c, c * 0.985, c * 0.96], a * 0.0006)
            }
            Surface::Brick => {
                // Running bond: 0.25 m x 0.0625 m modules (Dutch waalformaat
                // with 10 mm joints), half-brick offset every course.
                let course = (py / 0.0625).floor();
                let off = if course as i32 % 2 == 0 { 0.0 } else { 0.125 };
                let col = ((px + off) / 0.25).floor();
                let (bx, by) = (((px + off) / 0.25).fract() * 0.25, (py / 0.0625).fract() * 0.0625);
                let joint = bx < 0.01 || by < 0.0125;
                let id = hash(col as i32 & 3, course as i32 & 15, 7);
                if joint {
                    let g = 0.42 + fbm(u, v, 64, 2, 9) * 0.05;
                    ([g, g * 0.97, g * 0.92], -0.008)
                } else {
                    let t = 0.75 + id * 0.35 + fbm(u, v, 32, 3, 11) * 0.12;
                    let burnt = hash(col as i32 & 3, course as i32 & 15, 13) < 0.15;
                    let base = if burnt { [0.16, 0.08, 0.05] } else { [0.36, 0.13, 0.07] };
                    ([base[0] * t, base[1] * t, base[2] * t], fbm(u, v, 48, 2, 5) * 0.0015)
                }
            }
            Surface::Oak => {
                // 0.2 m planks along u, lengths of 1.2 m staggered per row.
                let row = (py / 0.2).floor();
                let off = hash(row as i32 & 15, 0, 3) * 1.2;
                let seg = ((px + off) / 1.2).floor();
                let (lx, ly) = (((px + off) / 1.2).fract() * 1.2, (py / 0.2).fract() * 0.2);
                let groove = lx < 0.002 || ly < 0.0015;
                let id = hash(seg as i32 & 1, row as i32 & 15, 21);
                // Grain: noise stretched along the plank.
                let grain = fbm(u * 0.15 + id, v * 4.0, 16, 4, 31);
                let ring = ((grain * 9.0 + id * 6.0).sin() * 0.5 + 0.5).powf(3.0);
                let t = 0.8 + id * 0.35 - ring * 0.18;
                let c = [0.42 * t, 0.26 * t, 0.13 * t];
                if groove { ([c[0] * 0.4, c[1] * 0.4, c[2] * 0.4], -0.0015) } else { (c, ring * 0.0002) }
            }
            Surface::Tile => {
                // 0.6 m tiles, 3 mm grout.
                let (tx, ty) = ((px / 0.6).fract() * 0.6, (py / 0.6).fract() * 0.6);
                let grout = tx < 0.003 || ty < 0.003;
                let id = hash((px / 0.6) as i32, (py / 0.6) as i32, 41);
                if grout { ([0.32, 0.31, 0.29], -0.002) } else {
                    let g = 0.52 + id * 0.04 + fbm(u, v, 12, 4, 43) * 0.06;
                    ([g, g * 0.98, g * 0.95], 0.0)
                }
            }
            Surface::Roof => {
                let g = 0.12 + fbm(u, v, 128, 2, 51) * 0.08;
                ([g, g, g * 0.95], fbm(u, v, 128, 2, 51) * 0.004)
            }
            Surface::Concrete => {
                // Board-smooth concrete: soft mottling and a few pores.
                let a = fbm(u, v, 6, 4, 61) * 0.06 + fbm(u, v, 48, 2, 63) * 0.02;
                let pore = fbm(u, v, 160, 1, 67) < -0.42;
                let c = 0.46 + a;
                if pore { ([c * 0.7; 3], -0.001) } else { ([c, c * 0.99, c * 0.96], a * 0.0008) }
            }
            Surface::Grass => {
                let a = fbm(u, v, 96, 3, 71);
                let c = 0.09 + a * 0.05;
                ([c * 0.9, c * 1.6 + 0.02, c * 0.55], a * 0.01)
            }
            Surface::Frame | Surface::Furniture | Surface::Metal | Surface::Fabric | Surface::Stone | Surface::Terrazzo => ([0.5; 3], 0.0),
        }
    }
}

/// How a surface's pieces lie: AtomicBuildings' `pattern`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Layout {
    /// The surface's fixed look (what IFC files get).
    Standard,
    /// Boards with staggered ends (`planks`).
    Boards,
    /// Long thin battens with open joints (`slats`).
    Slats,
    /// Grain with no joints: veneer, a wood with no pattern given.
    Veneer,
    /// Planks in herringbone (`herringbone`).
    Herringbone,
    /// Bricks, every other course half a brick over (`running-bond`).
    RunningBond,
    /// Bricks in straight columns (`stack-bond`).
    StackBond,
    /// Irregular stones of about the module size (`random-stone`).
    Fieldstone,
    /// Coursed blocks of varying length (`ashlar`).
    Ashlar,
    /// A square or rectangular grid of tiles (`tiles`).
    Grid,
    /// Chips in cement (`terrazzo`).
    Terrazzo,
    /// Concrete cast against boards (`board-formed`).
    BoardFormed,
}

/// A surface laid out as a design asks: which pieces, how big, how wide the
/// joints. Drawn neutral (see [`Style::add`]) and tinted per material.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub surface: Surface,
    pub layout: Layout,
    /// One piece as seen on the surface, metres: along u, along v (a wall's
    /// width and height; a floor's x and z).
    pub module: [f32; 2],
    /// Joint, groove or gap between pieces, metres.
    pub joint: f32,
}

/// A procedural texel function: colour (linear) and height (metres) at
/// (u, v) in 0..1 of one repeat.
type Painter = Box<dyn Fn(f32, f32) -> ([f32; 3], f32)>;

/// Pixels of a styled texture (square, tileable). The renderer resizes
/// every layer to one square size, so a tile that is not square is stretched
/// here and drawn in metres.
const STYLED_SIZE: u32 = 1024;

impl Style {
    /// The fixed look of a surface.
    pub fn of(surface: Surface) -> Style {
        match surface {
            Surface::Stone => Style::laid(Layout::Fieldstone, None, None),
            Surface::Terrazzo => Style::laid(Layout::Terrazzo, None, None),
            s => Style { surface: s, layout: Layout::Standard, module: [s.tile_metres(); 2], joint: 0.0 },
        }
    }

    /// A pattern at `module` and `joint`, each defaulting to what that
    /// pattern usually is.
    pub fn laid(layout: Layout, module: Option<[f32; 2]>, joint: Option<f32>) -> Style {
        let (surface, m, j) = match layout {
            Layout::Standard | Layout::Boards => (Surface::Oak, [0.14, 1.8], 0.002),
            Layout::Slats => (Surface::Oak, [0.045, 2.4], 0.008),
            Layout::Veneer => (Surface::Oak, [0.5, 2.0], 0.0),
            Layout::Herringbone => (Surface::Oak, [0.09, 0.6], 0.001),
            Layout::RunningBond | Layout::StackBond => (Surface::Brick, [0.215, 0.065], 0.01),
            Layout::Fieldstone => (Surface::Stone, [0.4, 0.2], 0.015),
            Layout::Ashlar => (Surface::Stone, [0.5, 0.25], 0.006),
            Layout::Grid => (Surface::Tile, [0.6, 0.6], 0.003),
            Layout::Terrazzo => (Surface::Terrazzo, [1.2, 1.2], 0.0),
            Layout::BoardFormed => (Surface::Concrete, [1.2, 0.15], 0.003),
        };
        let module = module.filter(|m| m.iter().all(|v| v.is_finite() && (0.01..=8.0).contains(v))).unwrap_or(m);
        let joint = joint.filter(|j| j.is_finite() && (0.0..=0.05).contains(j)).unwrap_or(j);
        Style { surface, layout, module, joint }
    }

    /// The same surface as one of the fixed looks, tinted (Plaster,
    /// Concrete, Grass).
    pub fn standard(surface: Surface) -> Style { Style { surface, layout: Layout::Standard, module: [surface.tile_metres(); 2], joint: 0.0 } }

    /// Texture name in a scene: one texture per look, however many colours.
    pub fn key(&self) -> String {
        if self.layout == Layout::Standard { return format!("architecture/{:?}", self.surface); }
        format!("architecture/{:?}/{:?}/{:.4}x{:.4}/j{:.4}", self.surface, self.layout, self.module[0], self.module[1], self.joint)
    }

    /// Board across and along dimensions, and whether the boards run along u.
    fn boards(&self) -> (f32, f32, bool) {
        let [w, h] = self.module;
        (w.min(h), w.max(h), w > h)
    }

    /// Boards across and boards' lengths along one repeat.
    fn board_counts(&self) -> (i32, i32) {
        let (a, l, _) = self.boards();
        (((0.6 / a - 1e-3).ceil() as i32).clamp(1, 48), ((1.2 / l - 1e-3).ceil() as i32).clamp(1, 6))
    }

    /// Pieces across and down one repeat for the jointed layouts: even, so
    /// the half-brick offset closes.
    fn counts(&self) -> (i32, i32) {
        let (px, py) = (self.module[0] + self.joint, self.module[1] + self.joint);
        let even = |n: f32| (((n.ceil() as i32) + 1) & !1).clamp(2, 64);
        match self.layout {
            Layout::Fieldstone => (((2.0 / self.module[0]).ceil() as i32).clamp(3, 64), ((2.0 / self.module[1]).ceil() as i32).clamp(3, 64)),
            Layout::Ashlar => (even(2.0 / px), even(2.0 / py)),
            _ => (even(1.0 / px), even(1.0 / py)),
        }
    }

    /// Metres covered by one repeat, along u and along v.
    pub fn tile(&self) -> [f32; 2] {
        let [mw, mh] = self.module;
        match self.layout {
            Layout::Standard => [self.surface.tile_metres_plain(); 2],
            Layout::Boards | Layout::Slats | Layout::Veneer | Layout::BoardFormed => {
                let (a, l, horizontal) = self.boards();
                let (nc, ns) = self.board_counts();
                if horizontal { [l * ns as f32, a * nc as f32] } else { [a * nc as f32, l * ns as f32] }
            }
            Layout::Herringbone => { let (_, l, _) = self.boards(); [2.0 * l; 2] }
            Layout::RunningBond | Layout::StackBond | Layout::Ashlar | Layout::Grid => {
                let (nc, nr) = self.counts();
                [nc as f32 * (mw + self.joint), nr as f32 * (mh + self.joint)]
            }
            Layout::Fieldstone => { let (nc, nr) = self.counts(); [nc as f32 * mw, nr as f32 * mh] }
            Layout::Terrazzo => [mw, mh],
        }
    }

    /// Texture, normal map, and the mean colour of the pieces (joints left
    /// out: a design's colour is the brick's, not the wall's).
    fn draw(&self) -> (RgbaImage, RgbaImage, [f32; 3]) {
        let size = if self.layout == Layout::Standard { SIZE } else { STYLED_SIZE };
        let n = size as f32;
        let paint = self.painter();
        let mut height = vec![0f32; (size * size) as usize];
        let mut albedo = RgbaImage::new(size, size);
        let (mut sum, mut count) = ([0f64; 3], 0u64);
        for y in 0..size {
            for x in 0..size {
                let (c, h) = paint(x as f32 / n, y as f32 / n);
                height[(y * size + x) as usize] = h;
                albedo.put_pixel(x, y, Rgba([to_srgb(c[0]), to_srgb(c[1]), to_srgb(c[2]), 255]));
                if h > -0.0005 { for k in 0..3 { sum[k] += c[k] as f64; } count += 1; }
            }
        }
        let mean = if count == 0 { [0.5; 3] } else { sum.map(|s| (s / count as f64) as f32) };
        // Normal from height differences (wrapping), in metres per metre.
        let [tw, th] = self.tile();
        let (tx, ty) = (tw / n, th / n);
        let at = |x: i32, y: i32| height[((y.rem_euclid(size as i32)) as u32 * size + x.rem_euclid(size as i32) as u32) as usize];
        let normal = RgbaImage::from_fn(size, size, |x, y| {
            let (x, y) = (x as i32, y as i32);
            let dx = (at(x + 1, y) - at(x - 1, y)) / (2.0 * tx);
            let dy = (at(x, y + 1) - at(x, y - 1)) / (2.0 * ty);
            // OpenGL convention: green points to the image's top row.
            let v = glam::Vec3::new(-dx, dy, 1.0).normalize();
            Rgba([((v.x * 0.5 + 0.5) * 255.0) as u8, ((v.y * 0.5 + 0.5) * 255.0) as u8, ((v.z * 0.5 + 0.5) * 255.0) as u8, 255])
        });
        (albedo, normal, mean)
    }

    fn painter(&self) -> Painter {
        let st = *self;
        match st.layout {
            Layout::Standard => { let s = st.surface; Box::new(move |u, v| s.texel(u, v)) }
            Layout::Boards | Layout::Slats | Layout::Veneer | Layout::BoardFormed => boards(st),
            Layout::Herringbone => herringbone(st),
            Layout::RunningBond | Layout::StackBond => bond(st),
            Layout::Fieldstone => fieldstone(st),
            Layout::Ashlar => ashlar(st),
            Layout::Grid => grid(st),
            Layout::Terrazzo => terrazzo(st),
        }
    }

    /// A material of this style: its texture (one per look in a scene,
    /// kept in `cache` with its mean colour) tinted so the pieces average
    /// `color` (linear). `roughness` and `metal` as the design says.
    pub fn add(self, scene: &mut Scene, cache: &mut HashMap<String, (i32, [f32; 3])>, color: [f32; 3], roughness: f32, metal: f32, two_sided: bool) -> u32 {
        let key = self.key();
        let &mut (layer, mean) = cache.entry(key.clone()).or_insert_with(|| {
            let (albedo, normal, mean) = self.draw();
            let layer = match scene.textures.iter().position(|t| t.name == key) {
                Some(l) => l as i32,
                None => {
                    let layer = scene.add_image(&key, albedo);
                    scene.textures[layer as usize].normal = Some(normal);
                    layer
                }
            };
            (layer, mean)
        });
        let tint = std::array::from_fn(|k| (color[k] / mean[k].max(0.01)).clamp(0.0, 6.0));
        let flags = if two_sided { TWO_SIDED } else { 0 };
        scene.add_material(Material { tint, layer, roughness, flags, scale: 1.0, metal })
    }

    /// The style a design's material asks for, from its glTF `extras`
    /// (`pattern`, `category`, `moduleSize`, `jointWidth`) and name. `None`
    /// for what is drawn flat: its glTF colour alone (textile, metal, glass,
    /// plastic, water...).
    pub fn from_extras(extras: &serde_json::Value, name: &str) -> Option<Style> {
        let module = extras["moduleSize"].as_array().filter(|a| a.len() == 2)
            .and_then(|a| Some([a[0].as_f64()? as f32, a[1].as_f64()? as f32]));
        let joint = extras["jointWidth"].as_f64().map(|j| j as f32);
        let category = extras["category"].as_str().map(str::to_lowercase).or_else(|| category_from_name(name));
        let laid = |l| Some(Style::laid(l, module, joint));
        match extras["pattern"].as_str().unwrap_or("none") {
            "planks" => return laid(Layout::Boards),
            "slats" => return laid(Layout::Slats),
            "herringbone" => return laid(Layout::Herringbone),
            "running-bond" => return laid(Layout::RunningBond),
            "stack-bond" => return laid(Layout::StackBond),
            "random-stone" => return laid(Layout::Fieldstone),
            "ashlar" => return laid(Layout::Ashlar),
            "tiles" => return laid(Layout::Grid),
            "terrazzo" => return laid(Layout::Terrazzo),
            "board-formed" => return laid(Layout::BoardFormed),
            _ => {}
        }
        match category.as_deref()? {
            "wood" => laid(Layout::Veneer),
            "stone" => laid(Layout::Fieldstone),
            "brick" => laid(Layout::RunningBond),
            "concrete" => Some(Style::standard(Surface::Concrete)),
            "plaster" => Some(Style::standard(Surface::Plaster)),
            "plant" => Some(Style::standard(Surface::Grass)),
            _ => None,
        }
    }
}

/// A material category from its name, for designs that give no `category`.
fn category_from_name(name: &str) -> Option<String> {
    let n = name.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
    Some(if has(&["glass", "glazing", "water"]) { "glass" }
        else if has(&["brick", "klinker"]) { "brick" }
        else if has(&["plaster", "stucco", "render"]) { "plaster" }
        else if has(&["stone", "granite", "slate", "sandstone", "limestone", "marble"]) { "stone" }
        else if has(&["concrete", "screed", "cement"]) { "concrete" }
        else if has(&["wood", "timber", "oak", "walnut", "cedar", "pine", "larch", "teak", "ash ", "beech", "maple", "birch", "bamboo", "plywood", "mahogany"]) { "wood" }
        else if has(&["lawn", "grass", "turf", "sedum"]) { "plant" }
        else { return None }.to_string())
}

impl Surface {
    /// `tile_metres` of the fixed looks (not stone or terrazzo, which have
    /// none of their own).
    fn tile_metres_plain(self) -> f32 {
        match self { Surface::Stone | Surface::Terrazzo => 1.0, s => s.tile_metres() }
    }
}

/// Unbounded value noise, roughly -0.5..0.5 (local to a piece, so it need
/// not wrap).
fn noise(x: f32, y: f32, seed: u32) -> f32 {
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - ix as f32, y - iy as f32);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let h = |a: i32, b: i32| hash(a, b, seed);
    let top = h(ix, iy) + (h(ix + 1, iy) - h(ix, iy)) * sx;
    let bottom = h(ix, iy + 1) + (h(ix + 1, iy + 1) - h(ix, iy + 1)) * sx;
    top + (bottom - top) * sy - 0.5
}

fn fnoise(x: f32, y: f32, octaves: u32, seed: u32) -> f32 {
    (0..octaves).map(|o| noise(x * (1 << o) as f32, y * (1 << o) as f32, seed + o) / (1 << o) as f32).sum()
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Wood at `across` and `along` a board (metres): colour and grain height.
fn wood(id: f32, across: f32, along: f32, seed: u32) -> ([f32; 3], f32) {
    let grain = fnoise(across * 70.0 + id * 40.0, along * 2.5 + id * 9.0, 4, seed);
    let ring = ((grain * 9.0 + id * 6.0).sin() * 0.5 + 0.5).powf(3.0);
    let t = 0.8 + id * 0.35 - ring * 0.18;
    ([0.42 * t, 0.26 * t, 0.13 * t], ring * 0.0002)
}

fn boards(st: Style) -> Painter {
    let (a, l, horizontal) = st.boards();
    let (nc, ns) = st.board_counts();
    let [tw, th] = st.tile();
    let stagger = matches!(st.layout, Layout::Boards | Layout::BoardFormed);
    let (joint, layout) = (st.joint, st.layout);
    Box::new(move |u, v| {
        let (px, py) = (u * tw, v * th);
        let (s, t) = if horizontal { (py, px) } else { (px, py) };
        let col = (s / a).floor();
        let ci = (col as i32).rem_euclid(nc);
        let tt = t + if stagger { hash(ci, 0, 3) * l } else { 0.0 };
        let seg = (tt / l).floor();
        let (lx, ly) = (s - col * a, tt - seg * l);
        let id = hash(ci, (seg as i32).rem_euclid(ns), 21);
        let groove = lx < joint || (stagger && ly < joint * 0.75);
        if layout == Layout::BoardFormed {
            // Cast concrete keeps the boards' grain and the fins at their joints.
            let (_, grain) = wood(id, lx, ly, 31);
            let mottle = fbm(u, v, 6, 3, 61) * 0.06;
            let x = 0.46 * (0.92 + id * 0.16) + mottle - grain * 150.0;
            return if groove { ([x * 0.6; 3], -0.003) } else { ([x, x * 0.99, x * 0.96], grain * 3.0) };
        }
        let (c, h) = wood(id, lx, ly, 31);
        if groove { ([c[0] * 0.3, c[1] * 0.3, c[2] * 0.3], -0.0015) } else { (c, h) }
    })
}

fn herringbone(st: Style) -> Painter {
    let (a, l, _) = st.boards();
    let n = ((l / a).round() as i32).max(2);
    let cell = l / n as f32;
    let tile = 2.0 * l;
    let joint = st.joint;
    Box::new(move |u, v| {
        let (px, py) = (u * tile, v * tile);
        let (cx, cy) = ((px / cell).floor() as i32, (py / cell).floor() as i32);
        let (fx, fy) = (px / cell - cx as f32, py / cell - cy as f32);
        // Planks lie on a lattice (1,1), (-n,n); a horizontal one starts at
        // its lattice point, a vertical one ends n cells above it, to its left.
        let mut plank = None;
        for t in 0..n {
            let (sum, dif) = (cx - t + cy, cy - cx + t);
            if sum.rem_euclid(2) == 0 && dif.rem_euclid(2 * n) == 0 { plank = Some((true, cx - t, cy, t as f32 + fx, fy)); break; }
        }
        if plank.is_none() {
            for s in 0..n {
                let (ox, oy) = (cx + n, cy + s);
                if (ox + oy).rem_euclid(2) == 0 && (oy - ox).rem_euclid(2 * n) == 0 { plank = Some((false, ox, oy, (n - 1 - s) as f32 + fy, fx)); break; }
            }
        }
        let Some((horizontal, ox, oy, along, across)) = plank else { return ([0.1; 3], -0.002) };
        let (along, across) = (along * cell, across * cell);
        let id = hash(ox.rem_euclid(2 * n), oy.rem_euclid(2 * n), if horizontal { 1 } else { 2 });
        let (c, h) = wood(id, across, along, 41);
        if across < joint || along < joint { ([c[0] * 0.3, c[1] * 0.3, c[2] * 0.3], -0.0015) } else { (c, h) }
    })
}

/// Brick colour at (x, y) metres in a brick.
fn brick(id: f32, burnt: bool, x: f32, y: f32) -> [f32; 3] {
    let t = 0.75 + id * 0.35 + fnoise(x * 30.0 + id * 17.0, y * 30.0, 3, 11) * 0.12;
    let base = if burnt { [0.16, 0.08, 0.05] } else { [0.36, 0.13, 0.07] };
    base.map(|b| b * t)
}

fn bond(st: Style) -> Painter {
    let (px, py) = (st.module[0] + st.joint, st.module[1] + st.joint);
    let (nc, nr) = st.counts();
    let [tw, th] = st.tile();
    let running = st.layout == Layout::RunningBond;
    let j = st.joint;
    Box::new(move |u, v| {
        let (x, y) = (u * tw, v * th);
        let row = (y / py).floor();
        let ri = row as i32;
        let xo = x + if running && ri.rem_euclid(2) == 1 { 0.5 * px } else { 0.0 };
        let col = (xo / px).floor();
        let ci = (col as i32).rem_euclid(nc);
        let (bx, by) = (xo - col * px, y - row * py);
        if bx < j || by < j {
            let g = 0.42 + value(u, v, 96, 9) * 0.08;
            return ([g, g * 0.97, g * 0.92], -0.008);
        }
        let id = hash(ci, ri.rem_euclid(nr), 7);
        let burnt = hash(ci, ri.rem_euclid(nr), 13) < 0.12;
        (brick(id, burnt, bx, by), noise(bx * 60.0, by * 60.0, 5) * 0.0015)
    })
}

/// Stone colour for a stone `id` and its own coordinates.
fn stone_colour(id: f32, id2: f32, x: f32, y: f32) -> [f32; 3] {
    let warm = [0.32, 0.26, 0.18];
    let cool = [0.27, 0.26, 0.25];
    let t = 0.6 + id * 0.8 + fnoise(x * 18.0 + id2 * 31.0, y * 18.0, 3, 83) * 0.35;
    let speck = if noise(x * 220.0 + id2 * 5.0, y * 220.0, 87) > 0.3 { 0.8 } else { 1.0 };
    std::array::from_fn(|k| (cool[k] + (warm[k] - cool[k]) * id2) * t * speck)
}

fn fieldstone(st: Style) -> Painter {
    let [mw, mh] = st.module;
    let (nc, nr) = st.counts();
    let [tw, th] = st.tile();
    let j = st.joint;
    Box::new(move |u, v| {
        // A slow warp so the stones are not a jittered grid.
        let (wx, wy) = (value(u, v, 6, 91) * 0.35 * mw, value(u, v, 6, 93) * 0.35 * mh);
        let (x, y) = (u * tw + wx, v * th + wy);
        let (gx, gy) = (x / mw, y / mh);
        let (ix, iy) = (gx.floor() as i32, gy.floor() as i32);
        let (mut f1, mut f2, mut id) = (f32::MAX, f32::MAX, (0, 0, 0.0f32, 0.0f32));
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (cx, cy) = (ix + dx, iy + dy);
                let (hx, hy) = (cx.rem_euclid(nc), cy.rem_euclid(nr));
                let jx = 0.5 + (hash(hx, hy, 101) - 0.5) * 0.9;
                let jy = 0.5 + (hash(hx, hy, 103) - 0.5) * 0.9;
                let d = ((cx as f32 + jx - gx) * mw).hypot((cy as f32 + jy - gy) * mh);
                if d < f1 { f2 = f1; f1 = d; id = (cx, cy, hash(hx, hy, 105), hash(hx, hy, 107)); } else if d < f2 { f2 = d; }
            }
        }
        let edge = f2 - f1;
        if edge < j {
            let g = 0.34 + value(u, v, 96, 95) * 0.06;
            return ([g, g * 0.96, g * 0.9], -0.012);
        }
        let (lx, ly) = (x - (id.0 as f32) * mw, y - (id.1 as f32) * mh);
        // Pillowed faces, split-face roughness.
        let h = smoothstep(j, j + 0.03, edge) * 0.012 + fnoise(lx * 25.0, ly * 25.0, 3, 97) * 0.006;
        (stone_colour(id.2, id.3, lx, ly), h)
    })
}

fn ashlar(st: Style) -> Painter {
    let py = st.module[1] + st.joint;
    let (nc, nr) = st.counts();
    let [tw, th] = st.tile();
    let j = st.joint;
    // Block edges along each course, from varied lengths scaled to close the tile.
    let bounds: Vec<Vec<f32>> = (0..nr).map(|r| {
        let n = nc as usize;
        let w: Vec<f32> = (0..n).map(|k| 0.7 + 0.8 * hash(r, k as i32, 109)).collect();
        let total: f32 = w.iter().sum();
        let mut acc = 0.0;
        let mut edges = vec![0.0];
        for x in w { acc += x / total * tw; edges.push(acc); }
        edges
    }).collect();
    Box::new(move |u, v| {
        let (x, y) = ((u * tw).rem_euclid(tw), v * th);
        let row = (y / py).floor();
        let ri = (row as i32).rem_euclid(nr);
        let by = y - row * py;
        let edges = &bounds[ri as usize];
        let k = edges.partition_point(|&e| e <= x).saturating_sub(1).min(edges.len() - 2);
        let bx = x - edges[k];
        if bx < j || by < j {
            let g = 0.38 + value(u, v, 96, 111) * 0.05;
            return ([g, g * 0.97, g * 0.92], -0.006);
        }
        let (a, b) = (hash(ri, k as i32, 113), hash(ri, k as i32, 115));
        (stone_colour(a, b, bx, by), fnoise(bx * 30.0, by * 30.0, 3, 117) * 0.002)
    })
}

fn grid(st: Style) -> Painter {
    let (px, py) = (st.module[0] + st.joint, st.module[1] + st.joint);
    let (nc, nr) = st.counts();
    let [tw, th] = st.tile();
    let j = st.joint;
    Box::new(move |u, v| {
        let (x, y) = (u * tw, v * th);
        let (col, row) = ((x / px).floor(), (y / py).floor());
        let (ci, ri) = ((col as i32).rem_euclid(nc), (row as i32).rem_euclid(nr));
        let (bx, by) = (x - col * px, y - row * py);
        if bx < j || by < j { return ([0.3, 0.29, 0.27], -0.002); }
        let id = hash(ci, ri, 41);
        let vein = fnoise(bx * 7.0 + id * 11.0, by * 7.0, 4, 43) * 0.1;
        let g = 0.5 * (0.9 + id * 0.2) + vein;
        let bevel = smoothstep(j, j + 0.004, bx.min(by).min(px - bx).min(py - by));
        ([g, g, g], bevel * 0.0006 - 0.0006)
    })
}

fn terrazzo(st: Style) -> Painter {
    let [tw, th] = st.tile();
    let cell = 0.04;
    let (nc, nr) = (((tw / cell).round() as i32).max(1), ((th / cell).round() as i32).max(1));
    let (cw, ch) = (tw / nc as f32, th / nr as f32);
    Box::new(move |u, v| {
        let (x, y) = (u * tw, v * th);
        let (gx, gy) = ((x / cw).floor() as i32, (y / ch).floor() as i32);
        let m = 0.64 + value(u, v, 24, 121) * 0.06;
        let mut c = [m, m * 0.96, m * 0.88];
        let mut h = 0.0;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (cx, cy) = (gx + dx, gy + dy);
                let (hx, hy) = (cx.rem_euclid(nc), cy.rem_euclid(nr));
                if hash(hx, hy, 123) > 0.72 { continue; }
                let centre = ((cx as f32 + 0.2 + hash(hx, hy, 125) * 0.6) * cw, (cy as f32 + 0.2 + hash(hx, hy, 127) * 0.6) * ch);
                // A chip: an ellipse turned a random way, with a ragged edge.
                let (ax, ay) = (x - centre.0, y - centre.1);
                let ang = hash(hx, hy, 129) * std::f32::consts::PI;
                let (s, co) = ang.sin_cos();
                let (rx, ry) = (ax * co + ay * s, -ax * s + ay * co);
                let r = 0.004 + hash(hx, hy, 131) * 0.011;
                let d = (rx / (r * 1.4)).hypot(ry / (r * 0.8)) + value(u, v, 120, 133) * 0.5;
                if d < 1.0 {
                    let pick = hash(hx, hy, 135);
                    let chip: [f32; 3] = if pick < 0.3 { [0.85, 0.83, 0.78] } else if pick < 0.55 { [0.5, 0.5, 0.5] }
                        else if pick < 0.7 { [0.22, 0.22, 0.23] } else if pick < 0.85 { [0.62, 0.45, 0.26] }
                        else if pick < 0.95 { [0.3, 0.2, 0.14] } else { [0.05, 0.05, 0.05] };
                    let t = 0.9 + hash(hx, hy, 137) * 0.2;
                    c = chip.map(|k| k * t);
                    h = 0.0002;
                }
            }
        }
        (c, h)
    })
}

fn to_srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}

/// Hash of integer coordinates to 0..1.
fn hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6b343) ^ (y as u32).wrapping_mul(0xd8163841) ^ seed.wrapping_mul(0xcb1ab31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1e995);
    h ^= h >> 15;
    (h & 0xffffff) as f32 / 0xffffff as f32
}

/// Tileable value noise: `cells` cells over 0..1, wrapping.
fn value(u: f32, v: f32, cells: i32, seed: u32) -> f32 {
    let (x, y) = (u * cells as f32, v * cells as f32);
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - ix as f32, y - iy as f32);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let h = |a: i32, b: i32| hash(a.rem_euclid(cells), b.rem_euclid(cells), seed);
    let top = h(ix, iy) + (h(ix + 1, iy) - h(ix, iy)) * sx;
    let bottom = h(ix, iy + 1) + (h(ix + 1, iy + 1) - h(ix, iy + 1)) * sx;
    top + (bottom - top) * sy - 0.5
}

/// Fractal value noise, roughly -0.5..0.5, tileable.
fn fbm(u: f32, v: f32, cells: i32, octaves: u32, seed: u32) -> f32 {
    (0..octaves).map(|o| value(u, v, cells << o, seed + o) / (1 << o) as f32).sum::<f32>()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Layout; 11] = [Layout::Boards, Layout::Slats, Layout::Veneer, Layout::Herringbone, Layout::RunningBond, Layout::StackBond,
        Layout::Fieldstone, Layout::Ashlar, Layout::Grid, Layout::Terrazzo, Layout::BoardFormed];

    #[test]
    fn every_pattern_tiles_and_has_joints() {
        for layout in ALL {
            let st = Style::laid(layout, None, None);
            let paint = st.painter();
            let (mut differ, mut low, total) = (0, 0, 40 * 40);
            for i in 0..40 {
                for k in 0..40 {
                    let (u, v) = (0.013 + i as f32 * 0.0243, 0.007 + k as f32 * 0.0247);
                    let (c, h) = paint(u, v);
                    assert!(c.iter().all(|x| x.is_finite() && (0.0..1.5).contains(x)) && h.is_finite(), "{layout:?} {c:?} {h}");
                    // One repeat further on, in either direction, is the same texel.
                    let (c2, _) = paint(u + 1.0, v);
                    let (c3, _) = paint(u, v + 1.0);
                    if (0..3).any(|k| (c[k] - c2[k]).abs() > 2e-3 || (c[k] - c3[k]).abs() > 2e-3) { differ += 1; }
                    if h < -0.0005 { low += 1; }
                }
            }
            assert!(differ * 50 < total, "{layout:?} does not tile: {differ} of {total} samples differ");
            if !matches!(layout, Layout::Veneer | Layout::Terrazzo) { assert!(low > 0, "{layout:?} has no joints"); }
        }
    }

    #[test]
    fn module_sets_the_tile() {
        // Vertical cedar boards 12 cm by 3 m: five boards across, one length down.
        let cedar = Style::laid(Layout::Boards, Some([0.12, 3.0]), None);
        let [w, h] = cedar.tile();
        assert!((w - 0.6).abs() < 1e-4 && (h - 3.0).abs() < 1e-4, "{w} {h}");
        // Boards that run along u swap the two.
        let [w, h] = Style::laid(Layout::Boards, Some([3.0, 0.12]), None).tile();
        assert!((w - 3.0).abs() < 1e-4 && (h - 0.6).abs() < 1e-4, "{w} {h}");
        // A brick bond's repeat holds whole bricks and an even number of courses.
        let st = Style::laid(Layout::RunningBond, Some([0.21, 0.05]), None);
        let [w, h] = st.tile();
        let (nc, nr) = st.counts();
        assert!(nr % 2 == 0 && nc % 2 == 0 && (w - nc as f32 * 0.22).abs() < 1e-4 && (h - nr as f32 * 0.06).abs() < 1e-4);
        // Nonsense modules fall back to the pattern's own.
        assert_eq!(Style::laid(Layout::Grid, Some([0.0, f32::NAN]), Some(-1.0)).module, [0.6, 0.6]);
    }

    #[test]
    fn design_materials_pick_their_pattern() {
        let pick = |json: &str, name: &str| Style::from_extras(&serde_json::from_str(json).unwrap(), name);
        let cedar = pick(r#"{"pattern":"planks","moduleSize":[0.12,3.0]}"#, "Western red cedar").unwrap();
        assert_eq!((cedar.layout, cedar.surface, cedar.module), (Layout::Boards, Surface::Oak, [0.12, 3.0]));
        assert_eq!(pick(r#"{"pattern":"random-stone","category":"stone","moduleSize":[0.4,0.15]}"#, "x").unwrap().surface, Surface::Stone);
        assert_eq!(pick(r#"{"pattern":"ashlar"}"#, "x").unwrap().layout, Layout::Ashlar);
        assert_eq!(pick(r#"{"pattern":"stack-bond"}"#, "x").unwrap().layout, Layout::StackBond);
        assert_eq!(pick(r#"{"pattern":"board-formed"}"#, "x").unwrap().surface, Surface::Concrete);
        assert_eq!(pick(r#"{"pattern":"terrazzo"}"#, "x").unwrap().layout, Layout::Terrazzo);
        // No pattern: the category says; a name says when there is no category.
        assert_eq!(pick(r#"{"category":"wood","pattern":"none"}"#, "x").unwrap().layout, Layout::Veneer);
        assert_eq!(pick(r#"{"subject":"s"}"#, "Lime plaster, warm white").unwrap(), Style::standard(Surface::Plaster));
        assert_eq!(pick(r#"{"subject":"s"}"#, "Sand-cement screed").unwrap(), Style::standard(Surface::Concrete));
        assert_eq!(pick(r#"{"subject":"s"}"#, "Walnut").unwrap().layout, Layout::Veneer);
        // Plain categories and unknown names stay flat.
        for (json, name) in [(r#"{"category":"textile"}"#, "Boucle"), (r#"{"category":"metal","pattern":"none"}"#, "x"), (r#"{"category":"glass"}"#, "x"), (r#"{"subject":"s"}"#, "Mustard boucle")] {
            assert_eq!(pick(json, name), None, "{json}");
        }
    }

    #[test]
    fn a_style_is_tinted_to_the_design_colour() {
        let mut scene = Scene::default();
        let mut cache = HashMap::new();
        let target = [0.25, 0.07, 0.034];
        let st = Style::laid(Layout::RunningBond, Some([0.21, 0.05]), None);
        let m = st.add(&mut scene, &mut cache, target, 0.9, 0.0, false);
        // A second colour shares the texture.
        let m2 = st.add(&mut scene, &mut cache, [0.5, 0.5, 0.5], 0.5, 0.0, false);
        assert_eq!(scene.textures.len(), 1);
        let (layer, mean) = cache[&st.key()];
        let (a, b) = (&scene.materials[m as usize], &scene.materials[m2 as usize]);
        assert_eq!((a.layer, b.layer), (layer, layer));
        for k in 0..3 { assert!((a.tint[k] * mean[k] - target[k]).abs() < 1e-4, "{k}"); }
        assert_eq!(a.roughness, 0.9);
    }
}
