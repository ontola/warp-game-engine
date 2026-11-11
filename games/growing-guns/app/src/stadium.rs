//! The colosseum on screen and in the ears, after Godot's
//! `colosseum_builder.gd`, `colosseum_crowd.gd` and `crowd_audio.gd`: stepped
//! stone stands, a shadow-casting roof with a lamp ring, floodlight towers,
//! four live stadium screens, up to 15,000 spectators (`render::crowd`)
//! who cheer, cower and flee, die
//! where shots and blasts land (blood, scorch, gore, a red mist) and a crowd
//! sound that follows the match.
use crate::crowd_sound::CrowdSound;
use audio::sfx::Listener;
use glam::{Affine3A, Quat, Vec2, Vec3, Vec3Swizzles};
use growing_guns::stadium::{sq, Stadium, PieceKind, ROWS, ROW_DEPTH, ROW_RISE, SEGMENTS, FOUNDATION_Y};
use growing_guns::{Event, World};
use render::crowd::{Crowd, Seat};
use render::headlights::LocalLight;
use render::primitives::{self, Primitive};
use render::cluster::ClusterMesh;
use render::scene::{Material, EMISSIVE, LIVE_SCREEN, NO_SHADOW};
use render::slots::{SlotId, Slots};
use render::Scene;
use std::f32::consts::{PI, TAU};
mod gibs;

pub const CROWD_MAX: usize = 15_000;
#[derive(Clone, Copy, PartialEq)]
enum Density { Standard, Packed }

static DENSITY: std::sync::OnceLock<Density> = std::sync::OnceLock::new();

pub(crate) fn configure_density(args: &crate::Args) -> Result<(), String> {
    let density = match args.value("--crowd-density") {
        Some("standard") => Density::Standard,
        Some("packed") => Density::Packed,
        Some(_) => return Err("--crowd-density expects standard or packed".into()),
        None if args.flag("--crowd-density") => return Err("missing --crowd-density value".into()),
        None if crate::warp_humans_enabled() => Density::Packed,
        None => Density::Standard,
    };
    DENSITY.set(density).map_err(|_| "crowd density already configured".into())
}
const ROOF_DROP: f32 = 3.0;
const ROOF_STEPS: usize = 224;
const ROOF_INNER: f32 = 0.8135;

// A closed, continuous superellipse canopy. Adjacent sections share exactly
// the same boundary positions; no overlapping slab ends or internal caps.
fn roof_point(angle: f32, inner: bool) -> Vec3 {
    let r=if inner {ROOF_INNER} else {1.0};
    Vec3::new(angle.cos()*sq(angle)*r, if inner {-ROOF_DROP} else {0.0}, angle.sin()*sq(angle)*r)
}
fn roof_mesh(material: u32) -> Primitive { ring_mesh(material, ROOF_INNER, ROOF_DROP, 0.5) }
fn ring_mesh(material: u32, inner_ratio: f32, drop: f32, thickness: f32) -> Primitive {
    let point=|angle:f32,inner:bool| {let r=if inner {inner_ratio} else {1.0}; Vec3::new(angle.cos()*sq(angle)*r,if inner {-drop} else {0.0},angle.sin()*sq(angle)*r)};
    use render::cluster::Vertex;
    let mut mesh=Primitive {material,..Default::default()};
    for i in 0..ROOF_STEPS {
        let a=TAU*i as f32/ROOF_STEPS as f32;
        let b=TAU*((i+1)%ROOF_STEPS) as f32/ROOF_STEPS as f32;
        let [o0,o1,i0,i1]=[point(a,false),point(b,false),point(a,true),point(b,true)];
        for (points,side) in [([o0,i0,i1,o1],1i32),([o1,i1,i0,o0],-1),([o0,o1,o1,o0],2),([i1,i0,i0,i1],-2)] {
            let base=mesh.vertices.len() as u32;
            for (j,mut pos) in points.into_iter().enumerate() {
                pos.y+=if side.abs()==1 {side as f32*thickness*0.5} else {if j<2 {thickness*0.5} else {-thickness*0.5}};
                let angle=pos.z.atan2(pos.x);
                let tangent=point(angle+0.0001,false)-point(angle-0.0001,false);
                let radial=point(angle,false)-point(angle,true);
                let normal=if side.abs()==1 {tangent.cross(radial).normalize()*side as f32} else {Vec3::Y.cross(tangent).normalize()*if side>0 {1.0} else {-1.0}};
                mesh.vertices.push(Vertex {pos:pos.to_array(),normal:normal.to_array(),uv:[pos.x,pos.z]});
            }
            // Explicitly wind to the expected outward normals.
            for mut t in [[base,base+1,base+2],[base,base+2,base+3]] {
                let [v0,v1,v2]=t.map(|k| mesh.vertices[k as usize]);
                if (Vec3::from(v1.pos)-Vec3::from(v0.pos)).cross(Vec3::from(v2.pos)-Vec3::from(v0.pos)).dot(Vec3::from(v0.normal))<0.0 {t.swap(1,2);}
                mesh.indices.extend(t);
            }
        }
    }
    mesh
}
fn roof_radius(geo: Stadium) -> f32 {geo.inner_r+ROWS as f32*geo.row_depth()+geo.scale}
fn roof_proxies(geo: Stadium) -> Vec<Affine3A> {
    let radius=roof_radius(geo);
    (0..ROOF_STEPS).map(|i| {
        let a=TAU*(i as f32+0.5)/ROOF_STEPS as f32;
        let outer=roof_point(a,false)*Vec3::new(radius,geo.scale,radius)+Vec3::Y*(geo.wall_h()+0.6*geo.scale);
        let inner=roof_point(a,true)*Vec3::new(radius,geo.scale,radius)+Vec3::Y*(geo.wall_h()+0.6*geo.scale);
        let radial=outer-inner;
        let tangent=(roof_point(a+TAU/ROOF_STEPS as f32*0.5,false)-roof_point(a-TAU/ROOF_STEPS as f32*0.5,false))*Vec3::new(radius,geo.scale,radius);
        // Thin conservative static-light proxy follows the same roof curve.
        Affine3A::from_cols(tangent.into(),(Vec3::Y*0.5*geo.scale).into(),radial.into(),((outer+inner)*0.5).into())
    }).collect()
}
const MARKS: usize = 768;
const TV_COUNT: usize = 4;
const TV_WIDTH: f32 = 26.0;
const TV_HEIGHT: f32 = TV_WIDTH * 9.0 / 16.0;
const TV_TILT: f32 = 0.22;
const TV_OBSERVER_HEIGHT: f32 = 2.0;
const TV_CANOPY_CLEARANCE: f32 = 1.0;

const SANDSTONE: [[f32; 3]; 3] = [[0.74, 0.61, 0.42], [0.70, 0.57, 0.40], [0.77, 0.65, 0.46]];
const LIMESTONE: [[f32; 3]; 3] = [[0.72, 0.66, 0.53], [0.68, 0.63, 0.51], [0.75, 0.69, 0.56]];
const CROWD_COLORS: [[f32; 3]; 8] = [[0.34, 0.27, 0.21], [0.30, 0.25, 0.19], [0.29, 0.29, 0.30], [0.24, 0.21, 0.18], [0.38, 0.31, 0.22], [0.36, 0.20, 0.17], [0.21, 0.24, 0.32], [0.31, 0.33, 0.24]];
const CROWD_ACCENTS: [[f32; 3]; 3] = [[0.55, 0.18, 0.14], [0.2, 0.3, 0.5], [0.55, 0.45, 0.2]];
const SHADES: [f32; 3] = [0.67, 0.8, 0.93];
/// The arena's building stone, darkened (Godot: palette building.darkened(0.2)).
const ARENA_STONE: [f32; 3] = [0.48, 0.456, 0.424];

fn lin(c: [f32; 3]) -> [f32; 3] { c.map(|v| v.powf(2.2)) }
fn mat(scene: &mut Scene, c: [f32; 3], roughness: f32) -> u32 { scene.add_material(Material { tint: lin(c), layer: -1, roughness, flags: 0, scale: 1.0, metal: 0.0 }) }
fn glow(scene: &mut Scene, c: [f32; 3], nits: f32) -> u32 { scene.add_material(Material { tint: c, layer: -1, roughness: 0.8, flags: EMISSIVE, scale: nits, metal: 0.0 }) }

