//! Live numerical controls supplied by a game scene. Defaults are also the
//! comparison baseline used for visual change marks and editor handoffs.
use render::debug_ui::{
    DebugAction, DebugControl, DebugKey, DebugModel, DebugResponse, DebugUi, DebugValue,
};
use serde_json::json;
use std::io::Write;

pub(crate) const WINDOW_ID: u64 = 0x5343_454e_4500_0000;
const SAVE: u32 = crate::debug::GAME_IDS + 1;
const FIRST: u32 = crate::debug::GAME_IDS + 2;

pub struct SceneControl {
    pub label: &'static str,
    pub default: f64,
    pub min: f64,
    pub max: f64,
    pub step: f64,
}
pub struct SceneTuning {
    game: &'static str,
    pub title: &'static str,
    specs: &'static [SceneControl],
    values: Vec<f64>,
    notice: String,
    pub(crate) ui: DebugUi,
    model: DebugModel,
}
impl SceneTuning {
    pub fn new(game: &'static str, title: &'static str, specs: &'static [SceneControl]) -> Self {
        let mut tuning = Self {
            model: Default::default(),
            ui: DebugUi::for_window(WINDOW_ID, 12, title),
            game,
            title,
            specs,
            values: specs.iter().map(|p| p.default).collect(),
            notice: String::new(),
        };
        tuning.refresh();
        tuning
    }
    pub fn value(&self, index: usize) -> f64 {
        self.values[index]
    }
    pub(crate) fn set_visible(&mut self, open: bool) {
        self.ui.open = open;
        self.ui.cancel_interaction();
    }
    fn handle(&mut self, response: DebugResponse) -> bool {
        if let Some(action) = response.action {
            self.apply(action);
        }
        response.consumed
    }
    pub(crate) fn key(&mut self, key: DebugKey, size: [u32; 2]) -> bool {
        let response = self.ui.on_key(key, size, &self.model);
        self.handle(response)
    }
    pub(crate) fn press(&mut self, down: bool, size: [u32; 2]) {
        let response = if down {
            self.ui.on_pointer_down(size, &self.model)
        } else {
            self.ui.pointer_up()
        };
        self.handle(response);
    }
    pub(crate) fn scroll(&mut self, lines: f32, size: [u32; 2]) {
        self.ui.on_scroll(lines, size, &self.model);
    }
    pub(crate) fn tick(&mut self, dt: f64, size: [u32; 2]) {
        let response = self.ui.tick(dt, size, &self.model);
        self.handle(response);
    }
    pub(crate) fn draw(&mut self, hud: &mut render::hud::HudDrawList) {
        self.ui.draw(hud, &self.model);
    }
    fn refresh(&mut self) {
        let controls = &mut self.model.controls;
        controls.clear();
        for (i, (spec, &value)) in self.specs.iter().zip(&self.values).enumerate() {
            let label = if (value - spec.default).abs() > 1e-8 {
                format!("* {} (was {:.2})", spec.label, spec.default)
            } else {
                spec.label.into()
            };
            controls.push(DebugControl {
                id: FIRST + i as u32,
                label,
                menu: None,
                enabled: true,
                advanced: false,
                value: DebugValue::Number {
                    value,
                    min: spec.min,
                    max: spec.max,
                    step: spec.step,
                    decimals: 2,
                    unit: String::new(),
                },
                help: "Adjust live. Changed values show their original setting.".into(),
            });
        }
        controls.push(DebugControl {
            id: SAVE,
            menu: None,
            enabled: true,
            advanced: false,
            value: DebugValue::Button,
            label: if self.notice.is_empty() {
                "Save changes for LLM".into()
            } else {
                self.notice.clone()
            },
            help: "Save the changes to build/editor-changes and copy the JSON to the clipboard."
                .into(),
        });
        self.model.status = Some("A star marks a change from the default.".into());
    }
    pub fn apply(&mut self, action: DebugAction) {
        match action {
            DebugAction::SetNumber { id, value } if id >= FIRST && value.is_finite() => {
                let i = (id - FIRST) as usize;
                if let Some(spec) = self.specs.get(i) {
                    self.values[i] = value.clamp(spec.min, spec.max);
                    self.notice.clear();
                }
            }
            DebugAction::ResetDefaults => {
                for (v, spec) in self.values.iter_mut().zip(self.specs) {
                    *v = spec.default;
                }
                self.notice.clear();
            }
            DebugAction::Activate { id: SAVE } => {
                self.notice = match self.save() {
                    Ok(true) => "Saved and copied · save again".into(),
                    Ok(false) => "Saved to build/editor-changes".into(),
                    Err(e) => {
                        eprintln!("editor handoff: {e}");
                        "Save failed · retry".into()
                    }
                };
            }
            _ => return,
        }
        self.refresh();
    }
    fn report(&self) -> serde_json::Value {
        let changes: Vec<_> = self
            .specs
            .iter()
            .zip(&self.values)
            .filter(|(s, v)| (s.default - **v).abs() > 1e-8)
            .map(|(s, v)| json!({"parameter":s.label,"before":s.default,"after":v}))
            .collect();
        json!({"game":self.game,"scene_editor":self.title,"changes":changes})
    }
    fn save(&self) -> std::io::Result<bool> {
        let dir = std::path::Path::new("build/editor-changes");
        std::fs::create_dir_all(dir)?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = dir.join(format!("{}-{stamp}-{}.json", self.game, std::process::id()));
        let text = serde_json::to_string_pretty(&self.report())?;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        f.write_all(text.as_bytes())?;
        f.flush()?;
        println!("Editor changes saved to {}", path.display());
        Ok(crate::clipboard::set(&text))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scene_controls_have_an_independent_window_and_keyboard_state() {
        use render::hud::HudDrawList;
        let size = [1280, 720];
        let mut hud = HudDrawList::new(size);
        let mut graphics = DebugUi::default();
        graphics.open = true;
        graphics.draw(&mut hud, &DebugModel::default());
        let mut scene = SceneTuning::new(
            "racer",
            "Handling",
            &[SceneControl {
                label: "Steering",
                default: 1.,
                min: 0.,
                max: 2.,
                step: 0.1,
            }],
        );
        assert!(hud.text.iter().all(|t| !t.text.contains("Steering")));
        scene.set_visible(true);
        scene.draw(&mut hud);
        let titles: Vec<_> = hud
            .text
            .iter()
            .filter(|t| t.text.contains("F1 Dev Mode"))
            .collect();
        assert_eq!(titles.len(), 2);
        assert!(titles[0].text.contains("F2   Graphics"));
        assert!(titles[1].text.contains("F12   Handling"));
        assert!(titles[1].position[0] < titles[0].position[0]);
        let ids: std::collections::HashSet<_> = hud.text.iter().map(|t| t.id).collect();
        assert_eq!(ids.len(), hud.text.len());
        scene.key(DebugKey::Right, size);
        assert_eq!(scene.value(0), 1.1);
        scene.key(DebugKey::Close, size);
        assert!(!scene.ui.open && graphics.open);
    }

    #[test]
    fn live_edit_marks_reports_and_reverts() {
        let mut t = SceneTuning::new(
            "example",
            "Effects",
            &[SceneControl {
                label: "Glow",
                default: 1.,
                min: 0.,
                max: 2.,
                step: 0.1,
            }],
        );
        t.apply(DebugAction::SetNumber {
            id: FIRST,
            value: 1.5,
        });
        assert!(t.model.controls[0].label.starts_with("* Glow"));
        assert_eq!(
            t.report()["changes"][0],
            json!({"parameter":"Glow","before":1.,"after":1.5})
        );
        t.apply(DebugAction::SetNumber {
            id: FIRST,
            value: 1.,
        });
        assert_eq!(t.model.controls[0].label, "Glow");
        assert_eq!(t.report()["changes"], json!([]));
    }
}
