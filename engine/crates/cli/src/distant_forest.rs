//! Bounded background forest over a continuous terrain extension. The live
//! scene reuses native tree meshes and measures native habitat density; the
//! original voxel crowns remain an explicit alternative geometry experiment.

use glam::{Affine3A, Quat, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::scene::{Heightfield, Material, Scene};
use sideways::builder::Chunk;
use sideways::stage::Stage;
use std::{cell::RefCell, collections::{BTreeMap, BTreeSet, HashMap}, time::Instant};

const TILE: f32 = 64.0;
const STEP: f32 = 8.0;
const CELLS: usize = 8;
const TREE_GRID: f32 = 4.0;
const TREE_CELLS: i32 = 16;
const MAX_BACKGROUND_TREES: usize = 200_000;

#[derive(Clone, Copy)]
pub struct Config { pub radius: f32, pub max_tiles: usize, pub max_trees: usize, pub max_height_samples: usize }
impl Default for Config {
    fn default() -> Self { Self { radius: 768.0, max_tiles: 2048, max_trees: MAX_BACKGROUND_TREES, max_height_samples: 8_000_000 } }
}

fn hash(x: i32, z: i32, seed: u32) -> u32 {
    let mut n = (x as u32).wrapping_mul(0x9e3779b9) ^ (z as u32).wrapping_mul(0x85ebca6b) ^ seed;
    n ^= n >> 16; n = n.wrapping_mul(0x7feb352d); n ^= n >> 15; n = n.wrapping_mul(0x846ca68b); n ^ (n >> 16)
}
fn unit(n: u32) -> f32 { (n >> 8) as f32 / 16_777_216.0 }
fn smooth(t: f32) -> f32 { let t = t.clamp(0.0, 1.0); t * t * (3.0 - 2.0 * t) }
fn noise(x: f32, z: f32, seed: u32) -> f32 {
    let ix = x.floor() as i32; let iz = z.floor() as i32; let fx = smooth(x - x.floor()); let fz = smooth(z - z.floor());
    let a = unit(hash(ix, iz, seed)); let b = unit(hash(ix + 1, iz, seed));
    let c = unit(hash(ix, iz + 1, seed)); let d = unit(hash(ix + 1, iz + 1, seed));
    ((a + (b - a) * fx) * (1.0 - fz) + (c + (d - c) * fx) * fz) * 2.0 - 1.0
}

/// Broad peaks must rise above the native corridor's 15-25 m canopy. The
/// earlier ~22 m addition was mostly hidden from a chase-height camera.
/// Large warped ridges, lower saddles and finer shoulders vary the skyline
/// without changing the road or creating a regular ring of equal-height hills.
fn skyline_relief(x: f32, z: f32, seed: u32) -> f32 {
    let wx = x + noise(x / 620.0, z / 620.0, seed ^ 0x19e377) * 105.0;
    let wz = z + noise(x / 570.0, z / 570.0, seed ^ 0x5aa13e) * 105.0;
    let ridge = 1.0 - noise(wx / 390.0, wz / 390.0, seed).abs();
    let saddle = smooth((noise(x / 710.0, z / 710.0, seed ^ 0x8c47) + 0.65) / 1.30);
    let peak = 70.0 + 125.0 * ridge * ridge;
    let shoulders = 27.0 * noise(wx / 155.0, wz / 155.0, seed ^ 0x51f15e)
        + 11.0 * noise(wx / 72.0, wz / 72.0, seed ^ 0x1ca23b);
    ((peak + shoulders) * (0.42 + 0.58 * saddle)).max(18.0)
}

struct Ground<'a> { stage: &'a Stage, native: HashMap<(i32, i32), &'a Chunk>, cache: RefCell<HashMap<(i32,i32),f32>> }
impl Ground<'_> {
    fn native_height(&self, x: f32, z: f32) -> Option<f32> {
        let cell = ((x / TILE).floor() as i32, (z / TILE).floor() as i32);
        for dz in [-1, 0] { for dx in [-1, 0] {
            let Some(c) = self.native.get(&(cell.0 + dx, cell.1 + dz)) else { continue; };
            let u = (x - c.x0 as f32) / 2.0; let v = (z - c.z0 as f32) / 2.0;
            if !(0.0..=32.0).contains(&u) || !(0.0..=32.0).contains(&v) { continue; }
            let i = (u.floor() as usize).min(31); let j = (v.floor() as usize).min(31);
            let a = c.heights[j * 33 + i]; let b = c.heights[j * 33 + i + 1];
            let cc = c.heights[(j + 1) * 33 + i]; let d = c.heights[(j + 1) * 33 + i + 1];
            let fx = u - i as f32; let fz = v - j as f32;
            return Some(if fx + fz <= 1.0 { a + (b - a) * fx + (cc - a) * fz } else { d + (cc - d) * (1.0 - fx) + (b - d) * (1.0 - fz) });
        }}
        None
    }
    fn border_distance(&self, x: f32, z: f32) -> f32 {
        let c = ((x / TILE).floor() as i32, (z / TILE).floor() as i32);
        let mut distance = 128.0f32;
        for dz in -3..=3 { for dx in -3..=3 {
            let key = (c.0 + dx, c.1 + dz);
            if self.native.contains_key(&key) {
                let x0 = key.0 as f32 * TILE; let z0 = key.1 as f32 * TILE;
                let ex = (x0 - x).max(0.0).max(x - x0 - TILE); let ez = (z0 - z).max(0.0).max(z - z0 - TILE);
                distance = distance.min((ex * ex + ez * ez).sqrt());
            }
        }}
        distance
    }
    fn height(&self, x: f32, z: f32) -> f32 {
        // Mesh vertices and derivative probes all lie on the 2 m lattice.
        // Cache their expensive terrain/river queries across shared tile edges.
        let key=((x*0.5).round() as i32,(z*0.5).round() as i32);
        if let Some(&h)=self.cache.borrow().get(&key){return h;}
        let h=self.uncached_height(x,z);self.cache.borrow_mut().insert(key,h);h
    }
    fn uncached_height(&self, x: f32, z: f32) -> f32 {
        if let Some(h) = self.native_height(x, z) { return h; }
        let blend = smooth(self.border_distance(x, z) / 128.0);
        let seed = self.stage.noise_seed as u32;
        let hills = skyline_relief(x, z, seed);
        self.stage.height(x as f64, z as f64) as f32 + hills * blend
    }
    fn vertex(&self, x: f32, z: f32) -> Vertex {
        let n = Vec3::new(self.height(x - 2.0, z) - self.height(x + 2.0, z), 4.0,
            self.height(x, z - 2.0) - self.height(x, z + 2.0)).normalize();
        Vertex { pos: [x, self.height(x, z), z], normal: n.to_array(), uv: [100.0, 0.0] }
    }
}