/// Godot's stadium_tv.gd: side midpoints leave the corners for floodlights.
/// The picture faces local +Z, toward the field and slightly downwards.
fn tv_pose(geo: Stadium, index: usize) -> (Vec3, Quat) {
    let angle = TAU * index as f32 / TV_COUNT as f32;
    let dir = Vec3::new(angle.cos(), 0.0, angle.sin());
    let radius = geo.wall_r() - 1.35 * geo.scale;
    let half_height = TV_HEIGHT * 0.5 * geo.scale;
    // Merely clearing the rim in world Y leaves the lower picture hidden by
    // the canopy's inner lip. Solve the eye-to-bottom-corner sightline over
    // the actual tilted roof's upper inner edge, with one metre of clearance.
    // sq(angle) >= 1, so the side-midpoint lip is the nearest possible roof.
    let lip_r=roof_radius(geo)*ROOF_INNER;
    let lip_y=geo.wall_h()+(0.6-ROOF_DROP+0.25)*geo.scale;
    let bottom_r = (radius + half_height * TV_TILT.sin()).hypot(TV_WIDTH * 0.5 * geo.scale);
    let min_bottom_y = TV_OBSERVER_HEIGHT
        + (lip_y + TV_CANOPY_CLEARANCE - TV_OBSERVER_HEIGHT) * bottom_r / lip_r;
    let height = (geo.wall_h() + TV_HEIGHT * 0.62 * geo.scale).max(min_bottom_y + half_height * TV_TILT.cos());
    let position = dir * radius + Vec3::Y * height;
    let rotation = Quat::from_rotation_y(-angle - PI * 0.5) * Quat::from_rotation_x(TV_TILT);
    (position, rotation)
}

/// A box primitive at `center` with `size` (for composing small figures).
fn block(material: u32, center: Vec3, size: Vec3) -> Primitive {
    let mut p = primitives::cube(material);
    for v in &mut p.vertices { v.pos = (Vec3::from(v.pos) * size + center).to_array(); }
    p
}

struct Rng(u64);
impl Rng {
    fn f(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, a: f32, b: f32) -> f32 { a + (b - a) * self.f() }
    fn below(&mut self, n: usize) -> usize { (self.f() * n as f32) as usize % n.max(1) }
}

type Probes = Vec<render::irradiance_volume::Probe>;
enum SunUpload { Packed(Vec<u32>), Probes(Probes) }

/// One stadium sky-occlusion bake (`render::sky_occlusion::bake_diffuse`).
#[derive(Clone)]
struct SkyBake { emission: Probes, probes: Probes, planes: Vec<[f32;4]>, ms: f64 }

// Keep just two geometries per cache (~24 MiB total at the normal grid).
// Rewinding identical geometry must not rebake half a second of static light.
#[derive(PartialEq)]
struct BakeKey { grid: [u32; 7], boxes: Vec<[u32; 12]>, reflectance: u32, extra: [u32; 4] }
impl BakeKey {
    fn new(grid: render::irradiance_volume::Grid, boxes: &[Affine3A], reflectance: f32, extra: [u32; 4]) -> Self {
        Self { grid: [grid.origin.x.to_bits(), grid.origin.y.to_bits(), grid.origin.z.to_bits(), grid.cell.to_bits(), grid.dims[0], grid.dims[1], grid.dims[2]],
            boxes: boxes.iter().map(|b| b.to_cols_array().map(f32::to_bits)).collect(), reflectance: reflectance.to_bits(), extra }
    }
}
struct BakeCache<T>(std::collections::VecDeque<(BakeKey, T)>);
impl<T: Clone> BakeCache<T> {
    fn get(&self, key: &BakeKey) -> Option<T> { self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()) }
    fn put(&mut self, key: BakeKey, value: T) {
        self.0.retain(|(k, _)| *k != key);
        self.0.push_back((key, value));
        while self.0.len() > 2 { self.0.pop_front(); }
    }
}
fn bake_sun_cached(grid: render::irradiance_volume::Grid, boxes: &[Affine3A], sky: &[render::irradiance_volume::Probe], reflectance: f32, sun: Vec3) -> Result<Probes, String> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<BakeCache<Probes>>> = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(BakeCache(Default::default())));
    let key = BakeKey::new(grid, boxes, reflectance, [sun.x.to_bits(), sun.y.to_bits(), sun.z.to_bits(), 0]);
    if let Some(probes) = cache.lock().unwrap().get(&key) { return Ok(probes); }
    let probes = render::sky_occlusion::bake_sun(&grid, boxes, sky, 96, reflectance, sun)?;
    cache.lock().unwrap().put(key, probes.clone());
    Ok(probes)
}

fn bake_sky(grid: render::irradiance_volume::Grid, boxes: &[Affine3A], reflectance: f32, half: f32) -> SkyBake {
    let start = std::time::Instant::now();
    static CACHE: std::sync::OnceLock<std::sync::Mutex<BakeCache<SkyBake>>> = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(BakeCache(Default::default())));
    let separate = std::env::var("GG_SKY_SEPARATION").as_deref() == Ok("1");
    let key = BakeKey::new(grid, boxes, reflectance, [half.to_bits(), separate as u32, 0, 0]);
    if let Some(mut bake) = cache.lock().unwrap().get(&key) {
        bake.ms = start.elapsed().as_secs_f64() * 1000.0;
        return bake;
    }
    let probes = render::sky_occlusion::bake_diffuse(&grid, boxes, 96, reflectance).expect("valid stadium occluders");
    let planes = if separate {
        render::sky_occlusion::bake_separation_planes(&grid, boxes).expect("valid separation geometry")
    } else { Vec::new() };
    // Four non-overlapping rectangles: the outer lava sea, excluding the
    // arena floor. Average hot-lava radiance is fixed; only geometry rebakes it.
    let radiance=Vec3::new(1.0,0.147,0.0129)*8000.0;
    let rects=[([-450.0,-450.0],[-half,450.0]),([half,-450.0],[450.0,450.0]),([-half,-450.0],[half,-half]),([-half,half],[half,450.0])];
    let emitters=rects.map(|(min,max)| render::sky_occlusion::PlaneEmitter { y: growing_guns::arena::LAVA_KILL_Y, min: min.into(), max: max.into(), radiance });
    let emission=render::sky_occlusion::bake_emission(&grid,boxes,&emitters,96).expect("valid lava emitters");
    let bake = SkyBake { emission, probes, planes, ms: start.elapsed().as_secs_f64()*1000.0 };
    cache.lock().unwrap().put(key, bake.clone());
    bake
}

// Flat irregular stains, never flattened spheres hovering across several rows.
fn stain_mesh(material: u32) -> Primitive {
    let mut mesh = Primitive { material, ..Default::default() };
    mesh.vertices.push(render::cluster::Vertex { pos:[0.0;3], normal:Vec3::Y.to_array(), uv:[0.5;2] });
    for i in 0..24 {
        let a = i as f32 * TAU / 24.0;
        let r = 0.76 + 0.16 * (a * 5.0).sin() + 0.08 * (a * 9.0 + 1.0).sin();
        mesh.vertices.push(render::cluster::Vertex { pos:[a.cos()*r,0.0,a.sin()*r], normal:Vec3::Y.to_array(), uv:[0.5+a.cos()*0.5,0.5+a.sin()*0.5] });
        mesh.indices.extend([0, (i+1)%24+1, i+1]);
    }
    mesh
}

