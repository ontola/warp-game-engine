//! Sandboxed, hot-reloadable decisions; no clocks, IO or random state in Luau.
use script::{Host, Limits, mlua::LuaSerdeExt};
use serde::{Serialize, de::DeserializeOwned};
pub struct Rules {
    pub host: Host,
}
#[derive(serde::Deserialize)]
pub struct Balance {
    pub countdown: f64,
    pub respawn_seconds: f64,
    pub race_timeout: f64,
    pub refill_rate: f64,
    pub pickup_seconds: f64,
    pub pickup_spacing: f64,
    pub warp_seconds: f64,
    pub warp_speed: f64,
    pub sentry_seconds: f64,
    pub sentry_interval: f64,
    pub sentry_damage: f64,
    pub rail_damage: f64,
    pub emp_seconds: f64,
    pub missile_damage: f64,
    pub missile_radius: f32,
    pub bomb_damage: f64,
    pub bomb_radius: f32,
    pub projectile_timeout: f64,
}
#[derive(serde::Deserialize)]
struct TrackRecipe {
    dimensions: crate::generation::Dimensions,
    corner_scale: f64,
    width_scale: f64,
    hard_flight: bool,
    stadium: bool,
    boosts: Vec<[f64; 2]>,
    loops: Vec<f64>,
    tunnels: Vec<[f64; 2]>,
    corners: Vec<[f64; 3]>,
    sections: Vec<(String, f64, f64)>,
    jumps: Vec<[f64; 2]>,
    exposed: Vec<[f64; 2]>,
}
impl Rules {
    pub fn environment(&mut self, biome: &str) -> Result<crate::environment::Environment, String> {
        let e: crate::environment::Environment =
            self.call("environment", serde_json::json!({"biome":biome}))?;
        if !(-500.0..=0.0).contains(&e.ground)
            || !(0.0..=100.0).contains(&e.hills)
            || !(45.0..=300.0).contains(&e.spacing)
            || !(0.0..=0.95).contains(&e.skip)
            || !(5.0..=600.0).contains(&e.height[0])
            || !(e.height[0]..=800.0).contains(&e.height[1])
            || !(0.0..=150000.0).contains(&e.sun)
            || !(0.0..=50000.0).contains(&e.sky)
            || !(0.0..=50000.0).contains(&e.ambient)
            || !(0.0..=20.0).contains(&e.exposure)
        {
            return Err("environment values outside supported bounds".into());
        }
        Ok(e)
    }