struct Patch { ring: Vec<Vec3>, centre: Vec3 }
impl Patch {
    fn height(&self, x: f32, z: f32) -> Option<f32> {
        self.surface(x, z).map(|(height, _)| height)
    }
    fn surface(&self, x: f32, z: f32) -> Option<(f32, f32)> {
        let a = self.centre;
        for i in 0..self.ring.len() {
            let b = self.ring[i]; let c = self.ring[(i + 1) % self.ring.len()];
            let det = (b.x - a.x) * (c.z - a.z) - (b.z - a.z) * (c.x - a.x);
            let u = ((x - a.x) * (c.z - a.z) - (z - a.z) * (c.x - a.x)) / det;
            let v = ((b.x - a.x) * (z - a.z) - (b.z - a.z) * (x - a.x)) / det;
            if u >= -1e-4 && v >= -1e-4 && u + v <= 1.0001 {
                let normal_y = (b - a).cross(c - a).normalize().y.abs();
                return Some((a.y + (b.y - a.y) * u + (c.y - a.y) * v, normal_y));
            }
        }
        None
    }
}
struct LandTile { key: (i32, i32), patches: Vec<Patch>, vertices: Vec<Vertex>, indices: Vec<u32> }
impl LandTile {
    fn new(key: (i32, i32), ground: &Ground<'_>) -> Self {
        let mut out = Self { key, patches: Vec::new(), vertices: Vec::new(), indices: Vec::new() };
        for j in 0..CELLS { for i in 0..CELLS {
            let x = key.0 as f32 * TILE + i as f32 * STEP; let z = key.1 as f32 * TILE + j as f32 * STEP;
            let corners = [[x,z], [x,z+STEP], [x+STEP,z+STEP], [x+STEP,z]];
            let seam = [i == 0 && ground.native.contains_key(&(key.0-1,key.1)),
                j == CELLS-1 && ground.native.contains_key(&(key.0,key.1+1)),
                i == CELLS-1 && ground.native.contains_key(&(key.0+1,key.1)),
                j == 0 && ground.native.contains_key(&(key.0,key.1-1))];
            let centre = ground.vertex(x + STEP * 0.5, z + STEP * 0.5);
            let base = out.vertices.len() as u32; out.vertices.push(centre);
            let mut ring = Vec::new();
            for edge in 0..4 {
                let count = if seam[edge] { 4 } else { 1 };
                for step in 0..count {
                    let t = step as f32 / count as f32;
                    let a = corners[edge]; let b = corners[(edge + 1) % 4];
                    let vertex = ground.vertex(a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t);
                    ring.push(Vec3::from(vertex.pos)); out.vertices.push(vertex);
                }
            }
            for k in 0..ring.len() as u32 { out.indices.extend([base, base + 1 + k, base + 1 + (k + 1) % ring.len() as u32]); }
            out.patches.push(Patch { ring, centre: Vec3::from(centre.pos) });
        }}
        out
    }
    fn height(&self, x: f32, z: f32) -> Option<f32> {
        let u = (x - self.key.0 as f32 * TILE) / STEP; let v = (z - self.key.1 as f32 * TILE) / STEP;
        if !(0.0..=CELLS as f32).contains(&u) || !(0.0..=CELLS as f32).contains(&v) { return None; }
        self.patches[(v.floor() as usize).min(CELLS - 1) * CELLS + (u.floor() as usize).min(CELLS - 1)].height(x,z)
    }
    fn surface(&self, x: f32, z: f32) -> Option<(f32, f32)> {
        let u = (x - self.key.0 as f32 * TILE) / STEP;
        let v = (z - self.key.1 as f32 * TILE) / STEP;
        if !(0.0..=CELLS as f32).contains(&u) || !(0.0..=CELLS as f32).contains(&v) { return None; }
        self.patches[(v.floor() as usize).min(CELLS - 1) * CELLS + (u.floor() as usize).min(CELLS - 1)].surface(x,z)
    }
}