fn mark_surface(pieces: &[growing_guns::stadium::Piece], p: Vec3, radius: f32) -> Option<(Vec3, Vec3, f32)> {
    let mut best = None;
    let mut distance = f32::INFINITY;
    for piece in pieces {
        let rotation = Quat::from_rotation_y(piece.yaw);
        let local = rotation.conjugate() * (p-piece.center);
        let half = piece.size * 0.5;
        // Real tread and inward riser, not an imaginary smooth slope.
        for (axis, sign) in [(1usize,1.0f32),(2usize,-1.0f32)] {
            let axes = if axis==1 {[0,2]} else {[0,1]};
            let r = radius.min(half[axes[0]]*0.8).min(half[axes[1]]*0.8).max(0.02);
            let mut q = local;
            q[axis] = half[axis] * sign;
            for &a in &axes {q[a]=q[a].clamp(-half[a]+r,half[a]-r);}
            let d = q.distance_squared(local);
            if d < distance {
                distance=d;
                let mut n=Vec3::ZERO;n[axis]=sign;
                let n=rotation*n;
                best=Some((piece.center+rotation*q+n*0.012,n,r));
            }
        }
    }
    best
}

pub struct StadiumVisuals {
    enabled: bool,
    /// Bake the sky cache on worker threads and keep the previous one until
    /// it lands (play: a bake is 0.25 to 1.3 s, which stalled the frame and
    /// the audio). Off for scripts and benches, so their captures stay exact.
    pub bake_in_background: bool,
    sky_job: Option<std::sync::mpsc::Receiver<SkyBake>>,
    sun_job: Option<(Vec3, std::sync::mpsc::Receiver<Result<SunUpload, String>>)>,
    sky_boxes: Vec<Affine3A>,
    sky_planes: Vec<[f32;4]>,
    sky_probes: Vec<render::irradiance_volume::Probe>,
    sky_emission: Probes,
    sky_dirty: bool,
    sky_reflectance: f32,
    sky_sun: Option<Vec3>,
    stands: Slots,
    roof: Slots,
    tv: Slots,
    /// Shared picture quad and dark housing cube, also used for the legs.
    tv_mesh: [usize; 2],
    /// Stone per quarry (sandstone 0..3, limestone 3..6), roof stone likewise.
    stone: [usize; 6],
    masonry: Option<[[usize; 2]; 6]>,
    roof_stone: [usize; 6],
    wall_stone: [usize; 6],
    lamp: usize,
    shaft: usize,
    bank: usize,
    /// (body, arms) per look: colour x shade.
    looks: Vec<(usize, usize)>,
    pub crowd: Crowd,
    pub sound: CrowdSound,
    marks: Slots,
    mark_ring: Vec<SlotId>,
    mark_next: usize,
    scorch: usize,
    blood:crate::blood::Splats,
    blood_grid:growing_guns::shape::BoxGrid,
    gore: gibs::GibMeshes,
    pub geo: Stadium,
    mark_surfaces: Vec<growing_guns::stadium::Piece>,
    lights: Vec<LocalLight>,
    /// Narrow scatter-only beams inside each floodlight's cone, for the
    /// night's volumetric light (they light no surface).
    beams: Vec<LocalLight>,
    rng: Rng,
}

fn stadium_rng(seed: u64) -> (Rng, usize) {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let quarry = if rng.f() < 0.5 { 0 } else { 3 } + rng.below(3);
    (rng, quarry)
}

// One figure per occupied seat, preserving the game RNG draw order.
fn seat_layout(rng: &mut Rng, geo: Stadium, looks: &[(usize,usize)]) -> Vec<Seat> {
    seats_at_density(rng, geo, looks, DENSITY.get().copied().unwrap_or(Density::Standard))
}

fn seats_at_density(rng: &mut Rng, geo: Stadium, looks: &[(usize,usize)], density: Density) -> Vec<Seat> {
    let seg = Stadium::seg_ang();
    let mut seats = Vec::new();
    use growing_guns::shape::{BoxGrid, Boxes, YawBox};
    let treads = geo.pieces().into_iter().filter(|p| p.kind == PieceKind::Step)
        .map(|p| YawBox::new(p.center, p.size, p.yaw)).collect();
    let grid = BoxGrid::new(treads, 4.0);
    // Each row contributes its full perimeter. Widen the spacing uniformly on a
    // large arena before placing anyone, so the last sections also get seats.
    let (spacing, jitter, skip) = match density {
        Density::Standard => (0.9, 0.15, 0.08),
        Density::Packed => {
            let mean_radius = geo.inner_r + ROWS as f32 * geo.row_depth() * 0.5;
            let ring: f32 = (0..SEGMENTS).map(|i| sq(seg * (i as f32 + 0.5)) * seg).sum();
            let length = mean_radius * ring * ROWS as f32;
            ((length / CROWD_MAX as f32).max(0.58), 0.035, 0.025)
        }
    };
    'all: for i in 0..SEGMENTS {
        let ang = seg * (i as f32 + 0.5);
        for row in 0..ROWS {
            let y = geo.base_y + row as f32 * geo.row_rise();
            let row_r = geo.inner_r + (row as f32 + 0.5) * geo.row_depth();
            let count = (row_r * sq(ang) * seg / spacing) as usize;
            for k in 0..count {
                if rng.f() < skip { continue; }
                // Eight radial access aisles leave room to stand among the crowd.
                let midpoint = (k as f32 + 0.5) / count as f32 - 0.5;
                if density == Density::Packed && i % 7 == 0 && (midpoint * row_r * sq(ang) * seg).abs() < 1.15 {
                    continue;
                }
                // The tread is a straight box. Keep each foot on its actual tread,
                // including at corners where the curved seat ring crossed adjacent steps.
                let offset = midpoint * seg;
                let a = ang + offset + rng.range(-jitter, jitter) / row_r.max(1.0);
                let radial = Vec3::new(ang.cos(), 0.0, ang.sin());
                let tangent = Vec3::new(-ang.sin(), 0.0, ang.cos());
                let mid = row_r * sq(ang);
                let along = mid * (a-ang).tan() + rng.range(-jitter, jitter);
                let mut pos = radial * (mid + rng.range(-0.25, 0.25)) + tangent * along + Vec3::Y * y;
                let above = geo.wall_h() + 1.0;
                if let Some((distance, _, _)) = grid.ray(pos.with_y(above), Vec3::NEG_Y, above - geo.base_y + 1.0) {
                    pos.y = above - distance;
                }
                let colour = if rng.f() < 0.08 { CROWD_COLORS.len() + rng.below(CROWD_ACCENTS.len()) } else { rng.below(CROWD_COLORS.len()) };
                let look = looks[colour * SHADES.len() + rng.below(SHADES.len())];
                let hop = if rng.f() < 0.3 { 0.0 } else { rng.range(0.15, 0.42) };
                seats.push(Seat { pos, yaw: -a - PI * 0.5, scale: rng.range(1.55, 1.95), width: rng.range(0.72, 0.98), phase: rng.f(), tempo: rng.f(), hop, body: look.0, arms: look.1 });
                if seats.len() >= CROWD_MAX { break 'all; }
            }
        }
    }
    seats
}

pub(crate) fn review_seats(world: &World) -> Vec<Seat> {
    let (mut rng, _) = stadium_rng(world.arena.seed);
    let geo = world.arena.stadium;
    let looks = [(0,0); (CROWD_COLORS.len()+CROWD_ACCENTS.len())*SHADES.len()];
    seat_layout(&mut rng, geo, &looks)
}

/// A local scale playtest starts in an empty space between two neighbours on one tread.
pub(crate) fn start_in_stands(world: &mut World) -> Result<(), String> {
    let seats = review_seats(world);
    let pair = seats.windows(2).find(|s| {
        let gap = s[0].pos.distance(s[1].pos);
        (2.4..4.0).contains(&gap) && (s[0].pos.y - s[1].pos.y).abs() < 0.01
            && (s[0].yaw - s[1].yaw).abs() < 0.08
    }).ok_or("no open space on a stadium tread")?;
    let feet = (pair[0].pos + pair[1].pos) * 0.5;
    world.release_cage();
    let player = &mut world.players[0];
    player.pos = feet + Vec3::Y * growing_guns::HALF_HEIGHT;
    player.vel = Vec3::ZERO;
    let toward = pair[0].pos - feet;
    player.yaw = (-toward.x).atan2(-toward.z);
    player.pitch = 0.0;
    Ok(())
}

