//! Airstrike visuals: solid finned missile, nose-mounted targeting laser,
//! exhaust plumes anchored behind the nozzle, lights and lens flare.
use glam::{Affine3A, Quat, Vec3};
use growing_guns::{Strike, StrikeKind, AIR_STRIKE_TRAVEL};
use render::headlights::LocalLight;
use render::picture::Picture;
use render::sprites::{Sprite, SpriteImage};
use render::Scene;

pub(super) const SKY_OFFSET: Vec3 = Vec3::new(0.0, 263.0, 93.0);
const MAX_STRIKES: usize = 32;
const ROCKET_LENGTH_SCALE: f32 = 1.65;
const NOSE_TIP: f32 = 2.4 * ROCKET_LENGTH_SCALE;
const NOZZLE_END: f32 = 2.2 * ROCKET_LENGTH_SCALE;
pub(super) const IMAGE: SpriteImage = SpriteImage(0x6c61_7365);
/// The sprite's full width; Godot's core is 3.5 cm and its sheath 8.5 cm.
const WIDTH: f32 = 0.3;
const CORE: f32 = 0.0132;
const SHEATH: f32 = 0.03;
const GLOW: f32 = 0.096;
/// Godot `MARKER_COLOR` (linear) for the sheath and glow.
const RED: [f32; 3] = [1.0, 0.08, 0.04];
/// Peak radiance: very bright (Godot core emission 16). The sprite shader
/// limits what reaches the fp16 target, so the white-hot core saturates up
/// close while a distant laser, a pixel or two wide, still burns bright.
const NITS: f32 = 160_000.0;
/// The laser's glow never narrows below this on screen, so far up it still
/// reads as a burning line.
const MIN_PIXELS: f32 = 6.0;

pub(super) const PLUME_IMAGE: SpriteImage = SpriteImage(0x706c_756d);
/// Radiance per unit of Godot emission energy, as for the laser (16 ->
/// 160 000 nits); the sprite shader caps what reaches the fp16 target.
const NITS_PER_ENERGY: f32 = 10_000.0;
/// The plumes at that scale burn out to white in daylight; this keeps the
/// orange plume and halo orange around a white-hot core, as Godot's
/// tonemapped glow reads.
const PLUME_GAIN: f32 = 0.25;
/// Godot light energy to candela (`render::fx::Effects::light_scale`).
const CD_PER_ENERGY: f32 = 37_000.0;

pub(super) fn active(strike: &Strike) -> bool {
    strike.kind == StrikeKind::AirStrike && strike.target.is_finite()
        && strike.age.is_finite() && (0.0..AIR_STRIKE_TRAVEL).contains(&strike.age)
}

/// The laser's texture: across, a white core, a red sheath and a faint red
/// glow (Gaussian, windowed to the edge); the same along its length.
pub(super) fn picture() -> Picture {
    let (w, h) = (256u32, 4u32);
    let half = WIDTH * 0.5;
    let mut row = Vec::with_capacity(w as usize * 4);
    for x in 0..w {
        let d = ((x as f32 + 0.5) / w as f32 * 2.0 - 1.0) * half;
        let g = |s: f32| (-(d / s).powi(2)).exp();
        let window = (1.0 - (d / half).powi(2)).max(0.0).powi(2);
        let (core, sheath, glow) = (g(CORE), 0.5 * g(SHEATH), 0.3 * g(GLOW));
        // Premultiplied light: white from the core, red from the rest.
        let rgb = [0, 1, 2].map(|c| (core + (sheath + glow) * RED[c]) * window);
        let scale = rgb.iter().cloned().fold(0.0f32, f32::max).max(1e-6);
        // A picture is straight sRGB with alpha: alpha carries the light's
        // brightest channel, the colour its hue.
        let col = rgb.map(|v| ((v / scale).max(0.0).powf(1.0 / 2.2) * 255.0 + 0.5) as u8);
        row.extend([col[0], col[1], col[2], (scale.min(1.0) * 255.0 + 0.5) as u8]);
    }
    Picture::new(w, h, row.repeat(h as usize)).expect("fixed size")
}

