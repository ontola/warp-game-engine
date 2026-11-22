//! The models as the Godot game builds them, exported by its
//! tools/warp_kits.gd (see tools/export-kits.sh): node trees of meshes with
//! their materials, lights and particle emitters, one per monster kind, the
//! wizard, each biome's props in a few looks, wall rocks and torches.
use glam::{Mat4, Vec3};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
pub struct File {
    pub meshes: Vec<MeshD>,
    pub materials: Vec<Value>,
    pub textures: Vec<TextureD>,
    pub kits: HashMap<String, Value>,
}

#[derive(Deserialize)]
pub struct MeshD {
    #[serde(rename = "type")]
    pub kind: String,
    pub surfaces: Vec<SurfaceD>,
}

#[derive(Deserialize)]
pub struct SurfaceD {
    pub positions: [usize; 2],
    pub normals: [usize; 2],
    pub uvs: [usize; 2],
    pub colors: [usize; 2],
    pub indices: [usize; 2],
    pub material: i64,
}

#[derive(Deserialize)]
pub struct TextureD {
    pub file: String,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub normal_map: bool,
    /// A height picture for a normal map: Godot's bump_strength.
    #[serde(default)]
    pub bump_strength: f32,
}

/// One node of a kit: where it sits under its parent and what it draws.
#[derive(Clone, Debug)]
pub struct Node {
    pub name: String,
    pub xf: Mat4,
    pub visible: bool,
    pub mesh: Option<usize>,
    pub material: Option<usize>,
    pub shadow: bool,
    pub light: Option<Light>,
    pub particles: Option<Value>,
    pub script: String,
}

#[derive(Clone, Copy, Debug)]
pub struct Light {
    /// sRGB, as Godot's light_color.
    pub color: [f32; 3],
    pub energy: f32,
    pub range: f32,
    pub attenuation: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Kit {
    pub nodes: Vec<Node>,
    pub tree: kern::parenting::ModelHierarchy,
    pub eyes: Vec<usize>,
    pub base_scale: f32,
}

impl Kit {
    /// Node `i` in the kit's own space (the root's transform left out).
    pub fn model_xf(&self, i: usize) -> Mat4 {
        self.tree.model_transform(i, |j| self.nodes[j].xf)
    }

    pub fn shown(&self, i: usize) -> bool {
        self.tree.ancestors(i).all(|j| self.nodes[j].visible)
    }
}

pub struct Kits {
    pub dir: PathBuf,
    pub file: File,
    pub bin: Vec<u8>,
    kits: HashMap<String, Kit>,
}

fn mat4(v: &Value) -> Mat4 {
    let a: Vec<f32> = v.as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_default();
    if a.len() == 16 { Mat4::from_cols_slice(&a) } else { Mat4::IDENTITY }
}

fn rgb(v: &Value) -> [f32; 3] {
    [0, 1, 2].map(|i| v[i].as_f64().unwrap_or(0.0) as f32)
}

fn index(v: &Value) -> Option<usize> {
    v.as_i64().filter(|&i| i >= 0).map(|i| i as usize)
}

impl Kits {
    /// The kits next to the binary's sources (app/assets/kits), or `dir`.
    pub fn load(dir: Option<&Path>) -> Result<Kits, String> {
        let dir = dir.map(Path::to_path_buf).unwrap_or_else(default_dir);
        let text = vfs::read_to_string(dir.join("kits.json")).map_err(|e| format!("{}: {e}", dir.join("kits.json").display()))?;
        let file: File = serde_json::from_str(&text).map_err(|e| format!("kits.json: {e}"))?;
        let gz = vfs::read(dir.join("meshes.bin.gz")).map_err(|e| format!("meshes.bin.gz: {e}"))?;
        let mut bin = Vec::new();
        flate2::read::GzDecoder::new(&gz[..]).read_to_end(&mut bin).map_err(|e| format!("meshes.bin.gz: {e}"))?;
        let mut kits = HashMap::new();
        for (name, v) in &file.kits {
            let Some(nodes) = v["nodes"].as_array() else { continue };
            let parents = nodes.iter().map(|n| index(&n["parent"])).collect();
            let nodes: Vec<Node> = nodes
                .iter()
                .map(|n| Node {
                    name: n["name"].as_str().unwrap_or("").to_string(),
                    xf: mat4(&n["xf"]),
                    visible: n["visible"].as_bool().unwrap_or(true),
                    mesh: index(&n["mesh"]),
                    material: index(&n["material"]),
                    shadow: n["shadow"].as_bool().unwrap_or(true),
                    light: n.get("light").filter(|l| l.is_object()).map(|l| Light {
                        color: rgb(&l["color"]),
                        energy: l["energy"].as_f64().unwrap_or(1.0) as f32,
                        range: l["range"].as_f64().unwrap_or(5.0) as f32,
                        attenuation: l["attenuation"].as_f64().unwrap_or(1.0) as f32,
                    }),
                    particles: n.get("particles").cloned(),
                    script: n["script"].as_str().unwrap_or("").to_string(),
                })
                .collect();
            let named = v["named"].as_object().map(|o| o.iter().filter_map(|(k, i)| Some((k.clone(), index(i)?))).collect()).unwrap_or_default();
            let tree = kern::parenting::ModelHierarchy::new(parents, named)
                .map_err(|err| format!("kit {name}: {err}"))?;
            let eyes = v["eyes"].as_array().map(|a| a.iter().filter_map(index).collect()).unwrap_or_default();
            let base_scale = v["base_scale"].as_f64().unwrap_or(1.0) as f32;
            let mut nodes = nodes;
            // A burrower's body was hidden in Godot until its script showed
            // it (Enemy._go_under at spawn); here the view shows and sinks it.
            if name.starts_with("enemy/") && let Some(root) = nodes.first_mut() {
                root.visible = true;
            }
            kits.insert(name.clone(), Kit { nodes, tree, eyes, base_scale });
        }
        Ok(Kits { dir, file, bin, kits })
    }

