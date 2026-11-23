//! A positional contact layered over an animated skin palette.
//! Bind once per skeleton. Target and bend hint use the sampled model's space.
use glam::{Affine3A, Mat4, Quat, Vec3};

/// Rigid prop attachment calibrated against one reference pose. The socket
/// follows the final skin palette, including edits inherited from parent bones.
pub struct JointMount {
    index: usize,
    bind: Mat4,
    reference_inverse: Quat,
}

impl JointMount {
    pub fn new(index: usize, bind: Mat4, reference: &[Mat4]) -> Self {
        let reference_inverse = (reference[index] * bind)
            .to_scale_rotation_translation()
            .1
            .normalize()
            .inverse();
        Self {
            index,
            bind,
            reference_inverse,
        }
    }

    /// Place the prop-local contact at this joint, in character model space.
    pub fn place(&self, pose: &[Mat4], reference_orientation: Quat, contact: Vec3) -> Affine3A {
        let (_, hand, wrist) = (pose[self.index] * self.bind).to_scale_rotation_translation();
        let rotation = hand.normalize() * self.reference_inverse * reference_orientation;
        Affine3A::from_rotation_translation(rotation, wrist - rotation * contact)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LimbPart {
    Upper,
    Lower,
    End,
}

pub struct Reach {
    length_scale: [f32; 2],
    pivots: [(usize, Vec3); 3],
    members: Vec<(usize, LimbPart)>,
}

impl Reach {
    /// Standard arm names (LeftArm, LeftForeArm, LeftHand and finger joints),
    /// including Mixamo namespace prefixes.
    pub fn arm<'a>(joints: impl Iterator<Item = (&'a str, Mat4)>, left: bool) -> Option<Self> {
        let side = if left { "Left" } else { "Right" };
        Self::bind(joints, |name| {
            let bone = name.rsplit([':', '_']).next()?.strip_prefix(side)?;
            match bone {
                "Arm" => Some((LimbPart::Upper, true)),
                "ForeArm" => Some((LimbPart::Lower, true)),
                "Hand" => Some((LimbPart::End, true)),
                finger if finger.starts_with("Hand") => Some((LimbPart::End, false)),
                _ => None,
            }
        })
    }

    /// Classify every affected skin entry, including duplicate binds and fingers.
    /// The bool identifies a pivot (shoulder, elbow, wrist for an arm).
    pub fn bind<'a>(
        joints: impl Iterator<Item = (&'a str, Mat4)>,
        classify: impl Fn(&str) -> Option<(LimbPart, bool)>,
    ) -> Option<Self> {
        let mut pivots = [None; 3];
        let mut members = Vec::new();
        for (i, (name, rest)) in joints.enumerate() {
            if let Some((part, pivot)) = classify(name) {
                members.push((i, part));
                if pivot {
                    pivots[part as usize].get_or_insert((i, rest.transform_point3(Vec3::ZERO)));
                }
            }
        }
        Some(Self {
            length_scale: [1.0; 2],
            pivots: [pivots[0]?, pivots[1]?, pivots[2]?],
            members,
        })
    }

    fn orient_end(&self, pose: &mut [Mat4], bind_to_grip: Quat) {
        let (i, rest) = self.pivots[2];
        let wrist = pose[i].transform_point3(rest);
        let turn = bind_to_grip
            * pose[i]
                .to_scale_rotation_translation()
                .1
                .normalize()
                .inverse();
        let around = Mat4::from_rotation_translation(turn, wrist - turn * wrist);
        for &(index, part) in &self.members {
            if part == LimbPart::End {
                pose[index] = around * pose[index];
            }
        }
    }

    /// Current upper and lower segment lengths combined, in model units.
    pub fn span(&self, pose: &[Mat4]) -> f32 {
        let [a, b, c] = self.pivots.map(|(i, rest)| pose[i].transform_point3(rest));
        a.distance(b) * self.length_scale[0] + b.distance(c) * self.length_scale[1]
    }