/// Horizontal habitat area, not total map area or surface area of steep cliffs.
/// Match builder::terrain_chunk: normal.y >= .75, water at least .4 m below
/// ground, and uniformly sampled 4.5..9 m road clearance. Integrating that last
/// probability avoids inventing a hard clearance boundary for the comparison.
fn habitat_weight(road_distance: f32, normal_y: f32, height: f32, water: f32) -> f64 {
    if !normal_y.is_finite() || normal_y < 0.75 || water > height - 0.4 { return 0.0; }
    ((road_distance - 4.5) / 4.5).clamp(0.0, 1.0) as f64
}

#[derive(Clone, Copy, Default)]
struct NativeDensity { trees: usize, habitat_m2: f64 }
impl NativeDensity {
    fn per_m2(self) -> f64 {
        if self.habitat_m2 > 0.0 { self.trees as f64 / self.habitat_m2 } else { 0.0 }
    }
}

/// Count actual native placements, including their original noise/gap policy.
/// Only the denominator is sampled: 2 m horizontal cells, exactly the native
/// generation grid. Adjacent chunks provide derivative halo samples; at the
/// outer corridor boundary the unchanged Stage height supplies that halo.
fn native_density(chunks: &[Chunk], ground: &Ground<'_>) -> NativeDensity {
    let mut out = NativeDensity::default();
    for chunk in chunks {
        out.trees += chunk.trees.len();
        for j in 0..32 { for i in 0..32 {
            let x = chunk.x0 as f32 + i as f32 * 2.0;
            let z = chunk.z0 as f32 + j as f32 * 2.0;
            let k = j * 33 + i;
            let h = chunk.heights[k];
            let halo = |x: f32, z: f32| ground.native_height(x,z)
                .unwrap_or_else(|| ground.stage.height(x as f64,z as f64) as f32);
            let left = if i > 0 { chunk.heights[k-1] } else { halo(x-2.0,z) };
            let back = if j > 0 { chunk.heights[k-33] } else { halo(x,z-2.0) };
            let normal_y = Vec3::new(left-chunk.heights[k+1],4.0,back-chunk.heights[k+33]).normalize().y;
            let water = if ground.stage.rivers.is_empty() { f32::NEG_INFINITY }
                else { ground.stage.water_level(x as f64,z as f64) as f32 };
            out.habitat_m2 += 4.0 * habitat_weight(chunk.road_d[k],normal_y,h,water);
        }}
    }
    out
}

#[derive(Clone, Copy)]
struct TreePlacement { rank: f32, key: u32, x: f32, y: f32, z: f32 }

/// Weighted sampling without replacement retains broad native forest gaps,
/// while the count comes from measured near-tree density rather than another
/// hand-tuned far density. The world-cell key fixes placement across cameras.
fn forest_rank(key: u32, thin: f32) -> Option<f32> {
    if thin <= -0.25 { return None; }
    let weight = 0.28 + 0.25 * thin;
    Some(-unit(key).max(1.0 / 16_777_216.0).ln() / weight)
}