    pub fn kit(&self, name: &str) -> Option<&Kit> {
        self.kits.get(name)
    }

    /// A plain kit entry (walls/…, crag/…: a mesh or materials).
    pub fn entry(&self, name: &str) -> &Value {
        self.file.kits.get(name).unwrap_or(&Value::Null)
    }

    /// The looks of one prop kind in one biome, `prop/<biome>/<kind>/<n>`.
    pub fn prop_variants(&self, biome: &str, kind: &str) -> usize {
        (0..16).take_while(|v| self.kits.contains_key(&format!("prop/{biome}/{kind}/{v}"))).count()
    }

    pub fn f32s(&self, [at, n]: [usize; 2]) -> Vec<f32> {
        self.bin[at..at + n * 4].chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
    }

    pub fn u32s(&self, [at, n]: [usize; 2]) -> Vec<u32> {
        self.bin[at..at + n * 4].chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
    }

    /// The middle of a mesh's bounds, in its own space.
    pub fn mesh_centre(&self, mesh: usize) -> Option<Vec3> {
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for s in &self.file.meshes.get(mesh)?.surfaces {
            for p in self.f32s(s.positions).chunks_exact(3) {
                lo = lo.min(Vec3::from_slice(p));
                hi = hi.max(Vec3::from_slice(p));
            }
        }
        (lo.x <= hi.x).then(|| (lo + hi) * 0.5)
    }

    pub fn material(&self, i: usize) -> &Value {
        &self.file.materials[i]
    }
}

/// One surface's arrays, unpacked.
pub struct Arrays {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub uvs: Vec<[f32; 2]>,
    pub colors: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
}

impl Kits {
    pub fn arrays(&self, s: &SurfaceD) -> Arrays {
        let p = self.f32s(s.positions);
        let n = self.f32s(s.normals);
        let u = self.f32s(s.uvs);
        let c = self.f32s(s.colors);
        Arrays {
            positions: p.chunks_exact(3).map(Vec3::from_slice).collect(),
            normals: n.chunks_exact(3).map(Vec3::from_slice).collect(),
            uvs: u.chunks_exact(2).map(|c| [c[0], c[1]]).collect(),
            colors: c.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect(),
            indices: self.u32s(s.indices),
        }
    }
}

pub fn default_dir() -> PathBuf {
    // In a browser the page fetched the assets under this name (web.rs).
    if cfg!(target_arch = "wasm32") {
        return PathBuf::from(crate::web::KITS);
    }
    if let Some(d) = std::env::var_os("SPELLSTORM_KITS") {
        return PathBuf::from(d);
    }
    // Next to the executable (a packaged build), else in the source tree.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent().map(|d| d.join("kits")).filter(|d| d.join("kits.json").exists()) {
            return d;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/kits")
}
