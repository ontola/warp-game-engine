//! Debug edits on a fresh model-space skin palette, applied after animation/IK.
use glam::{EulerRot, Mat4, Quat, Vec3};
use serde_json::{Value, json};

pub struct PoseJoint {
    pub name: String,
    pub parent: Option<usize>,
    pub degrees: Vec3,
    pub position: Vec3,
    entries: Vec<(usize, Mat4)>,
    delta: Mat4,
}

pub struct PoseEditor {
    pub joints: Vec<PoseJoint>,
}

impl PoseEditor {
    /// Supply unique logical names and parent names in any palette order.
    /// Duplicate entries for a joint receive the same model-space correction.
    pub fn bind<'a>(
        entries: impl Iterator<Item = (&'a str, Mat4)>,
        parent: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, String> {
        let mut pending: Vec<PoseJoint> = Vec::new();
        for (i, (name, rest)) in entries.enumerate() {
            if let Some(j) = pending.iter_mut().find(|j| j.name == name) {
                j.entries.push((i, rest));
            } else {
                pending.push(PoseJoint {
                    name: name.into(),
                    parent: None,
                    degrees: Vec3::ZERO,
                    position: Vec3::ZERO,
                    entries: vec![(i, rest)],
                    delta: Mat4::IDENTITY,
                });
            }
        }
        let mut joints: Vec<PoseJoint> = Vec::new();
        while !pending.is_empty() {
            let idx = pending
                .iter()
                .position(|j| parent(&j.name).is_none_or(|p| !pending.iter().any(|n| n.name == p)))
                .ok_or("cycle in debug skeleton")?;
            let mut j = pending.remove(idx);
            j.parent = parent(&j.name).and_then(|p| joints.iter().position(|n| n.name == p));
            joints.push(j);
        }
        Ok(Self { joints })
    }

    /// Degrees are XYZ Euler offsets in each animated joint's local axes.
    pub fn apply(&mut self, pose: &mut [Mat4]) {
        for i in 0..self.joints.len() {
            let inherited = self.joints[i]
                .parent
                .map_or(Mat4::IDENTITY, |p| self.joints[p].delta);
            let j = &mut self.joints[i];
            let (k, bind) = j.entries[0];
            let joint = inherited * pose[k] * bind;
            j.position = joint.transform_point3(Vec3::ZERO);
            j.delta = inherited;
            if j.degrees != Vec3::ZERO {
                let basis = joint.to_scale_rotation_translation().1.normalize();
                let d = j.degrees * (std::f32::consts::PI / 180.0);
                let q = basis * Quat::from_euler(EulerRot::XYZ, d.x, d.y, d.z) * basis.inverse();
                j.delta =
                    Mat4::from_rotation_translation(q, j.position - q * j.position) * inherited;
            }
            if j.delta != Mat4::IDENTITY {
                for &(k, _) in &j.entries {
                    pose[k] = j.delta * pose[k];
                }
            }
        }
    }

    pub fn snapshot(&self) -> Value {
        Value::Array(self.joints.iter().map(|j| json!({"name":j.name,
            "parent":j.parent.map(|i| &self.joints[i].name), "model_position_m":j.position.to_array(),
            "local_offset_xyz_deg":j.degrees.to_array()})).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_rotation_moves_descendants_and_duplicate_bind_entries_together() {
        let joint = |x| Mat4::from_translation(Vec3::X * x);
        let mut editor = PoseEditor::bind(
            [
                ("wrist", joint(2.0)),
                ("upper", joint(0.0)),
                ("elbow", joint(1.0)),
                ("wrist", joint(2.0)),
            ]
            .into_iter(),
            |name| match name {
                "wrist" => Some("elbow".into()),
                "elbow" => Some("upper".into()),
                _ => None,
            },
        )
        .unwrap();
        editor.joints[0].degrees.z = 90.0;
        let mut palette = [Mat4::IDENTITY; 4];
        editor.apply(&mut palette);
        assert!(editor.joints[2].position.distance(Vec3::Y * 2.0) < 1e-5);
        assert_eq!(palette[0], palette[3]);
        assert_eq!(editor.snapshot()[0]["local_offset_xyz_deg"][2], 90.0);
        editor.joints[0].degrees = Vec3::ZERO;
        let mut palette = [Mat4::IDENTITY; 4];
        editor.apply(&mut palette);
        assert_eq!(palette, [Mat4::IDENTITY; 4]);
    }
}
