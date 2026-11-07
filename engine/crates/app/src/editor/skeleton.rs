use super::label;
use body::pose_editor::PoseEditor;
use glam::Vec2;
use glam::{Mat4, Vec3};
use render::Camera;
use render::{editor_style as style, hud::HudDrawList};

pub struct SkeletonOverlay {
    pub editor: PoseEditor,
    pub selected: usize,
    pub shown: bool,
    cursor: Option<[f32; 2]>,
    projected: Vec<Option<Vec2>>,
    drag: Option<usize>,
    direct: Option<(bool, Vec2)>,
    inv_vp: Mat4,
    vp: Mat4,
    size: Vec2,
}

impl SkeletonOverlay {
    pub fn new(editor: PoseEditor) -> Result<Self, String> {
        if editor.joints.is_empty() {
            return Err("pose viewport needs at least one joint".into());
        }
        let selected = 0;
        Ok(Self {
            cursor: None,
            projected: vec![None; editor.joints.len()],
            editor,
            selected,
            shown: true,
            drag: None,
            direct: None,
            inv_vp: Mat4::IDENTITY,
            vp: Mat4::IDENTITY,
            size: Vec2::ONE,
        })
    }

    pub fn pointer(&mut self, p: crate::Pointer) -> bool {
        let panel = p
            .position
            .is_some_and(|xy| style::EditorButton([8., 40., 242., 156.]).contains(xy));
        self.pointer_at(p, panel)
    }
    pub(super) fn pointer_at(&mut self, p: crate::Pointer, panel: bool) -> bool {
        self.cursor = p.position;
        if !p.held && !p.pressed {
            self.drag = None;
        }
        if !self.shown {
            self.direct = None;
            self.drag = None;
            return false;
        }
        let Some([x, y]) = p.position else {
            return false;
        };

        let cursor = Vec2::new(x, y);
        if let Some((rotate, last)) = self.direct {
            if if rotate { p.right_held } else { p.held } {
                let joint = &mut self.editor.joints[self.selected];
                let ndc = self.vp.project_point3(joint.position);
                let unproject = |at: Vec2| {
                    self.inv_vp.project_point3(Vec3::new(
                        2. * at.x / self.size.x - 1.,
                        1. - 2. * at.y / self.size.y,
                        ndc.z,
                    ))
                };
                if rotate {
                    let right = (unproject(last + Vec2::X) - unproject(last)).normalize_or(Vec3::X);
                    let up = (unproject(last - Vec2::Y) - unproject(last)).normalize_or(Vec3::Y);
                    let d = cursor - last;
                    let turn = glam::Quat::from_axis_angle(up, d.x * 0.012)
                        * glam::Quat::from_axis_angle(right, d.y * 0.012);
                    let r = joint.degrees * (std::f32::consts::PI / 180.);
                    let q = joint.basis.inverse()
                        * turn
                        * joint.basis
                        * glam::Quat::from_euler(glam::EulerRot::XYZ, r.x, r.y, r.z);
                    let (a, b, c) = q.to_euler(glam::EulerRot::XYZ);
                    joint.degrees = Vec3::new(a, b, c) * (180. / std::f32::consts::PI);
                } else {
                    joint.local_offset +=
                        joint.basis.inverse() * (unproject(cursor) - unproject(last));
                }
                self.direct = Some((rotate, cursor));
                return true;
            } else {
                self.direct = None;
            }
        }
        if p.pressed || p.right_pressed {
            if panel {
                if !p.pressed {
                    return true;
                }
                if Self::PREV.contains([x, y]) || Self::NEXT.contains([x, y]) {
                    let n = self.editor.joints.len();
                    self.selected = (self.selected
                        + if Self::PREV.contains([x, y]) {
                            n - 1
                        } else {
                            1
                        })
                        % n;
                } else if (94.0..169.0).contains(&y) {
                    self.drag = Some(((y - 94.0) / 25.0) as usize);
                } else if Self::RESET.contains([x, y]) {
                    self.editor.joints[self.selected].degrees = Vec3::ZERO;
                    self.editor.joints[self.selected].local_offset = Vec3::ZERO;
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
                self.direct = Some((p.right_pressed, cursor));
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

    pub(super) fn slider_held(&self) -> bool {
        self.drag.is_some()
    }
    pub fn manipulating(&self) -> bool {
        self.direct.is_some()
    }
    pub fn draw(&mut self, hud: &mut HudDrawList, cam: &Camera, model: Mat4) {
        self.draw_rig(hud, cam, model);
        if self.shown {
            self.panel(hud);
        }
    }
    pub(super) fn draw_rig(&mut self, hud: &mut HudDrawList, cam: &Camera, model: Mat4) {
        if !self.shown {
            return;
        }
        let [w, h] = hud.size.map(|x| x as f32);
        let vp = Mat4::perspective_rh(cam.fov_y, w / h, cam.near, 1000.0)
            * Mat4::look_at_rh(cam.pos, cam.target, Vec3::Y)
            * model;
        self.size = Vec2::new(w, h);
        self.vp = vp;
        self.inv_vp = vp.inverse();
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
            hud.rect([pos.x - 4., pos.y - 4., 8., 8.], c, clip);
        }
    }

    pub(super) fn panel(&self, hud: &mut HudDrawList) {
        let clip = [8., 40., 242., 156.];
        let j = &self.editor.joints[self.selected];
        for (id, button, text) in [
            (47, Self::PREV, "<"),
            (48, Self::NEXT, ">"),
            (46, Self::RESET, "Reset joint"),
        ] {
            button.draw(
                hud,
                id,
                text,
                clip,
                15.,
                self.cursor.is_some_and(|p| button.contains(p)),
                true,
            );
        }
        label(
            hud,
            40,
            [45., 43.],
            j.name.rsplit([':', '_']).next().unwrap_or(&j.name).into(),
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
                style::HIGHLIGHT,
                clip,
            );
        }
        label(hud, 49, [126., 171.], "F12 saves pose".into());
    }

    const PREV: style::EditorButton = style::EditorButton([12., 42., 28., 23.]);
    const NEXT: style::EditorButton = style::EditorButton([216., 42., 28., 23.]);
    const RESET: style::EditorButton = style::EditorButton([12., 170., 110., 23.]);
}
