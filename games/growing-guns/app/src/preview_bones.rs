//! Growing Guns' palette uses Mixamo names without exported parent indices.
use super::{Character, HudDrawList, Mat4, Vec3, label};
use body::pose_editor::PoseEditor;
use glam::Vec2;
use render::Camera;

fn parent(name: &str) -> Option<String> {
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

pub(super) struct Bones {
    pub editor: PoseEditor,
    pub selected: usize,
    pub shown: bool,
    projected: Vec<Option<Vec2>>,
    drag: Option<usize>,
}

impl Bones {
    pub fn new(model: &Character) -> Result<Self, String> {
        let editor = PoseEditor::bind(model.joints(), parent)?;
        let selected = editor
            .joints
            .iter()
            .position(|j| j.name.ends_with("LeftArm"))
            .unwrap_or(0);
        Ok(Self {
            projected: vec![None; editor.joints.len()],
            editor,
            selected,
            shown: true,
            drag: None,
        })
    }

    pub fn pointer(&mut self, p: app::Pointer) -> bool {
        if !p.held && !p.pressed {
            self.drag = None;
        }
        if !self.shown {
            return false;
        }
        let Some([x, y]) = p.position else {
            return false;
        };
        let panel = (8.0..250.0).contains(&x) && (40.0..196.0).contains(&y);
        if p.pressed {
            if panel {
                if y < 67.0 {
                    let n = self.editor.joints.len();
                    self.selected = (self.selected + if x < 129.0 { n - 1 } else { 1 }) % n;
                } else if (94.0..169.0).contains(&y) {
                    self.drag = Some(((y - 94.0) / 25.0) as usize);
                } else if y >= 169.0 {
                    self.editor.joints[self.selected].degrees = Vec3::ZERO;
                }
            } else if let Some((i, _)) = self
                .projected
                .iter()
                .enumerate()
                .filter_map(|(i, v)| Some((i, v.as_ref()?.distance(Vec2::new(x, y)))))
                .filter(|(_, d)| *d <= 10.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
            {
                self.selected = i;
                return true;
            }
        }
        if let Some(axis) = self.drag {
            self.editor.joints[self.selected].degrees[axis] =
                (((x - 134.0) / 100.0).clamp(0.0, 1.0) * 2.0 - 1.0) * 180.0;
            return true;
        }
        panel && (p.held || p.pressed)
    }

    pub fn draw(&mut self, hud: &mut HudDrawList, cam: &Camera, model: Mat4) {
        if !self.shown {
            return;
        }
        let [w, h] = hud.size.map(|x| x as f32);
        let vp = Mat4::perspective_rh(cam.fov_y, w / h, cam.near, 1000.0)
            * Mat4::look_at_rh(cam.pos, cam.target, Vec3::Y)
            * model;
        let clip = [0., 0., w, h];
        for (j, pos) in self.editor.joints.iter().zip(&mut self.projected) {
            let p = vp * j.position.extend(1.0);
            *pos = (p.w > cam.near)
                .then(|| Vec2::new((1. + p.x / p.w) * w * 0.5, (1. - p.y / p.w) * h * 0.5));
        }
        for (i, j) in self.editor.joints.iter().enumerate() {
            let Some(pos) = self.projected[i] else {
                continue;
            };
            let c = if i == self.selected || j.parent == Some(self.selected) {
                [1., 0.65, 0.1, 1.]
            } else {
                [0.1, 0.9, 0.85, 0.85]
            };
            if let Some(prev) = j.parent.and_then(|p| self.projected[p]) {
                hud.line(prev.to_array(), pos.to_array(), 2.0, c, clip);
            }
            hud.rect([pos.x - 3., pos.y - 3., 6., 6.], c, clip);
        }
        hud.rect([8., 40., 242., 156.], [0.04, 0.05, 0.07, 0.94], clip);
        let j = &self.editor.joints[self.selected];
        label(
            hud,
            40,
            [14., 43.],
            format!(
                "<   {}   >",
                j.name.rsplit([':', '_']).next().unwrap_or(&j.name)
            ),
        );
        let segment = self
            .editor
            .joints
            .iter()
            .find(|child| child.parent == Some(self.selected));
        label(
            hud,
            41,
            [14., 69.],
            segment.map_or("End joint".into(), |child| {
                format!(
                    "To {}  {:.1} cm",
                    child.name.rsplit([':', '_']).next().unwrap_or(&child.name),
                    child.position.distance(j.position) * 100.
                )
            }),
        );
        for axis in 0..3 {
            let y = 94. + axis as f32 * 25.;
            let value = j.degrees[axis];
            label(
                hud,
                42 + axis as u64,
                [14., y],
                format!("Local {}  {:.0} deg", ["X", "Y", "Z"][axis], value),
            );
            hud.rect([134., y + 10., 100., 3.], [0.45, 0.48, 0.5, 1.], clip);
            hud.rect(
                [131. + (value / 180. + 1.) * 50., y + 5., 6., 14.],
                [0.37, 0.85, 0.72, 1.],
                clip,
            );
        }
        label(
            hud,
            46,
            [14., 171.],
            "Reset joint     F12 saves pose".into(),
        );
    }
}