    /// Nearest reachable wrist position. Useful when two limbs constrain a shared prop.
    pub fn closest_target(&self, pose: &[Mat4], target: Vec3) -> Vec3 {
        let [root, joint, end] = self.pivots.map(|(i, rest)| pose[i].transform_point3(rest));
        let a = root.distance(joint) * self.length_scale[0];
        let b = joint.distance(end) * self.length_scale[1];
        let delta = target - root;
        root + delta.normalize_or((end - root).normalize_or(Vec3::Y))
            * delta.length().clamp((a - b).abs(), a + b)
    }

    /// Keep the sampled wrist orientation and finger pose. Unreachable targets
    /// stop at the arm's reach. Weight zero preserves the exact input palette.
    pub fn apply(&self, pose: &mut [Mat4], target: Vec3, bend_hint: Vec3, weight: f32) {
        let weight = weight.clamp(0.0, 1.0);
        if weight == 0.0 {
            return;
        }
        let [root, joint, end] = self.pivots.map(|(i, rest)| pose[i].transform_point3(rest));
        let upper = joint - root;
        let lower = end - joint;
        let (a, b) = (
            upper.length() * self.length_scale[0],
            lower.length() * self.length_scale[1],
        );
        if a.min(b) < 1e-6 {
            return;
        }
        let goal = end.lerp(target, weight) - root;
        let axis = goal.normalize_or((end - root).normalize_or(upper.normalize()));
        let dist = goal.length().clamp((a - b).abs().max(1e-6), a + b);
        let along = ((a * a - b * b + dist * dist) / (2.0 * dist)).clamp(-a, a);
        let project = |v: Vec3| v - axis * axis.dot(v);
        let bend = project(upper)
            .try_normalize()
            .unwrap_or_else(|| project(bend_hint).normalize_or(axis.any_orthonormal_vector()));
        let elbow = root + axis * along + bend * (a * a - along * along).max(0.0).sqrt();
        let wrist = root + axis * dist;
        let move_segment = |from: Vec3, to: Vec3, old: Vec3, new: Vec3, scale: f32| {
            let turn = Quat::from_rotation_arc(old.normalize(), new.normalize());
            if scale == 1.0 {
                return Mat4::from_rotation_translation(turn, to - turn * from);
            }
            // Stretch only along the bone axis, around its proximal joint.
            let axis = old.normalize();
            let stretch = |v: Vec3| turn * (v + axis * (axis.dot(v) * (scale - 1.0)));
            Mat4::from_cols(
                stretch(Vec3::X).extend(0.0),
                stretch(Vec3::Y).extend(0.0),
                stretch(Vec3::Z).extend(0.0),
                (to - stretch(from)).extend(1.0),
            )
        };
        let transforms = [
            move_segment(root, root, upper, elbow - root, self.length_scale[0]),
            move_segment(joint, elbow, lower, wrist - elbow, self.length_scale[1]),
            Mat4::from_translation(wrist - end),
        ];
        for &(index, part) in &self.members {
            pose[index] = transforms[part as usize] * pose[index];
        }
    }
}

/// Right-handed two-point prop contact over a sampled humanoid pose.
pub struct TwoHandHold {
    left: Reach,
    right: Reach,
}

