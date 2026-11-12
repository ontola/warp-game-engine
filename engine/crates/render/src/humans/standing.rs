//! CPU sampling for characters standing amongst GPU spectators. The stationary
//! pose follows crowd_pose.wgsl and humans.wgsl, using the same seat and clock.
use super::Instance;
use crate::crowd::Seat;
use glam::{Affine3A, Quat, Vec3};

#[derive(Clone, Copy)]
pub struct StandingPose {
    /// Right and left wrists relative to the feet of a unit-height, +Z facing Human.
    pub wrists: [Affine3A; 2],
    pub lift: f32,
    pub chest: Quat,
    pub glance: Quat,
    pub knee_bend: f32,
    pub pelvis: Vec3,
}

fn smooth(lo: f32, hi: f32, value: f32) -> f32 {
    let t = ((value - lo) / (hi - lo)).clamp(0., 1.);
    t * t * (3. - 2. * t)
}

fn alarm_pulse(clock: f32, salt: u32) -> f32 {
    let mut bits = clock.floor() as u32 ^ salt;
    bits = (bits ^ (bits >> 16)).wrapping_mul(0x7feb352d);
    bits = (bits ^ (bits >> 15)).wrapping_mul(0x846ca68b);
    bits ^= bits >> 16;
    let attack = 0.22 + 0.17 * (bits & 255) as f32 / 255.;
    let strength = 0.18 + 0.82 * ((bits >> 8) & 255) as f32 / 255.;
    strength * smooth(0., attack, clock.fract()) * (1. - smooth(attack + 0.1, 0.90, clock.fract()))
}

impl Instance {
    pub fn standing_pose(
        &self,
        seat: &Seat,
        time: f32,
        excitement: f32,
        fear: f32,
        humans: bool,
    ) -> StandingPose {
        let t = time * (1.4 + seat.tempo * 2.2) + seat.phase * 6.2832;
        let up = t
            .sin()
            .max(0.)
            .max((t * 1.93 + 1.7).sin().max(0.) * excitement);
        let hop = (seat.hop + excitement * 0.22) * (1. + excitement * 0.8) * (1. - fear);
        let cheer_lift = if humans {
            smooth(0.28, 0.85, excitement)
        } else {
            1.
        };
        let lift = up * hop * 0.4 * seat.scale * cheer_lift;
        let seed = self.size_phase[2];
        let tempo = self.size_phase[3];
        let beat = time * (2. + 1.4 * tempo) + 6.2832 * seed;
        let cheer = smooth(-0.18, 0.65, (beat * 0.29 + 4. * seed).sin())
            * (0.18 + 0.82 * excitement)
            * (1. - fear);
        let alarm = smooth(0.08 + 0.12 * seed, 0.57 + 0.16 * seed, fear);
        let shift = (beat * 0.23 + 5. * seed).sin();
        let breath = (beat * 0.54).sin();
        let lead = if seed < 0.5 { -1. } else { 1. };
        let mut lean = 0.042 * shift + 0.02 * cheer * breath;
        let mut twist = 0.10 * shift + 0.13 * cheer * lead;
        let mut glance = 0.30 * (beat * 0.31 + seed * 12.).sin() + 0.08 * (beat * 0.64).sin();
        let mut nod = 0.035 * breath + 0.10 * cheer * (beat - 0.45).sin() + 0.16 * fear;
        let mut reach = [0.; 2];
        if alarm > 0. {
            let clock = time * (1.55 + 0.75 * tempo) + seed * 19.;
            let salt = (seed * 16777216.) as u32;
            reach = [
                alarm_pulse(clock, salt),
                alarm_pulse(
                    clock * (0.89 + 0.19 * self.identity[2]) + 0.37,
                    salt ^ 0x9e3779b9,
                ),
            ];
            let flinch = (reach[0] - reach[1]) * alarm;
            lean += 0.05 * flinch;
            twist += 0.095 * flinch;
            glance += 0.15 * flinch;
            nod -= 0.075 * alarm * reach[0].max(reach[1]);
        }
        let chest = Quat::from_rotation_z(lean) * Quat::from_rotation_y(-twist);
        let glance = Quat::from_rotation_y(-glance) * Quat::from_rotation_x(nod);
        let mut raised = 0.0_f32;
        let mut wrists = [false, true].map(|left| {
            let side = if left { 1. } else { -1. };
            let leads = left == (seed >= 0.5);
            let weight = if leads { 1. } else { 0.30 };
            let wave = (if left { beat + 1.2 + seed } else { beat }).sin();
            let mut shoulder =
                -0.10 - weight * cheer * (1.80 + 0.43 * wave) + 0.065 * wave - 0.57 * fear;
            let lowered = 1. - smooth(0.18, 0.78, -shoulder);
            let bend = (0.25 + 0.13 * (wave + 1.)) * (1. - lowered)
                + (0.16 + 0.035 * (wave + 1.)) * lowered;
            let mut elbow = -bend - cheer * weight * (0.56 - 0.30 * wave) - 0.72 * fear;
            if alarm > 0. {
                let reach = reach[usize::from(left)];
                let reach_weight = if leads {
                    0.34 + 0.66 * smooth(0.18, 0.72, tempo)
                } else {
                    1.
                };
                shoulder += alarm * (-0.72 - reach_weight * (1.45 + 0.72 * reach) - shoulder);
                elbow += alarm * (-1.28 + reach_weight * (0.28 + 0.65 * reach) - elbow);
            }
            if !humans {
                shoulder = -2.4 * (1. - fear) * up;
                elbow = -0.18;
            }
            raised = raised.max(-shoulder);
            let tuck = 1. - smooth(0.18, 0.78, -shoulder);
            let shoulder_at = Vec3::new(side * 0.1008, 0.805, -0.012);
            let elbow_at = Vec3::new(side * 0.1878, 0.6994, -0.0132);
            let wrist_at = Vec3::new(side * 0.259, 0.637, 0.084);
            let lower = Quat::from_rotation_x(elbow + 0.44 * tuck);
            let upper = Quat::from_rotation_x(shoulder + 0.27 * tuck)
                * Quat::from_rotation_z(-(0.61 - 0.05 * tuck) * side);
            let p = shoulder_at + upper * (elbow_at + lower * (wrist_at - elbow_at) - shoulder_at)
                - Vec3::Y * (0.012 * tuck);
            let collar = Vec3::new(side * 0.025, 0.79, 0.);
            let raise = Quat::from_rotation_z(side * 0.14 * smooth(0.4, 2., -shoulder));
            Affine3A::from_rotation_translation(
                raise * upper * lower,
                collar + raise * (p - collar),
            )
        });
        let phase = time * (2. + tempo) + 6.2832 * seed;
        let knee_bend = 0.12 + 0.24 * smooth(0.20, 1.6, raised) * (0.5 + 0.5 * phase.sin());
        let drop = 0.004 + 0.481 * (1. - knee_bend.cos()) + 0.22 * fear;
        let pelvis = Vec3::new(0.42 * lean, -drop, 0.);
        let waist = Vec3::Y * 0.49;
        let torso = Affine3A::from_rotation_translation(chest, pelvis + waist - chest * waist);
        for wrist in &mut wrists {
            *wrist = torso * *wrist;
        }
        StandingPose {
            wrists,
            lift,
            chest,
            glance,
            knee_bend,
            pelvis,
        }
    }
}
