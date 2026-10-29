//! The colosseum on screen and in the ears, after Godot's
//! `colosseum_builder.gd`, `colosseum_crowd.gd` and `crowd_audio.gd`: stepped
//! stone stands, a shadow-casting roof with a lamp ring, floodlight towers,
//! four live stadium screens, up to 15,000 spectators (`render::crowd`)
//! who cheer, cower and flee, die
//! where shots and blasts land (blood, scorch, gore, a red mist) and a crowd
//! sound that follows the match.
use crate::crowd_sound::CrowdSound;
use audio::sfx::Listener;
use glam::{Affine3A, Quat, Vec2, Vec3};
use growing_guns::stadium::{sq, Stadium, PieceKind, ROWS, ROW_DEPTH, ROW_RISE, SEGMENTS};
use growing_guns::{Event, World};
use render::crowd::{Crowd, Seat};
use render::headlights::LocalLight;
use render::particles::{Burst, Particles};
use render::primitives::{self, Primitive};
use render::cluster::ClusterMesh;
use render::scene::{Material, EMISSIVE, LIVE_SCREEN, NO_SHADOW};
use render::slots::{SlotId, Slots};
use render::Scene;
use std::f32::consts::{PI, TAU};

pub const CROWD_MAX: usize = 15_000;
const CROWD_SPACING: f32 = 0.9;
const CROWD_SKIP_CHANCE: f32 = 0.08;
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
fn roof_radius(geo: Stadium) -> f32 {geo.inner_r+ROWS as f32*ROW_DEPTH+1.0}
fn roof_proxies(geo: Stadium) -> Vec<Affine3A> {
    let radius=roof_radius(geo);
    (0..ROOF_STEPS).map(|i| {
        let a=TAU*(i as f32+0.5)/ROOF_STEPS as f32;
        let outer=roof_point(a,false)*Vec3::new(radius,1.0,radius)+Vec3::Y*(geo.wall_h()+0.6);
        let inner=roof_point(a,true)*Vec3::new(radius,1.0,radius)+Vec3::Y*(geo.wall_h()+0.6);
        let radial=outer-inner;
        let tangent=(roof_point(a+TAU/ROOF_STEPS as f32*0.5,false)-roof_point(a-TAU/ROOF_STEPS as f32*0.5,false))*Vec3::new(radius,1.0,radius);
        // Thin conservative static-light proxy follows the same roof curve.
        Affine3A::from_cols(tangent.into(),(Vec3::Y*0.5).into(),radial.into(),((outer+inner)*0.5).into())
    }).collect()
}
const MARKS: usize = 256;
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
    let radius = geo.wall_r() - 1.35;
    let half_height = TV_HEIGHT * 0.5;
    // Merely clearing the rim in world Y leaves the lower picture hidden by
    // the canopy's inner lip. Solve the eye-to-bottom-corner sightline over
    // the actual tilted roof's upper inner edge, with one metre of clearance.
    // sq(angle) >= 1, so the side-midpoint lip is the nearest possible roof.
    let lip_r=roof_radius(geo)*ROOF_INNER;
    let lip_y=geo.wall_h()+0.6-ROOF_DROP+0.25;
    let bottom_r = (radius + half_height * TV_TILT.sin()).hypot(TV_WIDTH * 0.5);
    let min_bottom_y = TV_OBSERVER_HEIGHT
        + (lip_y + TV_CANOPY_CLEARANCE - TV_OBSERVER_HEIGHT) * bottom_r / lip_r;
    let height = (geo.wall_h() + TV_HEIGHT * 0.62).max(min_bottom_y + half_height * TV_TILT.cos());
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

pub struct StadiumVisuals {
    sky_boxes: Vec<Affine3A>,
    sky_probes: Vec<render::irradiance_volume::Probe>,
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
    blood: usize,
    gore: usize,
    pub geo: Stadium,
    lights: Vec<LocalLight>,
    /// Narrow scatter-only beams inside each floodlight's cone, for the
    /// night's volumetric light (they light no surface).
    beams: Vec<LocalLight>,
    rng: Rng,
}

