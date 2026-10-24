//! Car materials from `engine/data/car_materials.toml`, read at run time:
//! paint finish and colours, wear and grime per car, and the other car
//! parts (tyres, rims, chrome, plastic) by glTF material name. The shader
//! side is render/src/shaders/car_material.wgsl.
//!
//! One car's paint is the `[paint.<name>]` preset it picks, overridden in
//! turn by `[all]`, `[model.<id>]`, `[car.<index>]` and `--car-set`.
//! A part (`[part.<name>]`, chosen by `match`) takes the car's grime keys
//! and then its own. `--car-set [N:][part.]key=value` changes one key for
//! all cars, or car N, or one part.

use render::scene::GpuCarMaterial;
use std::path::{Path, PathBuf};
use toml::{Table, Value};

/// The checked-in copy, used when the file is not on disk.
const BUILT_IN: &str = include_str!("../../../data/car_materials.toml");

/// The kinds car_material.wgsl knows (its CAR_* constants).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Paint = 0,
    Rubber = 1,
    Metal = 2,
    Chrome = 3,
    Plastic = 4,
    Textured = 6,
    Lamp = 7,
}

/// Only original, measured rear-lens triangles receive these internal slots.
/// The shader uses second.xy for red/white radiance and second.w for this role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LampRole { RearAtlas = 0, Red = 1, Reverse = 2 }
impl LampRole {
    pub fn part(self) -> &'static str {
        match self {Self::RearAtlas=>"__lamp_rear_atlas",Self::Red=>"__lamp_rear_red",Self::Reverse=>"__lamp_rear_reverse"}
    }
    pub fn from_part(part:&str) -> Option<Self> {
        match part {"__lamp_rear_atlas"=>Some(Self::RearAtlas),"__lamp_rear_red"=>Some(Self::Red),"__lamp_rear_reverse"=>Some(Self::Reverse),_=>None}
    }
}

/// Keys only paint uses; parts never inherit them from the car.
const PAINT_ONLY: [&str; 16] = ["paint", "finish", "colour", "second", "roughness", "metal", "clearcoat", "coat_roughness", "flake", "flake_size", "flake_spread", "orange_peel", "chips", "scratches", "swirls", "kind"];

/// One car material slot in the scene, in `Scene::car_materials` order.
#[derive(Clone, Debug)]
pub struct Slot {
    /// Car index in the race (the `[car.N]` table).
    pub car: usize,
    /// Model id: the glb's file stem, e.g. "mv6" (the `[model.<id>]` table).
    pub model: String,
    /// The game's colour for this car (linear), unless the data sets one.
    pub game_colour: [f32; 3],
    /// The `[part.<name>]` this glTF material matched.
    pub part: String,
    /// The glTF material's own colour, roughness and metalness.
    pub gltf: [f32; 5],
    pub body: [f32; 4],
    pub axles: [f32; 4],
}

#[derive(Clone)]
pub struct Looks {
    doc: Table,
    overrides: Vec<String>,
    /// Where it was read from, for reloading; None: the built-in copy.
    pub path: Option<PathBuf>,
}

impl Looks {
    /// `path`, or engine/data/car_materials.toml in this checkout, or the
    /// built-in copy. `overrides` are `--car-set` values.
    pub fn load(path: Option<&Path>, overrides: Vec<String>) -> Result<Looks, String> {
        let default = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/car_materials.toml");
        let path = path.map(Path::to_path_buf).or(Some(default).filter(|p| p.exists()));
        let text = match &path {
            Some(p) => std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?,
            None => BUILT_IN.to_owned(),
        };
        let looks = Looks { doc: text.parse::<Table>().map_err(|e| format!("car materials: {e}"))?, overrides, path };
        for o in &looks.overrides {
            looks.parse_override(o)?;
        }
        Ok(looks)
    }

    /// Read the same file again (after an edit).
    pub fn reload(&self) -> Result<Looks, String> {
        Looks::load(self.path.as_deref(), self.overrides.clone())
    }

    /// The file's modification time, to poll for edits.
    pub fn stamp(&self) -> Option<std::time::SystemTime> {
        std::fs::metadata(self.path.as_ref()?).and_then(|m| m.modified()).ok()
    }

    fn table(&self, path: &[&str]) -> Option<&Table> {
        let mut t = &self.doc;
        for k in path {
            t = t.get(*k)?.as_table()?;
        }
        Some(t)
    }