impl StadiumVisuals {
    /// A sky or sunlight bake is still running (a probe would measure it too).
    pub fn baking(&self) -> bool { self.sky_job.is_some() || self.sun_job.is_some() }
    /// Meshes and reserved instances; call before the renderer exists.
    pub fn reserve(scene: &mut Scene,blood:crate::blood::Splats) -> Self {
        if !crate::graphics::get().stadium {
            let crowd = Crowd::reserve(scene, 0);
            scene.crowd = None; // no GPU crowd buffers or dispatch, not merely invisible seats
            return Self { enabled:false, bake_in_background:false, sky_job:None,sun_job:None,
                sky_boxes:Vec::new(),sky_planes:Vec::new(),sky_probes:Vec::new(),sky_emission:Vec::new(),sky_dirty:false,sky_reflectance:0.0,sky_sun:None,
                stands:Slots::reserve(scene,0),roof:Slots::reserve(scene,0),tv:Slots::reserve(scene,0),tv_mesh:[0;2],
                stone:[0;6],masonry:None,roof_stone:[0;6],wall_stone:[0;6],lamp:0,shaft:0,bank:0,looks:Vec::new(),
                crowd,sound:CrowdSound::new(None),marks:Slots::reserve(scene,0),mark_ring:Vec::new(),mark_next:0,
                scorch:0,blood,blood_grid:Default::default(),gore:Default::default(),geo:Stadium::new(80.0,true,12.0),mark_surfaces:Vec::new(),lights:Vec::new(),beams:Vec::new(),rng:Rng(1) };
        }
        let cube = |scene: &mut Scene, m: u32| scene.add_mesh(&primitives::cube(m).mesh());
        let quarry: Vec<[f32; 3]> = SANDSTONE.iter().chain(LIMESTONE.iter()).map(|c| [0, 1, 2].map(|k| c[k] + (ARENA_STONE[k] - c[k]) * 0.22)).collect();
        let layer = crate::view::stone_layer(scene);
        let quarry_mat = |scene: &mut Scene, tint: [f32; 3]| {
            let m = mat(scene, tint, 0.95);
            if layer >= 0 {
                let stone = &mut scene.materials[m as usize];
                stone.layer = layer; stone.flags = render::scene::ROCK; stone.scale = 0.0;
            } else { scene.materials[m as usize].roughness = 1.0; }
            m
        };
        let stone: [usize; 6] = std::array::from_fn(|i| { let m = quarry_mat(scene, quarry[i]); cube(scene, m) });
        // One shared clustered panel per material/type. No per-brick instances,
        // physics bodies, updates, or extra shadow casters.
        let masonry = (std::env::var("GG_STADIUM_BRICKS").as_deref() != Ok("0")).then(||
            std::array::from_fn(|i| {
                let m = quarry_mat(scene, quarry[i]);
                [(4, 8), (6, 1)].map(|(cols, rows)| {
                    let fine = render::masonry::panel(m, cols, rows, 11).mesh();
                    scene.add_mesh(&fine)
                })
            }));
        let roof_stone: [usize; 6] = std::array::from_fn(|i| { let m = quarry_mat(scene, quarry[i].map(|v| v * 0.75)); scene.add_mesh(&roof_mesh(m).mesh()) });
        let wall_stone: [usize;6]=std::array::from_fn(|i| {let m=quarry_mat(scene,quarry[i]);scene.add_mesh(&ring_mesh(m,0.9795,0.0,1.0).mesh())});
        let lamp = { let m = glow(scene, [1.0, 0.85, 0.6], 9000.0); cube(scene, m) };
        let shaft = { let m = mat(scene, [0.16, 0.16, 0.18], 0.8); cube(scene, m) };
        let bank = { let m = glow(scene, [1.0, 0.96, 0.85], 11000.0); cube(scene, m) };
        // All four screens share the primary view before its HUD. The renderer
        // refreshes this layer after rendering all views, so the picture is
        // safely sampled on the following frame without another scene render.
        let tv_layer = scene.add_live_screen_texture("growing-guns-stadium-tv", 0, TV_WIDTH / TV_HEIGHT)
            .expect("the stadium screen uses a valid source view and aspect ratio");
        let tv_material = scene.add_material(Material { tint: [1.0; 3], layer: tv_layer,
            roughness: 1.0, flags: EMISSIVE | LIVE_SCREEN, scale: 12000.0, metal: 0.0 });
        let tv_picture = scene.add_mesh(&primitives::quad(tv_material).mesh());
        let tv_housing = { let m = mat(scene, [0.09, 0.09, 0.11], 0.85); cube(scene, m) };
        // Spectators: torso + head and the two arms, one look per tunic
        // colour and shade (Godot tints per instance; Warp per material).
        let mut looks = Vec::new();
        let detailed = std::env::var("GG_CROWD_MODE").as_deref() != Ok("blocks");
        for c in CROWD_COLORS.iter().chain(CROWD_ACCENTS.iter()) {
            for (variant, &sh) in SHADES.iter().enumerate() {
                let tunic = mat(scene, c.map(|v| v * sh), 1.0);
                let skin_tint = if detailed { [[0.76, 0.59, 0.46], [0.48, 0.31, 0.21], [0.64, 0.44, 0.30]][variant] } else { [0.68, 0.52, 0.40].map(|v| v * sh) };
                let skin = mat(scene, skin_tint, 1.0);
                if detailed {
                    let dark = mat(scene, [[0.16, 0.18, 0.23], [0.24, 0.17, 0.11], [0.20, 0.22, 0.16]][variant], 1.0);
                    let (body, arms) = crate::spectators::meshes(tunic, skin, dark, variant);
                    looks.push((scene.add_mesh(&body), scene.add_mesh(&arms)));
                    continue;
                }
                let torso = block(tunic, Vec3::new(0.0, 0.31, 0.0), Vec3::new(0.30, 0.62, 0.19));
                let head = block(skin, Vec3::new(0.0, 0.78, 0.0), Vec3::new(0.22, 0.24, 0.20));
                let body = scene.add_mesh(&ClusterMesh::build(&[torso.part(), head.part()]));
                let la = block(tunic, Vec3::new(-0.21, 0.44, 0.0), Vec3::new(0.09, 0.34, 0.10));
                let ra = block(tunic, Vec3::new(0.21, 0.44, 0.0), Vec3::new(0.09, 0.34, 0.10));
                let arms = scene.add_mesh(&ClusterMesh::build(&[la.part(), ra.part()]));
                looks.push((body, arms));
            }
        }
        let scorch = { let m = mat(scene, [0.04, 0.035, 0.03], 1.0); scene.add_mesh(&stain_mesh(m).mesh()) };
        let gore = gibs::GibMeshes::new(scene);
        let pieces = SEGMENTS * (ROWS + 4) + SEGMENTS / 2 + 16;
        let stands = Slots::reserve(scene, pieces);
        // The colosseum walls and tiers occlude direct sun and atmospheric haze.
        let roof = Slots::reserve(scene, 1);
        // Immutable architecture gets a world-fixed sun map beyond the moving
        // detailed maps. Do not cache the crowd, gore, or destructible arena.
        scene.static_sun_instances.extend(stands.first..stands.first+stands.capacity());
        scene.static_sun_instances.extend(roof.first..roof.first+roof.capacity());
        let tv = Slots::reserve(scene, TV_COUNT * 4);
        for i in 0..tv.capacity() { scene.instances[tv.first + i].flags |= NO_SHADOW; }
        let marks = Slots::reserve(scene, MARKS);
        for i in 0..MARKS { scene.instances[marks.first + i].flags |= NO_SHADOW; }
        let mut crowd = Crowd::reserve(scene, CROWD_MAX);
        if detailed { crowd.arm_pivot(0.655); }
        StadiumVisuals { enabled:true, bake_in_background: false, sky_job: None, sun_job: None, sky_planes: Vec::new(), sky_boxes: Vec::new(), sky_probes: Vec::new(), sky_emission: Vec::new(), sky_dirty: false, sky_reflectance: -1.0, sky_sun: None, stands, roof, tv, tv_mesh: [tv_picture, tv_housing], stone, masonry, roof_stone, wall_stone, lamp, shaft, bank, looks, crowd, sound: CrowdSound::new(None), marks, mark_ring: Vec::new(), mark_next: 0,

            scorch, blood, blood_grid:Default::default(), gore, geo: Stadium::new(80.0, true, 12.0), mark_surfaces: Vec::new(), lights: Vec::new(), beams: Vec::new(), rng: Rng(1) }
    }

