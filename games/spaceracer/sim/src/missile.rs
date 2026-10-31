//! Cruise missile route integration. Luau owns timing, speed, lock and terminal policy.
use crate::{
    handling::{DT, smooth, toward},
    race::{Projectile, Race},
};
use serde::Deserialize;
use serde_json::json;
#[derive(Deserialize)]
struct Guidance {
    speed: f32,
    launch_blend: f32,
    lock: bool,
    terminal: bool,
    fuse: f64,
    laser_seconds: f32,
}
impl Race {
    pub(crate) fn guide_missile(&mut self, p: &mut Projectile) -> Result<bool, String> {
        if !p.locked && !p.terminal {
            p.target = (0..self.pilots.len())
                .filter(|&j| j != p.owner && !self.pilots[j].finished && !self.pilots[j].crashed && self.pilots[j].warp_time<=0.)
                .min_by_key(|&j| self.drivers[j].rank);
        }
        if p.target
            .is_some_and(|j| self.pilots[j].finished || self.pilots[j].crashed || self.pilots[j].warp_time>0.)
        {
            p.target = None;
            p.evaded = true;
        }
        let range = p
            .target
            .map_or(1e6, |j| self.pose(j).origin.distance(p.position));
        let along = p.target.map_or(1e6, |j| {
            let length = self.world.track.length;
            (self.pilots[j].distance - p.distance + length * 0.5).rem_euclid(length) - length * 0.5
        });
        let g:Guidance=self.rules.call("missile",json!({"age":p.time,"launch_speed":p.launch_speed,"target":p.target.is_some(),"range":range,"along":along}))?;
        if !g.speed.is_finite()
            || !(1.0..=1000.).contains(&g.speed)
            || !(0.0..=1.0).contains(&g.launch_blend)
            || !(0.1..=20.).contains(&g.fuse)
            || !(0.0..=2.).contains(&g.laser_seconds)
        {
            return Err("invalid missile guidance policy".into());
        }
        p.locked |= g.lock || g.terminal;
        p.terminal |= g.terminal;
        p.distance += g.speed as f64 * DT;
        p.travel += g.speed * DT as f32;
        if p.terminal && !p.evaded {
            if let Some(j) = p.target {
                // A fresh violent jink very close to impact can defeat terminal guidance.
                let m = &self.drivers[j].motion;
                let lateral = m.ground_acceleration.dot(self.pose(j).basis.x_axis).abs();
                if self.clock - self.drivers[j].last_jink < 0.23
                    && range / g.speed < 0.23
                    && lateral > 500.
                    && self.drivers[j].input.drive.steer.abs() > 0.85
                {
                    p.evaded = true;
                } else {
                    let goal = self.pose(j).origin;
                    p.position +=
                        (goal - p.position).normalize_or_zero() * (g.speed * DT as f32).min(range);
                }
            }
        }
        if p.evaded {
            p.position += p.velocity * DT as f32;
        } else if !p.terminal {
            let n = self.world.track.sample(p.distance);
            let target_x = p
                .target
                .map_or(p.x, |j| self.pilots[j].x)
                .clamp(-n.geometry.width * 0.7, n.geometry.width * 0.7);
            p.x = toward(p.x, target_x, 18. * DT);
            if n.split_gap > 0.01 && p.x.abs() < n.split_gap + 6. {
                p.x = if p.x < 0. { -1. } else { 1. } * (n.split_gap + 6.);
            }
            let route = n.point(p.x, 10.);
            let loft = p.launch.origin
                + p.launch.basis
                    * glam::Vec3::new(
                        0.,
                        1.15 + 7. * smooth(0., 0.38, p.time) as f32,
                        p.travel - 1.8,
                    );
            let t = g.launch_blend;
            p.position = loft.lerp(route, t * t * (3. - 2. * t));
        }
        p.velocity = (p.position - p.previous) / DT as f32;
        Ok(p.target.is_none() && p.time > g.fuse)
    }
}