    /// The part a glTF material belongs to: the first `[part.*]` whose
    /// `match` list has a substring of the name; "other" when none does.
    pub fn part_for(&self, material_name: &str) -> (String, Kind) {
        if let Some(parts) = self.table(&["part"]) {
            for (name, t) in parts {
                let hit = t.get("match").and_then(Value::as_array).is_some_and(|m| m.iter().filter_map(Value::as_str).any(|s| material_name.contains(s)));
                if hit {
                    return (name.clone(), kind(t.get("kind").and_then(Value::as_str).unwrap_or("textured")));
                }
            }
        }
        ("other".into(), Kind::Textured)
    }

    /// `[N:][part.]key=value` as (car, part, key, value).
    fn parse_override(&self, o: &str) -> Result<(Option<usize>, Option<String>, String, Value), String> {
        let (lhs, rhs) = o.split_once('=').ok_or_else(|| format!("--car-set {o}: expected key=value"))?;
        let (car, key) = match lhs.split_once(':') {
            Some((n, k)) => (Some(n.trim().parse::<usize>().map_err(|_| format!("--car-set {o}: bad car index"))?), k),
            None => (None, lhs),
        };
        let (part, key) = match key.split_once('.') {
            Some((p, k)) => (Some(p.trim().to_string()), k),
            None => (None, key),
        };
        let rhs = rhs.trim();
        let value = rhs.parse::<f64>().map(Value::Float).unwrap_or_else(|_| Value::String(rhs.trim_matches('"').to_string()));
        Ok((car, part, key.trim().to_string(), value))
    }

    /// Everything that applies to one car's paint, merged.
    fn car_table(&self, car: usize, model: &str) -> Table {
        let mut merged = Table::new();
        let layers = [self.table(&["all"]), self.table(&["model", model]), self.table(&["car", &car.to_string()])];
        let mut cli = Table::new();
        for o in &self.overrides {
            if let Ok((c, None, k, v)) = self.parse_override(o) {
                if c.is_none() || c == Some(car) {
                    cli.insert(k, v);
                }
            }
        }
        // The paint preset is chosen by the last layer that names one.
        let paint = layers.iter().flatten().chain(std::iter::once(&&cli)).filter_map(|t| t.get("paint").and_then(Value::as_str)).last().unwrap_or("gloss").to_string();
        if let Some(preset) = self.table(&["paint", &paint]) {
            merged.extend(preset.clone());
        }
        for t in layers.into_iter().flatten() {
            merged.extend(t.clone());
        }
        merged.extend(cli);
        merged
    }

