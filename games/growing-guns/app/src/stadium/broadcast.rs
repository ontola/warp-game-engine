//! Match-time cuts on the four screens. The TV's RNG is independent of gameplay.
use super::{PieceKind, Rng};
use glam::{Quat, Vec3};
use growing_guns::stadium::Piece;
use render::{Camera, crowd::Crowd, screen_feed::BroadcastShot};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq)]
enum Angle {
    Player,
    Section,
    Battle,
    Close,
}

fn angle(seed: u64, cycle: u64, phase: f32) -> (Angle, Rng) {
    let mut rng = Rng(seed ^ cycle.wrapping_mul(0x9e3779b97f4a7c15));
    let start = rng.range(8., 12.);
    let close = rng.f() < 0.7;
    let angle = if cycle % 2 == 0 && (3.0..6.0).contains(&phase) {
        Angle::Battle
    } else if (start..start + 3.).contains(&phase) {
        Angle::Section
    } else if close && ((start + 3.)..(start + 5.5)).contains(&phase) {
        Angle::Close
    } else {
        Angle::Player
    };
    (angle, rng)
}

pub(super) struct Broadcast {
    layer: u32,
    seed: u64,
    origin: Option<f32>,
    focus: Vec3,
    radius: f32,
    cut: Option<(u64, Angle)>,
    subject: Option<usize>,
    shot: Option<Arc<BroadcastShot>>,
}
impl Broadcast {
    pub fn new(layer: u32) -> Self {
        Self {
            layer,
            seed: 0,
            origin: None,
            focus: Vec3::ZERO,
            radius: 15.,
            cut: None,
            subject: None,
            shot: None,
        }
    }
    pub fn reset(&mut self, seed: u64) {
        *self = Self {
            seed,
            ..Self::new(self.layer)
        };
    }
    pub fn track(&mut self, world: &growing_guns::World) {
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        // A player visiting the stands should not pull the lens off the fight.
        let field = world.arena.size * 0.5 + 2.;
        for p in world
            .players
            .iter()
            .filter(|p| p.health > 0 && p.pos.x.abs().max(p.pos.z.abs()) <= field && p.pos.y >= -2.)
        {
            lo = lo.min(p.pos);
            hi = hi.max(p.pos);
        }
        if hi.is_finite() {
            self.focus = (lo + hi) * 0.5;
            self.radius = (0.5 * (hi - lo).length() + 2.).max(7.);
        } else {
            self.focus = Vec3::Y * 2.;
            self.radius = field * 0.7;
        }
    }
    pub fn feed(
        &mut self,
        now: f32,
        crowd: &Crowd,
        steps: &[Piece],
        light: &render::Lighting,
    ) -> Option<(u32, Arc<BroadcastShot>, u32)> {
        if crowd.is_empty() {
            return None;
        }
        let elapsed = (now - *self.origin.get_or_insert(now)).max(0.);
        let cycle = (elapsed / 20.) as u64;
        let (angle, mut rng) = angle(self.seed, cycle, elapsed % 20.);
        let cut = Some((cycle, angle));
        if self.cut != cut || self.subject.is_some_and(|i| !crowd.is_alive(i)) {
            self.cut = cut;
            self.shot = None;
            self.subject = None;
            if angle == Angle::Battle {
                // Look over the boundary wall from one of the upper rows.
                let mut seat = rng.below(crowd.len());
                for _ in 0..5 {
                    let next = rng.below(crowd.len());
                    if crowd.position(next).y > crowd.position(seat).y {
                        seat = next;
                    }
                }
                let eye = crowd.position(seat) + Vec3::Y * 1.75;
                let distance = eye.distance(self.focus).max(1.);
                let fov = (2. * (self.radius / distance).atan())
                    .clamp(24_f32.to_radians(), 78_f32.to_radians());
                self.shot = Some(Arc::new(BroadcastShot {
                    camera: Camera {
                        pos: eye,
                        target: self.focus,
                        fov_y: fov,
                        near: 0.18,
                    },
                    size: [320, 180],
                    seats: None,
                    backdrop: Vec::new(),
                }));
            } else if angle != Angle::Player && crowd.uses_humans() {
                let first = rng.below(crowd.len());
                if let Some(i) = (0..crowd.len())
                    .map(|k| (first + k) % crowd.len())
                    .find(|&i| crowd.is_alive(i))
                {
                    self.subject = Some(i);
                    self.shot = Some(Arc::new(make_shot(crowd, i, angle, steps, light)));
                }
            }
        }
        self.shot.as_ref().map(|shot| {
            (
                self.layer,
                shot.clone(),
                (elapsed * if angle == Angle::Battle { 10. } else { 12. }) as u32,
            )
        })
    }
    pub fn mode(&self) -> &'static str {
        match self.cut.map(|c| c.1).filter(|_| self.shot.is_some()) {
            Some(Angle::Battle) => "battle overview",
            Some(Angle::Close) => "audience close",
            Some(Angle::Section) => "audience wide",
            _ => "player",
        }
    }
}