/// A falling rocket: where it is, its unit flight direction and
/// `approach`, u² of the flight fraction u (Godot's swell).
struct Flight { pos: Vec3, dir: Vec3, approach: f32 }

/// Godot tweens the rocket along the straight path with a quadratic ease-in.
fn flight(s: &Strike) -> Option<Flight> {
    if !active(s) { return None; }
    let u = (s.age / AIR_STRIKE_TRAVEL) as f32;
    let sky = s.target + SKY_OFFSET;
    Some(Flight { pos: sky.lerp(s.target, u * u), dir: (s.target - sky).normalize(), approach: u * u })
}

/// Fixed-tick emission; old smoke remains in world space after impact.
pub(in crate::view) fn smoke(strikes: &[Strike], tick: u64, now: f32, fx: &mut render::fx::Effects) {
    if tick % 2 != 0 { return; }
    for s in strikes.iter().filter(|s| active(s)).take(4) {
        let Some(f)=flight(s) else {continue};
        let old=Strike {age:(s.age-2.0*growing_guns::DT64).max(0.0),..s.clone()};
        let prev=flight(&old).unwrap();
        let count=((f.pos-prev.pos).length()/3.0).ceil().clamp(1.0,3.0) as usize;
        for n in 0..count {
            let t=(n as f32+0.5)/count as f32;
            let at=prev.pos.lerp(f.pos,t)-f.dir*(NOZZLE_END+6.0);
            fx.exhaust_smoke(now,at,5.0);
        }
    }
}

/// Smooth random engine surges at 24 Hz, deterministic for capture/replay.
fn flicker(t: f32, seed: f32) -> f32 {
    let x=t*24.0; let i=x.floor();let u=x-i;let u=u*u*(3.0-2.0*u);
    let hash=|n:f32| ((n*127.1+seed*31.7).sin()*43758.5453).fract().abs();
    0.55+1.15*(hash(i)+(hash(i+1.0)-hash(i))*u)
}

/// The strikes as drawn `behind` seconds before the world's tick (the
/// effects clock trails the simulation by part of a tick): at 240 m/s the
/// rocket would otherwise jump 4 m per tick and judder at 120 Hz.
pub(super) fn drawn(strikes: &[Strike], behind: f32) -> Vec<Strike> {
    strikes.iter().map(|s| Strike { age: (s.age - behind.clamp(0.0, growing_guns::DT) as f64).max(0.0), ..s.clone() }).collect()
}

/// A solid finned missile, with only the exhaust throat emissive.
pub(super) struct Rocket { mesh: usize }