fn select_trees(trees: &mut Vec<TreePlacement>, native: NativeDensity, habitat_m2: f64, budget: usize) -> usize {
    let requested = (native.per_m2() * habitat_m2).round().max(0.0) as usize;
    trees.sort_unstable_by(|a,b| a.rank.total_cmp(&b.rank).then(a.key.cmp(&b.key))
        .then(a.x.total_cmp(&b.x)).then(a.z.total_cmp(&b.z)));
    trees.truncate(requested.min(budget).min(MAX_BACKGROUND_TREES));
    requested
}

fn background_cells(stage: &Stage, native: &HashMap<(i32,i32), &Chunk>, radius: f32) -> BTreeSet<(i32,i32)> {
    let mut cells = BTreeSet::new(); let reach = (radius / TILE).ceil() as i32;
    for p in stage.points.iter().step_by(32) {
        let centre = ((p.x / TILE).floor() as i32, (p.z / TILE).floor() as i32);
        for dz in -reach..=reach { for dx in -reach..=reach {
            let key = (centre.0 + dx, centre.1 + dz);
            if native.contains_key(&key) { continue; }
            let x = (key.0 as f32 + 0.5) * TILE - p.x; let z = (key.1 as f32 + 0.5) * TILE - p.z;
            if x*x+z*z <= radius*radius { cells.insert(key); }
        }}
    }
    cells
}

fn bounds(native: &HashMap<(i32,i32), &Chunk>, far: &BTreeSet<(i32,i32)>) -> ((i32,i32),(i32,i32),usize) {
    let mut lo = (i32::MAX,i32::MAX); let mut hi = (i32::MIN,i32::MIN);
    for &(x,z) in native.keys().chain(far.iter()) { lo.0=lo.0.min(x);lo.1=lo.1.min(z);hi.0=hi.0.max(x);hi.1=hi.1.max(z); }
    let w = (hi.0-lo.0+1) as usize * 32 + 1; let h = (hi.1-lo.1+1) as usize * 32 + 1;
    (lo,hi,w.saturating_mul(h))
}

fn extended_heightfield(old: &Heightfield, tiles: &BTreeMap<(i32,i32), LandTile>, lo:(i32,i32), hi:(i32,i32)) -> Heightfield {
    let w = (hi.0-lo.0+1) as u32 * 32 + 1; let h = (hi.1-lo.1+1) as u32 * 32 + 1;
    let origin = [lo.0 as f32*TILE,lo.1 as f32*TILE]; let mut heights = vec![-1e9; (w*h) as usize];
    for tile in tiles.values() {
        let ox = (tile.key.0-lo.0) as u32 * 32; let oz = (tile.key.1-lo.1) as u32 * 32;
        for j in 0..=32 { for i in 0..=32 {
            let x = tile.key.0 as f32*TILE+i as f32*2.0; let z = tile.key.1 as f32*TILE+j as f32*2.0;
            heights[((oz+j)*w+ox+i) as usize] = tile.height(x,z).expect("background tile surface covers its grid");
        }}
    }
    // Copy valid native samples LAST, bit-for-bit; no existing height is
    // resampled or coarsened and the reflection ground grid remains 2 metres.
    let ox = ((old.origin[0]-origin[0])/2.0).round() as u32; let oz = ((old.origin[1]-origin[1])/2.0).round() as u32;
    for j in 0..old.h { for i in 0..old.w {
        let value=old.heights[(j*old.w+i) as usize];
        if value > -1e8 { heights[((oz+j)*w+ox+i) as usize]=value; }
    }}
    Heightfield { origin, cell:2.0,w,h,heights }
}