fn make_shot(
    crowd: &Crowd,
    subject: usize,
    angle: Angle,
    steps: &[Piece],
    light: &render::Lighting,
) -> BroadcastShot {
    let feet = crowd.position(subject);
    let height = crowd.body_scale(subject).y;
    let front = -Vec3::new(feet.x, 0., feet.z).normalize_or_zero();
    let close = angle == Angle::Close;
    let (distance, lift, fov, size) = if close {
        (3.2, 0.66, 17_f32, [384, 216])
    } else {
        (7.5, 2.8, 39_f32, [192, 108])
    };
    let target = feet + Vec3::Y * height * if close { 0.84 } else { 0.74 };
    let camera = Camera {
        pos: target + front * distance + Vec3::Y * lift,
        target,
        fov_y: fov.to_radians(),
        near: 0.12,
    };
    let mut backdrop = Vec::new();
    // Nearby real tread and wall boxes provide depth and hide feet in the rows behind.
    for p in steps
        .iter()
        .filter(|p| feet.distance(p.center) < 20. + p.size.length() * 0.5)
    {
        let rotation = Quat::from_rotation_y(p.yaw);
        let stone = match p.kind {
            PieceKind::Step => Vec3::new(0.42, 0.33, 0.23),
            _ => Vec3::new(0.30, 0.25, 0.20),
        };
        for (n, u, v) in [
            (Vec3::Y, Vec3::X, Vec3::Z),
            (Vec3::X, Vec3::Y, Vec3::Z),
            (Vec3::NEG_X, Vec3::Y, Vec3::Z),
            (Vec3::Z, Vec3::X, Vec3::Y),
            (Vec3::NEG_Z, Vec3::X, Vec3::Y),
        ] {
            let irradiance = light.sky_lux * (0.50 + 0.25 * n.y)
                + light.sun_lux * (rotation * n).dot(Vec3::from(light.sun_dir)).max(0.);
            let x = stone
                * (irradiance * render::auto_exposure::exposure(light.ev100)
                    / std::f32::consts::PI)
                    .max(0.06);
            let color = (x / (Vec3::ONE + x)).to_array();
            for (s, t) in [
                (-1., -1.),
                (1., -1.),
                (1., 1.),
                (-1., -1.),
                (1., 1.),
                (-1., 1.),
            ] {
                let pos = p.center + rotation * ((n + s * u + t * v) * p.size * 0.5);
                backdrop.push((pos.to_array(), color));
            }
        }
    }
    // Seats are stored section by section. Include neighbouring rows on either side.
    let seats = subject.saturating_sub(256) as u32..(subject + 257).min(crowd.len()) as u32;
    BroadcastShot {
        camera,
        size,
        seats: Some(seats),
        backdrop,
    }
}
