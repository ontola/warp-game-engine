//! Interactive tools for inspecting an animated model in a Warp game window.
mod menus;
mod skeleton;
use body::pose_editor::PoseEditor;
use glam::{Mat4, Quat, Vec3};
pub use menus::EditorMenus;
use render::editor_window::{EditorWindow, WindowEvent, WindowFrame};
use render::{
    Camera, editor_style as style,
    hud::{FontFace, HudDrawList, TextAlign, TextRun},
};
pub use skeleton::SkeletonOverlay;
const WIDTH: f32 = 242.0;
const ROW: f32 = 26.0;
const TRACK: f32 = 134.0;

pub struct PoseViewport<const N: usize> {
    pub bones: SkeletonOverlay,
    rows: [(&'static str, f32, &'static str); N],
    home: Camera,
    window: EditorWindow,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    pan: Vec3,
    pan_start: Option<[f32; 2]>,
    pub values: [f32; N],
    pub paused: bool,
    cursor: Option<[f32; 2]>,
    slider: Option<usize>,
    orbit: Option<[f32; 2]>,
}

impl<const N: usize> PoseViewport<N> {
    pub fn new(
        editor: PoseEditor,
        home: Camera,
        rows: [(&'static str, f32, &'static str); N],
    ) -> Result<Self, String> {
        if rows
            .iter()
            .any(|(_, limit, _)| !limit.is_finite() || *limit <= 0.)
        {
            return Err("slider limits must be positive".into());
        }
        Ok(Self {
            bones: SkeletonOverlay::new(editor)?,
            rows,
            home,
            window: Default::default(),
            yaw: 0.,
            pitch: 0.,
            zoom: 0.,
            pan: Vec3::ZERO,
            pan_start: None,
            values: [0.; N],
            paused: false,
            cursor: None,
            slider: None,
            orbit: None,
        })
    }
    pub fn function_key(&mut self, n: u8) -> bool {
        if n != 2 {
            return false;
        }
        self.set_windows_open(!self.windows_open());
        true
    }
    pub fn windows_open(&self) -> bool {
        self.bones.shown
    }
    pub fn set_windows_open(&mut self, open: bool) {
        self.bones.shown = open;
        self.window.cancel();
        self.slider = None;
        self.orbit = None;
    }