/// Greedy rectangles on exposed voxel faces. Three variants share all their
/// geometry across instances. These are original opaque silhouette meshes,
/// not the separate alpha-card surface-density bake and not solidified cards.
fn tree_mesh(variant:u32, crown:u32, bark:u32) -> ClusterMesh {
    let dims=[15usize,27,15]; let cell=0.55f32; let seed=0xfee1deadu32^variant.wrapping_mul(7919);
    let mut vox=vec![0u8;dims.iter().product()];
    let at=|x:usize,y:usize,z:usize| (z*dims[1]+y)*dims[0]+x;
    for z in 0..dims[2] { for y in 0..dims[1] { for x in 0..dims[0] {
        let px=(x as f32+0.5-dims[0] as f32*0.5)*cell;
        let pz=(z as f32+0.5-dims[2] as f32*0.5)*cell;let py=(y as f32+0.5)*cell;
        let radius=(px*px+pz*pz*(0.88+variant as f32*0.1)).sqrt();
        let mut filled=false;
        for (base,top,r) in [(1.6,8.2,3.6),(5.0,11.2,2.9),(8.0,14.2,2.0)] {
            if py>=base && py<=top {
                let shape=r*((top-py)/(top-base)).powf(0.75);
                let irregular=0.89+unit(hash(x as i32,z as i32,seed^y as u32))*0.17;
                filled |= radius<shape*irregular;
            }
        }
        vox[at(x,y,z)]=if filled{1}else if py<4.0 && radius<0.39{2}else{0};
    }}}
    let mut verts=[Vec::<Vertex>::new(),Vec::<Vertex>::new()];let mut indices=[Vec::<u32>::new(),Vec::<u32>::new()];
    for axis in 0..3 { let u=(axis+1)%3;let v=(axis+2)%3;
        for sign in [-1i32,1] { for plane in 0..dims[axis] {
            let mut mask=vec![0u8;dims[u]*dims[v]];
            for row in 0..dims[v] { for col in 0..dims[u] {
                let mut p=[0usize;3];p[axis]=plane;p[u]=col;p[v]=row;
                let value=vox[at(p[0],p[1],p[2])];if value==0{continue;}
                let neighbor=plane as i32+sign;
                let blocked=if neighbor<0 || neighbor>=dims[axis] as i32{false}else{
                    p[axis]=neighbor as usize;vox[at(p[0],p[1],p[2])]!=0};
                if !blocked{mask[row*dims[u]+col]=value;}
            }}
            for row in 0..dims[v] { for col in 0..dims[u] {
                let value=mask[row*dims[u]+col];if value==0{continue;}
                let mut width=1;while col+width<dims[u] && mask[row*dims[u]+col+width]==value{width+=1;}
                let mut height=1;
                while row+height<dims[v] && (col..col+width).all(|x|mask[(row+height)*dims[u]+x]==value){height+=1;}
                for y in row..row+height{for x in col..col+width{mask[y*dims[u]+x]=0;}}
                let part=value as usize-1;let base=verts[part].len() as u32;
                let mut normal=Vec3::ZERO;normal[axis]=sign as f32;
                for (x,y) in [(col,row),(col+width,row),(col+width,row+height),(col,row+height)] {
                    let mut p=Vec3::ZERO;p[axis]=(plane+if sign>0{1}else{0}) as f32;p[u]=x as f32;p[v]=y as f32;
                    p=p*cell-Vec3::new(dims[0] as f32*cell*0.5,0.0,dims[2] as f32*cell*0.5);
                    verts[part].push(Vertex{pos:p.to_array(),normal:normal.to_array(),uv:[0.0,0.0]});
                }
                let order=if sign>0{[0,1,2,0,2,3]}else{[0,2,1,0,3,2]};indices[part].extend(order.map(|i|base+i));
            }}
        }}
    }
    ClusterMesh::build(&[Part{vertices:&verts[0],indices:&indices[0],material:crown},Part{vertices:&verts[1],indices:&indices[1],material:bark}])
}

