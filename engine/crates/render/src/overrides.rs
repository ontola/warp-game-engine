//! Artist edits on top of rule placement (crate::placement), keyed on the
//! stable object ids: remove an object, replace it (move, turn, rescale,
//! swap its variant or kind), or add one that no rule makes. The edits are a
//! small diff that survives regeneration: a tweaked rule, another seed for
//! the detail or a changed terrain moves nothing that was pinned, and an
//! object a rule no longer makes simply has nothing to remove.
//!
//! Heights are kept relative to the ground (`lift`), so an edited object still
//! stands on the land when the terrain under it changes.
//!
//! Saved as a plain text file, one edit per line, so diffs stay readable:
//!
//! ```text
//! # warp overrides 1
//! remove 0001000000500007
//! replace 0001000000300004 rule=1 x=12.5 z=-40 lift=0 yaw=1.2 scale=1,1.3,1 variant=2
//! add ffff000000000001 rule=2 x=30 z=8 lift=-0.2 yaw=0 scale=2,2,2 variant=0
//! ```
//!
//! Added objects take ids with rule field 0xffff (kind 255, rule 255, which
//! no rule may use), numbered from 1, so they never collide with placed ones.
use crate::placement::{Instance, Rule, World};
use glam::{Vec2, Vec3};
use std::collections::BTreeMap;

/// Ids of added objects have this in their rule field (bits 48..64).
pub const ADDED: u64 = 0xffff << 48;

/// An object as an edit pins it: where it stands relative to the ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pinned {
    pub rule: u32,
    pub x: f32,
    pub z: f32,
    /// Metres above (negative: into) the ground at (x, z).
    pub lift: f32,
    pub yaw: f32,
    pub scale: Vec3,
    pub variant: u32,
}

impl Pinned {
    /// `instance` as it stands in `world`.
    pub fn of(instance: &Instance, world: &dyn World) -> Self {
        let ground = world.ground(instance.pos.x, instance.pos.z).pos.y;
        Self { rule: instance.rule, x: instance.pos.x, z: instance.pos.z, lift: instance.pos.y - ground,
            yaw: instance.yaw, scale: instance.scale, variant: instance.variant }
    }
    fn instance(&self, id: u64, world: &dyn World) -> Instance {
        Instance { rule: self.rule, id, pos: Vec3::new(self.x, world.ground(self.x, self.z).pos.y + self.lift, self.z),
            yaw: self.yaw, scale: self.scale, variant: self.variant }
    }
}

/// What an edit does to the object with its id.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Edit {
    Remove,
    /// The object stands like this, whether or not a rule still makes it.
    Replace(Pinned),
    /// An object no rule makes (id has `ADDED` in its rule field).
    Add(Pinned),
}

/// A world's edits, by object id (sorted, so the file is stable).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Overrides {
    pub edits: BTreeMap<u64, Edit>,
}

impl Overrides {
    pub fn is_empty(&self) -> bool { self.edits.is_empty() }

    pub fn remove(&mut self, id: u64) { self.edits.insert(id, Edit::Remove); }

    /// Pin object `id` as `pinned` (for example `Pinned::of` a placed object,
    /// then moved).
    pub fn replace(&mut self, id: u64, pinned: Pinned) { self.edits.insert(id, Edit::Replace(pinned)); }

    /// Add an object; returns its new id.
    pub fn add(&mut self, pinned: Pinned) -> u64 {
        let next = self.edits.range(ADDED..).next_back().map_or(1, |(id, _)| (id & !ADDED) + 1);
        let id = ADDED | next;
        self.edits.insert(id, Edit::Add(pinned));
        id
    }

    /// `placed` (from `placement::place(rules, ..)` around `centre` within
    /// `radius`) with the edits applied: removed and replaced objects leave,
    /// then every replaced and added object of one of `rules` inside the
    /// disc stands where it is pinned. Placed objects keep their order;
    /// pinned ones follow in id order.
    pub fn apply(&self, placed: Vec<Instance>, rules: &[Rule], world: &dyn World, centre: Vec2, radius: f32) -> Vec<Instance> {
        if self.edits.is_empty() { return placed; }
        let mut out: Vec<Instance> = placed.into_iter().filter(|o| !self.edits.contains_key(&o.id)).collect();
        for (&id, edit) in &self.edits {
            let (Edit::Replace(p) | Edit::Add(p)) = edit else { continue };
            if rules.iter().any(|r| r.id == p.rule) && Vec2::new(p.x - centre.x, p.z - centre.y).length() <= radius {
                out.push(p.instance(id, world));
            }
        }
        out
    }