impl StadiumVisuals {
    /// Meshes and reserved instances; call before the renderer exists.
    pub fn reserve(scene: &mut Scene) -> Self {
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
                [(4, 8), (6, 1)].map(|(cols, rows)| scene.add_mesh(&render::masonry::panel(m, cols, rows, 11).mesh()))
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
        let scorch = { let m = mat(scene, [0.04, 0.035, 0.03], 1.0); scene.add_mesh(&primitives::sphere(m, 4, 10).mesh()) };
        let blood = { let m = mat(scene, [0.48, 0.04, 0.03], 0.4); scene.add_mesh(&primitives::sphere(m, 4, 10).mesh()) };
        let gore = { let m = mat(scene, [0.42, 0.03, 0.02], 0.5); scene.add_mesh(&primitives::hewn_block(m, 1, 0.2, 0.2, 9).mesh()) };
        let pieces = SEGMENTS * (ROWS + 4) + SEGMENTS / 2 + 16;
        let stands = Slots::reserve(scene, pieces);
        for i in 0..pieces { scene.instances[stands.first + i].flags |= NO_SHADOW; }
        let roof = Slots::reserve(scene, 1);
        let tv = Slots::reserve(scene, TV_COUNT * 4);
        for i in 0..tv.capacity() { scene.instances[tv.first + i].flags |= NO_SHADOW; }
        let marks = Slots::reserve(scene, MARKS);
        for i in 0..MARKS { scene.instances[marks.first + i].flags |= NO_SHADOW; }
        let mut crowd = Crowd::reserve(scene, CROWD_MAX);
        if detailed { crowd.arm_pivot(0.655); }
        StadiumVisuals { sky_boxes: Vec::new(), sky_probes: Vec::new(), sky_dirty: false, sky_reflectance: -1.0, sky_sun: None, stands, roof, tv, tv_mesh: [tv_picture, tv_housing], stone, masonry, roof_stone, wall_stone, lamp, shaft, bank, looks, crowd, sound: CrowdSound::new(None), marks, mark_ring: Vec::new(), mark_next: 0,
            scorch, blood, gore, geo: Stadium::new(80.0, true, 12.0), lights: Vec::new(), beams: Vec::new(), rng: Rng(1) }
    }

    /// Give the crowd its voice (recordings and a speaker).
    pub fn set_sound(&mut self, s: CrowdSound) { self.sound = s; }

