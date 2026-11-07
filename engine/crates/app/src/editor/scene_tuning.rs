//! Live numerical controls supplied by a game scene. Defaults are also the
//! comparison baseline used for visual change marks and editor handoffs.
use render::debug_ui::{DebugAction, DebugControl, DebugMenu, DebugValue};
use serde_json::json;
use std::io::Write;

pub const PAGE: u32 = crate::debug::GAME_IDS;
const SAVE: u32 = PAGE + 1;
const FIRST: u32 = PAGE + 2;

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
}
impl SceneTuning {
    pub fn new(game: &'static str, title: &'static str, specs: &'static [SceneControl]) -> Self {
        Self {
            game,
            title,
            specs,
            values: specs.iter().map(|p| p.default).collect(),
            notice: String::new(),
        }
    }
    pub fn value(&self, index: usize) -> f64 {
        self.values[index]
    }
    pub fn menu(&self) -> (Vec<DebugMenu>, Vec<DebugControl>) {
        let mut menu = super::EditorMenus::default();
        menu.page(
            PAGE,
            None,
            self.title,
            "Live scene controls. A star marks a change from the default.",
        );
        for (i, (spec, &value)) in self.specs.iter().zip(&self.values).enumerate() {
            let label = if (value - spec.default).abs() > 1e-8 {
                format!("* {} (was {:.2})", spec.label, spec.default)
            } else {
                spec.label.into()
            };
            menu.control(
                PAGE,
                FIRST + i as u32,
                &label,
                DebugValue::Number {
                    value,
                    min: spec.min,
                    max: spec.max,
                    step: spec.step,
                    decimals: 2,
                    unit: String::new(),
                },
                "Adjust live. Changed values show their original setting.",
            );
        }
        menu.control(PAGE,SAVE,if self.notice.is_empty(){"Save changes for LLM"}else{&self.notice},DebugValue::Button,"Write the before/after values to build/editor-changes and copy the JSON to the clipboard.");
        menu.finish()
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
            _ => {}
        }
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
        assert!(t.menu().1[0].label.starts_with("* Glow"));
        assert_eq!(
            t.report()["changes"][0],
            json!({"parameter":"Glow","before":1.,"after":1.5})
        );
        t.apply(DebugAction::SetNumber {
            id: FIRST,
            value: 1.,
        });
        assert_eq!(t.menu().1[0].label, "Glow");
        assert_eq!(t.report()["changes"], json!([]));
    }
}
