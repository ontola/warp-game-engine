use web_time::Instant;
use crate::gun::Contacts;
use glam::Vec3;
use script::{
    Host, Limits,
    mlua::{Error, Vector},
};
use std::{
    path::Path,
    time::Duration,
};

pub struct HandAnimation {
    host: Host,
    poll: Instant,
    pub status: String,
}

impl HandAnimation {
    pub fn load() -> Result<Self, String> {
        let mut host = Host::new(Limits::default(), |_, _| Ok(()))?;
        let dir = std::env::var_os("GG_ANIMATIONS")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../animations"));
        let name = "support_hand.luau";
        if dir.join(name).is_file() {
            host.only(name);
            host.watch(&dir);
        } else {
            host.add_source(name, include_str!("../../animations/support_hand.luau"))?;
        }
        Ok(Self {
            host,
            poll: Instant::now(),
            status: String::new(),
        })
    }

    pub fn pose(&mut self, c: Contacts, seconds: f32) -> (Vec3, f32) {
        if self.poll.elapsed() >= Duration::from_millis(200) {
            self.host.poll();
            self.poll = Instant::now();
        }
        let v = |p: Vec3| Vector::new(p.x, p.y, p.z);
        let result = self
            .host
            .each("pose")
            .into_iter()
            .next()
            .and_then(|(i, f)| {
                self.host.run(i, "pose", |_| {
                    let (w, roll): (Vector, f32) =
                        f.call((v(c.magazine), v(c.foregrip), c.extension, seconds))?;
                    let point = Vec3::new(w.x(), w.y(), w.z());
                    if !point.is_finite() || !roll.is_finite() {
                        return Err(Error::runtime("expected finite wrist and roll"));
                    }
                    Ok((point, roll.to_radians()))
                })
            });
        for msg in self.host.drain_log() {
            self.status = msg;
            eprintln!("animation: {}", self.status);
        }
        result.unwrap_or((c.magazine, 0.0))
    }
}