    pub fn balance(&mut self) -> Result<Balance, String> {
        let b: Balance = self.call("balance", ())?;
        let values = [
            b.countdown,
            b.respawn_seconds,
            b.race_timeout,
            b.refill_rate,
            b.pickup_seconds,
            b.pickup_spacing,
            b.warp_seconds,
            b.warp_speed,
            b.sentry_seconds,
            b.sentry_interval,
            b.sentry_damage,
            b.rail_damage,
            b.emp_seconds,
            b.missile_damage,
            b.missile_radius as f64,
            b.bomb_damage,
            b.bomb_radius as f64,
            b.projectile_timeout,
        ];
        if values
            .iter()
            .any(|x| !x.is_finite() || *x < 0. || *x > 10000.)
            || b.pickup_spacing < 100.
            || b.sentry_interval < 1. / 120.
            || b.respawn_seconds < 0.1
            || b.race_timeout < 10.
            || b.missile_radius < 1.
            || b.bomb_radius < 1.
        {
            return Err("balance values outside supported bounds".into());
        }
        Ok(b)
    }
    pub fn track(
        &mut self,
        seed: i64,
        difficulty: &str,
    ) -> Result<crate::generation::Recipe, String> {
        let t: TrackRecipe = self.call(
            "track",
            serde_json::json!({"seed":seed,"difficulty":difficulty}),
        )?;
        if !(0.0..=2.0).contains(&t.corner_scale) || !(0.65..=1.6).contains(&t.width_scale) {
            return Err("track width/corner scale outside supported bounds".into());
        }
        for spans in [&t.boosts, &t.tunnels, &t.jumps, &t.exposed] {
            if spans.len() > 32
                || spans.iter().any(|s| {
                    !s[0].is_finite()
                        || !s[1].is_finite()
                        || s[0] < 0.
                        || s[1] > 1.
                        || s[1] - s[0] < 0.005
                })
            {
                return Err("invalid track span".into());
            }
        }
        if t.loops.len() > 6 || t.loops.iter().any(|x| !(0.0..=0.89).contains(x)) {
            return Err("invalid loop placement".into());
        }
        if t.sections.len() > 32
            || t.corners.len() > 16
            || t.corners.iter().any(|c| {
                !(0.0..=1.0).contains(&c[0])
                    || !(-250.0..=250.0).contains(&c[1])
                    || !(0.04..=0.5).contains(&c[2])
            })
        {
            return Err("invalid corner or section count".into());
        }
        let d = &t.dimensions;
        for (v, lo, hi) in [
            (d.radius, 500., 2000.),
            (d.amplitude, 0., 300.),
            (d.stretch, 0.7, 2.),
            (d.climb, 0., 300.),
            (d.corner_a, 0.05, 0.5),
            (d.corner_b, 0.05, 0.5),
            (d.corner_shift, -0.2, 0.2),
            (d.loop_radius, 120., 400.),
        ] {
            if !(lo..=hi).contains(&v[0]) || !(v[0]..=hi).contains(&v[1]) {
                return Err("invalid track dimension range".into());
            }
        }
        if d.lobes[0] < 1
            || d.lobes[1] > 8
            || d.lobes[0] > d.lobes[1]
            || d.hills[0] < 1
            || d.hills[1] > 6
            || d.hills[0] > d.hills[1]
            || !(15.0..=60.0).contains(&d.road_width[0])
            || d.road_width[1..].iter().any(|v| !(0.0..=20.0).contains(v))
        {
            return Err("invalid track wave or width range".into());
        }
        let mut p = crate::generation::Recipe {
            dimensions: t.dimensions,
            name: [
                "Grand Circuit",
                "Underpass",
                "Switchback",
                "Sky Circus",
                "Velocity",
                "Pipeline",
            ][crate::generation::Recipe::index(seed)],
            stadium: false,
            corner_scale: 1.,
            width_scale: 1.,
            boosts: vec![],
            loops: vec![],
            tunnels: vec![],
            corners: vec![],
            sections: vec![],
            jumps: vec![],
            hard_flight: false,
            exposed: vec![],
        };
        p.corner_scale = t.corner_scale;
        p.width_scale = t.width_scale;
        p.hard_flight = t.hard_flight;
        p.stadium = t.stadium;
        p.boosts = t.boosts;
        p.loops = t.loops;
        p.tunnels = t.tunnels;
        p.corners = t.corners;
        p.jumps = t.jumps;
        p.exposed = t.exposed;
        p.sections = t
            .sections
            .into_iter()
            .map(|(kind, a, b)| {
                let kind = match kind.as_str() {
                    "open" => "open",
                    "halfpipe" => "halfpipe",
                    "tube" => "tube",
                    "split" => "split",
                    "narrows" => "narrows",
                    "chicane" => "chicane",
                    _ => return Err("invalid track section".to_string()),
                };
                if !(0.0..1.0).contains(&a)
                    || !b.is_finite()
                    || b > a + 0.5
                    || b - a < 0.005
                    || b > 1.
                {
                    return Err("invalid section span".into());
                }
                Ok((kind, a, b))
            })
            .collect::<Result<_, _>>()?;
        Ok(p)
    }
    pub fn new(path: Option<&std::path::Path>) -> Result<Self, String> {
        let mut host = Host::new(
            Limits {
                memory: 16 << 20,
                steps: 20000,
            },
            |_, _| Ok(()),
        )?;
        if let Some(path) = path {
            host.watch(path);
        } else {
            host.add_source("game.luau", include_str!("../../rules/game.luau"))?;
        }
        for m in host.modules() {
            if let Some(e) = &m.error {
                return Err(e.clone());
            }
        }
        for name in [
            "ai",
            "track",
            "plan",
            "pickup",
            "balance",
            "menu",
            "environment",
            "activate",
        ] {
            if !host.has(name) {
                return Err(format!("SpaceRacer rules must export {name}"));
            }
        }
        Ok(Self { host })
    }
    pub fn call<T: DeserializeOwned>(
        &mut self,
        name: &str,
        input: impl Serialize,
    ) -> Result<T, String> {
        let value = self
            .host
            .lua()
            .to_value(&input)
            .map_err(|e| e.to_string())?;
        let (module, f) = self
            .host
            .each(name)
            .into_iter()
            .last()
            .ok_or_else(|| format!("missing rule: {name}"))?;
        self.host
            .run(module, name, |lua| {
                let out: script::mlua::Value = f.call(value)?;
                lua.from_value(out)
            })
            .ok_or_else(|| self.host.drain_log().join("\n"))
    }
    pub fn poll(&mut self) -> Vec<String> {
        self.host.poll();
        self.host.drain_log()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scripts_control_balance_and_seed_recipes() {
        let mut r = Rules::new(None).unwrap();
        for seed in 31..37 {
            let t: serde_json::Value = r
                .call(
                    "track",
                    serde_json::json!({"seed":seed,"difficulty":"normal"}),
                )
                .unwrap();
            assert!(t["width_scale"].as_f64().unwrap() > 0.);
        }
        for n in 0..100 {
            let item: String = r
                .call("pickup", serde_json::json!({"rank":1,"roll":n as f64/100.}))
                .unwrap();
            assert_ne!(item, "warp");
        }
        let failed = r.host.load_text("game.luau", "this is not valid Luau");
        assert!(failed.is_err());
        let item: String = r
            .call("pickup", serde_json::json!({"rank":6,"roll":0.2}))
            .unwrap();
        assert_eq!(item, "warp");
    }
    #[test]
    fn edited_recipe_changes_geometry_without_native_rebuild() {
        let mut r = Rules::new(None).unwrap();
        let before = crate::generation::Generated::with_recipe(
            35,
            "normal",
            "city",
            r.track(35, "normal").unwrap(),
        );
        let source =
            include_str!("../../rules/game.luau").replace("width_scale=1,", "width_scale=1.3,");
        r.host.load_text("game.luau", &source).unwrap();
        let after = crate::generation::Generated::with_recipe(
            35,
            "normal",
            "city",
            r.track(35, "normal").unwrap(),
        );
        assert!(after.track.nodes[0].geometry.width > before.track.nodes[0].geometry.width * 1.2);
        let bad = source.replace("width_scale=1.3,", "width_scale=0/0,");
        r.host.load_text("game.luau", &bad).unwrap();
        assert!(r.track(35, "normal").is_err());
    }
    #[test]
    fn unsafe_script_values_cannot_allocate_unbounded_pickups() {
        let mut r = Rules::new(None).unwrap();
        r.host
            .load_text(
                "game.luau",
                &include_str!("../../rules/game.luau")
                    .replace("pickup_spacing=1300", "pickup_spacing=0"),
            )
            .unwrap();
        assert!(r.balance().is_err());
    }
}
