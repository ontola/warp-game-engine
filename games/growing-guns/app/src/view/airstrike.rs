//! Air-strike targeting laser from `scripts/air_strike.gd`: a thin
//! white-hot core inside a red glow, connecting the rocket's origin to its
//! target. Drawn as an additive light shaft (a sprite turned about the
//! laser towards each camera) whose texture carries the soft falloff from
//! the core into the red glow, so it reads as light, not as a box.
use glam::Vec3;
use growing_guns::{Strike, StrikeKind, AIR_STRIKE_TRAVEL};
use render::picture::Picture;
use render::sprites::{Sprite, SpriteImage};

pub(super) const SKY_OFFSET: Vec3 = Vec3::new(0.0, 263.0, 93.0);
const MAX_STRIKES: usize = 32;
pub(super) const IMAGE: SpriteImage = SpriteImage(0x6c61_7365);
/// The sprite's full width; Godot's core is 3.5 cm and its sheath 8.5 cm.
const WIDTH: f32 = 0.5;
const CORE: f32 = 0.022;
const SHEATH: f32 = 0.05;
const GLOW: f32 = 0.16;
/// Godot `MARKER_COLOR` (linear) for the sheath and glow.
const RED: [f32; 3] = [1.0, 0.08, 0.04];
/// Peak radiance: very bright (Godot core emission 16). The sprite shader
/// limits what reaches the fp16 target, so the white-hot core saturates up
/// close while a distant laser, a pixel or two wide, still burns bright.
const NITS: f32 = 160_000.0;
/// The laser's glow never narrows below this on screen, so far up it still
/// reads as a burning line.
const MIN_PIXELS: f32 = 10.0;

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

/// One shaft per live air strike, from its target up to where the rocket
/// comes from.
pub(super) fn sprites(strikes: &[Strike]) -> Vec<Sprite> {
    let axis = SKY_OFFSET.normalize();
    strikes.iter().filter(|s| active(s)).take(MAX_STRIKES).map(|s| {
        let mut sprite = Sprite::shaft(s.target + SKY_OFFSET * 0.5, [WIDTH, SKY_OFFSET.length()], axis, IMAGE, [0.0, 0.0, 1.0, 1.0], Vec3::splat(NITS));
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
    fn laser_spans_the_rocket_path() {
        let s = &sprites(&[strike(StrikeKind::AirStrike, 0.0)])[0];
        let target = strike(StrikeKind::AirStrike, 0.0).target;
        let Facing::Axis { up } = s.facing else { panic!("a shaft") };
        assert!((s.center - up * s.size[1] * 0.5).abs_diff_eq(target, 0.001));
        assert!((s.center + up * s.size[1] * 0.5).abs_diff_eq(target + SKY_OFFSET, 0.001));
        assert!(s.additive && s.size[0] <= 0.5);
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
    fn laser_core_is_white_and_its_edge_red() {
        let p = picture();
        let px = |x: u32| &p.rgba[(x * 4) as usize..(x * 4 + 4) as usize];
        let centre = px(p.width / 2);
        assert!(centre[0] > 240 && centre[1] > 200 && centre[3] > 240);
        let edge = px(p.width / 2 + p.width / 7);
        assert!(edge[0] > edge[1] * 3 && edge[3] < centre[3]);
    }
}