    pub fn camera(&self) -> Camera {
        let target = self.home.target + self.pan;
        Camera {
            target,
            pos: target + self.orientation() * Vec3::Z * self.distance(),
            ..self.home
        }
    }
    fn distance(&self) -> f32 {
        self.home.pos.distance(self.home.target) * self.zoom.exp()
    }
    pub fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({"paused":self.paused,"yaw":self.yaw,"pitch":self.pitch,"pan":self.pan.to_array(),"zoom":self.zoom,"pose_offsets":self.values.as_slice(),"selected_joint":self.bones.editor.joints[self.bones.selected].name,"joints":self.bones.editor.snapshot()})
    }

    pub fn draw(&mut self, size: [u32; 2], model_to_scene: Mat4, status: &str) -> HudDrawList {
        let scale = ui_scale(size);
        let mut hud = HudDrawList::new(size.map(|v| (v as f32 / scale).round() as u32));
        self.bones
            .draw_rig(&mut hud, &self.camera(), model_to_scene);

        hud.text.push(TextRun {
            id: 1,
            text: format!(
                "L: move   R: rotate/orbit   M: pan   Wheel: zoom   F2: pose editor   {status}"
            ),
            position: [14.0, 12.0],
            bounds: [0.0, 0.0, hud.size[0] as f32, hud.size[1] as f32],
            font_size: 17.0,
            color: style::FG,
            font: FontFace::Body,
            align: TextAlign::Left,
            width: hud.size[0] as f32 - 28.0,
        });
        if self.windows_open() {
            let (frame, local) = self.layout(hud.size);
            frame.draw(&mut hud, 500, 2, "Pose editor");
            let v = hud.vertices.len();
            let t = hud.text.len();
            self.bones.panel(&mut hud);
            self.hud(&mut hud);
            let (k, offset) = content_transform(frame, local);
            let transform = |p: [f32; 2]| [p[0] * k + offset[0], p[1] * k + offset[1]];
            let clip = frame.body();
            for vertex in &mut hud.vertices[v..] {
                vertex.position = transform(vertex.position);
                vertex.clip = clip;
            }
            for run in &mut hud.text[t..] {
                run.position = transform(run.position);
                run.bounds = clip;
                run.font_size *= k;
                run.width *= k;
            }
        }
        for v in &mut hud.vertices {
            v.position = v.position.map(|x| x * scale);
            v.clip = v.clip.map(|x| x * scale);
        }
        for t in &mut hud.text {
            t.position = t.position.map(|x| x * scale);
            t.bounds = t.bounds.map(|x| x * scale);
            t.font_size *= scale;
            t.width *= scale;
        }
        hud.size = size;
        style::style_text(&mut hud, 0);
        hud
    }
    // Joint rows start at y=40 and occupy 156 logical units.
    const CONTROLS_TOP: f32 = 204.;
    fn layout(&self, size: [u32; 2]) -> (WindowFrame, [f32; 4]) {
        let local = [
            8.,
            40.,
            242.,
            Self::CONTROLS_TOP - 40. + ROW * (N + 1) as f32,
        ];
        let frame = self.window.frame(
            size,
            [8., 32., local[2] + 6., local[3] + 32.],
            [160., 160.],
            0.72,
        );
        (frame, local)
    }
    pub fn pointer(&mut self, mut p: crate::Pointer, size: [u32; 2]) {
        let scale = ui_scale(size);
        let size = size.map(|v| (v as f32 / scale).round() as u32);
        p.position = p.position.map(|xy| xy.map(|v| v / scale));
        let mut owner = None;
        if self.windows_open() && !self.bones.manipulating() && self.orbit.is_none() {
            let (frame, local) = self.layout(size);
            let hit = p
                .position
                .is_some_and(|xy| style::EditorButton(frame.rect).contains(xy));
            if hit || self.window.dragging() || self.slider.is_some() || self.bones.slider_held() {
                match self
                    .window
                    .pointer(size, frame, [160., 160.], p.position, p.pressed, p.held)
                {
                    WindowEvent::Close => {
                        self.set_windows_open(false);
                        return;
                    }
                    WindowEvent::Handled => return,
                    WindowEvent::Content => {}
                }
                let (k, offset) = content_transform(frame, local);
                p.position = p
                    .position
                    .map(|xy| [(xy[0] - offset[0]) / k, (xy[1] - offset[1]) / k]);
                let controls = self.slider.is_some()
                    || (!self.bones.slider_held()
                        && p.position.is_some_and(|xy| xy[1] >= Self::CONTROLS_TOP));
                owner = Some(if controls { 1 } else { 0 });
            }
        }
        self.cursor = if owner == Some(1) { p.position } else { None };
        let controls = owner == Some(1);
        if !controls
            && self.slider.is_none()
            && self.orbit.is_none()
            && self.bones.pointer_at(p, owner == Some(0))
        {
            if self.bones.manipulating() {
                self.paused = true;
            }
            self.slider = None;
            self.orbit = None;
            return;
        }
        if !p.held && !p.pressed && !p.right_held && !p.right_pressed {
            self.orbit = None;
            self.slider = None;
        }
        let Some([x, y]) = p.position else {
            self.orbit = None;
            return;
        };
        let row = (y - Self::CONTROLS_TOP) / ROW;
        let panel = controls;
        if !panel {
            self.zoom = (self.zoom - p.wheel * 0.12).clamp(-1.6, 1.3);
        }
        if p.middle_pressed {
            self.pan_start = Some([x, y]);
        }
        if p.middle_held {
            if let Some(last) = self.pan_start {
                let orbit = self.orientation();
                let metres =
                    2. * self.distance() * (self.home.fov_y * 0.5).tan() / size[1].max(1) as f32;
                self.pan += orbit * Vec3::new(last[0] - x, y - last[1], 0.) * metres;
            }
            self.pan_start = Some([x, y]);
            return;
        }
        self.pan_start = None;
        if p.right_pressed && !panel {
            self.orbit = Some([x, y]);
        }
        if p.pressed {
            self.slider = None;
            self.orbit = None;
            if panel {
                if row < 0. || row >= (N + 1) as f32 {
                    return;
                }
                let index = row as usize;
                if index < N {
                    self.slider = Some(index);
                } else if Self::buttons()[0].contains([x, y]) {
                    self.paused = !self.paused;
                } else {
                    if !Self::buttons()[1].contains([x, y]) {
                        return;
                    }
                    self.pan = Vec3::ZERO;
                    self.yaw = 0.;
                    self.pitch = 0.;
                    self.zoom = 0.;
                    self.values.fill(0.);
                    self.paused = false;
                }
            } else {
                self.orbit = Some([x, y]);
            }
        }
        if p.held || p.pressed || p.right_held {
            if let Some(index) = self.slider {
                self.values[index] = (((x - TRACK) / (WIDTH - 12.0 - TRACK)).clamp(0.0, 1.0) * 2.0
                    - 1.0)
                    * self.rows[index].1;
            } else if let Some(prev) = self.orbit {
                self.yaw -= (x - prev[0]) * 0.008;
                self.pitch = (self.pitch + (y - prev[1]) * 0.008).clamp(-1.55, 1.0);
                self.orbit = Some([x, y]);
            }
        }
    }
    fn orientation(&self) -> Quat {
        let dir = (self.home.pos - self.home.target).normalize_or(Vec3::Z);
        Quat::from_rotation_y(self.yaw + dir.x.atan2(dir.z))
            * Quat::from_rotation_x((-self.pitch - dir.y.clamp(-1., 1.).asin()).clamp(-1.56, 1.56))
    }
    fn hud(&self, hud: &mut HudDrawList) {
        let clip = [0.0, 0.0, hud.size[0] as f32, hud.size[1] as f32];
        let top = Self::CONTROLS_TOP;
        for (i, (name, limit, unit)) in self.rows.iter().enumerate() {
            let y = top + i as f32 * ROW;
            let value = self.values[i];
            let w = WIDTH - 12.0 - TRACK;
            hud.rect([TRACK, y + 12.0, w, 3.0], [0.45, 0.48, 0.5, 1.0], clip);
            hud.rect(
                [
                    TRACK + w * (value / limit + 1.0) * 0.5 - 3.0,
                    y + 6.0,
                    6.0,
                    15.0,
                ],
                style::HIGHLIGHT,
                clip,
            );
            label(
                hud,
                100 + i as u64,
                [16.0, y + 3.0],
                format!("{name}  {value:.0} {unit}"),
            );
        }
        for (i, button) in Self::buttons().into_iter().enumerate() {
            let name = if i == 1 {
                "Reset"
            } else if self.paused {
                "Resume"
            } else {
                "Pause"
            };
            button.draw(
                hud,
                20 + i as u64,
                name,
                clip,
                15.,
                self.cursor.is_some_and(|p| button.contains(p)),
                true,
            );
        }
    }
    fn buttons() -> [style::EditorButton; 2] {
        let y = Self::CONTROLS_TOP + ROW * N as f32 + 2.;
        [
            style::EditorButton([12., y, 110., ROW - 4.]),
            style::EditorButton([126., y, 112., ROW - 4.]),
        ]
    }
}
fn content_transform(frame: WindowFrame, local: [f32; 4]) -> (f32, [f32; 2]) {
    let [x, y, w, h] = frame.body();
    let k = (w / local[2]).min(h / local[3]);
    (k, [x - local[0] * k, y - local[1] * k])
}
fn ui_scale(size: [u32; 2]) -> f32 {
    (size[1] as f32 / 410.)
        .min(size[0] as f32 / 570.)
        .clamp(0.8, 3.5)
}
fn label(hud: &mut HudDrawList, id: u64, position: [f32; 2], text: String) {
    let bounds = [0.0, 0.0, hud.size[0] as f32, hud.size[1] as f32];
    hud.text.push(TextRun {
        id,
        text,
        position,
        bounds,
        font_size: 15.0,
        color: style::FG,
        font: FontFace::Body,
        align: TextAlign::Left,
        width: WIDTH,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pose_panels_reopen_at_their_moved_positions_with_shortcuts() {
        let size = [900, 600];
        let rig = PoseEditor::bind([("Root", Mat4::IDENTITY)].into_iter(), |_| None).unwrap();
        let camera = Camera {
            pos: Vec3::Z * 3.,
            target: Vec3::ZERO,
            fov_y: 1.,
            near: 0.02,
        };
        let mut view = PoseViewport::new(rig, camera, [("Weight", 1., "")]).unwrap();
        {
            let name = "F2   Pose editor";
            let hud = view.draw(size, Mat4::IDENTITY, "");
            let header = hud
                .text
                .iter()
                .find(|r| r.text == format!("{name}    |    F1 all"))
                .unwrap()
                .position;
            let mut p = crate::Pointer {
                position: Some(header),
                pressed: true,
                held: true,
                ..Default::default()
            };
            view.pointer(p, size);
            p.pressed = false;
            p.position = Some([header[0] + 60., header[1]]);
            view.pointer(p, size);
            p.held = false;
            view.pointer(p, size);
            assert!(view.function_key(2) && !view.windows_open());
            assert!(view.function_key(2));
            let hud = view.draw(size, Mat4::IDENTITY, "");
            let moved = hud
                .text
                .iter()
                .find(|r| r.text == format!("{name}    |    F1 all"))
                .unwrap()
                .position;
            assert!((moved[0] - header[0] - 60.).abs() < 1., "{name}: {moved:?}");
        }
    }
    #[test]
    fn arbitrary_rig_can_be_picked_moved_and_exported_at_two_window_sizes() {
        for size in [[800, 600], [1920, 1080]] {
            let rig = PoseEditor::bind(
                [
                    ("Base", Mat4::IDENTITY),
                    ("Tip", Mat4::from_translation(Vec3::Y)),
                ]
                .into_iter(),
                |name| (name == "Tip").then(|| "Base".into()),
            )
            .unwrap();
            let camera = Camera {
                pos: Vec3::Z * 3.,
                target: Vec3::ZERO,
                fov_y: 1.,
                near: 0.02,
            };
            let mut viewport = PoseViewport::new(rig, camera, []).unwrap();
            viewport.bones.editor.apply(&mut [Mat4::IDENTITY; 2]);
            let hud = viewport.draw(size, Mat4::IDENTITY, "");
            assert_eq!(hud.size, size);
            let [x, y] = size.map(|v| v as f32 * 0.5);
            let press = crate::Pointer {
                position: Some([x, y]),
                pressed: true,
                held: true,
                ..Default::default()
            };
            viewport.pointer(press, size);
            viewport.pointer(
                crate::Pointer {
                    position: Some([x + 40., y]),
                    pressed: false,
                    ..press
                },
                size,
            );
            viewport.bones.editor.apply(&mut [Mat4::IDENTITY; 2]);
            let snapshot = viewport.snapshot();
            assert_eq!(snapshot["selected_joint"], "Base");
            assert!(
                snapshot["joints"][1]["model_position_m"][0]
                    .as_f64()
                    .unwrap()
                    > 0.01
            );
            assert!(viewport.paused);
            assert!(viewport.function_key(2) && !viewport.bones.shown);
            assert!(!viewport.function_key(10));
        }
    }
}