impl Rocket {
    pub fn new(scene: &mut Scene) -> Self {
        use render::{primitives::{self,Primitive}, cluster::{ClusterMesh,Vertex}};
        let shell=super::material(scene,[0.64,0.67,0.56],0.55);
        let dark=super::material(scene,[0.12,0.14,0.13],0.4);
        let band=super::material(scene,[0.85,0.27,0.07],0.6);
        let exhaust=super::glow(scene,[1.0,0.52,0.12],30000.0);
        let mut parts=Vec::new();
        let mut cylinder=|m,top,bottom,y,height| {
            let mut p=primitives::cylinder(m,top,bottom,24);
            for v in &mut p.vertices {
                v.pos[1]=v.pos[1]*height+y;
                let n=Vec3::from(v.normal)/Vec3::new(1.0,height,1.0);
                v.normal=n.normalize().to_array();
            }
            parts.push(p);
        };
        cylinder(shell,0.48,0.48,0.0,3.2);
        cylinder(dark,0.0,0.48,2.0,0.8);
        cylinder(band,0.492,0.492,1.15,0.22);
        cylinder(dark,0.49,0.34,-1.72,0.25);
        cylinder(dark,0.27,0.40,-2.0,0.35);
        cylinder(exhaust,0.32,0.32,-2.18,0.025);
        // Four swept tail fins, with actual thickness and flat face normals.
        let outline=[(0.38,-0.45),(1.15,-1.65),(1.15,-1.85),(0.38,-1.65)];
        for k in 0..4 {
            let rot=Quat::from_rotation_y(k as f32*std::f32::consts::FRAC_PI_2);
            let mut p=Primitive {material:dark,..Default::default()};
            let vertices:Vec<_>=[-0.05,0.05].into_iter().flat_map(|z| outline.map(|(x,y)|Vec3::new(x,y,z))).collect();
            for ids in [[0,2,1],[0,3,2],[4,5,6],[4,6,7],[0,1,5],[0,5,4],[1,2,6],[1,6,5],[2,3,7],[2,7,6],[3,0,4],[3,4,7]] {
                let [a,b,c]=ids.map(|i|rot*vertices[i]);
                let normal=(b-a).cross(c-a).normalize().to_array();
                for pos in [a,b,c] {p.indices.push(p.vertices.len() as u32);p.vertices.push(Vertex {pos:pos.to_array(),normal,uv:[0.0,0.0]});}
            }
            parts.push(p);
        }
        for part in &mut parts {
            for v in &mut part.vertices {
                v.pos[1] *= ROCKET_LENGTH_SCALE;
                v.normal=(Vec3::from(v.normal)/Vec3::new(1.0,ROCKET_LENGTH_SCALE,1.0)).normalize().to_array();
            }
        }
        let mesh=scene.add_mesh(&ClusterMesh::build(&parts.iter().map(Primitive::part).collect::<Vec<_>>()));
        Self {mesh}
    }

    pub fn parts(&self, strikes: &[Strike]) -> Vec<(usize, Affine3A)> {
        strikes.iter().filter_map(flight).take(MAX_STRIKES).map(|f| {
            (self.mesh,Affine3A::from_rotation_translation(Quat::from_rotation_arc(Vec3::Y,f.dir),f.pos))
        }).collect()
    }
}

/// Three additive cones behind the engine (`EnginePlume`, `EngineCorePlume`,
/// `EngineHaloPlume`): Godot's lengths, offsets, widest radius and colour,
/// and its emission ramps; the main plume stretches 0.85 -> 2.4 times.
pub(super) fn plumes(strikes: &[Strike], clock: f32) -> Vec<Sprite> {
    let pulse = (clock * 14.0).sin();
    strikes.iter().filter_map(flight).take(MAX_STRIKES).flat_map(|f| {
        let a = f.approach;
        let lerp = |x: f32, y: f32| x + (y - x) * a;
        [
            // (centre behind the rocket, length, widest radius, colour, energy)
            (-6.2, 6.8 * lerp(0.85, 2.4), 0.82, [1.0, 0.58, 0.12], (lerp(110.0, 240.0) + pulse * 16.0) * lerp(0.55, 0.82)),
            (-4.6, 4.8, 0.34, [1.0, 0.94, 0.78], lerp(180.0, 360.0) + pulse * 20.0),
            (-7.0, 9.0, 1.35, [1.0, 0.42, 0.06], 70.0 * 0.14),
        ].map(|(_y, length, radius, color, energy): (f32, f32, f32, [f32; 3], f32)| {
            let mut sprite = Sprite::shaft(f.pos - f.dir * (NOZZLE_END + length * 0.5), [radius * 2.0, length], f.dir, PLUME_IMAGE, [0.0, 0.0, 1.0, 1.0], Vec3::from(color) * energy * NITS_PER_ENERGY * PLUME_GAIN * flicker(f.approach.sqrt()*AIR_STRIKE_TRAVEL as f32,f.pos.x));
            sprite.depth_bias = 0.0;
            // Far up the rocket is a few pixels across; like Godot's distance
            // boost (and its bloom), the glow keeps a readable width.
            sprite.min_pixels = 4.0 + 4.0 * radius;
            sprite
        })
    }).collect()
}

