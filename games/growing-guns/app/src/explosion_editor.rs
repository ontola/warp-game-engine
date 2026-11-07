//! Live F12 authoring. The Luau knob block supplies the sliders and its returned
//! table maps them onto the conventional blast effect. Evaluate only on edits.
use app::editor::{SceneControl, SceneTuning};
use script::mlua::Table;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

const BUILTIN: &str = include_str!("../../mods/base/explosion_editor.luau");
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlastLook {
    pub size: f32,
    pub dust: f32,
    pub impulse: f32,
    pub green: f32,
    pub blue: f32,
}

impl Default for BlastLook {
    fn default() -> Self {
        static DEFAULT: std::sync::OnceLock<BlastLook> = std::sync::OnceLock::new();
        *DEFAULT.get_or_init(|| evaluate(BUILTIN).expect("built-in explosion preset"))
    }
}
fn evaluate(source: &str) -> Result<BlastLook, String> {
    let mut host = script::Host::new(Default::default(), |_, _| Ok(()))?;
    host.add_source("explosion_editor", source)?;
    let exports = host.modules()[0]
        .exports
        .as_ref()
        .ok_or("missing explosion exports")?;
    let t: Table = exports.get("blast_look").map_err(|e| e.to_string())?;
    let number = |key: &str, min: f32, max: f32| -> Result<f32, String> {
        let n: f32 = t.get(key).map_err(|e| e.to_string())?;
        if !n.is_finite() || !(min..=max).contains(&n) {
            return Err(format!("{key} outside {min}..{max}"));
        }
        Ok(n)
    };
    Ok(BlastLook {
        size: number("size", 0.01, 10.)?,
        dust: number("dust", 0., 10.)?,
        impulse: number("impulse", 0., 10.)?,
        green: number("green", 0., 1.)?,
        blue: number("blue", 0., 1.)?,
    })
}
#[derive(Clone)]
struct Param {
    key: String,
    label: String,
    authored: f64,
    baseline: f64,
    min: f64,
    max: f64,
}
fn schema(source: &str) -> Result<Vec<Param>, String> {
    let mut params = Vec::new();
    for k in live::knobs::parse(source) {
        let (label, range) = k
            .about
            .rsplit_once(',')
            .ok_or_else(|| format!("{}: expected label, min to max", k.name))?;
        let (min, max) =
            crate::knobs::range(range).ok_or_else(|| format!("{}: invalid range", k.name))?;
        let value: f64 = k
            .value
            .parse()
            .map_err(|_| format!("{}: expected numeric literal", k.name))?;
        if !min.is_finite()
            || !max.is_finite()
            || !(min..=max).contains(&value)
            || params.iter().any(|p: &Param| p.key == k.name)
        {
            return Err(format!("invalid knob {}", k.name));
        }
        params.push(Param {
            key: k.name,
            label: label.trim().into(),
            authored: value,
            baseline: value,
            min,
            max,
        });
    }
    Ok(params)
}
pub struct ExplosionEditor {
    pub panel: SceneTuning,
    pub look: BlastLook,
    params: Vec<Param>,
    source: String,
    applied: Vec<f64>,
    path: Option<PathBuf>,
    next_poll: Instant,
    last_read: String,
}
impl ExplosionEditor {
    pub fn new() -> Self {
        let mut e = Self {
            panel: SceneTuning::from_controls("growing-guns", "Explosions", vec![]),
            look: BlastLook::default(),
            params: vec![],
            source: String::new(),
            applied: vec![],
            path: crate::scripts::find_dir().map(|d| d.join("base/explosion_editor.luau")),
            next_poll: Instant::now(),
            last_read: String::new(),
        };
        e.reload(BUILTIN).expect("built-in explosion controls");
        e.update();
        e
    }
    fn resolve(source: &str, params: &[Param], values: &[f64]) -> Result<BlastLook, String> {
        let mut text = source.to_owned();
        for (p, &value) in params.iter().zip(values) {
            if value != p.authored {
                text = live::knobs::set(&text, &p.key, &value.to_string())?;
            }
        }
        evaluate(&text)
    }
    fn reload(&mut self, source: &str) -> Result<(), String> {
        let mut params = schema(source)?;
        let mut values = Vec::new();
        for p in &mut params {
            let mut value = p.authored;
            if let Some((i, old)) = self
                .params
                .iter()
                .enumerate()
                .find(|(_, old)| old.key == p.key)
            {
                p.baseline = old.baseline.clamp(p.min, p.max);
                if old.authored == p.authored {
                    value = self.panel.value(i).clamp(p.min, p.max);
                }
            }
            values.push(value);
        }
        let look = Self::resolve(source, &params, &values)?;
        // Value-only reloads keep pointer capture during a slider drag.
        if params.len() == self.params.len()
            && params.iter().zip(&self.params).all(|(a, b)| {
                a.key == b.key && a.label == b.label && a.min == b.min && a.max == b.max
            })
        {
            for (i, &v) in values.iter().enumerate() {
                if self.panel.value(i) != v {
                    self.panel.set_value(i, v);
                }
            }
        } else {
            self.panel.replace_controls(
                params
                    .iter()
                    .map(|p| SceneControl {
                        label: p.label.clone(),
                        default: p.baseline,
                        min: p.min,
                        max: p.max,
                        step: 0.05,
                    })
                    .collect(),
                values.clone(),
            );
        }
        self.params = params;
        self.look = look;
        self.applied = values;
        self.source = source.into();
        Ok(())
    }
    pub fn command(&mut self, args: &[&str]) -> Result<String, String> {
        self.update();
        if args.is_empty() {
            return Ok(self
                .params
                .iter()
                .enumerate()
                .map(|(i, p)| format!("{} = {}", p.key, self.panel.value(i)))
                .collect::<Vec<_>>()
                .join("\n"));
        }
        let [key, raw] = args else {
            return Err("fx [name value]".into());
        };
        let i = self
            .params
            .iter()
            .position(|p| p.key == *key)
            .ok_or_else(|| format!("unknown explosion knob {key}"))?;
        let value: f64 = raw.parse().map_err(|_| "expected a number")?;
        if !(self.params[i].min..=self.params[i].max).contains(&value) {
            return Err("value outside slider range".into());
        }
        self.panel.set_value(i, value);
        self.update();
        if self.panel.value(i) != value {
            return Err("explosion preset rejected value".into());
        }
        Ok(format!("{key} = {value}"))
    }
    pub fn update(&mut self) {
        if Instant::now() >= self.next_poll {
            self.next_poll = Instant::now() + Duration::from_millis(200);
            if let Some(source) = self
                .path
                .as_ref()
                .and_then(|p| std::fs::read_to_string(p).ok())
                && source != self.last_read
            {
                if let Err(e) = self.reload(&source) {
                    eprintln!("Explosion editor: {e}");
                }
                self.last_read = source;
            }
        }
        if self
            .applied
            .iter()
            .enumerate()
            .any(|(i, &v)| v != self.panel.value(i))
        {
            let values: Vec<_> = (0..self.params.len())
                .map(|i| self.panel.value(i))
                .collect();
            match Self::resolve(&self.source, &self.params, &values) {
                Ok(look) => {
                    self.look = look;
                    self.applied = values;
                }
                Err(e) => {
                    eprintln!("Explosion editor: {e}");
                    for (i, &v) in self.applied.iter().enumerate() {
                        self.panel.set_value(i, v);
                    }
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn live_schema_and_bindings_change_together() {
        let mut e = ExplosionEditor::new();
        let edited = BUILTIN
            .replace(
                "local K = {",
                "local K = {\n    big = 1, -- Extra size, 1 to 3",
            )
            .replace("size = K.fireball_size,", "size = K.big * K.fireball_size,");
        e.reload(&edited).unwrap();
        e.path = None;
        e.panel.set_value(0, 2.);
        e.update();
        assert_eq!(e.look.size, 2.);
        let external = edited.replace("big = 1,", "big = 3,");
        e.reload(&external).unwrap();
        assert_eq!(e.panel.value(0), 3.);
        assert_eq!(e.look.size, 3.);
        assert!(e.reload("broken Luau!").is_err());
        assert_eq!(e.look.size, 3.);
    }
}