    /// Give the crowd its voice (recordings and a speaker).
    pub fn set_sound(&mut self, s: CrowdSound) { self.sound = s; }

    /// Build the bowl for this arena (a new round). Upload with `flush`.
    pub fn set_arena(&mut self, scene: &mut Scene, world: &World) {
        if !self.enabled { self.geo=world.arena.stadium; return; }
        let a = &world.arena;
        self.geo = a.stadium;
        self.mark_surfaces = self.geo.pieces().into_iter().filter(|p| matches!(p.kind, PieceKind::Step | PieceKind::Facade | PieceKind::Wall)).collect();
        self.blood_grid=growing_guns::shape::BoxGrid::new(self.geo.collision(),4.);
        let (rng, quarry) = stadium_rng(a.seed);
        self.rng = rng;
        self.stands.clear(scene);
        self.roof.clear(scene);
        self.tv.clear(scene);
        self.marks.clear(scene);
        self.mark_ring.clear();
        let box_xf = |c: Vec3, size: Vec3, yaw: f32| Affine3A::from_scale_rotation_translation(size, Quat::from_rotation_y(yaw), c);
        let mut sky_boxes = Vec::new();
        for p in self.geo.pieces() {
            if !matches!(p.kind, PieceKind::Wall) { sky_boxes.push(box_xf(p.center, p.size, p.yaw)); }
            let mesh = self.masonry.as_ref().and_then(|m| match p.kind {
                PieceKind::Facade => Some(m[quarry][0]),
                PieceKind::Step => Some(m[quarry][1]),
                _ => None,
            }).unwrap_or(self.stone[quarry]);
            if !matches!(p.kind, PieceKind::Wall) { self.stands.add(scene, mesh, box_xf(p.center, p.size, p.yaw)); }
        }
        let scale = self.geo.scale;
        let wall_outer=self.geo.wall_r()+1.1*scale;
        let wall_top = self.geo.wall_h() + 0.6 * scale;
        let wall_height = wall_top - FOUNDATION_Y;
        let wall_mid = (wall_top + FOUNDATION_Y) * 0.5;
        self.stands.add(scene,self.wall_stone[quarry],Affine3A::from_scale_rotation_translation(Vec3::new(wall_outer,wall_height,wall_outer),Quat::IDENTITY,Vec3::Y*wall_mid));
        for i in 0..ROOF_STEPS {
            let angle=TAU*(i as f32+0.5)/ROOF_STEPS as f32;
            let point=|a:f32| Vec3::new(a.cos()*sq(a),0.0,a.sin()*sq(a))*wall_outer;
            let outer=point(angle); let inner=outer*0.9795;
            let tangent=point(angle+TAU/ROOF_STEPS as f32*0.5)-point(angle-TAU/ROOF_STEPS as f32*0.5);
            sky_boxes.push(Affine3A::from_cols(tangent.into(),(Vec3::Y*wall_height).into(),(outer-inner).into(),((outer+inner)*0.5+Vec3::Y*wall_mid).into()));
        }
        // A single watertight rounded ring, with no overlapping plate ends.
        let (inner, base) = (self.geo.inner_r, self.geo.base_y);
        let wall_h=self.geo.wall_h();
        let radius=roof_radius(self.geo);
        self.roof.add(scene,self.roof_stone[quarry],Affine3A::from_scale_rotation_translation(Vec3::new(radius,scale,radius),Quat::IDENTITY,Vec3::Y*(wall_h+0.6*scale)));
        sky_boxes.extend(roof_proxies(self.geo));
        for i in (0..SEGMENTS).step_by(2) {
            let ang=Stadium::seg_ang()*(i as f32+0.5);
            let at=roof_point(ang,true)*Vec3::new(radius,scale,radius)+Vec3::Y*(wall_h+(0.6-0.7)*scale);
            self.stands.add(scene,self.lamp,box_xf(at,Vec3::new(1.6,0.55,0.55)*scale,-ang+PI*0.5));
        }
        if let Some((offset, grid)) = scene.irradiance_volume {
            let mesh=&scene.meshes[self.stone[quarry]];
            let material=scene.clusters[mesh.first_cluster as usize].material as usize;
            let reflectance=Vec3::from(scene.materials[material].tint).dot(Vec3::new(0.2126,0.7152,0.0722)).clamp(0.0,0.95);
            if sky_boxes != self.sky_boxes || reflectance != self.sky_reflectance {
                self.sky_reflectance=reflectance;
                self.sky_boxes = sky_boxes;
                if self.bake_in_background {
                    let (tx, rx) = std::sync::mpsc::channel();
                    let boxes = self.sky_boxes.clone();
                    let half=world.arena.size*0.5;
                    std::thread::spawn(move || { let _ = tx.send(bake_sky(grid, &boxes, reflectance, half)); });
                    // A newer arena replaces a bake still running.
                    self.sky_job = Some(rx);
                } else {
                    self.sky_job = None;
                    let bake = bake_sky(grid, &self.sky_boxes, reflectance, world.arena.size*0.5);
                    self.apply_sky(scene, offset, grid, bake);
                }
            }
        }
        // Floodlight towers on the four corners, banks leaning over the bowl.
        self.lights.clear();
        self.beams.clear();
        let tower_h = wall_h + 9.0 * scale;
        for i in 0..4 {
            let ang = TAU * (i as f32 + 0.5) / 4.0;
            let dir = Vec3::new(ang.cos(), 0.0, ang.sin());
            let r = (inner + ROWS as f32 * self.geo.row_depth() * 0.72) * sq(ang);
            self.stands.add(scene, self.shaft, box_xf(dir * r + Vec3::Y * tower_h * 0.5, Vec3::new(2.0 * scale, tower_h, 2.0 * scale), 0.0));
            let at = dir * (r - 2.2 * scale) + Vec3::Y * (tower_h + 1.2 * scale);
            let aim = Vec3::Y * (base * 0.65);
            let face = Quat::from_rotation_arc(Vec3::Z, (aim - at).normalize());
            self.stands.add(scene, self.bank, Affine3A::from_scale_rotation_translation(Vec3::new(6.0, 3.2, 0.6) * scale, face, at));
            // Emit just in front of the bank's 0.6 m thick housing. An
            // emitter at its centre lets that housing occlude its own cone.
            let emitter = at + face * Vec3::Z * 0.4 * scale;
            // Cover the opposite bowl, with headroom before the range fade.
            // The old radius + 30 m cut the beam off near midfield.
            let range = ((emitter.length() + self.geo.wall_r() * sq(PI * 0.25)) / 0.70).min(500.0);
            self.lights.push(LocalLight { position: emitter, direction: (aim - emitter).normalize(), intensity_cd: Vec3::new(1.0, 0.96, 0.86) * 5.0 * 37_000.0,
                range_m: range, inner_cos: (30f32).to_radians().cos(), outer_cos: (38f32).to_radians().cos(), source_radius_m: 1.0, falloff: 1.0 });
            // Three lamps across the bank, fanned over the field: the beams one
            // sees in the night air. Same intensity and falloff as the flood.
            let flood = *self.lights.last().unwrap();
            let right = flood.direction.cross(Vec3::Y).normalize();
            for k in 0..3 {
                let side = k as f32 - 1.0;
                let turn = Quat::from_axis_angle(Vec3::Y, side * 0.34) * Quat::from_axis_angle(right, if k == 1 { -0.1 } else { 0.05 });
                self.beams.push(LocalLight { position: flood.position + right * side * 2.0, direction: turn * flood.direction,
                    inner_cos: 2.5f32.to_radians().cos(), outer_cos: 6.5f32.to_radians().cos(), source_radius_m: 0.5, ..flood });
            }
        }
        // The original boards are raised enough to see over the canopy.
        // The housing follows the tilted face; the vertical mounts stop below
        // its bottom edge rather than crossing the picture.
        for i in 0..TV_COUNT {
            let (at, rot) = tv_pose(self.geo, i);
            self.tv.add(scene, self.tv_mesh[0], Affine3A::from_scale_rotation_translation(
                Vec3::new(TV_WIDTH, TV_HEIGHT, 1.0) * scale, rot, at));
            self.tv.add(scene, self.tv_mesh[1], Affine3A::from_scale_rotation_translation(
                Vec3::new(TV_WIDTH + 1.2, TV_HEIGHT + 5.0, 0.8) * scale, rot,
                at + rot * Vec3::new(0.0, -1.9, -0.45) * scale));
            let dir = Vec3::new(at.x, 0.0, at.z).normalize();
            let tangent = Vec3::new(-dir.z, 0.0, dir.x);
            let leg_top = at.y - (TV_HEIGHT * 0.5 + 0.4) * scale;
            // Keep the original anchored foot when sightline clearance raises
            // the board; the extra span belongs to the narrow support legs.
            let leg_height = 16.0 * scale + at.y - (self.geo.wall_h() + TV_HEIGHT * 0.62 * self.geo.scale);
            for side in [-1.0, 1.0] {
                let leg = dir * (self.geo.wall_r() - 1.05 * scale) + tangent * (side * TV_WIDTH * 0.3 * scale)
                    + Vec3::Y * (leg_top - leg_height * 0.5);
                self.tv.add(scene, self.tv_mesh[1], box_xf(leg, Vec3::new(0.9, leg_height, 0.9), 0.0));
            }
        }
        let seats = seat_layout(&mut self.rng, self.geo, &self.looks);
        self.crowd.set_seats(scene, &seats, Vec2::ZERO);
        #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
        if crate::warp_humans_enabled() {
            self.crowd.set_humans(a.seed, &seats);
        }
        self.sound.kickoff();
    }