    /// The GPU material for one slot.
    pub fn material(&self, slot: &Slot) -> Result<GpuCarMaterial, String> {
        if let Some(role)=LampRole::from_part(&slot.part) {
            // Preserve authored lens colour/UVs and transparency. Paint edits
            // and generic dirt must not recolour the optical emission mask.
            return Ok(GpuCarMaterial {
                colour:[1.0,1.0,1.0,Kind::Lamp as u32 as f32],
                second:[0.0,0.0,0.0,role as u32 as f32],
                surface:[slot.gltf[3],slot.gltf[4],0.0,0.04],
                body:slot.body,axles:slot.axles,..Default::default()
            });
        }
        let car = self.car_table(slot.car, &slot.model);
        let mut t = Table::new();
        let paint = self.part_kind(&slot.part) == Kind::Paint;
        for (k, v) in &car {
            if paint || !PAINT_ONLY.contains(&k.as_str()) {
                t.insert(k.clone(), v.clone());
            }
        }
        if let Some(p) = self.table(&["part", &slot.part]) {
            for (k, v) in p {
                if k != "match" {
                    t.insert(k.clone(), v.clone());
                }
            }
        }
        for o in &self.overrides {
            if let Ok((c, Some(part), k, v)) = self.parse_override(o) {
                if part == slot.part && (c.is_none() || c == Some(slot.car)) {
                    t.insert(k, v);
                }
            }
        }
        let ctx = |e: String| format!("car {} ({}) part {}: {e}", slot.car, slot.model, slot.part);
        let num = |k: &str, d: f32| -> Result<f32, String> {
            match t.get(k) {
                None => Ok(d),
                Some(Value::Float(f)) => Ok(*f as f32),
                Some(Value::Integer(i)) => Ok(*i as f32),
                Some(v) => Err(ctx(format!("{k} = {v}: expected a number"))),
            }
        };
        let colour = |k: &str, d: [f32; 3]| -> Result<[f32; 3], String> { t.get(k).map(|v| parse_colour(v).map_err(|e| ctx(format!("{k}: {e}")))).unwrap_or(Ok(d)) };
        let kind = kind(t.get("kind").and_then(Value::as_str).unwrap_or(if paint { "paint" } else { "textured" }));
        let finish = match t.get("finish").and_then(Value::as_str).unwrap_or("solid") {
            "solid" => 0.0,
            "metallic" => 1.0,
            "pearl" => 2.0,
            "candy" => 3.0,
            other => return Err(ctx(format!("finish {other}: expected solid, metallic, pearl or candy"))),
        };
        let [gr, gg, gb, g_rough, g_metal] = slot.gltf;
        let base = if paint { slot.game_colour } else { [gr, gg, gb] };
        let c = colour("colour", base)?;
        let second = colour("second", c)?;
        let dirt = colour("dirt_colour", parse_colour(&Value::String("#8a7558".into()))?)?;
        Ok(GpuCarMaterial {
            colour: [c[0], c[1], c[2], kind as u32 as f32],
            second: [second[0], second[1], second[2], finish],
            surface: [num("roughness", if paint { 0.4 } else { g_rough })?, num("metal", if paint { 0.0 } else { g_metal })?, num("clearcoat", 0.0)?, num("coat_roughness", 0.04)?],
            flake: [num("flake", 0.0)?, num("flake_size", 0.0006)?, num("flake_spread", 0.25)?, num("orange_peel", 0.0)?],
            wear: [num("chips", 0.0)?, num("scratches", 0.0)?, num("swirls", 0.0)?, num("fade", 0.0)?],
            grime: [num("dust", 0.0)?, num("mud", 0.0)?, num("rust", 0.0)?, num("seed", (slot.car * 7919 + 17) as f32)?.round()],
            dirt: [dirt[0], dirt[1], dirt[2], num("wetness", 0.0)?],
            body: slot.body,
            axles: slot.axles,
        })
    }

    fn part_kind(&self, part: &str) -> Kind {
        kind(self.table(&["part", part]).and_then(|t| t.get("kind")).and_then(Value::as_str).unwrap_or("textured"))
    }

    /// Every slot's material, in scene order.
    pub fn materials(&self, slots: &[Slot]) -> Result<Vec<GpuCarMaterial>, String> {
        slots.iter().map(|s| self.material(s)).collect()
    }
}

/// Live editing for `play` and `shot --watch`: notices edits to the data
/// file and to the shading shaders (`KERN_SHADER_DIR`) and applies them to
/// a running renderer. Broken data or shader code keeps the previous one.
pub struct LiveEdit {
    looks: Looks,
    seen: (Option<std::time::SystemTime>, Option<std::time::SystemTime>),
    polled: std::time::Instant,
}

impl LiveEdit {
    pub fn new(looks: Looks) -> LiveEdit {
        LiveEdit { seen: (looks.stamp(), render::shaders::stamp()), looks, polled: std::time::Instant::now() }
    }

    pub fn looks(&self) -> &Looks {
        &self.looks
    }

    /// At most every `every`: None when nothing changed, else what was done
    /// (or failed), and when the newest edited file was saved.
    pub fn poll(&mut self, every: std::time::Duration, gpu: &render::Gpu, renderer: &mut render::Renderer, slots: &[Slot]) -> Option<(Vec<String>, Option<std::time::SystemTime>)> {
        if self.polled.elapsed() < every {
            return None;
        }
        self.polled = std::time::Instant::now();
        let now = (self.looks.stamp(), render::shaders::stamp());
        if now == self.seen {
            return None;
        }
        let mut log = Vec::new();
        if now.0 != self.seen.0 {
            match self.looks.reload().and_then(|l| l.materials(slots).map(|m| (l, m))).and_then(|(l, m)| renderer.set_car_materials(gpu, &m).map(|_| l)) {
                Ok(l) => {
                    self.looks = l;
                    log.push("car materials reloaded".into());
                }
                Err(e) => log.push(format!("car materials, keeping the previous ones: {e}")),
            }
        }
        if now.1 != self.seen.1 {
            let t = std::time::Instant::now();
            match renderer.reload_shaders(gpu) {
                Ok(rebuilt) => log.push(format!("shaders {} in {:.0} ms", if rebuilt { "rebuilt" } else { "unchanged" }, t.elapsed().as_secs_f64() * 1e3)),
                Err(e) => log.push(format!("shader error, keeping the previous pipelines:\n{e}")),
            }
        }
        self.seen = now;
        Some((log, [now.0, now.1].into_iter().flatten().max()))
    }
}