    /// Whether object `id` is gone or pinned elsewhere (so a far, painted
    /// copy of the generated object must not be drawn either).
    pub fn displaces(&self, id: u64) -> bool { matches!(self.edits.get(&id), Some(Edit::Remove | Edit::Replace(_))) }

    pub fn to_text(&self) -> String {
        let mut s = String::from("# warp overrides 1\n");
        for (id, edit) in &self.edits {
            match edit {
                Edit::Remove => s += &format!("remove {id:016x}\n"),
                Edit::Replace(p) => s += &format!("replace {id:016x} {}\n", pinned_text(p)),
                Edit::Add(p) => s += &format!("add {id:016x} {}\n", pinned_text(p)),
            }
        }
        s
    }

    /// Parse `to_text`'s format. Blank lines and `#` comments are skipped;
    /// an unknown word or a malformed line is an error naming the line.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut out = Self::default();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') { continue; }
            let err = |what: &str| format!("overrides line {}: {what}: {line}", n + 1);
            let mut words = line.split_whitespace();
            let verb = words.next().unwrap();
            let id = words.next().and_then(|w| u64::from_str_radix(w, 16).ok()).ok_or_else(|| err("expected a hex id"))?;
            let edit = match verb {
                "remove" => Edit::Remove,
                "replace" | "add" => {
                    let p = parse_pinned(words).map_err(|e| err(&e))?;
                    if verb == "add" { Edit::Add(p) } else { Edit::Replace(p) }
                }
                _ => return Err(err("expected remove, replace or add")),
            };
            if matches!(edit, Edit::Add(_)) != (id & ADDED == ADDED) {
                return Err(err("added ids (and only those) start with ffff"));
            }
            if out.edits.insert(id, edit).is_some() { return Err(err("id edited twice")); }
        }
        Ok(out)
    }
}

fn pinned_text(p: &Pinned) -> String {
    format!("rule={} x={} z={} lift={} yaw={} scale={},{},{} variant={}", p.rule, p.x, p.z, p.lift, p.yaw, p.scale.x, p.scale.y, p.scale.z, p.variant)
}