impl TwoHandHold {
    pub fn bind<'a>(joints: impl Iterator<Item = (&'a str, Mat4)> + Clone) -> Option<Self> {
        Some(Self {
            left: Reach::arm(joints.clone(), true)?,
            right: Reach::arm(joints, false)?,
        })
    }

    /// Retarget a held pose to different segment proportions on both arms.
    pub fn with_arm_proportions(mut self, upper: f32, forearm: f32) -> Self {
        assert!(upper.is_finite() && upper > 0.0 && forearm.is_finite() && forearm > 0.0);
        self.left.length_scale = [upper, forearm];
        self.right.length_scale = [upper, forearm];
        self
    }

    /// Orient a support palm after solving the wrists. The rotation maps the
    /// model's bind-space hand to the desired grip in posed model space.
    pub fn orient_support(&self, pose: &mut [Mat4], bind_to_grip: Quat) {
        self.left.orient_end(pose, bind_to_grip);
    }

    /// Rotate the primary hand around the solved wrist without moving the gun.
    pub fn orient_primary(&self, pose: &mut [Mat4], bind_to_grip: Quat) {
        self.right.orient_end(pose, bind_to_grip);
    }

    /// Primary and secondary are wrist sockets in model space. Place the prop
    /// at the returned primary wrist, preserving its requested orientation.
    pub fn apply(&self, pose: &mut [Mat4], primary: Vec3, secondary: Vec3) -> Vec3 {
        let spacing = secondary - primary;
        // Find a weapon translation that both arms can reach, keeping the
        // closest placement to the sampled primary hand when it already fits.
        let mut primary = self.right.closest_target(pose, primary);
        for _ in 0..8 {
            let wanted = primary + spacing;
            let miss = self.left.closest_target(pose, wanted) - wanted;
            if miss.length_squared() < 1e-10 {
                break;
            }
            primary = self.right.closest_target(pose, primary + miss);
        }
        self.right.apply(pose, primary, Vec3::NEG_X, 1.0);
        self.left.apply(pose, primary + spacing, Vec3::X, 1.0);
        primary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joint_mount_tracks_rotated_and_translated_hand_at_each_grip_size() {
        let bind = Mat4::from_translation(Vec3::Y);
        let reference = [Mat4::from_rotation_y(0.6)];
        let mount = JointMount::new(0, bind, &reference);
        let aim = Quat::from_rotation_x(0.2);
        let turn = Quat::from_rotation_z(1.1);
        let edited = [Mat4::from_rotation_translation(turn, Vec3::X) * reference[0]];
        for grip in [Vec3::ZERO, Vec3::new(0.0, -0.1, 0.2)] {
            let xf = mount.place(&edited, aim, grip);
            assert!(
                xf.transform_point3(grip)
                    .distance(edited[0].transform_point3(Vec3::Y))
                    < 1e-5
            );
            assert!(xf.transform_vector3(Vec3::Z).distance(turn * aim * Vec3::Z) < 1e-5);
        }
    }

    #[test]
    fn different_arm_lengths_follow_the_contact_without_stretch() {
        for size in [0.5, 1.0, 1.8] {
            let bind = [
                ("LeftArm", Vec3::ZERO),
                ("LeftForeArm", Vec3::X * size),
                ("LeftHand", Vec3::new(size, size * 0.8, 0.0)),
                ("LeftHandMiddle1", Vec3::new(size, size, 0.0)),
                ("Chest", Vec3::Y),
            ];
            let reach = Reach::arm(
                bind.iter().map(|(n, p)| (*n, Mat4::from_translation(*p))),
                true,
            )
            .unwrap();
            for goal in [Vec3::new(0.7, 0.2, 0.1) * size, Vec3::Y * 10.0, Vec3::ZERO] {
                let mut palette = [Mat4::IDENTITY; 5];
                reach.apply(&mut palette, goal, Vec3::Z, 1.0);
                let pos = |i: usize| palette[i].transform_point3(bind[i].1);
                assert!((pos(0).distance(pos(1)) - size).abs() < 1e-4);
                assert!((pos(1).distance(pos(2)) - size * 0.8).abs() < 1e-4);
                assert!(
                    (pos(2).length() - goal.length().clamp(size * 0.2, size * 1.8)).abs() < 1e-4
                );
                if goal.length() < size && goal != Vec3::ZERO {
                    assert!(pos(2).distance(goal) < 1e-4);
                }
                assert!(palette[2].abs_diff_eq(palette[3], 1e-6));
                assert_eq!(palette[4], Mat4::IDENTITY);
            }
        }
    }
}