/// The cone across and along: a soft core whose width narrows from the
/// engine end (row 0, the shaft's top towards the rocket) to 22 % at the
/// tail, brightest at the engine and fading out at both ends.
pub(super) fn plume_picture() -> Picture {
    let (w, h) = (64u32, 128u32);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let v = (y as f32 + 0.5) / h as f32;
        let width = 1.0 + (0.22 - 1.0) * v;
        let along = (1.0 - v).powf(0.7) * (v / 0.06).min(1.0) * ((1.0 - v) / 0.25).min(1.0);
        for x in 0..w {
            let d = ((x as f32 + 0.5) / w as f32 * 2.0 - 1.0).abs() / width;
            let across = (-(d / 0.55).powi(2)).exp() * (1.0 - d.min(1.0)).powf(0.5);
            rgba.extend([255, 255, 255, ((across * along).clamp(0.0, 1.0) * 255.0 + 0.5) as u8]);
        }
    }
    Picture::new(w, h, rgba).expect("fixed size")
}

/// The engine lights (`EngineCoreLight` 2 m and `EngineLight` 3 m behind
/// the nose), rising 16 -> 34 and 22 -> 46 with the approach, and the red
/// marker light 35 cm over each target (`StrikeTargetLight`, energy 9, 7 m).
pub(super) fn lights(strikes: &[Strike], clock: f32) -> Vec<LocalLight> {
    let pulse = (clock * 14.0).sin();
    let markers = strikes.iter().filter(|s| active(s)).take(4).map(|s| LocalLight {
        position: s.target + Vec3::Y * 0.35, direction: Vec3::NEG_Y, intensity_cd: Vec3::from(RED) * 9.0 * CD_PER_ENERGY,
        range_m: 7.0, inner_cos: -0.99, outer_cos: -1.0, source_radius_m: 0.1, falloff: 1.0,
    });
    markers.chain(strikes.iter().filter_map(flight).take(4).flat_map(|f| {
        let a = f.approach;
        [(-2.0, [1.0, 0.82, 0.42], 16.0 + 18.0 * a + pulse * 3.0, 28.0), (-3.0, [1.0, 0.62, 0.16], 22.0 + 24.0 * a + pulse * 4.0, 44.0)]
            .map(|(y, color, energy, range): (f32, [f32; 3], f32, f32)| LocalLight {
                position: f.pos + f.dir * y, direction: Vec3::NEG_Y, intensity_cd: Vec3::from(color) * energy * CD_PER_ENERGY,
                range_m: range, inner_cos: -0.99, outer_cos: -1.0, source_radius_m: 0.3, falloff: 1.0,
            })
    })).collect()
}