fn parse_pinned<'a>(words: impl Iterator<Item = &'a str>) -> Result<Pinned, String> {
    let mut p = Pinned { rule: 0, x: f32::NAN, z: f32::NAN, lift: 0.0, yaw: 0.0, scale: Vec3::ONE, variant: 0 };
    let mut rule = None;
    for word in words {
        let (key, value) = word.split_once('=').ok_or("expected key=value")?;
        let num = |v: &str| v.parse::<f32>().map_err(|_| format!("bad number for {key}"));
        match key {
            "rule" => rule = Some(value.parse::<u32>().map_err(|_| "bad rule")?),
            "x" => p.x = num(value)?,
            "z" => p.z = num(value)?,
            "lift" => p.lift = num(value)?,
            "yaw" => p.yaw = num(value)?,
            "variant" => p.variant = value.parse().map_err(|_| "bad variant")?,
            "scale" => {
                let v = value.split(',').map(num).collect::<Result<Vec<_>, _>>()?;
                p.scale = match v[..] { [s] => Vec3::splat(s), [x, y, z] => Vec3::new(x, y, z), _ => return Err("scale is s or x,y,z".into()) };
            }
            _ => return Err(format!("unknown key {key}")),
        }
    }
    p.rule = rule.ok_or("missing rule")?;
    if !(p.x.is_finite() && p.z.is_finite()) { return Err("missing x or z".into()); }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::placement::{self, TerrainWorld};
    use crate::terrain_field::Style;

    fn world(style: &Style) -> TerrainWorld<'_> { TerrainWorld { seed: 7, style, constraints: None } }

    #[test]
    fn edits_survive_a_tweaked_rule() {
        let style = Style::default();
        let land = world(&style);
        let (c, r) = (Vec2::ZERO, 250.0);
        let rules = [placement::tree_rule(0.6), placement::boulder_rule()];
        let placed = placement::place(&rules, &land, c, r);
        let trees: Vec<_> = placed.iter().filter(|o| o.rule == placement::TREE).copied().collect();
        assert!(trees.len() > 20, "fixture needs trees, got {}", trees.len());
        let mut edits = Overrides::default();
        edits.remove(trees[0].id);
        let mut moved = Pinned::of(&trees[1], &land);
        moved.x += 3.0;
        moved.variant = 9;
        edits.replace(trees[1].id, moved);
        let added = edits.add(Pinned { rule: placement::BOULDER, x: 10.0, z: 20.0, lift: -0.3, yaw: 0.5, scale: Vec3::splat(2.0), variant: 1 });
        let far = edits.add(Pinned { rule: placement::BOULDER, x: 900.0, z: 0.0, lift: 0.0, yaw: 0.0, scale: Vec3::ONE, variant: 0 });
        assert_eq!((added, far), (ADDED | 1, ADDED | 2));

        let check = |placed: Vec<Instance>, rules: &[placement::Rule]| {
            let out = edits.apply(placed.clone(), rules, &land, c, r);
            assert!(!out.iter().any(|o| o.id == trees[0].id), "removed stays removed");
            let m = out.iter().filter(|o| o.id == trees[1].id).collect::<Vec<_>>();
            assert_eq!(m.len(), 1, "a replaced object stands exactly once");
            assert_eq!((m[0].pos.x, m[0].variant), (trees[1].pos.x + 3.0, 9));
            let lift = trees[1].pos.y - land.ground(trees[1].pos.x, trees[1].pos.z).pos.y;
            assert!((m[0].pos.y - land.ground(m[0].pos.x, m[0].pos.z).pos.y - lift).abs() < 1e-3, "a moved object keeps its height above the ground");
            let a = out.iter().find(|o| o.id == added).expect("added object lands");
            assert!((a.pos.y - (land.ground(10.0, 20.0).pos.y - 0.3)).abs() < 1e-4);
            assert!(!out.iter().any(|o| o.id == far), "outside the disc");
            // Everything else is untouched and in order.
            let rest: Vec<_> = placed.iter().filter(|o| !edits.edits.contains_key(&o.id)).copied().collect();
            assert_eq!(&out[..rest.len()], &rest[..]);
        };
        check(placed, &rules);
        // Pinned objects only join the rules they belong to.
        let only_trees = edits.apply(Vec::new(), &rules[..1], &land, c, r);
        assert_eq!(only_trees.iter().map(|o| o.id).collect::<Vec<_>>(), [trees[1].id]);
        // A sparser tree rule: some cells lose their tree, the edits still land
        // (the replaced tree too, even where its rule no longer grows it).
        let sparser = [placement::tree_rule(0.2), placement::boulder_rule()];
        let thin = placement::place(&sparser, &land, c, r);
        assert!(thin.len() < trees.len());
        check(thin, &sparser);
    }

    #[test]
    fn text_round_trips_and_rejects_mistakes() {
        let mut edits = Overrides::default();
        edits.remove(0x0001_0000_0050_0007);
        edits.replace(0x0001_0000_0030_0004, Pinned { rule: 1, x: 12.5, z: -40.0, lift: 0.0, yaw: 1.2, scale: Vec3::new(1.0, 1.3, 1.0), variant: 2 });
        edits.add(Pinned { rule: 2, x: 30.0, z: 8.0, lift: -0.2, yaw: 0.0, scale: Vec3::splat(2.0), variant: 0 });
        let text = edits.to_text();
        assert_eq!(Overrides::parse(&text).unwrap(), edits);
        assert!(text.contains("remove 0001000000500007\n"));
        for bad in ["move 01", "remove", "add 0001000000000001 rule=1 x=0 z=0", "replace ffff000000000001 rule=1 x=0 z=0",
            "replace 01 x=0 z=0", "replace 01 rule=1 x=0", "replace 01 rule=1 x=0 z=0 colour=red", "remove 01\nremove 01"] {
            assert!(Overrides::parse(bad).is_err(), "{bad}");
        }
        assert!(Overrides::parse("# nothing\n\n").unwrap().is_empty());
    }
}