    /// The crowd reacts to this tick's events (Godot CrowdAudio +
    /// ColosseumCrowd): kills roar, hurts "ooh", blasts in the arena thrill,
    /// blasts and shots reaching the stands kill and terrify.
    #[allow(clippy::too_many_arguments)]
    pub fn absorb(&mut self, scene: &mut Scene, world: &World, ears: &[Listener], now: f32, dt: f32) {
        if !self.enabled { return; }
        for e in &world.events {
            match *e {
                Event::Hit { .. } => self.sound.hit(),
                Event::Kill { .. } => self.sound.roar(),
                Event::RoundOver { winner: Some(_) } => self.sound.celebrate(),
                Event::Explosion { pos, radius, .. } => {
                    let reach = radius * 1.2 + 1.5;
                    let d = self.geo.distance_to_crowd(pos);
                    if d <= reach {
                        let proximity = 1.0 - 0.5 * (d / reach);
                        self.sound.scare((0.3 + radius * 0.03) * proximity, self.geo.ring_point_toward(pos).to_array(), ears, 1.0, 0.45);
                        self.blast(scene,pos,reach*0.5,now);
                    } else {
                        self.sound.excite((0.05 + radius * 0.012).clamp(0.0, 0.3));
                    }
                }
                Event::Impact { pos, .. } if self.geo.distance_to_crowd(pos) <= 0.5 => {
                    self.sound.scare(0.09, self.geo.ring_point_toward(pos).to_array(), ears, 0.55, 1.0);
                }
                Event::TrackedBulletPath { id, from, to, .. } => self.trace(from, to, id, now),
                Event::BulletPath { from, to, .. } => self.trace(from, to, 0, now),
                _ => {}
            }
        }
        let at = self.geo.ring_point_toward(Vec3::new(self.rng.range(-1.0, 1.0), 0.0, self.rng.range(-1.0, 1.0)));
        self.sound.update(dt, Some((at.to_array(), ears)));
        let fear = if self.crowd.uses_humans() { self.sound.smoothed_fear() } else { self.sound.fear };
        self.crowd.mood(self.sound.enthusiasm, fear);
    }

    fn trace(&mut self, from: Vec3, to: Vec3, id: u64, now: f32) {
        // The whole clipped segment lies in the convex infield or outside the
        // vertical band. Include head height, near misses and displaced people.
        let inner = (self.geo.inner_r - 8.).max(0.);
        let outer = self.geo.wall_r() + 8.;
        if from.x.min(to.x) > outer
            || from.x.max(to.x) < -outer
            || from.z.min(to.z) > outer
            || from.z.max(to.z) < -outer
        {
            return;
        }
        if (from.xz().length_squared().max(to.xz().length_squared()) < inner * inner)
            || from.y.max(to.y) < self.geo.base_y - 4.
            || from.y.min(to.y) > self.geo.wall_h() + 5.
        {
            return;
        }
        self.crowd.bullet_path(from, to, id, now);
    }

    fn blast(&mut self, scene: &mut Scene, pos: Vec3, radius: f32, now: f32) {
        self.crowd.explosion(pos, radius, now);
        let outward = Vec3::new(pos.x, 0.0, pos.z).normalize_or(Vec3::X);
        let tangent = Vec3::Y.cross(outward);
        let slope = (outward * ROW_DEPTH + Vec3::Y * ROW_RISE).normalize();
        let reach = (radius * 0.5).clamp(1.0, 4.0);
        for i in 0..48 {
            let angle = i as f32 * 2.399963;
            let r = ((i as f32 + 0.5) / 48.0).sqrt() * reach;
            let p = pos + tangent * angle.cos() * r + slope * angle.sin() * r;
            let diameter = self.rng.range(0.45, 1.1) * (1.0 - 0.3 * r / reach);
            self.mark(scene, p, diameter, self.scorch);
        }
    }

    pub fn apply_hits(
        &mut self,
        scene: &mut Scene,
        fx: &mut crate::view::effects::EffectsView,
        now: f32,
    ) -> Result<(), String> {
        use growing_guns::shape::Boxes;
        use render::particles::Burst;
        for hit in self.crowd.take_hits()? {
            let n = hit.positions.len();
            for i in 0..n.min(32) {
                let p = hit.positions[i * n / n.min(32)];
                let travel = if hit.radius > 0. {
                    (p - hit.origin).normalize_or(Vec3::Y)
                } else {
                    hit.direction
                };
                // Project a fan from the contact. The same rays can reach several treads or a wall.
                for j in 0..if n == 1 { 7 } else { 3 } {
                    let scatter = Vec3::new(self.rng.range(-1., 1.), 0., self.rng.range(-1., 1.));
                    let dir = (travel * self.rng.range(0.5, 1.4) + scatter * 0.72
                        - Vec3::Y * if j == 0 { 2.2 } else { self.rng.range(0.25, 1.1) })
                    .normalize_or(Vec3::NEG_Y);
                    if let Some((d, normal, c)) = self.blood_grid.ray(p, dir, 6.) {
                        let size = self.rng.range(0.28, 0.83) * if j == 0 { 1.6 } else { 1. };
                        let seed = self.rng.below(1 << 24) as u32;
                        let xf = crate::blood::on_face(&self.blood_grid.boxes[c], p + dir * d, normal, dir, size, seed);
                        self.place_mark(scene, xf, self.blood.mesh(seed));
                    }
                }
            }
            // Skip expired transient effects on a replay seek; the persistent deaths and stains remain.
            if now - hit.time > 2.6 {
                continue;
            }
            let clouds = n.min(8);
            gibs::emit(scene, &mut fx.debris, &self.gore, &self.crowd, &hit);
            for i in 0..clouds {
                let p = hit.positions[i * n / clouds];
                let travel = if hit.radius > 0. {
                    (p - hit.origin).normalize_or(Vec3::Y)
                } else {
                    hit.direction
                };
                let dir = (travel + Vec3::Y * 0.38).normalize_or(Vec3::Y);
                fx.particles.burst(
                    scene,
                    p,
                    dir,
                    &Burst {
                        mesh: self.gore.cut[0],
                        count: 4,
                        spread: 0.94,
                        speed: [2.2, 6.4],
                        size: [0.072, 0.045],
                        life: [0.7, 1.35],
                        gravity: 16.,
                        stretch: 0.17,
                        ..Burst::drops()
                    },
                );
                fx.particles.burst(
                    scene,
                    p,
                    dir,
                    &Burst {
                        mesh: fx.meshes.blood,
                        count: 6,
                        spread: 0.66,
                        speed: [4., 9.5],
                        size: [0.045, 0.021],
                        life: [0.5, 1.1],
                        gravity: 15.,
                        stretch: 0.22,
                        ..Burst::drops()
                    },
                );
            }
            let center = hit.positions.iter().copied().sum::<Vec3>() / n as f32;
            if hit.radius > 0. {
                fx.blasts
                    .blood_mist(hit.time, center, (hit.radius * 0.75).clamp(0.45, 6.), n);
            } else {
                fx.blasts.gib_mist(hit.time, center, 0.6);
            }
        }
        Ok(())
    }