/// One shaft from the target to the moving rocket nose. It shrinks during
/// descent instead of continuing behind the rocket to the original sky position.
pub(super) fn sprites(strikes: &[Strike]) -> Vec<Sprite> {
    let axis = SKY_OFFSET.normalize();
    strikes.iter().filter_map(|s| flight(s).map(|f| (s,f))).take(MAX_STRIKES).map(|(s,f)| {
        let length = ((f.pos-s.target).length()-NOSE_TIP).max(0.0);
        let mut sprite = Sprite::shaft(s.target + axis * length * 0.5, [WIDTH, length], axis, IMAGE, [0.0, 0.0, 1.0, 1.0], Vec3::splat(NITS));
        sprite.depth_bias = 0.0;
        sprite.min_pixels = MIN_PIXELS;
        sprite
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use render::sprites::Facing;

    fn strike(kind: StrikeKind, age: f64) -> Strike {
        Strike { kind, target: Vec3::new(4.0, 1.5, -8.0), owner: Some(0), age }
    }

    #[test]
    fn engine_surges_are_bounded_and_smoke_emission_is_capped() {
        let values:Vec<_>=(0..240).map(|i|flicker(i as f32/120.0,7.0)).collect();
        assert!(values.iter().all(|&v| (0.55..=1.70).contains(&v)));
        assert!(values.windows(2).any(|v|(v[1]-v[0]).abs()>0.05));
        let mut fx=render::fx::Effects::default();
        let strikes=vec![strike(StrikeKind::AirStrike,2.0);32];
        smoke(&strikes,121,2.0,&mut fx); assert_eq!(fx.len(),0);
        smoke(&strikes,122,2.0,&mut fx); assert!(fx.len()>0 && fx.len()<=12);
    }

    #[test]
    fn laser_ends_at_the_moving_rocket_nose() {
        for age in [0.0, AIR_STRIKE_TRAVEL*0.5, AIR_STRIKE_TRAVEL-0.01] {
        let strike = strike(StrikeKind::AirStrike,age);
        let f=flight(&strike).unwrap();
        let s = &sprites(&[strike.clone()])[0];
        let target = strike.target;
        let Facing::Axis { up } = s.facing else { panic!("a shaft") };
        assert!((s.center - up * s.size[1] * 0.5).abs_diff_eq(target, 0.001));
        let tip = if (f.pos-target).length()>NOSE_TIP { f.pos+f.dir*NOSE_TIP } else {target};
        assert!((s.center + up * s.size[1] * 0.5).abs_diff_eq(tip, 0.001));
        assert!(s.additive && s.size[0] <= 0.3);
        }

    }

    #[test]
    fn laser_lasts_until_airstrike_impact_and_never_marks_ion() {
        for age in [0.0, 0.5, AIR_STRIKE_TRAVEL - growing_guns::DT64] {
            assert_eq!(sprites(&[strike(StrikeKind::AirStrike, age), strike(StrikeKind::IonCannon, age)]).len(), 1);
        }
        for age in [-1.0, AIR_STRIKE_TRAVEL, AIR_STRIKE_TRAVEL + 1.0, f64::NAN] {
            assert!(sprites(&[strike(StrikeKind::AirStrike, age)]).is_empty());
        }
    }

    #[test]
    fn rocket_falls_nose_first_with_plumes_behind_and_lights_that_swell() {
        let early = strike(StrikeKind::AirStrike, 0.2);
        let late = strike(StrikeKind::AirStrike, AIR_STRIKE_TRAVEL - 0.1);
        let f = flight(&late).unwrap();
        assert!(f.dir.y < -0.9 && f.pos.y > late.target.y);
        let behind = plumes(&[late.clone()], 0.0);
        assert_eq!(behind.len(), 3);
        // Every plume sits behind the rocket (towards the sky).
        assert!(behind.iter().all(|p| (p.center - f.pos).dot(f.dir) < 0.0));
        assert!(behind.iter().all(|p| (p.center-f.pos).dot(f.dir)+p.size[1]*0.5 <= -NOZZLE_END+0.01), "exhaust must not engulf the body");
        let sum = |s: &Strike| lights(&[s.clone()], 0.0).iter().skip(1).map(|l| l.intensity_cd.x).sum::<f32>();
        assert!(sum(&late) > 1.8 * sum(&early));
        assert!(plumes(&[strike(StrikeKind::IonCannon, 1.0)], 0.0).is_empty());
        assert!(lights(&[strike(StrikeKind::AirStrike, AIR_STRIKE_TRAVEL)], 0.0).is_empty());
    }

    #[test]
    fn flare_shows_only_as_the_rocket_closes_in() {
        let cam = render::Camera { pos: Vec3::new(4.0, 3.0, 20.0), target: Vec3::new(4.0, 120.0, -8.0), fov_y: 1.2, near: 0.1 };
        let drawn = |age: f64| {
            let mut list = render::hud::HudDrawList::new([960, 540]);
            flare::draw(&mut list, [0.0, 0.0, 960.0, 540.0], &cam, &[strike(StrikeKind::AirStrike, age)]);
            !list.vertices.is_empty()
        };
        assert!(!drawn(0.3), "far up the flare is below Godot's 0.015 cut");
        assert!(drawn(AIR_STRIKE_TRAVEL - 0.15));
    }

    #[test]
    fn laser_core_is_white_and_its_edge_red() {
        let p = picture();
        let px = |x: u32| &p.rgba[(x * 4) as usize..(x * 4 + 4) as usize];
        let centre = px(p.width / 2);
        assert!(centre[0] > 240 && centre[1] > 200 && centre[3] > 240);
        let edge = px(p.width / 2 + p.width / 7);
        assert!(edge[0] > edge[1] * 3 && edge[3] < centre[3]);
    }
}

/// The lens flare on an incoming rocket (`lens_flare_overlay.gd` with
/// `shaders/lens_flare.gdshader`, a port of shadertoy 4sX3Rs): only its ghost
/// chain, aimed through the view's centre from the rocket's tail. The source
/// is the strongest rocket (intensity = closeness², closeness = u² here),
/// Strong warm source flare and streaks, with ghost gain 1.5.
/// Godot adds it; the HUD blends, so each ghost is a soft disc whose alpha is
/// the light it would add.
pub(crate) mod flare {
    use super::{flight, Strike, Vec3};
    use render::hud::{HudDrawList, HudImage, ImageFilter};
    use render::picture::Picture;
    use std::sync::{Arc, OnceLock};

    const TINT: [f32; 3] = [1.08, 0.88, 0.62];
    const GAIN: f32 = 0.18;
    /// One ghost group: positions along the axis (multiples of the source's
    /// offset from centre; negative is across the centre), per-channel
    /// weights and peak, the picture (its profile) and its radius in screen
    /// heights.
    struct Ghost { k: f32, weight: [f32; 3], peak: f32, image: HudImage, radius: f32 }
    const HALO: HudImage = HudImage(0x6767_f1a1);
    const DISC: HudImage = HudImage(0x6767_f1a2);
    const GHOSTS: [Ghost; 4] = [
        // f2: 1 / (1 + 32 d²) at -0.8/-0.85/-0.9, weights .25/.23/.21.
        Ghost { k: -0.85, weight: [0.25, 0.23, 0.21], peak: 1.0, image: HALO, radius: 0.6 },
        // f4: max(0.01 - d^2.4, 0) x 6/5/3 at -0.4/-0.45/-0.5.
        Ghost { k: -0.45, weight: [6.0, 5.0, 3.0], peak: 0.01, image: DISC, radius: 0.147 },
        // f5: max(0.01 - d^5.5, 0) x 2 at -0.2/-0.4/-0.6 (wide and faint).
        Ghost { k: -0.4, weight: [2.0, 2.0, 2.0], peak: 0.01, image: DISC, radius: 0.43 },
        // f6: max(0.01 - d^1.6, 0) x 6/3/5 on the source's side, 0.3-0.35.
        Ghost { k: 0.325, weight: [6.0, 3.0, 5.0], peak: 0.01, image: DISC, radius: 0.056 },
    ];

    fn picture(profile: impl Fn(f32) -> f32) -> Arc<Picture> {
        let n = 64u32;
        let mut rgba = Vec::with_capacity((n * n * 4) as usize);
        for y in 0..n { for x in 0..n {
            let d = Vec3::new((x as f32 + 0.5) / n as f32 * 2.0 - 1.0, (y as f32 + 0.5) / n as f32 * 2.0 - 1.0, 0.0).length();
            rgba.extend([255, 255, 255, (profile(d.min(1.0)).clamp(0.0, 1.0) * 255.0 + 0.5) as u8]);
        } }
        Arc::new(Picture::new(n, n, rgba).expect("fixed size"))
    }
    fn halo() -> &'static Arc<Picture> {
        static P: OnceLock<Arc<Picture>> = OnceLock::new();
        // 1 / (1 + 32 d²) over 0.6 screen heights, windowed to the edge.
        P.get_or_init(|| picture(|d| { (-14.0*d*d).exp() * (1.0-d*d).powi(2) }))
    }
    fn disc() -> &'static Arc<Picture> {
        static P: OnceLock<Arc<Picture>> = OnceLock::new();
        P.get_or_init(|| picture(|d| 1.0 - d.powf(2.4)))
    }

    /// Draw the flare of the strongest incoming rocket into one view.
    pub fn draw(list: &mut HudDrawList, rect: [f32; 4], cam: &render::Camera, strikes: &[Strike]) {
        let [x, y, w, h] = rect;
        if w < 2.0 || h < 2.0 { return; }
        let view = glam::Mat4::look_at_rh(cam.pos, cam.target, Vec3::Y);
        let proj = glam::Mat4::perspective_rh(cam.fov_y, w / h, cam.near.max(0.01), 5000.0);
        let best = strikes.iter().filter_map(flight).filter_map(|f| {
            let intensity = f.approach * f.approach;
            if intensity < 0.015 { return None; }
            let tail = f.pos - f.dir * (super::NOZZLE_END + 1.2);
            let clip = proj * view * tail.extend(1.0);
            if clip.w <= 0.0 { return None; }
            let ndc = clip.truncate() / clip.w;
            let (sx, sy) = (x + (ndc.x * 0.5 + 0.5) * w, y + (0.5 - ndc.y * 0.5) * h);
            if sx < x - 80.0 || sx > x + w + 80.0 || sy < y - 80.0 || sy > y + h + 80.0 { return None; }
            Some((intensity * super::flicker(f.approach.sqrt()*super::AIR_STRIKE_TRAVEL as f32,f.pos.x), sx, sy))
        }).max_by(|a, b| a.0.total_cmp(&b.0));
        let Some((intensity, sx, sy)) = best else { return };
        let strength = 0.03 + (1.05 - 0.03) * intensity.powf(0.65);
        list.provide_image(HALO, halo(), ImageFilter::Linear);
        list.provide_image(DISC, disc(), ImageFilter::Linear);
        // The source's offset from the view centre, in screen heights.
        let (cx, cy) = (x + w * 0.5, y + h * 0.5);
        let (px, py) = ((sx - cx) / h, (sy - cy) / h);
        let clip = [x, y, w, h];
        // Soft concentric engine bloom; no hard anamorphic bars.
        let a=(strength*0.85).clamp(0.0,0.97);
        let radius=h*(0.12+0.06*strength.min(1.5));
        list.image([sx-radius,sy-radius,radius*2.0,radius*2.0],HALO,[0.0,0.0,1.0,1.0],[1.0,0.70,0.32,a*0.8],clip);
        let core=radius*0.35;
        list.image([sx-core,sy-core,core*2.0,core*2.0],HALO,[0.0,0.0,1.0,1.0],[1.0,0.97,0.82,a],clip);
        for g in &GHOSTS {
            let light = g.weight.map(|v| v * g.peak * 1.3 * GAIN * strength);
            let rgb = [0, 1, 2].map(|c| light[c] * TINT[c]);
            let alpha = rgb.iter().cloned().fold(0.0, f32::max).min(1.0);
            if alpha < 0.002 { continue; }
            let color = rgb.map(|v| (v / alpha).min(1.0));
            let (gx, gy) = (cx + g.k * px * h, cy + g.k * py * h);
            let r = g.radius * h;
            list.image([gx - r, gy - r, r * 2.0, r * 2.0], g.image, [0.0, 0.0, 1.0, 1.0], [color[0], color[1], color[2], alpha], clip);
        }
    }
}
