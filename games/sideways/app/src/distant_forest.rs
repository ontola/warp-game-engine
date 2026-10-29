//! Bounded background forest over a continuous terrain extension. The live
//! scene reuses native tree meshes and measures native habitat density; the
//! original voxel crowns remain an explicit alternative geometry experiment.

use glam::{Affine3A, Quat, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::scene::{GeometryCategory, Heightfield, Material, Scene};
use sideways::builder::Chunk;
use sideways::stage::Stage;
use std::{cell::RefCell, collections::{BTreeMap, BTreeSet, HashMap}, time::Instant};

const TILE: f32 = 64.0;
const STEP: f32 = 8.0;
#[cfg(test)]
const CELLS: usize = 8;
const OUTER_STEP: f32 = 16.0;
const INNER_RADIUS: f32 = 768.0;
const MAX_RADIUS: f32 = 1152.0;
const MAX_BACKGROUND_TILES: usize = 3072;
const MAX_HEIGHT_SAMPLES: usize = 8_000_000;
const TREE_GRID: f32 = 4.0;
const TREE_CELLS: i32 = 16;
const MAX_BACKGROUND_TREES: usize = 320_000;
// The former 128 m ramp could raise a 200 m peak with >2:1 grade right at
// the native border, excluding trees even when the underlying land was flat.
// Preserve peak elevation, but give it an actual broad mountain foot.
const FOOTHILL_WIDTH: f32 = 512.0;

#[derive(Clone, Copy)]
pub struct Config { pub radius: f32, pub max_tiles: usize, pub max_trees: usize, pub max_height_samples: usize }
impl Default for Config {
    fn default() -> Self { Self { radius: MAX_RADIUS, max_tiles: MAX_BACKGROUND_TILES, max_trees: MAX_BACKGROUND_TREES, max_height_samples: MAX_HEIGHT_SAMPLES } }
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
/// Rounded, broad ridges keep elevation without the derivative cusp of
/// abs(noise), or short steep shoulders that dominate an 8/16 m source mesh.
/// Their footprint is independent of the camera and loaded forest extent.
fn skyline_relief(x: f32, z: f32, seed: u32) -> f32 {
    let wx = x + noise(x / 620.0, z / 620.0, seed ^ 0x19e377) * 105.0;
    let wz = z + noise(x / 570.0, z / 570.0, seed ^ 0x5aa13e) * 105.0;
    let n = noise(wx / 640.0, wz / 640.0, seed);
    let crest_rounding = 0.18f32;
    let denominator = (1.0 + crest_rounding * crest_rounding).sqrt() - crest_rounding;
    let ridge = ((1.0 + crest_rounding * crest_rounding).sqrt()
        - (n * n + crest_rounding * crest_rounding).sqrt()) / denominator;
    let saddle = smooth((noise(x / 710.0, z / 710.0, seed ^ 0x8c47) + 0.65) / 1.30);
    let peak = 70.0 + 125.0 * ridge * ridge;
    let shoulders = 22.0 * noise(wx / 240.0, wz / 240.0, seed ^ 0x51f15e)
        + 7.0 * noise(wx / 128.0, wz / 128.0, seed ^ 0x1ca23b);
    ((peak + shoulders) * (0.42 + 0.58 * saddle)).max(18.0)
}

struct Ground<'a> {
    stage: &'a Stage,
    native: HashMap<(i32, i32), &'a Chunk>,
    cache: RefCell<HashMap<(i32,i32),f32>>,
    // Search the expanded foot once per 64 m tile, not at every vertex.
    // Store only actual native AABBs. The exact point-to-union distance keeps
    // shared edges continuous and the native border bit-for-bit unchanged.
    border_cache: RefCell<HashMap<(i32,i32),Vec<[f32;2]>>>,
}
impl<'a> Ground<'a> {
    fn new(stage: &'a Stage, native: HashMap<(i32,i32), &'a Chunk>) -> Self {
        Self {stage,native,cache:RefCell::new(HashMap::new()),border_cache:RefCell::new(HashMap::new())}
    }
}
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
        let mut cached = self.border_cache.borrow_mut();
        let nearby = cached.entry(c).or_insert_with(|| {
            let mut result = Vec::new();
            let reach = (FOOTHILL_WIDTH / TILE).ceil() as i32 + 1;
            for dz in -reach..=reach { for dx in -reach..=reach {
                let key = (c.0 + dx, c.1 + dz);
                if self.native.contains_key(&key) {
                    result.push([key.0 as f32 * TILE,key.1 as f32 * TILE]);
                }
            }}
            result
        });
        let mut squared = FOOTHILL_WIDTH * FOOTHILL_WIDTH;
        for &[x0,z0] in nearby.iter() {
            let ex = (x0-x).max(0.0).max(x-x0-TILE);
            let ez = (z0-z).max(0.0).max(z-z0-TILE);
            squared = squared.min(ex*ex+ez*ez);
        }
        squared.sqrt()
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
        let blend = smooth(self.border_distance(x, z) / FOOTHILL_WIDTH);
        let seed = self.stage.noise_seed as u32;
        let hills = skyline_relief(x, z, seed);
        self.stage.height(x as f64, z as f64) as f32 + hills * blend
    }
    fn vertex(&self, x: f32, z: f32) -> Vertex {
        let n = Vec3::new(self.height(x - 2.0, z) - self.height(x + 2.0, z), 4.0,
            self.height(x, z - 2.0) - self.height(x, z + 2.0)).normalize();
        Vertex { pos: [x, self.height(x, z), z], normal: n.to_array(), uv: [100.0, 1.0+n.y] }
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
struct LandTile { key: (i32, i32), step: f32, patches: Vec<Patch>, vertices: Vec<Vertex>, indices: Vec<u32> }
impl LandTile {
    fn new(key: (i32, i32), ground: &Ground<'_>, steps: &BTreeMap<(i32,i32),f32>) -> Self {
        let step=steps[&key];let cells=(TILE/step) as usize;
        let mut out = Self { key, step, patches: Vec::new(), vertices: Vec::new(), indices: Vec::new() };
        for j in 0..cells { for i in 0..cells {
            let x = key.0 as f32 * TILE + i as f32 * step; let z = key.1 as f32 * TILE + j as f32 * step;
            let corners = [[x,z], [x,z+step], [x+step,z+step], [x+step,z]];
            let neighbours=[(key.0-1,key.1),(key.0,key.1+1),(key.0+1,key.1),(key.0,key.1-1)];
            let border=[i==0,j==cells-1,i==cells-1,j==0];
            let centre = ground.vertex(x + step * 0.5, z + step * 0.5);
            let base = out.vertices.len() as u32; out.vertices.push(centre);
            let mut ring = Vec::new();
            for edge in 0..4 {
                // Coarse/fine borders use the finer neighbour's actual edge
                // vertices. Native edges retain every original 2 m sample.
                let neighbour_step=if ground.native.contains_key(&neighbours[edge]){2.0}
                    else{steps.get(&neighbours[edge]).copied().unwrap_or(step)};
                let count=if border[edge]{(step/step.min(neighbour_step)) as usize}else{1};
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
        self.surface(x,z).map(|(height,_)|height)
    }
    fn surface(&self, x: f32, z: f32) -> Option<(f32, f32)> {
        let cells=(TILE/self.step) as usize;
        let u = (x - self.key.0 as f32 * TILE) / self.step;
        let v = (z - self.key.1 as f32 * TILE) / self.step;
        if !(0.0..=cells as f32).contains(&u) || !(0.0..=cells as f32).contains(&v) { return None; }
        self.patches[(v.floor() as usize).min(cells - 1) * cells + (u.floor() as usize).min(cells - 1)].surface(x,z)
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
struct NativeDensity { trees: usize, habitat_m2: f64, steep_m2: f64 }
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
            if normal_y < 0.75 || !normal_y.is_finite() { out.steep_m2 += 4.0; }
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

/// Resolution is fixed in world space, never switched under a moving camera.
/// Every tile in the previous 768 m extent retains its 8 m source mesh.
fn terrain_steps(stage:&Stage,cells:&BTreeSet<(i32,i32)>)->BTreeMap<(i32,i32),f32> {
    cells.iter().map(|&key|{
        let x=(key.0 as f32+0.5)*TILE;let z=(key.1 as f32+0.5)*TILE;
        let inner=stage.points.iter().step_by(32).any(|p|(p.x-x).powi(2)+(p.z-z).powi(2)<=INNER_RADIUS*INNER_RADIUS);
        (key,if inner{STEP}else{OUTER_STEP})
    }).collect()
}

fn bounds(native: &HashMap<(i32,i32), &Chunk>, far: &BTreeSet<(i32,i32)>) -> ((i32,i32),(i32,i32),usize) {
    let mut lo = (i32::MAX,i32::MAX); let mut hi = (i32::MIN,i32::MIN);
    for &(x,z) in native.keys().chain(far.iter()) { lo.0=lo.0.min(x);lo.1=lo.1.min(z);hi.0=hi.0.max(x);hi.1=hi.1.max(z); }
    let w = (hi.0-lo.0+1) as usize * 32 + 1; let h = (hi.1-lo.1+1) as usize * 32 + 1;
    (lo,hi,w.saturating_mul(h))
}

struct Extent {cells:BTreeSet<(i32,i32)>,lo:(i32,i32),hi:(i32,i32),radius:f32}

fn choose_extent(stage:&Stage,native:&HashMap<(i32,i32),&Chunk>,density:NativeDensity,config:Config)->Result<Extent,String> {
    if !config.radius.is_finite(){return Err("background radius must be finite".into());}
    let mut radius=config.radius.clamp(384.0,MAX_RADIUS);
    let tree_budget=config.max_trees.min(MAX_BACKGROUND_TREES);
    loop {
        let cells=background_cells(stage,native,radius);let(lo,hi,samples)=bounds(native,&cells);
        // Conservatively assume every outer square metre is habitat. Reducing
        // the extent is preferable to thinning every distant forest when a
        // long/wide stage reaches the instance budget. Actual slope/water
        // exclusions can only reduce the measured requested tree count.
        let trees_upper=(density.per_m2()*cells.len() as f64*(TILE*TILE) as f64).ceil() as usize;
        if cells.len()<=config.max_tiles.min(MAX_BACKGROUND_TILES)
            && samples<=config.max_height_samples.min(MAX_HEIGHT_SAMPLES)
            && trees_upper<=tree_budget {return Ok(Extent{cells,lo,hi,radius});}
        if radius<=384.0{return Err(format!("background budget exceeded even at 384 m: {} tiles, {samples} height samples, up to {trees_upper} density-matched trees / budget {tree_budget}; native terrain retained",cells.len()));}
        radius=(radius-128.0).max(384.0);
    }
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

pub fn add(scene:&mut Scene,stage:&Stage,chunks:&[Chunk],terrain_material:u32,source_tree_meshes:&[usize],mesh_names:&mut Vec<String>,config:Config,mixed_species:bool)->Result<String,String>{
    let started=Instant::now();
    if mixed_species && (source_tree_meshes.len()!=crate::plant_population::TREE_IDS.len()
        || std::env::var("SIDEWAYS_GENERIC_VOXEL_TREES").as_deref()==Ok("1")) {
        return Err("mixed distant forest requires the four matching modular tree sources".into());
    }
    if chunks.is_empty() || stage.points.is_empty(){return Err("no native terrain to attach background to".into());}
    if (scene.heightfield.cell-2.0).abs()>1e-6{return Err("background requires the native 2 m heightfield".into());}
    let ground=Ground::new(stage,chunks.iter().map(|c|(c.cell,c)).collect());
    let native=native_density(chunks,&ground);
    let Extent{cells,lo,hi,radius}=choose_extent(stage,&ground.native,native,config)?;
    let steps=terrain_steps(stage,&cells);
    let tiles:BTreeMap<_,_>=cells.into_iter().map(|key|(key,LandTile::new(key,&ground,&steps))).collect();
    let hf=extended_heightfield(&scene.heightfield,&tiles,lo,hi);
    let terrain_triangles:usize=tiles.values().map(|t|t.indices.len()/3).sum();
    for tile in tiles.values(){
        let mesh=ClusterMesh::build_with_policy(&[Part{vertices:&tile.vertices,indices:&tile.indices,material:terrain_material}],
            |_|render::cluster::SimplificationPolicy::Terrain);
        let id=scene.add_mesh(&mesh);scene.add_classified_instance(id,Affine3A::IDENTITY,GeometryCategory::Terrain);mesh_names.push(format!("vista terrain {:?}",tile.key));
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
        let base=[[0.034,0.071,0.029],[0.043,0.080,0.035],[0.029,0.060,0.034]][variant as usize];
        let tint=std::array::from_fn(|i|base[i]*stage.vegetation.pine_tint()[i]);
        let material=scene.add_material(Material{tint,layer:-1,roughness:0.98,flags:0,scale:1.0,metal:0.0});
        let mesh=tree_mesh(variant,material,bark);let leaf=mesh.leaf_triangles();let root=mesh.root_triangles();
        tree_triangles+=leaf;tree_roots+=root;
        tree_lods.push(format!("v{variant}: {leaf}->{root} tris, {} clusters, {} levels",mesh.clusters.len(),mesh.levels));
        tree_ids.push(scene.add_mesh(&mesh));mesh_names.push(format!("original voxel pine {variant}"));
      }
    }
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
    let mut families=[0usize;3];let mut juveniles=0usize;
    for &TreePlacement{x,y,z,key,..} in &trees{
        let phenotype=stage.vegetation.tree(stage.noise_seed as u64,x,z);
        let scale=crate::tree_assets::scale_from_units(unit(key.rotate_left(7)),unit(key.rotate_left(19)),phenotype);
        let transform=Affine3A::from_scale_rotation_translation(scale,Quat::from_rotation_y(unit(key.rotate_left(3))*std::f32::consts::TAU),Vec3::new(x,y-0.12,z));
        let family=phenotype.archetype.index();
        let species=if mixed_species {crate::plant_population::stage_index(stage,x,z)}else{family};
        scene.add_classified_instance(tree_ids[species%tree_ids.len()],transform,GeometryCategory::Trees);scene.mark_foliage();
        families[family]+=1;juveniles+=usize::from(phenotype.juvenile);
    }
    let representation=if mixed_species {format!("matching modular species / 4 reused near-tree CLOD meshes with shared world-grove selection; {juveniles} juveniles")}
        else if !generic_voxels && !source_tree_meshes.is_empty(){format!("matching source trees / {} reused near-tree cluster LOD meshes, {} canopy / {} tapered conifer / {} stout trunk, {juveniles} juveniles",tree_ids.len(),families[0],families[1],families[2])}else{format!("experimental generic voxel trees / 3 shared meshes ({tree_triangles} leaf / {tree_roots} root tris total; {})",tree_lods.join("; "))};
    let far_density=if habitat_m2>0.0{trees.len() as f64/habitat_m2}else{0.0};
    let parity=if trees.len()==requested{"matched count"}else if requested>budget{"LIMITED BY INSTANCE BUDGET"}else{"LIMITED BY FOREST CANDIDATES"};
    let total_m2=tiles.len() as f64*(TILE*TILE) as f64;
    let eligible_percent=if total_m2>0.0{100.0*habitat_m2/total_m2}else{0.0};
    let outer_tiles=tiles.values().filter(|tile|tile.step==OUTER_STEP).count();
    let native_steep_percent=100.0*native.steep_m2/(chunks.len() as f64*(TILE*TILE) as f64);
    let instance_bytes=(trees.len()+tiles.len())*std::mem::size_of::<render::scene::GpuInstance>();
    let height_bytes=scene.heightfield.heights.len()*std::mem::size_of::<f32>();
    Ok(format!("distant forest: {} {representation}, {} terrain tiles / {terrain_triangles} tris ({outer_tiles} outer tiles at 16 m, inner terrain 8 m, stitched native edges 2 m), radius {radius:.0} m / requested {:.0} m, high rounded skyline / {FOOTHILL_WIDTH:.0} m foothills, seed {}, 2 m heightfield {} samples / {height_bytes} bytes (hard cap {}), background instance payload {instance_bytes} bytes (excludes renderer copies/CLOD traversal; extra trees remain shadow casters), {:.0} ms; habitat density native {:.1} trees/ha ({} actual trees / {:.0} horizontal m2 at 2 m sampling; {native_steep_percent:.1}% steep), far {:.1} trees/ha ({habitat_m2:.0} horizontal m2 at 4 m sampling; {eligible_percent:.1}% eligible, {steep_m2:.0} m2 steep and {wet_m2:.0} m2 wet excluded), requested {requested} / candidates {candidates} / instance budget {budget}: {parity}; background has no new driving collision",trees.len(),tiles.len(),config.radius,stage.noise_seed,scene.heightfield.heights.len(),MAX_HEIGHT_SAMPLES*4,started.elapsed().as_secs_f64()*1000.0,native.per_m2()*10_000.0,native.trees,native.habitat_m2,far_density*10_000.0))
}

#[cfg(test)]
mod tests{
    use super::*;
    fn flat_chunk(cell:(i32,i32))->Chunk {
        Chunk {cell,x0:cell.0 as f64*64.0,z0:cell.1 as f64*64.0,
            heights:vec![20.0;33*33],road_d:vec![100.0;33*33],trees:vec![],
            rocks:vec![],grass:vec![],ferns:vec![],stumps:vec![]}
    }
    #[test]
    fn widened_foothill_distance_matches_exact_native_union_across_tile_edges() {
        let stage=Stage::generate(&mut gdcompat::Pcg::new(7),10.0);
        let chunks=[flat_chunk((-4,-2)),flat_chunk((0,0)),flat_chunk((3,2))];
        let ground=Ground::new(&stage,chunks.iter().map(|c|(c.cell,c)).collect());
        for z in -40..=40 { for x in -40..=40 {
            let px=x as f32*16.0;let pz=z as f32*16.0;
            let expected=chunks.iter().map(|c| {
                let dx=(c.x0 as f32-px).max(0.0).max(px-c.x0 as f32-TILE);
                let dz=(c.z0 as f32-pz).max(0.0).max(pz-c.z0 as f32-TILE);
                (dx*dx+dz*dz).sqrt()
            }).fold(FOOTHILL_WIDTH,f32::min);
            assert_eq!(ground.border_distance(px,pz).to_bits(),expected.to_bits());
        }}
        assert!(ground.border_cache.borrow().values().all(|v|v.len()<=chunks.len()));
    }

    /// Actual captured first stage, without textures, renderer or physics.
    /// Evaluate both profiles on the *same* emitted fan triangles and fixed
    /// tree candidates, so a change in count cannot hide a slope-policy change.
    #[test]
    fn captured_stage_rolling_mountains_preserve_peaks_and_improve_eligible_slopes() {
        let started=Instant::now();
        let mut rng=gdcompat::Pcg::new(1);rng.next_u32();
        let stage=Stage::generate(&mut rng,3000.0+sideways::stage::RUNOUT);
        assert_eq!((stage.noise_seed,stage.detail_seed),(683407368,2033395789));
        let cells=sideways::builder::chunk_cells(&stage);
        let chunks:Vec<Chunk>=std::thread::scope(|s| {
            let per=cells.len().div_ceil(std::thread::available_parallelism().map(|n|n.get()).unwrap_or(1).min(16));
            let tasks:Vec<_>=cells.chunks(per).map(|part|s.spawn(||part.iter()
                .map(|&key|sideways::builder::terrain_chunk(&stage,key)).collect::<Vec<_>>())).collect();
            tasks.into_iter().flat_map(|t|t.join().unwrap()).collect()
        });
        let ground=Ground::new(&stage,chunks.iter().map(|c|(c.cell,c)).collect());
        let native=native_density(&chunks,&ground);
        let extent=choose_extent(&stage,&ground.native,native,Config::default()).unwrap();
        let steps=terrain_steps(&stage,&extent.cells);
        let old_cache=RefCell::new(HashMap::<(i32,i32),f32>::new());
        let old_height=|x:f32,z:f32| {
            let key=((x*0.5).round() as i32,(z*0.5).round() as i32);
            if let Some(&h)=old_cache.borrow().get(&key){return h;}
            let h=ground.native_height(x,z).unwrap_or_else(|| {
                let seed=stage.noise_seed as u32;
                let wx=x+noise(x/620.0,z/620.0,seed^0x19e377)*105.0;
                let wz=z+noise(x/570.0,z/570.0,seed^0x5aa13e)*105.0;
                let ridge=1.0-noise(wx/390.0,wz/390.0,seed).abs();
                let saddle=smooth((noise(x/710.0,z/710.0,seed^0x8c47)+0.65)/1.30);
                let hills=((70.0+125.0*ridge*ridge+27.0*noise(wx/155.0,wz/155.0,seed^0x51f15e)
                    +11.0*noise(wx/72.0,wz/72.0,seed^0x1ca23b))*(0.42+0.58*saddle)).max(18.0);
                stage.height(x as f64,z as f64) as f32+hills*smooth(ground.border_distance(x,z)/128.0)
            });
            old_cache.borrow_mut().insert(key,h);h
        };
        let old_point=|p:Vec3|Vec3::new(p.x,old_height(p.x,p.z),p.z);
        let mut slopes=[Vec::<f32>::new(),Vec::<f32>::new()];
        let mut steep=[0usize;2];let mut eligible=[0usize;2];let mut band_steep=[0usize;2];
        let mut band_samples=0usize;let mut maximum=[f32::NEG_INFINITY;2];
        let mut candidates=[0usize;2];let mut triangle_count=0usize;
        let seed=stage.noise_seed as u32^0xd157a47;
        for key in extent.cells.iter().copied() {
            let tile=LandTile::new(key,&ground,&steps);
            triangle_count+=tile.indices.len()/3;
            for v in &tile.vertices {maximum[0]=maximum[0].max(old_height(v.pos[0],v.pos[2]));maximum[1]=maximum[1].max(v.pos[1]);}
            let old=LandTile{key,step:tile.step,vertices:Vec::new(),indices:Vec::new(),patches:tile.patches.iter()
                .map(|p|Patch{centre:old_point(p.centre),ring:p.ring.iter().copied().map(old_point).collect()}).collect()};
            for j in 0..TREE_CELLS {for i in 0..TREE_CELLS {
                let gx=key.0*TREE_CELLS+i;let gz=key.1*TREE_CELLS+j;let h=hash(gx,gz,seed);
                let x=(gx as f32+0.05+unit(h)*0.9)*TREE_GRID;
                let z=(gz as f32+0.05+unit(h.rotate_left(13))*0.9)*TREE_GRID;
                let water=if stage.rivers.is_empty(){f32::NEG_INFINITY}else{stage.water_level(x as f64,z as f64) as f32};
                let in_band=ground.border_distance(x,z)<128.0;
                band_samples+=usize::from(in_band);
                for (index,surface) in [old.surface(x,z).unwrap(),tile.surface(x,z).unwrap()].into_iter().enumerate() {
                    let (y,normal_y)=surface;let rejected=normal_y<0.75;
                    slopes[index].push(normal_y.clamp(-1.0,1.0).acos().to_degrees());
                    steep[index]+=usize::from(rejected);band_steep[index]+=usize::from(rejected&&in_band);
                    if habitat_weight(f32::INFINITY,normal_y,y,water)>0.0 {
                        eligible[index]+=1;
                        candidates[index]+=usize::from(forest_rank(hash(gx,gz,seed^0xa1ce),stage.detail.get_2d(x*0.6,z*0.6)).is_some());
                    }
                }
            }}
        }
        for s in &mut slopes{s.sort_by(f32::total_cmp);}
        let total=slopes[0].len();let pct=|n:usize|100.0*n as f64/total as f64;
        let requested=eligible.map(|n|(native.per_m2()*n as f64*16.0).round() as usize);
        eprintln!("captured mountain audit seed1/3km: native {:.2}% steep, {:.2} trees/ha; {} tiles / {triangle_count} source tris; old/new steep {:.3}/{:.3}%, first128m {:.3}/{:.3}%, slope p50 {:.2}/{:.2}deg p95 {:.2}/{:.2}deg, peaks {:.2}/{:.2}m; eligible {:.0}/{:.0}m2; requested {requested:?} / candidates {candidates:?}, cap {MAX_BACKGROUND_TREES}; elapsed {:.3}s",
            100.0*native.steep_m2/(chunks.len() as f64*4096.0),native.per_m2()*10_000.0,extent.cells.len(),pct(steep[0]),pct(steep[1]),
            100.0*band_steep[0] as f64/band_samples as f64,100.0*band_steep[1] as f64/band_samples as f64,
            slopes[0][total/2],slopes[1][total/2],slopes[0][total*95/100],slopes[1][total*95/100],maximum[0],maximum[1],
            eligible[0] as f64*16.0,eligible[1] as f64*16.0,started.elapsed().as_secs_f64());
        assert!(steep[1]*2<steep[0],"the new footprint must materially reduce unsuitable slopes");
        assert!(band_steep[1]*4<band_steep[0],"the visible native-border ramp must lose its steep wall");
        assert!(maximum[1]>150.0 && maximum[1]>maximum[0]*0.85,"keep a genuinely high distant skyline");
        assert!(requested[1]<=MAX_BACKGROUND_TREES && requested[1]<=candidates[1],"density must fit without thinning");
        assert_eq!(triangle_count,369_376,"do not increase terrain draw geometry for this shape correction");
    }
    #[test]
    fn outer_sixteen_metre_tiles_stitch_exactly_to_eight_and_native_two_metre_edges() {
        let stage=Stage::generate(&mut gdcompat::Pcg::new(1),10.0);let chunk=flat_chunk((0,0));
        let ground=Ground::new(&stage,HashMap::from([((0,0),&chunk)]));
        let steps=BTreeMap::from([((1,0),STEP),((2,0),OUTER_STEP)]);
        let fine=LandTile::new((1,0),&ground,&steps);let coarse=LandTile::new((2,0),&ground,&steps);
        for k in 0..=32 {
            let z=k as f32*2.0;
            assert_eq!(fine.height(64.0,z).unwrap().to_bits(),20.0f32.to_bits(),"native border {z}");
            let a=fine.height(128.0,z).unwrap();let b=coarse.height(128.0,z).unwrap();
            assert!((a-b).abs()<2e-5,"coarse/fine seam at {z}: {a} versus {b}");
        }
        for k in 0..=8 {
            let z=k as f32*8.0;
            let a=fine.vertices.iter().find(|v|v.pos[0]==128.0&&v.pos[2]==z).unwrap();
            let b=coarse.vertices.iter().find(|v|v.pos[0]==128.0&&v.pos[2]==z).unwrap();
            assert_eq!(a.pos,b.pos);
            assert_eq!(a.normal,b.normal);
        }
        assert_eq!(coarse.indices.len()/3,68,"64 base triangles + four finer-edge splits");
        assert_eq!(fine.indices.len()/3,280,"256 base triangles + 24 native-edge splits");
        // Independently evaluate rendered triangle centroids; root placement
        // must use that mesh, not the un-tessellated high-frequency height.
        for tri in coarse.indices.chunks_exact(3) {
            let p:[Vec3;3]=std::array::from_fn(|i|Vec3::from(coarse.vertices[tri[i] as usize].pos));
            let centre=(p[0]+p[1]+p[2])/3.0;
            assert!((coarse.height(centre.x,centre.z).unwrap()-centre.y).abs()<0.002);
        }
    }
    #[test]
    fn mixed_resolution_heightfield_preserves_native_bits_and_samples_rendered_outer_mesh() {
        let stage=Stage::generate(&mut gdcompat::Pcg::new(2),10.0);let chunk=flat_chunk((0,0));
        let ground=Ground::new(&stage,HashMap::from([((0,0),&chunk)]));
        let steps=BTreeMap::from([((1,0),STEP),((2,0),OUTER_STEP)]);
        let tiles:BTreeMap<_,_>=steps.keys().map(|&key|(key,LandTile::new(key,&ground,&steps))).collect();
        let mut old=Heightfield{origin:[0.0,0.0],cell:2.0,w:33,h:33,heights:chunk.heights.clone()};
        old.heights[17]=-0.0;old.heights[18]=f32::from_bits(0x3f800001);
        let hf=extended_heightfield(&old,&tiles,(0,0),(2,0));
        assert_eq!((hf.w,hf.h,hf.cell),(97,33,2.0));
        for j in 0..33 {for i in 0..33 {assert_eq!(hf.heights[j*97+i].to_bits(),old.heights[j*33+i].to_bits());}}
        let outer=&tiles[&(2,0)];
        for j in 0..=32 {for i in 0..=32 {
            let expected=outer.height(128.0+i as f32*2.0,j as f32*2.0).unwrap();
            assert_eq!(hf.heights[j*97+i+64].to_bits(),expected.to_bits());
        }}
    }
    #[test]
    fn extended_extent_preserves_inner_resolution_and_reduces_range_before_density() {
        let stage=Stage::generate(&mut gdcompat::Pcg::new(1),500.0);let chunk=flat_chunk((0,0));
        let native=HashMap::from([((0,0),&chunk)]);let density=NativeDensity{trees:300,habitat_m2:10_000.0,..Default::default()};
        let old=background_cells(&stage,&native,INNER_RADIUS);
        let extended=choose_extent(&stage,&native,density,Config::default()).unwrap();
        assert_eq!(extended.radius,1152.0);assert!(old.is_subset(&extended.cells));
        assert!(extended.cells.len()>old.len());assert!(extended.cells.len()<=MAX_BACKGROUND_TILES);
        let (_,_,samples)=bounds(&native,&extended.cells);assert!(samples<=MAX_HEIGHT_SAMPLES);
        let steps=terrain_steps(&stage,&extended.cells);
        assert!(old.iter().all(|key|steps[key]==STEP));
        assert!(extended.cells.difference(&old).all(|key|steps[key]==OUTER_STEP));
        let tree_budget=(density.per_m2()*old.len() as f64*(TILE*TILE) as f64).ceil() as usize;
        let reduced=choose_extent(&stage,&native,density,Config{max_trees:tree_budget,..Default::default()}).unwrap();
        assert_eq!(reduced.radius,INNER_RADIUS);assert_eq!(reduced.cells,old);
        let repeated=choose_extent(&stage,&native,density,Config::default()).unwrap();assert_eq!(extended.cells,repeated.cells);
        assert!(choose_extent(&stage,&native,density,Config{radius:f32::NAN,..Default::default()}).is_err());
        assert!(choose_extent(&stage,&native,density,Config{max_height_samples:100,..Default::default()}).is_err());
        eprintln!("bounded vista: {} -> {} tiles, {} outer 16 m tiles, {samples} height samples ({} bytes), tree cap {MAX_BACKGROUND_TREES} ({} bytes instance payload; GPU copies/traversal/shadows additional)",old.len(),extended.cells.len(),extended.cells.len()-old.len(),samples*4,MAX_BACKGROUND_TREES*std::mem::size_of::<render::scene::GpuInstance>());
    }
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
        let ground=Ground::new(&stage,chunks.iter().map(|c|(c.cell,c)).collect());
        let measured=native_density(&chunks[4..5],&ground);
        assert_eq!(measured.trees,64);
        assert_eq!(measured.habitat_m2,2048.0,"half of a flat 4096 m2 tile clears the native random road offset");
        assert_eq!(measured.per_m2(),1.0/32.0);
    }
    #[test]
    fn far_count_matches_measured_habitat_density_and_honors_budget() {
        let native=NativeDensity{trees:64,habitat_m2:2048.0,..Default::default()};
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
            tiles.insert(key,LandTile{key,step:STEP,patches,vertices:vec![],indices:vec![]});
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
