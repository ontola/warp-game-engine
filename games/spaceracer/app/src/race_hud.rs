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
    next_ready: Option<bool>,
    i: usize,
    r: [f32; 4],
    cam: &Camera,
    up: Vec3,
    curvature: f32,
    speed_kmh_per_unit: f32,
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
        match next_ready {
            Some(true) => {
                v.button("A", [31., 290.], white);
                v.text("Next race", [44., 282.], 13., TextAlign::Left, white);
            }
            Some(false) => v.text("Preparing the next race…", [24., 282.], 12., TextAlign::Left, dim),
            None => {}
        }
        v.text("Start · race menu", [24., 306.], 10., TextAlign::Left, dim);
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
        &format!("{}", (p.speed.abs() * speed_kmh_per_unit as f64) as u32),
        [w - 54., height - 82.],
        31.,
        TextAlign::Right,
        white,
    );
    v.text("km/h", [w - 46., height - 61.], 10., TextAlign::Left, dim);
    let center = [w - 89., height - 96.];
    // On a recharge strip a bright wave climbs the lit segments and the next
    // segment blinks, so filling up reads at a glance.
    let charging = p.charging && !p.airborne && !p.crashed;
    let wave = race.clock as f32 * 3.;
    for k in 0..24 {
        let a = PI * (0.16 + 0.68 * k as f32 / 24.);
        let b = a + PI * 0.68 / 24. * 0.76;
        let lit = ((23 - k) as f64) < p.energy * 24. / 100.;
        let filling = charging && !lit && ((23 - k) as f64) < p.energy * 24. / 100. + 1.;
        let color = if lit {
            if charging {
                let pulse = (((23 - k) as f32 / 24. - wave).rem_euclid(1.) * PI * 2.).cos() * 0.5 + 0.5;
                [0.35 + 0.45 * pulse, 1., 0.75 + 0.25 * pulse, 0.85 + 0.15 * pulse]
            } else if p.energy < 25. {
                [1., 0.32, 0.18, 0.85]
            } else {
                cyan
            }
        } else if filling {
            [0.45, 1., 0.8, 0.35 + 0.5 * (wave * PI * 2.).sin().abs()]
        } else {
            [0.15, 0.35, 0.43, 0.23]
        };
        if lit {
            v.arc(center, 76., a, b, if charging { 8. } else { 5. }, [color[0], color[1], color[2], if charging { 0.14 } else { 0.06 }]);
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
        if d.active.is_none() {
            v.button("X", [w * 0.5 - 88., height * 0.82], white);
        }
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
    // Tell the shooter what their weapon did: a hit, or a kill (which wins).
    let destroyed = race.destroyed.filter(|&(owner, _, at)| owner == i && race.clock - at < 1.6);
    let hit = race.hit.filter(|&(owner, _, at)| owner == i && race.clock - at < 1.1);
    if let Some((_, target, at)) = destroyed {
        let pop = 1. + 0.35 * (1. - ((race.clock - at) as f32 / 0.18).min(1.));
        v.text(
            &format!("DESTROYED {}", race.drivers[target].name.to_uppercase()),
            [w * 0.5, height * 0.4],
            24. * pop,
            TextAlign::Center,
            [255, 150, 90, 245],
        );
    } else if let Some((_, target, at)) = hit {
        let pop = 1. + 0.25 * (1. - ((race.clock - at) as f32 / 0.15).min(1.));
        v.text(
            &format!("HIT {}", race.drivers[target].name.to_uppercase()),
            [w * 0.5, height * 0.4],
            18. * pop,
            TextAlign::Center,
            [255, 225, 150, 235],
        );
    }
    let preview = if d.item == Some(Weapon::GlideBomb)
        && d.active.is_none()
        && p.emp_time <= 0.
    {
        Some(race.bomb_preview(i))
    } else {
        None
    };
    if let Some((lock, points)) = &preview {
        let vp = Mat4::perspective_rh(cam.fov_y, r[2] / r[3], 0.15, 10000.)
            * Mat4::look_at_rh(cam.pos, cam.target, up);
        let project = |point: Vec3| -> Option<[f32; 2]> {
            let q = vp * point.extend(1.);
            if q.w <= 0. {
                return None;
            }
            let q = q.truncate() / q.w;
            let a = Vec2::new(q.x * 0.5, -q.y * 0.5);
            let a = Vec2::splat(0.5) + a * (1. + curvature * a.length_squared() * 4.);
            Some([a.x * w, a.y * height])
        };
        let color = if lock.is_some() {
            [1., 0.65, 0.2, 0.7]
        } else {
            [0.4, 0.85, 1., 0.35]
        };
        for (k, pair) in points.windows(2).enumerate() {
            if k % 2 == 0 {
                if let (Some(a), Some(b)) = (project(pair[0]), project(pair[1])) {
                    v.line(a, b, 0.8, color);
                }
            }
        }
        if let Some(point) = points.last().and_then(|p| project(*p)) {
            v.arc(point, 12., 0., PI * 2., 1., color);
            v.circle(point, 2., color);
        }
    }
    let rail_target=d.rail_preview.and_then(|shot|shot.target);
    if race.rail_ready(i) {
        let pose=race.pose(i);
        let ahead=pose.origin+pose.basis*(spaceracer::railgun::MUZZLE+Vec3::Z*200.);
        let vp=Mat4::perspective_rh(cam.fov_y,r[2]/r[3],0.15,10000.)*Mat4::look_at_rh(cam.pos,cam.target,up);
        let q=vp*ahead.extend(1.);
        if q.w>0. {
            let center=Vec2::new(q.x/q.w*0.5+0.5,0.5-q.y/q.w*0.5)*Vec2::new(w,height);
            let radius=height*race.combat.rail_guide_radius;
            let color=if rail_target.is_some(){[0.55,1.,0.73,0.85]}else{[0.55,0.94,1.,0.45]};
            let point=|angle:f32| {
                let pixel=center+Vec2::new(angle.cos(),angle.sin())*radius;
                let a=pixel/Vec2::new(w,height)-Vec2::splat(0.5);
                ((Vec2::splat(0.5)+a*(1.+curvature*a.length_squared()*4.))*Vec2::new(w,height)).to_array()
            };
            for k in 0..64 {v.line(point(k as f32*PI/32.),point((k+1) as f32*PI/32.),1.3,color);}
        }
    }
    let target = match d.item {
        Some(Weapon::Railgun) if race.rail_ready(i) => rail_target,
        Some(Weapon::GlideBomb) => preview.as_ref().and_then(|v| v.0),
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
                    } else if d.item==Some(Weapon::Railgun) {
                        [0.55,1.,0.73,0.85]
                    } else {
                        [1., 0.55, 0.2, 0.85]
                    };
                    v.line(a, [a[0] - sx * 8., a[1]], 1., c);
                    v.line(a, [a[0], a[1] - sy * 6.], 1., c);
                }
            }
        }
    }
    // Railgun: a lock arrow above whoever is inside the circle, so the player
    // sees who a tap will hit. Dim grey when scenery blocks the shot.
    if race.rail_ready(i) {
        let lock = rail_target.map(|j| (j, true))
            .or(d.rail_candidate.map(|j| (j, false)));
        if let Some((j, clear)) = lock {
            let vp = Mat4::perspective_rh(cam.fov_y, r[2] / r[3], 0.15, 10000.)
                * Mat4::look_at_rh(cam.pos, cam.target, up);
            let pose = race.pose(j);
            let q = vp * (pose.origin + pose.basis.y_axis * 3.5).extend(1.);
            if q.w > 0. {
                let q = q.truncate() / q.w;
                let point = Vec2::new(q.x * 0.5, -q.y * 0.5);
                let at = Vec2::splat(0.5) + point * (1. + curvature * point.length_squared() * 4.);
                let tip = [at.x * w, at.y * height - 14.];
                let c = if clear { [0.55, 1., 0.73, 0.95] } else { [0.6, 0.66, 0.72, 0.55] };
                let bob = if clear { (race.clock as f32 * 6.).sin() * 2. } else { 0. };
                let tip = [tip[0], tip[1] + bob];
                let l = [tip[0] - 11., tip[1] - 13.];
                let rr = [tip[0] + 11., tip[1] - 13.];
                v.line(l, tip, 2.2, c);
                v.line(rr, tip, 2.2, c);
                v.line(l, rr, 1.2, c);
                if clear {
                    v.text("LOCK", [tip[0], tip[1] - 30.], 10., TextAlign::Center, [140, 255, 186, 235]);
                }
            }
        }
    }
}