fn kind(name: &str) -> Kind {
    match name {
        "paint" => Kind::Paint,
        "rubber" => Kind::Rubber,
        "metal" => Kind::Metal,
        "chrome" => Kind::Chrome,
        "plastic" => Kind::Plastic,
        _ => Kind::Textured,
    }
}

/// "#rrggbb" (sRGB, like the game's colours) or [r, g, b] in linear 0..1.
fn parse_colour(v: &Value) -> Result<[f32; 3], String> {
    let lin = |x: f32| if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) };
    match v {
        Value::String(s) => {
            let h = s.trim_start_matches('#');
            let n = u32::from_str_radix(h, 16).ok().filter(|_| h.len() == 6).ok_or_else(|| format!("{s}: expected #rrggbb"))?;
            Ok([lin(((n >> 16) & 255) as f32 / 255.0), lin(((n >> 8) & 255) as f32 / 255.0), lin((n & 255) as f32 / 255.0)])
        }
        Value::Array(a) if a.len() == 3 => {
            let mut c = [0.0; 3];
            for (k, x) in a.iter().enumerate() {
                c[k] = x.as_float().or_else(|| x.as_integer().map(|i| i as f64)).ok_or("colour components must be numbers")? as f32;
            }
            Ok(c)
        }
        other => Err(format!("{other}: expected \"#rrggbb\" or [r, g, b] (linear)")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(part: &str) -> Slot {
        Slot { car: 1, model: "mv6".into(), game_colour: [0.5, 0.1, 0.1], part: part.into(), gltf: [0.2, 0.2, 0.2, 0.7, 0.0], body: [0.0; 4], axles: [0.0; 4] }
    }

    #[test]
    fn checked_in_file_resolves_for_every_car_and_part() {
        let looks = Looks { doc: BUILT_IN.parse().unwrap(), overrides: vec![], path: None };
        let parts: Vec<String> = looks.table(&["part"]).unwrap().keys().cloned().collect();
        for car in 0..8 {
            for part in &parts {
                let m = looks.material(&Slot { car, ..slot(part) }).unwrap();
                assert!([m.colour, m.second, m.surface, m.flake, m.wear, m.grime, m.dirt].iter().flatten().all(|x| x.is_finite()));
            }
        }
        assert_eq!(looks.part_for("PlaceholderMV683_Bodymat.001").1, Kind::Paint);
        assert_eq!(looks.part_for("RB1c_Tire_1k.002").1, Kind::Rubber);
    }

    #[test]
    fn layers_and_overrides_apply_in_order() {
        let doc = r##"
            [paint.matte]
            finish = "solid"
            roughness = 0.7
            [paint.shiny]
            finish = "metallic"
            roughness = 0.3
            [all]
            paint = "matte"
            dust = 0.2
            [car.1]
            paint = "shiny"
            colour = "#ff0000"
            [part.paint]
            match = ["Paint"]
            kind = "paint"
            [part.tyre]
            match = ["Tire"]
            kind = "rubber"
            roughness = 0.9
        "##;
        let looks = Looks { doc: doc.parse().unwrap(), overrides: vec!["1:rust=0.5".into(), "tyre.roughness=0.8".into(), "0:dust=0.9".into()], path: None };
        let paint = looks.material(&slot("paint")).unwrap();
        assert_eq!(paint.second[3], 1.0, "car 1 picks the metallic preset");
        assert_eq!(paint.surface[0], 0.3);
        assert_eq!(paint.colour[0], 1.0);
        assert_eq!(paint.grime[0], 0.2, "car 0's override does not reach car 1");
        assert_eq!(paint.grime[2], 0.5);
        let tyre = looks.material(&slot("tyre")).unwrap();
        assert_eq!(tyre.surface[0], 0.8, "part override wins");
        assert_eq!(tyre.colour[3], Kind::Rubber as u32 as f32);
        assert_eq!(tyre.colour[0], 0.2, "parts keep the glTF colour, not the paint");
        assert_eq!(tyre.grime[2], 0.5, "parts inherit the car's grime");
        let car0 = looks.material(&Slot { car: 0, ..slot("paint") }).unwrap();
        assert_eq!(car0.surface[0], 0.7);
        assert_eq!(car0.grime[0], 0.9);
    }
}