    pub fn gore_mesh(&self) -> Option<usize> {
        self.enabled.then_some(self.gore.cut[0])
    }

    /// A flat mark on the stands' slope (visible from the arena floor).
    fn mark(&mut self, scene: &mut Scene, p: Vec3, diameter: f32, mesh: usize) {
        let Some((at, normal, radius)) = mark_surface(&self.mark_surfaces, p, diameter * 0.5) else { return; };
        let rot = Quat::from_rotation_arc(Vec3::Y, normal) * Quat::from_rotation_y(self.rng.f() * TAU);
        let xf = Affine3A::from_scale_rotation_translation(Vec3::new(radius, 1.0, radius), rot, at);
        self.place_mark(scene,xf,mesh);
    }

    fn place_mark(&mut self,scene:&mut Scene,xf:Affine3A,mesh:usize){
        if self.mark_ring.len() < MARKS {
            if let Some(id) = self.marks.add(scene, mesh, xf) { self.mark_ring.push(id); }
        } else {
            let id = self.mark_ring[self.mark_next % MARKS];
            self.mark_next += 1;
            self.marks.set(scene, id, mesh, xf);
        }
    }

    fn apply_sky(&mut self, scene: &mut Scene, offset: usize, grid: render::irradiance_volume::Grid, bake: SkyBake) {
        self.sky_probes = bake.probes;
        self.sky_emission = bake.emission;
        self.sky_planes = bake.planes;
        let mut words = render::irradiance_volume::pack(&grid, &self.sky_probes).unwrap();
        words[7] = 2;
        scene.aux[offset..offset + words.len()].copy_from_slice(&words);
        self.sky_sun = None;
        self.sky_dirty = true;
        eprintln!("stadium sky cache: {} boxes, {} probes, {:.1} ms bake", self.sky_boxes.len(), self.sky_probes.len(), bake.ms);
    }

