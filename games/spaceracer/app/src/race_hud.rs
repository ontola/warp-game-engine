//! Source-proportioned 800x450 visor instrumentation, scaled separately per viewport.
use glam::{Mat4, Vec2, Vec3};
use render::{
    Camera,
    hud::{FontFace, HudDrawList, TextAlign, TextRun},
};
use spaceracer::race::{Race, Weapon};
use std::f32::consts::PI;
pub fn font() -> FontFace {
    render::hud::register_font(include_bytes!("../../assets/fonts/OpenSans-SemiBold.ttf"))
}
struct Visor<'a> {
    h: &'a mut HudDrawList,
    r: [f32; 4],
    s: f32,
    id: u64,
}
impl Visor<'_> {
    fn point(&self, p: [f32; 2]) -> [f32; 2] {
        [self.r[0] + p[0] * self.s, self.r[1] + p[1] * self.s]
    }
    fn text(&mut self, t: &str, p: [f32; 2], size: f32, align: TextAlign, color: [u8; 4]) {
        let id = self.id;
        self.id += 1;
        self.h.outline(id, 0.65 * self.s, [2, 8, 16, 170]);
        self.h.text.push(TextRun {
            id,
            text: t.into(),
            position: [
                self.point(p)[0]
                    - match align {
                        TextAlign::Left => 0.,
                        TextAlign::Center => self.r[2] * 0.5,
                        TextAlign::Right => self.r[2],
                    },
                self.point(p)[1],
            ],
            bounds: self.r,
            font_size: size * self.s,
            color,
            font: font(),
            align,
            width: self.r[2],
        });
    }
    fn line(&mut self, a: [f32; 2], b: [f32; 2], width: f32, c: [f32; 4]) {
        self.h
            .line(self.point(a), self.point(b), width * self.s, c, self.r);
    }
    fn arc(&mut self, p: [f32; 2], radius: f32, a: f32, b: f32, width: f32, c: [f32; 4]) {
        self.h.arc(
            self.point(p),
            radius * self.s,
            a,
            b,
            width * self.s,
            c,
            self.r,
        );
    }
    fn circle(&mut self, p: [f32; 2], r: f32, c: [f32; 4]) {
        self.h.circle(self.point(p), r * self.s, c, self.r);
    }
    fn button(&mut self, key: &str, p: [f32; 2], color: [u8; 4]) {
        self.arc(p, 7., 0., PI * 2., 1., color.map(|v| v as f32 / 255.));
        self.text(key, [p[0], p[1] - 7.5], 10., TextAlign::Center, color);
    }
}
pub fn draw(
    h: &mut HudDrawList,
    race: &Race,
    i: usize,
    r: [f32; 4],
    cam: &Camera,
    up: Vec3,
    curvature: f32,
) {
    let scale = (r[2] / 800.).min(r[3] / 450.);
    let w = r[2] / scale;
    let height = r[3] / scale;
    let mut v = Visor {
        h,
        r,
        s: scale,
        id: 1000 + i as u64 * 100,
    };
    let p = &race.pilots[i];
    let d = &race.drivers[i];
    let white = [224, 246, 255, 232];
    let dim = [157, 206, 226, 200];
    let cyan = [0.25, 0.85, 1., 0.75];
    if race.over {
        if p.finished && d.rank == 1 {
            v.text(
                "YOU WIN",
                [w * 0.5, 8.],
                38.,
                TextAlign::Center,
                [255, 237, 168, 240],
            );
        }
        v.text("Race complete", [24., 57.], 22., TextAlign::Left, white);
        let mut order: Vec<_> = (0..race.pilots.len()).collect();
        order.sort_by_key(|&j| race.drivers[j].rank);
        for (row, j) in order.into_iter().enumerate() {
            let y = 96. + row as f32 * 28.;
            let other = &race.drivers[j];
            v.text(&other.rank.to_string(), [24., y], 13., TextAlign::Left, dim);
            v.text(&other.name, [52., y], 13., TextAlign::Left, white);
            v.text(
                &other
                    .finish_time
                    .map_or("DNF".into(), |t| format!("{t:.2}s")),
                [294., y],
                12.,
                TextAlign::Right,
                dim,
            );
            v.line([24., y + 24.], [294., y + 24.], 0.5, [0.3, 0.7, 0.8, 0.25]);
        }
        v.text("Start · race menu", [24., 302.], 10., TextAlign::Left, dim);
        return;
    }
    if p.finished {
        if d.rank == 1 {
            v.text(
                "YOU WIN",
                [w * 0.5, height * 0.34 - 58.],
                58.,
                TextAlign::Center,
                [255, 237, 168, 240],
            );
            for side in [-1., 1.] {
                let x = w * 0.5 + side * 161.;
                v.line(
                    [x - side * 12., height * 0.34 - 50.],
                    [x, height * 0.34 - 50.],
                    1.5,
                    [0.6, 0.91, 1., 0.5],
                );
                v.line(
                    [x, height * 0.34 - 50.],
                    [x, height * 0.34 + 9.],
                    1.5,
                    [0.6, 0.91, 1., 0.5],
                );
            }
        }
        v.line(
            [24., height - 65.],
            [224., height - 65.],
            1.,
            [0.25, 0.8, 0.9, 0.3],
        );
        v.text(
            if d.rank == 1 { "WINNER" } else { "FINISHED" },
            [24., height - 62.],
            20.,
            TextAlign::Left,
            white,
        );
        v.text(
            &format!("{} · {:.2}s", d.name, d.finish_time.unwrap_or(race.clock)),
            [24., height - 32.],
            12.,
            TextAlign::Left,
            dim,
        );
        v.text(
            &format!("{:05}", race.world.track.seed),
            [w - 24., height - 30.],
            10.,
            TextAlign::Right,
            dim,
        );
        return;
    }
    v.text(&d.name, [40., 12.], 12., TextAlign::Left, white);
    v.text(
        &format!("LAP {}/{}", p.lap.min(race.laps), race.laps),
        [40., 29.],
        10.,
        TextAlign::Left,
        dim,
    );
    v.text(
        &d.rank.to_string(),
        [w - 43., 10.],
        29.,
        TextAlign::Right,
        white,
    );
    v.text(
        &format!("/ {}", race.pilots.len()),
        [w - 36., 23.],
        12.,
        TextAlign::Left,
        dim,
    );
    let seconds = d.finish_time.unwrap_or(race.clock);
    v.text(
        &format!("{:02}:{:05.2}", (seconds / 60.) as u32, seconds % 60.),
        [w - 18., 49.],
        10.,
        TextAlign::Right,
        white,
    );
    v.text(
        &format!("{}", (p.speed.abs() * 3.6) as u32),
        [w - 54., height - 82.],
        31.,
        TextAlign::Right,
        white,
    );
    v.text("km/h", [w - 46., height - 61.], 10., TextAlign::Left, dim);
    let center = [w - 89., height - 96.];
    for k in 0..24 {
        let a = PI * (0.16 + 0.68 * k as f32 / 24.);
        let b = a + PI * 0.68 / 24. * 0.76;
        let lit = ((23 - k) as f64) < p.energy * 24. / 100.;
        let color = if lit {
            if p.energy < 25. {
                [1., 0.32, 0.18, 0.85]
            } else {
                cyan
            }
        } else {
            [0.15, 0.35, 0.43, 0.23]
        };
        if lit {
            v.arc(center, 76., a, b, 5., [color[0], color[1], color[2], 0.06]);
        }
        v.arc(center, 76., a, b, 2.2, color);
    }
    v.arc(
        center,
        80.,
        PI * 0.16,
        PI * 0.84,
        0.5,
        [0.35, 0.8, 1., 0.22],
    );
    let status = if p.boost > 0. {
        "BOOST"
    } else if p.airborne {
        "FLIGHT"
    } else if p.unload > 0.2 {
        "LIFTING"
    } else if p.slide > 0.2 {
        "SLIDING"
    } else if p.slipstream > 0.2 {
        "SLIPSTREAM"
    } else if p.lap == 1 {
        "BOOST ON LAP 2"
    } else {
        ""
    };
    v.text(status, [w - 16., height - 19.], 8., TextAlign::Right, dim);
    // One complete track silhouette, with player positions; no per-frame mesh allocation.
    let nodes = &race.world.track.nodes;
    let mut lo = Vec2::splat(f32::INFINITY);
    let mut hi = Vec2::splat(f32::NEG_INFINITY);
    for n in nodes {
        let q = Vec2::new(n.geometry.p.x, n.geometry.p.z);
        lo = lo.min(q);
        hi = hi.max(q);
    }
    let mid = (lo + hi) * 0.5;
    let factor = 64. / (hi - lo).max_element().max(1.);
    let map = |point: Vec3| {
        let q = (Vec2::new(point.x, point.z) - mid) * factor;
        [52. + q.x, height - 65. + q.y]
    };
    let stride = (nodes.len() / 160).max(1);
    for k in (0..nodes.len()).step_by(stride) {
        v.line(
            map(nodes[k].geometry.p),
            map(nodes[(k + stride) % nodes.len()].geometry.p),
            1.,
            [0.25, 0.7, 1., 0.7],
        );
    }
    for j in 0..race.pilots.len() {
        v.circle(
            map(race.pose(j).origin),
            if j == i { 2.5 } else { 1.5 },
            if j == i {
                [0.8, 1., 0.9, 1.]
            } else {
                [0.8, 0.5, 0.35, 0.65]
            },
        );
    }
    v.text(
        &format!(
            "{:05} · {} · {}",
            race.world.track.seed,
            race.world.track.difficulty.to_uppercase(),
            race.biome.to_uppercase()
        ),
        [18., height - 21.],
        8.,
        TextAlign::Left,
        dim,
    );
    if let Some(item) = d.item {
        if d.active.is_none() {v.button("X", [w * 0.5 - 88., height * 0.82], white);}
        v.text(
            item.name(),
            [w * 0.5 - 74., height * 0.82 - 8.],
            12.,
            TextAlign::Left,
            white,
        );
        if d.active.is_some() {
            v.text(
                "STORED",
                [w * 0.5 - 74., height * 0.82 + 9.],
                7.,
                TextAlign::Left,
                dim,
            );
        }
    }
    if let Some(item) = d.active {
        v.text(
            &format!("{}  {:0.1}s", item.name(), d.active_time),
            [w * 0.5, height * 0.82 - 30.],
            9.,
            TextAlign::Center,
            dim,
        );
    }
    if d.checkpoint.missed {
        v.text(
            "CHECKPOINT MISSED",
            [w * 0.5, height * 0.28],
            23.,
            TextAlign::Center,
            [255, 150, 100, 245],
        );
        v.button("Y", [w * 0.5 - 25., height * 0.28 + 43.], white);
        v.text(
            "RESET",
            [w * 0.5 - 12., height * 0.28 + 35.],
            10.,
            TextAlign::Left,
            white,
        );
    } else if p.airborne {
        v.button("Y", [w * 0.5 - 26., height * 0.9], white);
        v.text(
            "RESET",
            [w * 0.5 - 12., height * 0.9 - 8.],
            10.,
            TextAlign::Left,
            dim,
        );
    }
    if race.countdown > 0. {
        v.text(
            &format!("{}", race.countdown.ceil() as u32),
            [w * 0.5, height * 0.35],
            64.,
            TextAlign::Center,
            white,
        );
    }
    if let Some((owner, target, at)) = race.destroyed {
        if owner == i && race.clock - at < 0.5 {
            v.text(
                &format!("DESTROYED {}", race.drivers[target].name),
                [w * 0.5, height * 0.4],
                18.,
                TextAlign::Center,
                white,
            );
        }
    }
    let target = match d.item {
        Some(Weapon::Railgun) => race.target(i, 0.975, 700.),
        Some(Weapon::GlideBomb) if p.airborne => race.target(i, 0.7, 1000.),
        _ => None,
    };
    let mut markers = Vec::new();
    if let Some(j) = target {
        markers.push((j, false));
    }
    for projectile in &race.projectiles {
        if projectile.kind == Weapon::Missile && !projectile.evaded {
            if let Some(j) = projectile.target {
                if !markers.contains(&(j, true)) {
                    markers.push((j, true));
                }
            }
        }
    }
    for (j, incoming) in markers {
        let vp = Mat4::perspective_rh(cam.fov_y, r[2] / r[3], 0.15, 10000.)
            * Mat4::look_at_rh(cam.pos, cam.target, up);
        let q = vp * race.pose(j).origin.extend(1.);
        if q.w > 0. && (!incoming || (race.clock * 5.).fract() < 0.7) {
            let q = q.truncate() / q.w;
            let point = Vec2::new(q.x * 0.5, -q.y * 0.5);
            let marker = Vec2::splat(0.5) + point * (1. + curvature * point.length_squared() * 4.);
            let center = [marker.x * w, marker.y * height];
            for sx in [-1., 1.] {
                for sy in [-1., 1.] {
                    let a = [center[0] + sx * 23., center[1] + sy * 17.];
                    let c = if incoming {
                        [1., 0.12, 0.08, 0.9]
                    } else {
                        [1., 0.55, 0.2, 0.85]
                    };
                    v.line(a, [a[0] - sx * 8., a[1]], 1., c);
                    v.line(a, [a[0], a[1] - sy * 6.], 1., c);
                }
            }
        }
    }
}