    /// Build the bowl for this arena (a new round). Upload with `flush`.
    pub fn set_arena(&mut self, scene: &mut Scene, world: &World) {
        let a = &world.arena;
        self.geo = a.stadium;
        self.rng = Rng(a.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        self.stands.clear(scene);
        self.roof.clear(scene);
        self.tv.clear(scene);
        self.marks.clear(scene);
        self.mark_ring.clear();
        let quarry = if self.rng.f() < 0.5 { 0 } else { 3 } + self.rng.below(3);
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
        let wall_outer=self.geo.wall_r()+1.1;
        self.stands.add(scene,self.wall_stone[quarry],Affine3A::from_scale_rotation_translation(Vec3::new(wall_outer,self.geo.wall_h()+0.6,wall_outer),Quat::IDENTITY,Vec3::Y*(self.geo.wall_h()+0.6)*0.5));
        for i in 0..ROOF_STEPS {
            let angle=TAU*(i as f32+0.5)/ROOF_STEPS as f32;
            let point=|a:f32| Vec3::new(a.cos()*sq(a),0.0,a.sin()*sq(a))*wall_outer;
            let outer=point(angle); let inner=outer*0.9795;
            let tangent=point(angle+TAU/ROOF_STEPS as f32*0.5)-point(angle-TAU/ROOF_STEPS as f32*0.5);
            sky_boxes.push(Affine3A::from_cols(tangent.into(),(Vec3::Y*(self.geo.wall_h()+0.6)).into(),(outer-inner).into(),((outer+inner)*0.5+Vec3::Y*(self.geo.wall_h()+0.6)*0.5).into()));
        }
        // A single watertight rounded ring, with no overlapping plate ends.
        let (inner, base) = (self.geo.inner_r, self.geo.base_y);
        let wall_h=self.geo.wall_h();
        let radius=roof_radius(self.geo);
        self.roof.add(scene,self.roof_stone[quarry],Affine3A::from_scale_rotation_translation(Vec3::new(radius,1.0,radius),Quat::IDENTITY,Vec3::Y*(wall_h+0.6)));
        sky_boxes.extend(roof_proxies(self.geo));
        for i in (0..SEGMENTS).step_by(2) {
            let ang=Stadium::seg_ang()*(i as f32+0.5);
            let at=roof_point(ang,true)*Vec3::new(radius,1.0,radius)+Vec3::Y*(wall_h+0.6-0.7);
            self.stands.add(scene,self.lamp,box_xf(at,Vec3::new(1.6,0.55,0.55),-ang+PI*0.5));
        }
        if let Some((offset, grid)) = scene.irradiance_volume {
            let mesh=&scene.meshes[self.stone[quarry]];
            let material=scene.clusters[mesh.first_cluster as usize].material as usize;
            let reflectance=Vec3::from(scene.materials[material].tint).dot(Vec3::new(0.2126,0.7152,0.0722)).clamp(0.0,0.95);
            if sky_boxes != self.sky_boxes || reflectance != self.sky_reflectance {
                self.sky_reflectance=reflectance;
                self.sky_sun=None;
                let start = std::time::Instant::now();
                self.sky_probes = render::sky_occlusion::bake_diffuse(&grid, &sky_boxes, 96, reflectance).expect("valid stadium occluders");
                self.sky_boxes = sky_boxes;
                let mut words = render::irradiance_volume::pack(&grid, &self.sky_probes).unwrap();
                words[7] = 2;
                scene.aux[offset..offset + words.len()].copy_from_slice(&words);
                self.sky_dirty = true;
                eprintln!("stadium sky cache: {} boxes, {} probes, {:.1} ms bake", self.sky_boxes.len(), self.sky_probes.len(), start.elapsed().as_secs_f64()*1000.0);
            }
        }
        // Floodlight towers on the four corners, banks leaning over the bowl.
        self.lights.clear();
        self.beams.clear();
        let tower_h = wall_h + 9.0;
        for i in 0..4 {
            let ang = TAU * (i as f32 + 0.5) / 4.0;
            let dir = Vec3::new(ang.cos(), 0.0, ang.sin());
            let r = (inner + ROWS as f32 * ROW_DEPTH * 0.72) * sq(ang);
            self.stands.add(scene, self.shaft, box_xf(dir * r + Vec3::Y * tower_h * 0.5, Vec3::new(2.0, tower_h, 2.0), 0.0));
            let at = dir * (r - 2.2) + Vec3::Y * (tower_h + 1.2);
            let face = Quat::from_rotation_arc(Vec3::Z, -dir) * Quat::from_rotation_x(0.5);
            self.stands.add(scene, self.bank, Affine3A::from_scale_rotation_translation(Vec3::new(6.0, 3.2, 0.6), face, at));
            // Godot SpotLight3D: energy 5, 38 degrees, aimed at the centre.
            self.lights.push(LocalLight { position: at, direction: (-at).normalize(), intensity_cd: Vec3::new(1.0, 0.96, 0.86) * 5.0 * 37_000.0,
                range_m: (r + 30.0).min(200.0), inner_cos: (30f32).to_radians().cos(), outer_cos: (38f32).to_radians().cos(), source_radius_m: 1.0, falloff: 1.0 });
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
                Vec3::new(TV_WIDTH, TV_HEIGHT, 1.0), rot, at));
            self.tv.add(scene, self.tv_mesh[1], Affine3A::from_scale_rotation_translation(
                Vec3::new(TV_WIDTH + 1.2, TV_HEIGHT + 5.0, 0.8), rot,
                at + rot * Vec3::new(0.0, -1.9, -0.45)));
            let dir = Vec3::new(at.x, 0.0, at.z).normalize();
            let tangent = Vec3::new(-dir.z, 0.0, dir.x);
            let leg_top = at.y - TV_HEIGHT * 0.5 - 0.4;
            // Keep the original anchored foot when sightline clearance raises
            // the board; the extra span belongs to the narrow support legs.
            let leg_height = 16.0 + at.y - (self.geo.wall_h() + TV_HEIGHT * 0.62);
            for side in [-1.0, 1.0] {
                let leg = dir * (self.geo.wall_r() - 1.05) + tangent * (side * TV_WIDTH * 0.3)
                    + Vec3::Y * (leg_top - leg_height * 0.5);
                self.tv.add(scene, self.tv_mesh[1], box_xf(leg, Vec3::new(0.9, leg_height, 0.9), 0.0));
            }
        }
        self.seat(scene, base, inner);
        self.sound.kickoff();
    }

    /// Godot `_build_crowd`: one figure per 0.9 m of each row, a few empty
    /// seats, varied height, tempo and hop; 30% sit still.
    fn seat(&mut self, scene: &Scene, base: f32, inner: f32) {
        let seg = Stadium::seg_ang();
        let mut seats = Vec::new();
        'all: for i in 0..SEGMENTS {
            let ang = seg * (i as f32 + 0.5);
            for row in 0..ROWS {
                let y = base + row as f32 * ROW_RISE;
                let row_r = inner + (row as f32 + 0.45) * ROW_DEPTH;
                let count = (row_r * sq(ang) * seg / CROWD_SPACING) as usize;
                for k in 0..count {
                    if self.rng.f() < CROWD_SKIP_CHANCE { continue; }
                    let mut a = ang + (k as f32 + 0.5) / count as f32 * seg - seg * 0.5;
                    a += self.rng.range(-0.15, 0.15) / row_r.max(1.0);
                    let r = row_r * sq(a);
                    let pos = Vec3::new(a.cos() * r + self.rng.range(-0.15, 0.15), y, a.sin() * r + self.rng.range(-0.25, 0.25));
                    let colour = if self.rng.f() < 0.08 { CROWD_COLORS.len() + self.rng.below(CROWD_ACCENTS.len()) } else { self.rng.below(CROWD_COLORS.len()) };
                    let look = self.looks[colour * SHADES.len() + self.rng.below(SHADES.len())];
                    let hop = if self.rng.f() < 0.3 { 0.0 } else { self.rng.range(0.15, 0.42) };
                    seats.push(Seat { pos, yaw: -a - PI * 0.5, scale: self.rng.range(1.5, 1.85), phase: self.rng.f(), tempo: self.rng.f(), hop, body: look.0, arms: look.1 });
                    if seats.len() >= CROWD_MAX { break 'all; }
                }
            }
        }
        self.crowd.set_seats(scene, &seats, Vec2::ZERO);
    }

    /// The crowd reacts to this tick's events (Godot CrowdAudio +
    /// ColosseumCrowd): kills roar, hurts "ooh", blasts in the arena thrill,
    /// blasts and shots reaching the stands kill and terrify.
    #[allow(clippy::too_many_arguments)]
    pub fn absorb(&mut self, scene: &mut Scene, fx: &mut Particles, blasts: &mut render::fx::Effects, world: &World, ears: &[Listener], now: f32, dt: f32) {
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
                        let kills = self.blast(scene, fx, blasts, pos, reach * 0.5, now);
                        self.sound.fear = (self.sound.fear + kills as f32 * 0.01).clamp(0.0, 1.0);
                    } else {
                        self.sound.excite((0.05 + radius * 0.012).clamp(0.0, 0.3));
                    }
                }
                Event::Impact { pos, .. } if self.geo.distance_to_crowd(pos) <= 0.5 => {
                    self.sound.scare(0.09, self.geo.ring_point_toward(pos).to_array(), ears, 0.55, 1.0);
                    self.crowd.flee(pos, 1.2, now);
                    let dead = self.crowd.kill_near(pos, 1.7, 2);
                    for &p in &dead { let d = 1.0 + self.rng.f() * 0.5; self.mark(scene, p, d, self.blood); }
                    if !dead.is_empty() { self.gore(scene, fx, pos, dead.len()); }
                }
                _ => {}
            }
        }
        let at = self.geo.ring_point_toward(Vec3::new(self.rng.range(-1.0, 1.0), 0.0, self.rng.range(-1.0, 1.0)));
        self.sound.update(dt, Some((at.to_array(), ears)));
        self.crowd.mood(self.sound.enthusiasm, self.sound.fear);
    }

    /// A blast reaching the stands: everyone near bolts, everyone inside
    /// dies, scorch and blood marks, gore and a red mist.
    fn blast(&mut self, scene: &mut Scene, fx: &mut Particles, blasts: &mut render::fx::Effects, pos: Vec3, radius: f32, now: f32) -> usize {
        self.crowd.flee(pos, radius, now);
        let dead = self.crowd.kill_near(pos, radius, usize::MAX);
        self.mark(scene, pos, (radius * 1.2).clamp(2.5, 8.0), self.scorch);
        if !dead.is_empty() {
            let mag = (1.0 + dead.len() as f32 * 0.035).clamp(1.0, 2.0);
            for &p in dead.iter().take(22) { let s = self.rng.range(1.4, 2.4) * mag; self.mark(scene, p, s, self.blood); }
            self.gore(scene, fx, pos, dead.len());
            blasts.blood_mist(now, pos, (radius * 0.75).max(1.2), dead.len());
        }
        dead.len()
    }

    fn gore(&mut self, scene: &mut Scene, fx: &mut Particles, at: Vec3, kills: usize) {
        let n = (kills * 6).clamp(16, 120) as u32;
        fx.burst(scene, at, Vec3::Y, &Burst { mesh: self.gore, count: n, spread: 1.3, speed: [3.0, 9.0], size: [0.3, 0.2], life: [2.0, 2.6], ..Burst::debris() });
    }

    /// A flat mark on the stands' slope (visible from the arena floor).
    fn mark(&mut self, scene: &mut Scene, p: Vec3, diameter: f32, mesh: usize) {
        let flat = Vec2::new(p.x, p.z);
        let g = &self.geo;
        let r = (flat.length() / sq(p.z.atan2(p.x))).clamp(g.inner_r + 0.2, g.inner_r + ROWS as f32 * ROW_DEPTH - 0.2);
        let dir = Vec3::new(flat.x, 0.0, flat.y).normalize_or(Vec3::X);
        let pitch = ROW_RISE.atan2(ROW_DEPTH);
        let n = (Vec3::Y * pitch.cos() - dir * pitch.sin()).normalize();
        let s = sq(p.z.atan2(p.x));
        let y = g.base_y + (r - g.inner_r) / ROW_DEPTH * ROW_RISE;
        let at = Vec3::new(dir.x * r * s, y, dir.z * r * s) + n * 0.08;
        let rot = Quat::from_rotation_arc(Vec3::Y, n) * Quat::from_rotation_y(self.rng.f() * TAU);
        let xf = Affine3A::from_scale_rotation_translation(Vec3::new(diameter * 0.5, 0.02, diameter * 0.5 * self.rng.range(0.75, 1.0)), rot, at);
        if self.mark_ring.len() < MARKS {
            if let Some(id) = self.marks.add(scene, mesh, xf) { self.mark_ring.push(id); }
        } else {
            let id = self.mark_ring[self.mark_next % MARKS];
            self.mark_next += 1;
            self.marks.set(scene, id, mesh, xf);
        }
    }

    pub fn invalidate_lighting(&mut self) { self.sky_dirty=true; }

    pub fn lights(&self) -> &[LocalLight] { &self.lights }
    /// Scatter-only beams (`render::Renderer::set_volumetric_beams`).
    pub fn beams(&self) -> &[LocalLight] { &self.beams }

    pub fn flush(&mut self, gpu: &render::Gpu, renderer: &render::Renderer, scene: &Scene, now: f32, light: &render::Lighting) -> Result<(), String> {
        if let Some((_,grid))=scene.irradiance_volume {
            if self.sky_dirty || self.sky_sun!=Some(light.sun_dir) {
                let start=std::time::Instant::now();
                let probes=render::sky_occlusion::bake_sun(&grid,&self.sky_boxes,&self.sky_probes,96,self.sky_reflectance,light.sun_dir)?;
                renderer.set_sky_sun(gpu,&probes)?;
                self.sky_sun=Some(light.sun_dir);
                self.sky_dirty=false;
                eprintln!("stadium sun return: {:.1} ms bake",start.elapsed().as_secs_f64()*1000.0);
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
