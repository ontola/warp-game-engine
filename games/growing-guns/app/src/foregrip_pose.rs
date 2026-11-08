use assets::skinned::Character;
use body::pose_editor::PoseEditor;
use glam::Vec3;
use serde::Deserialize;

#[derive(Deserialize)]
struct BoneOffset {
    name: String,
    local_translation_m: [f32; 3],
    local_offset_xyz_deg: [f32; 3],
}

pub(crate) struct ForegripPose {
    offsets: Vec<(usize, Vec3, Vec3)>,
    previous: f32,
}

impl ForegripPose {
    pub fn bind(model: &Character, editor: &PoseEditor) -> Result<Self, String> {
        let mut offsets = Vec::new();
        if model.name == "knight" {
            let rows: Vec<BoneOffset> = serde_json::from_str(include_str!(
                "../../assets/characters/knight_foregrip_pose.json"
            ))
            .map_err(|e| e.to_string())?;
            for row in rows {
                let index = editor
                    .joints
                    .iter()
                    .position(|j| j.name == row.name)
                    .ok_or_else(|| format!("missing foregrip bone {}", row.name))?;
                offsets.push((
                    index,
                    Vec3::from_array(row.local_translation_m),
                    Vec3::from_array(row.local_offset_xyz_deg),
                ));
            }
        }
        Ok(Self {
            offsets,
            previous: 0.0,
        })
    }

    pub fn set_extension(&mut self, editor: &mut PoseEditor, extension: f32) {
        let weight = extension.clamp(0.0, 1.0);
        if weight == self.previous {
            return;
        }
        for &(i, offset, degrees) in &self.offsets {
            let joint = &mut editor.joints[i];
            joint.local_offset += offset * (weight - self.previous);
            joint.degrees += degrees * (weight - self.previous);
        }
        self.previous = weight;
    }
}

pub(crate) fn parent(name: &str) -> Option<String> {
    let core = name.rsplit([':', '_']).next()?;
    let prefix = &name[..name.len() - core.len()];
    let center = match core {
        "Spine" => "Hips",
        "Spine1" => "Spine",
        "Spine2" => "Spine1",
        "Neck" => "Spine2",
        "Head" => "Neck",
        _ => "",
    };
    if !center.is_empty() {
        return Some(format!("{prefix}{center}"));
    }
    let side = ["Left", "Right"]
        .into_iter()
        .find(|s| core.starts_with(s))?;
    let limb = &core[side.len()..];
    let base = match limb {
        "Shoulder" => return Some(format!("{prefix}Spine2")),
        "Arm" => "Shoulder",
        "ForeArm" => "Arm",
        "Hand" => "ForeArm",
        "UpLeg" => return Some(format!("{prefix}Hips")),
        "Leg" => "UpLeg",
        "Foot" => "Leg",
        "ToeBase" => "Foot",
        "Eye" => return Some(format!("{prefix}Head")),
        n if n.starts_with("Hand") && n.ends_with('1') => "Hand",
        n if n.starts_with("Hand") => {
            let digit = n.chars().last()?.to_digit(10)?.checked_sub(1)?;
            return Some(format!("{prefix}{side}{}{digit}", &n[..n.len() - 1]));
        }
        _ => return None,
    };
    Some(format!("{prefix}{side}{base}"))
}