pub fn add(scene:&mut Scene,stage:&Stage,chunks:&[Chunk],terrain_material:u32,source_tree_meshes:&[usize],mesh_names:&mut Vec<String>,config:Config)->Result<String,String>{
    let started=Instant::now();
    if chunks.is_empty() || stage.points.is_empty(){return Err("no native terrain to attach background to".into());}
    if (scene.heightfield.cell-2.0).abs()>1e-6{return Err("background requires the native 2 m heightfield".into());}
    let ground=Ground{stage,native:chunks.iter().map(|c|(c.cell,c)).collect(),cache:RefCell::new(HashMap::new())};
    let mut radius=config.radius.clamp(384.0,896.0);
    let (cells,lo,hi)=loop{
        let cells=background_cells(stage,&ground.native,radius);let(lo,hi,samples)=bounds(&ground.native,&cells);
        if cells.len()<=config.max_tiles && samples<=config.max_height_samples{break(cells,lo,hi);}
        if radius<=384.0{return Err(format!("background budget exceeded even at 384 m: {} tiles, {samples} height samples; native terrain retained",cells.len()));}
        radius=(radius-128.0).max(384.0);
    };
    let tiles:BTreeMap<_,_>=cells.into_iter().map(|key|(key,LandTile::new(key,&ground))).collect();
    let hf=extended_heightfield(&scene.heightfield,&tiles,lo,hi);
    let terrain_triangles:usize=tiles.values().map(|t|t.indices.len()/3).sum();
    for tile in tiles.values(){
        let mesh=ClusterMesh::build(&[Part{vertices:&tile.vertices,indices:&tile.indices,material:terrain_material}]);
        let id=scene.add_mesh(&mesh);scene.add_instance(id,Affine3A::IDENTITY);mesh_names.push(format!("vista terrain {:?}",tile.key));
    }
    scene.heightfield=hf;
    let mut tree_ids=Vec::new();let mut tree_triangles=0;let mut tree_roots=0;let mut tree_lods=Vec::new();
    // Preserve species, crown height and lighting in the live game. The
    // original generic voxel pines remain an explicit geometry experiment;
    // a future voxel LOD must be derived from these same source trees.
    let generic_voxels=std::env::var("SIDEWAYS_GENERIC_VOXEL_TREES").as_deref()==Ok("1");
    if !generic_voxels && !source_tree_meshes.is_empty() {
        tree_ids.extend_from_slice(source_tree_meshes);
    } else {
      let bark=scene.add_material(Material{tint:[0.07,0.047,0.025],layer:-1,roughness:0.98,flags:0,scale:1.0,metal:0.0});
      for variant in 0..3{
        let tint=[[0.034,0.071,0.029],[0.043,0.080,0.035],[0.029,0.060,0.034]][variant as usize];
        let material=scene.add_material(Material{tint,layer:-1,roughness:0.98,flags:0,scale:1.0,metal:0.0});
        let mesh=tree_mesh(variant,material,bark);let leaf=mesh.leaf_triangles();let root=mesh.root_triangles();
        tree_triangles+=leaf;tree_roots+=root;
        tree_lods.push(format!("v{variant}: {leaf}->{root} tris, {} clusters, {} levels",mesh.clusters.len(),mesh.levels));
        tree_ids.push(scene.add_mesh(&mesh));mesh_names.push(format!("original voxel pine {variant}"));
      }
    }
    let native=native_density(chunks,&ground);
    let seed=stage.noise_seed as u32 ^ 0xd157a47;let mut trees=Vec::new();
    let mut habitat_m2=0.0;let mut steep_m2=0.0;let mut wet_m2=0.0;
    for tile in tiles.values(){for j in 0..TREE_CELLS{for i in 0..TREE_CELLS{
        let gx=tile.key.0*TREE_CELLS+i;let gz=tile.key.1*TREE_CELLS+j;let key=hash(gx,gz,seed);
        let x=(gx as f32+0.05+unit(key)*0.9)*TREE_GRID;
        let z=(gz as f32+0.05+unit(key.rotate_left(13))*0.9)*TREE_GRID;
        let (y,normal_y)=tile.surface(x,z).expect("tree root lies on its background tile");
        let water=if stage.rivers.is_empty(){f32::NEG_INFINITY}else{stage.water_level(x as f64,z as f64) as f32};
        // All background tiles are beyond the full native road corridor, so
        // road clearance is already satisfied. Use the actual raised mesh
        // for both slope eligibility and roots, never the old Stage height.
        let habitat=habitat_weight(f32::INFINITY,normal_y,y,water);
        habitat_m2+=(TREE_GRID*TREE_GRID) as f64*habitat;
        if normal_y<0.75 || !normal_y.is_finite(){steep_m2+=(TREE_GRID*TREE_GRID) as f64;}
        else if water>y-0.4{wet_m2+=(TREE_GRID*TREE_GRID) as f64;}
        if habitat==0.0{continue;}
        let thin=stage.detail.get_2d(x*0.6,z*0.6);
        let Some(rank)=forest_rank(hash(gx,gz,seed^0xa1ce),thin) else{continue;};
        trees.push(TreePlacement{rank,key,x,y,z});
    }}}
    let candidates=trees.len();let budget=config.max_trees.min(MAX_BACKGROUND_TREES);
    let requested=select_trees(&mut trees,native,habitat_m2,budget);
    for &TreePlacement{x,y,z,key,..} in &trees{
        let wide=0.85+unit(key.rotate_left(7))*0.60;let tall=0.85+unit(key.rotate_left(19))*0.55;
        let transform=Affine3A::from_scale_rotation_translation(Vec3::new(wide,tall,wide),Quat::from_rotation_y(unit(key.rotate_left(3))*std::f32::consts::TAU),Vec3::new(x,y-0.12,z));
        scene.add_instance(tree_ids[key as usize%tree_ids.len()],transform);scene.mark_foliage();
    }
    let representation=if !generic_voxels && !source_tree_meshes.is_empty(){format!("matching source trees / {} reused near-tree cluster LOD meshes",tree_ids.len())}else{format!("experimental generic voxel trees / 3 shared meshes ({tree_triangles} leaf / {tree_roots} root tris total; {})",tree_lods.join("; "))};
    let far_density=if habitat_m2>0.0{trees.len() as f64/habitat_m2}else{0.0};
    let parity=if trees.len()==requested{"matched count"}else if requested>budget{"LIMITED BY INSTANCE BUDGET"}else{"LIMITED BY FOREST CANDIDATES"};
    let total_m2=tiles.len() as f64*(TILE*TILE) as f64;
    let eligible_percent=if total_m2>0.0{100.0*habitat_m2/total_m2}else{0.0};
    Ok(format!("distant forest: {} {representation}, {} terrain tiles / {terrain_triangles} tris, radius {radius:.0} m, high warped skyline ridges, 2 m heightfield {} samples, {:.0} ms; habitat density native {:.1} trees/ha ({} actual trees / {:.0} horizontal m2 at 2 m sampling), far {:.1} trees/ha ({habitat_m2:.0} horizontal m2 at 4 m sampling; {eligible_percent:.1}% eligible, {steep_m2:.0} m2 steep and {wet_m2:.0} m2 wet excluded), requested {requested} / candidates {candidates} / instance budget {budget}: {parity}; background has no new driving collision",trees.len(),tiles.len(),scene.heightfield.heights.len(),started.elapsed().as_secs_f64()*1000.0,native.per_m2()*10_000.0,native.trees,native.habitat_m2,far_density*10_000.0))
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn habitat_matches_native_clearance_slope_and_water_rules() {
        let dry=f32::NEG_INFINITY;
        assert_eq!(habitat_weight(4.5,1.0,10.0,dry),0.0);
        assert_eq!(habitat_weight(6.75,1.0,10.0,dry),0.5);
        assert_eq!(habitat_weight(9.0,0.75,10.0,dry),1.0);
        assert_eq!(habitat_weight(100.0,0.749,10.0,dry),0.0);
        assert_eq!(habitat_weight(100.0,1.0,10.0,9.7),0.0);
        assert_eq!(habitat_weight(100.0,1.0,10.0,9.5),1.0);
        assert!(forest_rank(100,-0.25).is_none());
        assert!(forest_rank(100,0.0).unwrap().is_finite());
    }
    #[test]
    fn native_calibration_counts_actual_trees_per_eligible_horizontal_area() {
        use sideways::builder::{Spot,V3Ser};
        let mut stage=Stage::generate(&mut gdcompat::Pcg::new(1),10.0);
        stage.rivers.clear();
        let mut chunks=Vec::new();
        for z in -1..=1 {for x in -1..=1 {
            chunks.push(Chunk{cell:(x,z),x0:x as f64*64.0,z0:z as f64*64.0,
                heights:vec![20.0;33*33],road_d:vec![6.75;33*33],
                trees:vec![Spot{pos:V3Ser(2.0,20.0,2.0),yaw:0.0,scale:1.0,scale_y:1.0};64],
                rocks:vec![],grass:vec![],ferns:vec![],stumps:vec![]});
        }}
        let ground=Ground{stage:&stage,native:chunks.iter().map(|c|(c.cell,c)).collect(),cache:RefCell::new(HashMap::new())};
        let measured=native_density(&chunks[4..5],&ground);
        assert_eq!(measured.trees,64);
        assert_eq!(measured.habitat_m2,2048.0,"half of a flat 4096 m2 tile clears the native random road offset");
        assert_eq!(measured.per_m2(),1.0/32.0);
    }
    #[test]
    fn far_count_matches_measured_habitat_density_and_honors_budget() {
        let native=NativeDensity{trees:64,habitat_m2:2048.0};
        let candidates:Vec<_>=(0..2560).map(|i|{
            let key=hash(i,0,71);
            TreePlacement{rank:forest_rank(key,0.2).unwrap(),key,x:i as f32,y:1.0,z:2.0}
        }).collect();
        let mut full=candidates.clone();
        let requested=select_trees(&mut full,native,16.0*2560.0,200_000);
        assert_eq!((requested,full.len()),(1280,1280));
        let mut limited=candidates.clone();
        assert_eq!(select_trees(&mut limited,native,16.0*2560.0,100),1280);
        assert_eq!(limited.len(),100);
        assert!(limited.iter().zip(&full).all(|(a,b)|a.key==b.key));
        let mut repeated=candidates;
        select_trees(&mut repeated,native,16.0*2560.0,200_000);
        assert!(full.iter().zip(&repeated).all(|(a,b)|a.key==b.key));
    }
    #[test]
    fn far_habitat_uses_raised_mesh_slope_and_exact_root_height() {
        let patch=Patch{centre:Vec3::new(4.0,102.0,4.0),ring:vec![
            Vec3::new(0.0,100.0,0.0),Vec3::new(0.0,104.0,8.0),
            Vec3::new(8.0,104.0,8.0),Vec3::new(8.0,100.0,0.0)]};
        let (height,normal_y)=patch.surface(3.2,6.7).unwrap();
        assert!((height-103.35).abs()<1e-4);
        assert!((normal_y-1.0/1.25f32.sqrt()).abs()<1e-5);
        assert_eq!(habitat_weight(f32::INFINITY,normal_y,height,100.0),1.0);
    }
    #[test]
    fn skyline_has_tall_peaks_and_saddles_without_camera_dependent_shape() {
        let mut lo = f32::INFINITY;
        let mut hi = 0.0f32;
        for z in -12..=12 { for x in -12..=12 {
            let h = skyline_relief(x as f32 * 97.0, z as f32 * 97.0, 7919);
            assert!(h.is_finite() && (18.0..=233.0).contains(&h));
            assert_eq!(h.to_bits(), skyline_relief(x as f32 * 97.0, z as f32 * 97.0, 7919).to_bits());
            lo = lo.min(h); hi = hi.max(h);
        }}
        assert!(hi > 140.0 && hi - lo > 90.0, "the skyline needs distinct high ridges and low saddles: {lo}..{hi}");
    }
    #[test]
    fn exact_patch_height_is_shared_by_roots_and_heightfield(){
        let patch=Patch{centre:Vec3::new(4.0,3.0,4.0),ring:vec![Vec3::new(0.0,1.0,0.0),Vec3::new(0.0,5.0,8.0),Vec3::new(8.0,5.0,8.0),Vec3::new(8.0,1.0,0.0)]};
        for z in [0.0,2.0,4.0,7.5,8.0]{for x in [0.0,1.0,4.0,7.7,8.0]{assert!((patch.height(x,z).unwrap()-(1.0+z*0.5)).abs()<1e-5);}}
    }
    #[test]
    fn extension_preserves_native_f32_bits_and_two_metre_spacing(){
        let plane=|x:f32,z:f32|Vec3::new(x,10.0+x*0.1+z*0.25,z);
        let mut tiles=BTreeMap::new();
        for key in [(-1,0),(0,0)]{
            let mut patches=Vec::new();
            for j in 0..CELLS{for i in 0..CELLS{
                let x=key.0 as f32*TILE+i as f32*STEP;let z=j as f32*STEP;
                patches.push(Patch{centre:plane(x+4.0,z+4.0),ring:vec![plane(x,z),plane(x,z+8.0),plane(x+8.0,z+8.0),plane(x+8.0,z)]});
            }}
            tiles.insert(key,LandTile{key,patches,vertices:vec![],indices:vec![]});
        }
        // Include negative zero and an adjacent floating-point value: an
        // apparently equal resampling is insufficient; preservation is exact.
        let original=Heightfield{origin:[2.0,4.0],cell:2.0,w:3,h:3,
            heights:vec![-0.0,f32::from_bits(0x3f800001),2.0,3.0,-1e9,5.0,6.0,7.0,8.0]};
        let extended=extended_heightfield(&original,&tiles,(-1,0),(0,0));
        assert_eq!(extended.cell.to_bits(),original.cell.to_bits());
        assert_eq!(extended.origin,[-64.0,0.0]);assert_eq!((extended.w,extended.h),(65,33));
        for j in 0..original.h{for i in 0..original.w{
            let old=original.heights[(j*original.w+i) as usize];
            let value=extended.heights[((j+2)*extended.w+i+33) as usize];
            if old > -1e8{assert_eq!(old.to_bits(),value.to_bits(),"native sample changed at {i},{j}");}
            else{assert!((value-plane(4.0,6.0).y).abs()<1e-5,"native empty space should receive background height");}
        }}
        let far=extended.heights[(extended.w+1) as usize];
        assert!((far-plane(-62.0,2.0).y).abs()<1e-5);
    }
    #[test]
    fn original_voxel_tree_has_bounded_geometry_and_outward_face_winding(){
        for variant in 0..3{
            let mesh=tree_mesh(variant,0,1);assert!(mesh.leaf_triangles()>50 && mesh.leaf_triangles()<6000);
            assert!(mesh.root_triangles()*2<mesh.leaf_triangles(),
                "voxel pine {variant} did not simplify below half: {} leaf, {} root triangles, {} levels",
                mesh.leaf_triangles(),mesh.root_triangles(),mesh.levels);
            for cluster in mesh.clusters.iter().filter(|c|c.own_error==0.0){for packed in &mesh.tris[cluster.first_tri as usize..(cluster.first_tri+cluster.tri_count) as usize]{
                let v:[Vertex;3]=std::array::from_fn(|k|mesh.vertices[mesh.cluster_verts[(cluster.first_vert+((packed>>(8*k))&255)) as usize] as usize]);
                let p=v.map(|v|Vec3::from(v.pos));let face=(p[1]-p[0]).cross(p[2]-p[0]);
                assert!(face.dot(Vec3::from(v[0].normal))>0.0);
            }}
        }
    }
}
