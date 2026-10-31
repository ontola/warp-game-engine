//! Per-view finish camera. Entry is vehicle-relative so it never freezes at the line.
use glam::{Quat, Vec3};
use render::Camera;
use spaceracer::{race::Race, race_rules::Pose};
#[derive(Clone, Default)]
pub struct Replay {
    entry: Option<(Vec3, Quat, f32)>,
}
impl Replay {
    pub fn update(
        &mut self,
        race: &Race,
        i: usize,
        pose: Pose,
        chase: &Camera,
        up: Vec3,
        aspect: f32,
    ) -> (Camera, Vec3) {
        let Some(v) = &race.drivers[i].victory else {
            self.entry = None;
            return (chase.clone(), up);
        };
        let local = pose.basis.inverse();
        let entry = self.entry.get_or_insert_with(|| {
            let rotation =
                Quat::from_mat4(&glam::Mat4::look_at_rh(chase.pos, chase.target, up).inverse());
            (
                local * (chase.pos - pose.origin),
                Quat::from_mat3(&local) * rotation,
                chase.fov_y,
            )
        });
        let n = race.world.track.sample(race.pilots[i].distance);
        let mut offset = Vec3::from(v.shot.offset);
        offset.x = offset.x.clamp(
            -n.geometry.width as f32 * 0.65,
            n.geometry.width as f32 * 0.65,
        );
        if n.tunnel || n.geometry.shape_angle > 2. {
            offset.x = offset.x.clamp(-7., 7.);
            offset.y = offset.y.min(7.);
        }
        let target = pose.origin + pose.basis.y_axis;
        let desired = pose.origin + pose.basis * offset;
        let t = spaceracer::handling::smooth(0., 1.3, v.age) as f32;
        let mut pos = (pose.origin + pose.basis * entry.0).lerp(desired, t);
        if let Some(hit) = race.environment.trace(&race.obstacles, target, pos) {
            pos = hit.position + hit.normal * 0.7;
        }
        let rot =
            Quat::from_mat4(&glam::Mat4::look_at_rh(pos, target, pose.basis.y_axis).inverse());
        let rot = (Quat::from_mat3(&pose.basis) * entry.1)
            .slerp(rot, t)
            .normalize();
        let fitted =
            2. * ((v.shot.fov.to_radians() * 0.5).tan() * (16. / 9. / aspect).min(1.)).atan();
        (
            Camera {
                pos,
                target: pos - rot * Vec3::Z * 30.,
                fov_y: entry.2 + (fitted - entry.2) * t,
                near: 0.15,
            },
            rot * Vec3::Y,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finish_entry_is_continuous_and_split_lens_preserves_horizontal_fov() {
        let mut r = Race::new(35, "easy", "city", 1, 1);
        let pose = r.pose(0);
        let chase = Camera {
            pos: pose.origin + pose.basis * Vec3::new(0., 6., -20.),
            target: pose.origin,
            fov_y: 70f32.to_radians(),
            near: 0.15,
        };
        r.begin_victory(0);
        r.over = true;
        let mut replay = Replay::default();
        let (entry, up) = replay.update(&r, 0, pose, &chase, pose.basis.y_axis, 32. / 9.);
        assert!(entry.pos.distance(chase.pos) < 0.001);
        assert!(
            (entry.target - entry.pos)
                .normalize()
                .dot((chase.target - chase.pos).normalize())
                > 0.9999
        );
        assert!(up.is_finite());
        for _ in 0..180 {
            r.step(&[]);
        }
        let (shot, _) = replay.update(&r, 0, r.pose(0), &chase, pose.basis.y_axis, 32. / 9.);
        let expected = 2. * ((58f32.to_radians() * 0.5).tan() * 0.5).atan();
        assert!((shot.fov_y - expected).abs() < 0.0001);
        assert!(shot.pos.is_finite());
    }
}
