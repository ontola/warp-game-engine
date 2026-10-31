//! Original warp, rotary sentry and powered glide behaviour. Policy values come
//! from Luau; bounded vector integration and swept contact stay native.
use crate::{
    handling::{DT, Input, toward},
    race::{Beam, Blast, Projectile, Race, Weapon},
};
use glam::{Mat3, Quat, Vec3};
#[derive(Clone, serde::Deserialize)]
pub struct Config {
    pub bomb_speed: f32,
    pub bomb_accel: f32,
    pub bomb_burn: f32,
    pub bomb_turn: f32,
    pub bomb_gravity: f32,
    pub bomb_drag: f32,
    pub bomb_eject: f32,
    pub bomb_range: f32,
    pub bomb_cone: f32,
    pub bomb_fuse: f32,
    pub sentry_ahead: f64,
    pub sentry_range: f32,
    pub sentry_cone: f32,
    pub sentry_spinup: f64,
}
impl Config {
    pub fn load(rules: &mut crate::rules::Rules) -> Result<Self, String> {
        let c: Self = rules.call("combat", ())?;
        let values = [
            c.bomb_speed,
            c.bomb_accel,
            c.bomb_burn,
            c.bomb_turn,
            c.bomb_gravity,
            c.bomb_drag,
            c.bomb_eject,
            c.bomb_range,
            c.bomb_fuse,
            c.sentry_range,
            c.sentry_ahead as f32,
            c.sentry_spinup as f32,
        ];
        if values
            .iter()
            .any(|x| !x.is_finite() || *x < 0. || *x > 2000.)
            || c.bomb_speed < 80.
            || c.bomb_turn > 4.
            || !(0.0..=1.0).contains(&c.bomb_cone)
            || !(0.0..=1.0).contains(&c.sentry_cone)
        {
            return Err("invalid combat policy".into());
        }
        Ok(c)
    }
    pub fn bomb_velocity(
        &self,
        v: Vec3,
        position: Vec3,
        destination: Option<Vec3>,
        dt: f32,
        age: f32,
    ) -> Vec3 {
        let mut next = (v - Vec3::Y * self.bomb_gravity * dt) * (-self.bomb_drag * dt).exp();
        let burn = (self.bomb_burn - age).clamp(0., dt);
        if next.length_squared() > 1. && burn > 0. {
            next = next.normalize()
                * toward(
                    next.length() as f64,
                    self.bomb_speed as f64,
                    (self.bomb_accel * burn) as f64,
                ) as f32;
        }
        if let Some(goal) = destination {
            if let (Some(dir), Some(want)) =
                (next.try_normalize(), (goal - position).try_normalize())
            {
                let angle = dir.dot(want).clamp(-1., 1.).acos();
                if angle > 0.0001 {
                    next = Quat::IDENTITY.slerp(
                        Quat::from_rotation_arc(dir, want),
                        (self.bomb_turn * dt / angle).min(1.),
                    ) * next;
                }
            }
        }
        next.clamp_length_max(self.bomb_speed)
    }
}
impl Race {
    pub fn bomb_launch(&self, i: usize) -> (Vec3, Vec3) {
        let pose = self.pose(i);
        let v = self.drivers[i].motion.air.velocity;
        let dir = v
            .try_normalize()
            .unwrap_or(pose.basis.z_axis)
            .lerp(pose.basis.z_axis, 0.35)
            .normalize_or_zero();
        (
            pose.origin + pose.basis * Vec3::new(0., 1.15, -1.8),
            (dir * (v.length() + self.combat.bomb_eject).min(self.combat.bomb_speed)
                + pose.basis.y_axis * 8.)
                .clamp_length_max(self.combat.bomb_speed),
        )
    }
    fn predicted(&self, j: usize, t: f32) -> Vec3 {
        let p = &self.pilots[j];
        if p.airborne {
            self.drivers[j].motion.air.position + self.drivers[j].motion.air.velocity * t
        } else {
            self.world
                .track
                .sample(p.distance + p.speed * p.heading.cos() * t as f64)
                .point(p.x, 2.)
        }
    }
    pub fn bomb_lead(&self, j: usize, position: Vec3, speed: f32) -> Vec3 {
        let p = &self.pilots[j];
        let offset = self.pose(j).origin - position;
        let v = if p.airborne {
            self.drivers[j].motion.air.velocity
        } else {
            self.drivers[j].motion.ground_velocity
        };
        let a = speed * speed - v.length_squared();
        let b = offset.dot(v);
        let disc = b * b + a * offset.length_squared();
        let mut t = if a > 0.01 && disc >= 0. {
            (b + disc.sqrt()) / a
        } else {
            offset.length() / speed.max(80.)
        }
        .clamp(0., 4.);
        if !p.airborne {
            for _ in 0..3 {
                t = t * 0.35 + position.distance(self.predicted(j, t)) / speed.max(80.) * 0.65;
            }
        }
        self.predicted(j, t.min(4.))
    }
    pub fn bomb_target(&self, i: usize) -> Option<(usize, Vec3)> {
        let (at, velocity) = self.bomb_launch(i);
        let forward = velocity.normalize_or_zero();
        let mut best = None;
        let mut score = f32::INFINITY;
        for j in 0..self.pilots.len() {
            let p = &self.pilots[j];
            if i == j || p.crashed || p.finished || p.warp_time > 0. {
                continue;
            }
            let current = self.pose(j).origin;
            let offset = current - at;
            let dist = offset.length();
            let dot = forward.dot(offset.normalize_or_zero());
            if dist < 20. || dist > self.combat.bomb_range || dot < self.combat.bomb_cone {
                continue;
            }
            let predicted = self.bomb_lead(j, at, self.combat.bomb_speed);
            if forward.dot((predicted - at).normalize_or_zero()) < self.combat.bomb_cone {
                continue;
            }
            let value = 1. - dot + 0.12 * dist / self.combat.bomb_range;
            if value >= score
                || self
                    .obstacles
                    .iter()
                    .any(|o| o.trace(at, current).is_some())
            {
                continue;
            }
            score = value;
            best = Some((j, predicted));
        }
        best
    }
    /// Cheap current-tick guide: no full-track searches or speculative scene sweeps.
    pub fn bomb_preview(&self, i: usize) -> (Option<usize>, Vec<Vec3>) {
        let (mut at, mut velocity) = self.bomb_launch(i);
        let lock = self.bomb_target(i);
        let mut points = vec![at];
        let step = lock.map_or(0.24, |(_, goal)| {
            (at.distance(goal) / self.combat.bomb_speed / 8.).min(0.24)
        });
        for n in 0..if lock.is_some() { 8 } else { 25 } {
            let next =
                self.combat
                    .bomb_velocity(velocity, at, lock.map(|x| x.1), step, n as f32 * step);
            at += (velocity + next) * 0.5 * step;
            velocity = next;
            points.push(at);
        }
        if let Some((_, goal)) = lock {
            points.push(goal);
        }
        (lock.map(|x| x.0), points)
    }
    pub(crate) fn step_bomb(&self, p: &mut Projectile) -> Option<Vec3> {
        let mut goal = None;
        if let Some(j) = p.target {
            let target = &self.pilots[j];
            let offset = self.pose(j).origin - p.position;
            if target.crashed
                || target.finished
                || target.warp_time > 0.
                || p.velocity
                    .normalize_or_zero()
                    .dot(offset.normalize_or_zero())
                    < 0.087156
            {
                p.target = None;
            } else {
                goal = Some(self.bomb_lead(j, p.position, p.velocity.length()));
            }
        }
        let next = self.combat.bomb_velocity(
            p.velocity,
            p.position,
            goal,
            DT as f32,
            (p.time - DT) as f32,
        );
        p.position += (p.velocity + next) * (0.5 * DT as f32);
        p.velocity = next;
        let mut hit = self
            .environment
            .trace(&self.obstacles, p.previous, p.position)
            .map(|h| h.position);
        let mut best = hit.map_or(f32::INFINITY, |h| h.distance_squared(p.previous));
        let (distance, n, x) =
            self.world
                .track
                .project((p.previous + p.position) * 0.5, p.distance, 350.);
        p.distance = distance;
        if n.supported(x, 0.) {
            let surface = n.point(x, 0.);
            let normal = n.frame(x).y_axis;
            let a = (p.previous - surface).dot(normal);
            let b = (p.position - surface).dot(normal);
            if (a > 0.85 && b <= 0.85) || (a < -0.85 && b >= -0.85) {
                let at = p.previous.lerp(
                    p.position,
                    ((a - a.signum() * 0.85) / (a - b)).clamp(0., 1.),
                );
                let d = at.distance_squared(p.previous);
                if d < best {
                    hit = Some(at);
                    best = d;
                }
            }
        }
        let owner = self.pose(p.owner).origin;
        if p.time >= 0.65 && p.previous.distance(owner) > 20. {
            p.owner_clear = true;
        }
        if p.previous.distance(owner) > self.balance.bomb_radius + 8. {
            p.owner_blast_clear = true;
        }
        for j in 0..self.pilots.len() {
            if (j == p.owner && !p.owner_clear)
                || self.pilots[j].crashed
                || self.pilots[j].finished
                || self.pilots[j].warp_time > 0.
            {
                continue;
            }
            let at = self.pose(j).origin;
            let delta = p.position - p.previous;
            let t =
                ((at - p.previous).dot(delta) / delta.length_squared().max(0.0001)).clamp(0., 1.);
            let point = p.previous + delta * t;
            let radius = if p.target == Some(j) && p.time > 0.22 {
                self.combat.bomb_fuse
            } else {
                4.
            };
            if at.distance(point) < radius && point.distance_squared(p.previous) < best {
                hit = Some(point);
                best = point.distance_squared(p.previous);
            }
        }
        hit
    }
    pub(crate) fn bomb_explode(&mut self, p: &Projectile, at: Vec3) {
        self.blasts.push(Blast {
            position: at,
            radius: self.balance.bomb_radius,
            age: 0.,
            kind: Weapon::GlideBomb,
        });
        for j in 0..self.pilots.len() {
            if j == p.owner && !p.owner_blast_clear {
                continue;
            }
            let delta = self.pose(j).origin - at;
            let strength = (1. - delta.length() / self.balance.bomb_radius)
                .max(0.)
                .powf(0.65) as f64;
            if strength <= 0. {
                continue;
            }
            let side = delta.dot(self.pose(j).basis.x_axis).signum() as f64;
            self.damage(
                p.owner,
                j,
                self.balance.bomb_damage * strength,
                side * 105. * strength,
            );
            if self.pilots[j].warp_time > 0. {
                continue;
            }
            self.pilots[j].speed *= 1. - 0.55 * strength;
            if self.pilots[j].airborne {
                self.drivers[j].motion.air.velocity +=
                    (delta.normalize_or_zero() + Vec3::Y * 0.25) * (110. * strength as f32);
            }
        }
    }
    pub fn sentry_basis(&self, i: usize) -> Mat3 {
        let pose = self.pose(i);
        let pivot = pose.origin + pose.basis * Vec3::new(0., 1.95, -1.8);
        if let Some(j) = self.drivers[i].sentry_target {
            let z = (self.pose(j).origin - pivot).normalize_or_zero();
            let x = pose.basis.y_axis.cross(z).normalize_or_zero();
            if x.length_squared() > 0.5 {
                return Mat3::from_cols(x, z.cross(x), z);
            }
        }
        let d = &self.drivers[i];
        let active = d.active == Some(Weapon::Sentry) && self.pilots[i].emp_time <= 0.;
        pose.basis
            * Mat3::from_quat(if active {
                Quat::from_rotation_y((self.clock as f32 * 1.8 + i as f32 * 0.7).sin() * 0.72)
                    * Quat::from_rotation_x((self.clock as f32 * 1.1).sin() * 0.06)
            } else {
                Quat::from_rotation_x(0.14)
            })
    }
    pub(crate) fn step_sentry(&mut self, i: usize) {
        self.drivers[i].active_time = (self.drivers[i].active_time - DT).max(0.);
        self.drivers[i].active_age += DT;
        let pose = self.pose(i);
        let mut target = None;
        let mut ahead = self.combat.sentry_ahead;
        if self.pilots[i].emp_time <= 0. {
            for j in 0..self.pilots.len() {
                let p = &self.pilots[j];
                if i == j || p.crashed || p.finished || p.warp_time > 0. {
                    continue;
                }
                let progress = p.distance - self.pilots[i].distance;
                let delta = self.pose(j).origin - pose.origin;
                if progress > 0.
                    && progress < ahead
                    && delta.length() < self.combat.sentry_range
                    && delta.normalize_or_zero().dot(pose.basis.z_axis) > self.combat.sentry_cone
                {
                    ahead = progress;
                    target = Some(j);
                }
            }
        }
        self.drivers[i].sentry_target = target;
        if self.clock + 1e-8 < self.drivers[i].shot_time {
            return;
        }
        self.drivers[i].shot_time = self.clock + self.balance.sentry_interval;
        if let Some(j) = target {
            let from = pose.origin
                + pose.basis * Vec3::new(0., 1.95, -1.8)
                + self.sentry_basis(i).z_axis * 2.6;
            let to = self.pose(j).origin;
            if self.obstacles.iter().any(|o| o.trace(from, to).is_some()) {
                return;
            }
            self.drivers[i].shot_count += 1;
            self.beams.push(Beam {
                owner: i,
                serial: self.drivers[i].shot_count,
                from,
                to,
                age: 0.,
                rail: false,
            });
            self.damage(i, j, self.balance.sentry_damage, 0.);
            self.pilots[j].speed *= 0.995_f64.powf(self.balance.sentry_interval / 0.4);
        }
    }
    pub(crate) fn step_warp(&mut self, i: usize) -> Result<(), String> {
        #[derive(serde::Deserialize)]
        struct Policy {
            speed: f64,
            acceleration: f64,
            lateral: f64,
            strength: f32,
        }
        let before = self.pose(i);
        let d = &mut self.drivers[i];
        d.active_age += DT;
        d.active_time = (d.active_time - DT).max(0.);
        let upcoming = self.world.track.sample(self.pilots[i].distance + 65.);
        let c:Policy=self.rules.call("warp",serde_json::json!({"age":d.active_age,"remaining":d.active_time,"curve":upcoming.curve}))?;
        if ![c.speed, c.acceleration, c.lateral, c.strength as f64]
            .iter()
            .all(|v| v.is_finite() && *v >= 0. && *v <= 2000.)
        {
            return Err("invalid warp policy".into());
        }
        d.warp_fx = c.strength.clamp(0., 1.);
        let p = &mut self.pilots[i];
        let old = p.speed;
        p.speed = toward(p.speed, c.speed, DT * c.acceleration);
        p.distance += p.speed * DT;
        let n = self.world.track.sample(p.distance);
        if n.air_gap {
            d.active_time = d.active_time.max(0.65);
        }
        let lane = if n.split_gap > 0.01 {
            let side = if n.preferred_route != 0. {
                n.preferred_route
            } else {
                if i % 2 == 0 { 1. } else { -1. }
            };
            side * (n.split_gap + (n.geometry.width - n.split_gap) * 0.5)
        } else {
            0.
        };
        p.x = toward(p.x, lane, DT * c.lateral);
        p.heading = 0.;
        p.slip = 0.;
        p.trim = 0.;
        p.unload = 0.;
        p.lift = 0.;
        p.slide = 0.;
        p.drifting = false;
        p.airborne = false;
        p.on_pad = false;
        p.launch_cooldown = 0.5;
        p.acceleration = (p.speed - old) / DT;
        p.warp_time = d.active_time;
        p.constrain(n.surface());
        d.motion.landing_blend = 0.;
        d.input.drive = Input {
            throttle: 1.,
            ..Default::default()
        };
        let pose = d.motion.pose(p, &self.world.track, self.clock);
        let velocity = (pose.origin - before.origin) / DT as f32;
        d.motion.ground_acceleration = (velocity - d.motion.ground_velocity) / DT as f32;
        d.motion.ground_velocity = velocity;
        if d.active_time <= 0. {
            d.active = None;
        }
        Ok(())
    }
    pub(crate) fn interrupt_warp(&mut self, i: usize) {
        if self.drivers[i].active != Some(Weapon::Warp) {
            return;
        }
        let pose = self.pose(i);
        self.drivers[i].active = None;
        self.drivers[i].active_time = 0.;
        self.pilots[i].warp_time = 0.;
        if self.world.track.sample(self.pilots[i].distance).air_gap {
            self.drivers[i].motion.launch(&mut self.pilots[i], pose);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::race::Controls;
    fn race() -> Race {
        let mut r = Race::new(35, "normal", "city", 1, 3);
        r.countdown = 0.;
        r.pickups.clear();
        for (i, d) in r.drivers.iter_mut().enumerate() {
            d.bot = false;
            r.pilots[i].distance = -500. - i as f64 * 30.;
        }
        r.pilots[0].distance = 0.;
        r.pilots[0].x = 0.;
        r
    }
    fn fire(r: &mut Race, w: Weapon) {
        r.drivers[0].item = Some(w);
        r.weapons(&[Controls {
            fire: true,
            ..Default::default()
        }]);
    }
    #[test]
    fn warp_phases_crosses_gaps_and_exits_at_safe_speed() {
        let mut r = race();
        fire(&mut r, Weapon::Warp);
        r.pilots[0].speed = 300.;
        r.damage(1, 0, 50., 60.);
        assert_eq!(r.pilots[0].energy, 100.);
        for n in &mut r.world.track.nodes {
            n.air_gap = true;
            n.curve = 0.03;
        }
        r.drivers[0].active_time = 0.01;
        for _ in 0..150 {
            r.step_warp(0).unwrap();
            assert!(!r.pilots[0].airborne);
            assert!(r.drivers[0].active_time >= 0.65);
        }
        for n in &mut r.world.track.nodes {
            n.air_gap = false;
        }
        for _ in 0..90 {
            if r.drivers[0].active.is_some() {
                r.step_warp(0).unwrap();
            }
        }
        assert_eq!(r.drivers[0].active, None);
        assert!(r.pilots[0].speed < 200.);
        assert!(r.pilots[0].distance > 500.);
    }
    #[test]
    fn warp_rejects_air_launch_and_emp_restores_real_flight_in_gap() {
        let mut r = race();
        r.pilots[0].airborne = true;
        fire(&mut r, Weapon::Warp);
        assert_eq!(r.drivers[0].item, Some(Weapon::Warp));
        assert!(r.drivers[0].active.is_none());
        r.pilots[0].airborne = false;
        fire(&mut r, Weapon::Warp);
        for n in &mut r.world.track.nodes {
            n.air_gap = true;
        }
        r.drivers[0].motion.ground_velocity = Vec3::Z * 250.;
        r.interrupt_warp(0);
        assert!(r.pilots[0].airborne);
        assert_eq!(r.drivers[0].motion.air.velocity, Vec3::Z * 250.);
        assert_eq!(r.pilots[0].warp_time, 0.);
    }
    #[test]
    fn minigun_preserves_dps_and_cannot_fire_through_scenery() {
        let mut r = race();
        r.pilots[1].distance = 90.;
        r.pilots[1].x = 0.;
        fire(&mut r, Weapon::Sentry);
        r.clock = 1.;
        r.drivers[0].shot_time = 1.;
        for _ in 0..120 {
            r.weapons(&[]);
            r.clock += DT;
        }
        assert!(
            (r.pilots[1].energy - 90.).abs() < 0.6,
            "{}",
            r.pilots[1].energy
        );
        assert!((19..=21).contains(&r.drivers[0].shot_count));
        let a = r.pose(0).origin;
        let b = r.pose(1).origin;
        let center = (a + b) * 0.5;
        r.obstacles.push(crate::environment::Obstacle {
            min: center - Vec3::splat(15.),
            max: center + Vec3::splat(15.),
        });
        let count = r.drivers[0].shot_count;
        for _ in 0..60 {
            r.weapons(&[]);
            r.clock += DT;
        }
        assert_eq!(count, r.drivers[0].shot_count);
    }
    #[test]
    fn glide_motor_turn_limit_and_gravity_are_not_velocity_lerp() {
        let r = race();
        let c = &r.combat;
        let mut v = Vec3::Z * 180.;
        let mut p = Vec3::ZERO;
        for k in 0..120 {
            let next = c.bomb_velocity(v, p, None, DT as f32, k as f32 * DT as f32);
            p += (v + next) * 0.5 * DT as f32;
            v = next;
        }
        assert!(v.length() > 330. && v.y < 0.);
        let before = v.normalize();
        let after = c.bomb_velocity(v, p, Some(p + Vec3::X * 100.), DT as f32, 2.);
        assert!(before.angle_between(after.normalize()) < 0.02);
    }
    #[test]
    fn glide_preview_matches_launch_and_intercepts_moving_rival() {
        let mut r = race();
        r.pilots[1].distance = 230.;
        r.pilots[1].x = 0.;
        r.pilots[1].speed = 130.;
        r.pilots[0].airborne = true;
        let n = r.world.track.sample(0.);
        r.drivers[0].motion.air.position = n.point(0., 65.);
        r.drivers[0].motion.air.frame = n.geometry.basis();
        r.drivers[0].motion.air.velocity = n.geometry.basis().z_axis * 220.;
        let preview = r.bomb_preview(0);
        assert_eq!(preview.0, Some(1));
        fire(&mut r, Weapon::GlideBomb);
        assert_eq!(r.projectiles[0].target, preview.0);
        let mut p = r.projectiles.remove(0);
        let mut hit = None;
        for _ in 0..720 {
            r.pilots[1].distance += r.pilots[1].speed * DT;
            p.previous = p.position;
            p.time += DT;
            hit = r.step_bomb(&mut p);
            if hit.is_some() {
                break;
            }
        }
        let at = hit.expect("powered lead pursuit should reach the moving road target");
        assert!(at.distance(r.pose(1).origin) < r.balance.bomb_radius);
        r.bomb_explode(&p, at);
        assert!(r.pilots[1].energy < 50.);
    }
    #[test]
    fn glide_launch_blast_does_not_hit_owner_before_clearance() {
        let mut r = race();
        r.pilots[0].airborne = true;
        fire(&mut r, Weapon::GlideBomb);
        let mut p = r.projectiles.remove(0);
        let at = r.pose(0).origin;
        r.bomb_explode(&p, at);
        assert_eq!(r.pilots[0].energy, 100.);
        p.owner_blast_clear = true;
        r.bomb_explode(&p, at);
        assert!(r.pilots[0].crashed);
    }
}