    /// Take a finished background sky bake (`bake_in_background`).
    pub fn poll_sky(&mut self, scene: &mut Scene) {
        let Some(rx) = &self.sky_job else { return };
        match rx.try_recv() {
            Ok(bake) => {
                self.sky_job = None;
                if let Some((offset, grid)) = scene.irradiance_volume { self.apply_sky(scene, offset, grid, bake); }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => { eprintln!("stadium sky bake failed"); self.sky_job = None; }
        }
    }

    pub fn invalidate_lighting(&mut self) { self.sky_dirty=true; }

    fn upload_sun(&mut self, gpu: &render::Gpu, renderer: &render::Renderer, upload: SunUpload) -> Result<(), String> {
        let start=std::time::Instant::now();
        let result=match upload {
            SunUpload::Packed(words)=>renderer.set_packed_irradiance_volume(gpu,&words),
            SunUpload::Probes(probes) if self.sky_planes.is_empty()=>renderer.set_sky_sun(gpu,&probes),
            SunUpload::Probes(probes)=>renderer.set_sky_sun_separated(gpu,&probes,&self.sky_planes),
        };
        crate::play::hitch("stadium light upload",start);
        result
    }

    pub fn lights(&self) -> &[LocalLight] { &self.lights }
    /// Scatter-only beams (`render::Renderer::set_volumetric_beams`).
    pub fn beams(&self) -> &[LocalLight] { &self.beams }

    pub fn flush(&mut self, gpu: &render::Gpu, renderer: &render::Renderer, scene: &Scene, now: f32, light: &render::Lighting) -> Result<(), String> {
        if !self.enabled { return Ok(()); }
        if let Some((_,grid))=scene.irradiance_volume {
            if let Some((sun, rx)) = &self.sun_job {
                match rx.try_recv() {
                    Ok(probes) => {
                        let sun = *sun;
                        self.sun_job = None;
                        self.upload_sun(gpu, renderer, probes?)?;
                        self.sky_sun = Some(sun);
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => { self.sun_job = None; return Err("stadium sun bake failed".into()); }
                }
            }
            // Wait for the sky bake: the sun return builds on its probes.
            if self.sky_job.is_none() && self.sun_job.is_none() && (self.sky_dirty || self.sky_sun!=Some(light.sun_dir)) {
                self.sky_dirty=false;
                let (boxes, sky, reflectance, sun) = (self.sky_boxes.clone(), self.sky_probes.clone(), self.sky_reflectance, light.sun_dir);
                // RGB9E5 packing performs hundreds of thousands of log/pow
                // operations. Finish it in the worker, not on the draw thread.
                let emission=(std::env::var("GG_LAVA_BOUNCE").as_deref()!=Ok("0")).then(||self.sky_emission.clone());
                let planes=self.sky_planes.clone();
                let bake = move || -> Result<SunUpload,String> {
                    let start=std::time::Instant::now();
                    let probes=bake_sun_cached(grid,&boxes,&sky,reflectance,sun)?;
                    let upload=if let Some(emission)=emission {
                        SunUpload::Packed(render::irradiance_volume::pack_sky_emission(&grid,&probes,&emission,&planes)?)
                    } else { SunUpload::Probes(probes) };
                    eprintln!("stadium sun return: {:.1} ms bake+pack",start.elapsed().as_secs_f64()*1000.0);
                    Ok(upload)
                };
                if self.bake_in_background {
                    let (tx, rx) = std::sync::mpsc::channel();
                    std::thread::spawn(move || { let _ = tx.send(bake()); });
                    self.sun_job = Some((sun, rx));
                } else {
                    self.upload_sun(gpu, renderer, bake()?)?;
                    self.sky_sun = Some(sun);
                }
            }
        }
        self.stands.flush(gpu, renderer, scene)?;
        self.roof.flush(gpu, renderer, scene)?;
        self.tv.flush(gpu, renderer, scene)?;
        self.marks.flush(gpu, renderer, scene)?;
        self.crowd.flush(gpu, renderer, now)
    }

    /// For state dumps.
    pub fn geometry_counts(&self, scene: &Scene) -> serde_json::Value {
        if self.crowd.uses_humans() {
            return serde_json::json!({"renderer": "warp humans", "seated": self.crowd.len(),
                "alive": self.crowd.alive(), "detail_selection": "per view"});
        }
        let (mut source, mut root, mut clusters) = (0usize, 0usize, 0usize);
        for &(body, arms) in &self.looks {
            let mut counts = [0usize; 3];
            for mesh in [body, arms] {
                let m = &scene.meshes[mesh];
                for c in &scene.clusters[m.first_cluster as usize..(m.first_cluster + m.cluster_count) as usize] {
                    if c.own_error == 0.0 { counts[0] += c.tri_count as usize; }
                    if c.parent_error > 1e30 { counts[1] += c.tri_count as usize; }
                    counts[2] += 1;
                }
            }
            source = source.max(counts[0]); root = root.max(counts[1]); clusters = clusters.max(counts[2]);
        }
        serde_json::json!({"seated": self.crowd.len(), "alive": self.crowd.alive(),
            "instances_per_person": 2, "shared_looks": self.looks.len(),
            "max_source_triangles_per_person": source, "max_root_triangles_per_person": root,
            "max_clusters_per_person": clusters, "alive_source_triangle_upper_bound": self.crowd.alive() * source})
    }

    /// For state dumps.
    pub fn state(&self) -> serde_json::Value {
        let tv_positions: [[f32; 3]; TV_COUNT] = std::array::from_fn(|i| tv_pose(self.geo, i).0.to_array());
        serde_json::json!({ "spectators": self.crowd.len(), "alive": self.crowd.alive(), "enthusiasm": self.sound.enthusiasm, "fear": self.sound.fear,
            "inner_r": self.geo.inner_r, "base_y": self.geo.base_y,
            "tv": { "screens": self.tv.len() / 4, "source_view": 0, "aspect": TV_WIDTH / TV_HEIGHT, "positions": tv_positions } })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_crowd_fills_every_sector_within_the_reserved_pool() {
        let looks = [(0, 0); (CROWD_COLORS.len()+CROWD_ACCENTS.len())*SHADES.len()];
        for width in [44.0, 72.0, 96.0] {
            let geo = Stadium::for_arena(width, true, growing_guns::arena::LEVEL_SCALE);
            let layout = |density| seats_at_density(&mut stadium_rng(52).0, geo, &looks, density);
            let standard = layout(Density::Standard);
            let packed = layout(Density::Packed);
            assert!(packed.len() >= standard.len(), "width {width}: {} vs {}", packed.len(), standard.len());
            if width == 44.0 {
                assert!(packed.len() > standard.len() * 7 / 5);
            }
            assert!(packed.len() <= CROWD_MAX);
            let mut sectors = [0; SEGMENTS];
            for seat in &packed {
                let angle = seat.pos.z.atan2(seat.pos.x).rem_euclid(TAU);
                let index = (angle / Stadium::seg_ang()) as usize % SEGMENTS;
                sectors[index] += 1;
            }
            assert!(sectors.iter().all(|n| *n > 50));
        }
    }

    #[test]
    fn scale_playtest_stands_at_eye_level_on_a_real_tread() {
        use growing_guns::shape::Boxes;
        let mut world = World::new(52, &[false, true, true, true]);
        start_in_stands(&mut world).unwrap();
        let player = &world.players[0];
        let feet = player.pos.y - growing_guns::HALF_HEIGHT;
        let (down, _, _) = world.arena.grid.ray(player.pos, Vec3::NEG_Y, 2.0).unwrap();
        assert!((player.pos.y - down - feet).abs() < 0.01);
        assert!((player.eye().y - feet - 1.60).abs() < 0.01);
        let seats = review_seats(&world);
        let neighbour = seats.iter().find(|s| {
            let delta = s.pos - player.pos;
            let ahead = growing_guns::look_dir(player.yaw, 0.0);
            delta.dot(ahead) > 1.0 && delta.dot(ahead) < 2.1
                && delta.cross(ahead).y.abs() < 0.05 && (s.pos.y - feet).abs() < 0.01
        }).unwrap();
        assert!((1.55..=1.95).contains(&neighbour.scale));
    }

    #[test]
    fn canopy_is_closed_with_no_overlapping_internal_faces() {
        use std::collections::HashMap;
        for mesh in [roof_mesh(0),ring_mesh(0,0.9795,0.0,1.0)] {
        assert_eq!(mesh.indices.len()/3,ROOF_STEPS*8);
        let mut edges=HashMap::new();
        let key=|p:[f32;3]| p.map(|v| (v*1_000_000.0).round() as i32);
        for t in mesh.indices.chunks_exact(3) {
            let p=t.iter().map(|&i| mesh.vertices[i as usize]).collect::<Vec<_>>();
            let cross=(Vec3::from(p[1].pos)-Vec3::from(p[0].pos)).cross(Vec3::from(p[2].pos)-Vec3::from(p[0].pos));
            assert!(cross.length_squared()>1e-16);
            assert!(cross.dot(Vec3::from(p[0].normal))>0.0);
            for e in 0..3 {let a=key(p[e].pos);let b=key(p[(e+1)%3].pos);*edges.entry(if a<b {(a,b)} else {(b,a)}).or_insert(0)+=1;}
        }
        assert!(edges.values().all(|&n| n==2),"each edge must have exactly two adjacent triangles");
        }
    }

    #[test]
    fn stadium_screens_face_inward_and_down_without_flipping_the_picture() {
        let geo = Stadium::new(80.0, true, 12.0);
        let picture = primitives::quad(0);
        for i in 0..TV_COUNT {
            let (at, rot) = tv_pose(geo, i);
            let xf = Affine3A::from_scale_rotation_translation(Vec3::new(TV_WIDTH, TV_HEIGHT, 1.0), rot, at);
            let vertices: Vec<Vec3> = picture.vertices.iter().map(|v| xf.transform_point3(Vec3::from(v.pos))).collect();
            let normal = (vertices[1] - vertices[0]).cross(vertices[2] - vertices[0]).normalize();
            let inward = -Vec3::new(at.x, 0.0, at.z).normalize();
            assert!(normal.dot(inward) > 0.97, "screen {i} must face the field");
            assert!(normal.y < -0.2 && normal.y > -0.23, "screen {i} must tilt down");
            assert!((vertices[3] - vertices[0]).y > TV_HEIGHT * 0.97, "screen {i} must keep the picture upright");
            assert!((normal.dot(Vec3::Y * 2.0 - at)) > 0.0, "screen {i} must show its front from the floor");
            assert!(((vertices[1] - vertices[0]).length() - TV_WIDTH).abs() < 1e-4);
            assert!(((vertices[3] - vertices[0]).length() - TV_HEIGHT).abs() < 1e-4);
            assert!(vertices.iter().all(|v| v.y > geo.wall_h() + 0.6), "screen {i} must clear the rim");
        }
    }

    #[test]
    fn stadium_screen_bottom_sightlines_clear_the_canopy_geometry() {
        // Trace several rays across each lower picture edge against the actual
        // rotated roof boxes, including segment joins and different bowls.
        fn crosses_unit_box(a: Vec3, b: Vec3) -> bool {
            let d = b - a;
            let (mut lo, mut hi) = (0.0f32, 1.0f32);
            for axis in 0..3 {
                if d[axis].abs() < 1e-6 {
                    if a[axis].abs() > 0.5 { return false; }
                } else {
                    let t0 = (-0.5 - a[axis]) / d[axis];
                    let t1 = (0.5 - a[axis]) / d[axis];
                    lo = lo.max(t0.min(t1));
                    hi = hi.min(t0.max(t1));
                    if lo > hi { return false; }
                }
            }
            true
        }
        for size in [45.0, 80.0, 120.0] {
            for wall_height in [0.0, 12.0, 24.0] {
                let geo = Stadium::new(size, wall_height > 0.0, wall_height);
                let roofs:Vec<_>=roof_proxies(geo).into_iter().map(|xf| xf.inverse()).collect();
                for screen in 0..TV_COUNT {
                    let (at, rot) = tv_pose(geo, screen);
                    for fraction in [-0.5, -0.25, 0.0, 0.25, 0.5] {
                        let bottom = at + rot * Vec3::new(TV_WIDTH * fraction, -TV_HEIGHT * 0.5, 0.0);
                        for roof in &roofs {
                            assert!(!crosses_unit_box(roof.transform_point3(Vec3::Y * TV_OBSERVER_HEIGHT), roof.transform_point3(bottom)),
                                "screen {screen}, width fraction {fraction}, arena {size}, wall {wall_height}: canopy blocks the picture");
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod mark_tests {
    use super::*;
    #[test]
    fn stains_lie_on_real_faces_and_fit_within_them() {
        let piece=growing_guns::stadium::Piece{center:Vec3::ZERO,size:Vec3::new(4.0,1.4,2.2),yaw:0.4,kind:PieceKind::Step};
        let rot=Quat::from_rotation_y(piece.yaw);
        for local in [Vec3::new(0.0,1.2,0.0),Vec3::new(0.0,0.0,-1.8),Vec3::new(1.98,1.0,0.98)] {
            let (at,n,r)=mark_surface(&[piece],rot*local,2.0).unwrap();
            let p=rot.conjugate()*(at-n*0.012);let n=rot.conjugate()*n;let half=piece.size*0.5;
            let axis=if n.y>0.5{1}else{2};assert!((p[axis].abs()-half[axis]).abs()<1e-5);
            for i in 0..3 {if i!=axis{assert!(p[i].abs()+r<=half[i]+1e-5);}}
        }
        assert!(stain_mesh(0).vertices.iter().all(|v|v.pos[1]==0.0));
    }
}
