//! Bounded, on-demand indoor irradiance bakes. The simulation supplies the room's
//! opaque geometry and lamps; only the renderer consumes the asynchronous result.
use glam::Vec3;
use render::headlights::{self, LocalLight};
use render::irradiance_volume::{Grid, Probe};
use render::room_bake::{self, Occluders, Placement, Settings};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}, mpsc};

pub const MAX_ROOM_PROBES: usize = 19 * 6 * 19;
const MAX_ROOMS: usize = 4;

/// Fit the occupied room plus a full cell of padding on each face into one
/// atlas region (at most 19 * 6 * 19 = 2166 probes). Centering the unused
/// half-cell distributes its transition zone evenly on both sides.
pub fn grid(min: Vec3, max: Vec3) -> Result<Grid, String> {
    let span = max - min;
    if !min.is_finite() || !max.is_finite() || !span.is_finite() || span.min_element() <= 0. {
        return Err("room lighting needs finite bounds with positive volume".into());
    }
    let cell = (span.x / 16.).max(span.z / 16.).max(span.y / 3.).max(0.5) * (1. + f32::EPSILON);
    let mut g = Grid::covering(min, max, cell);
    let steps = Vec3::from_array(g.dims.map(|n| (n - 1) as f32));
    g.origin = min + (span - steps * cell) * 0.5;
    g.validate()?;
    Ok(g)
}

pub struct BakedRoom {
    /// Worker time for this geometry and its lights, excluding queue time.
    pub elapsed_ms: f64,
    pub grid: Grid,
    pub sky: Vec<Probe>,
    pub emission: Vec<Probe>,
    pub planes: Vec<[f32; 4]>,
}

struct Request {
    id: u32,
    grid: Grid,
    occluders: Occluders,
    lamps: Vec<LocalLight>,
    sun: Vec3,
    cancelled: Arc<AtomicBool>,
}

impl Request {
    fn bake(&self) -> Result<Option<BakedRoom>, String> {
        let cancelled = || self.cancelled.load(Ordering::Relaxed);
        if cancelled() { return Ok(None); }
        let started = web_time::Instant::now();
        let quality = Settings { sky_rays: 64, bounce_rays: 32, bounces: 2, reflectance: 0.5 };
        let placed = Placement::new(&self.grid, &self.occluders);
        if cancelled() { return Ok(None); }
        let sky = room_bake::bake_placed(&self.grid, &self.occluders, &placed, self.sun, &quality)?;
        if cancelled() { return Ok(None); }
        let mut emission = vec![Probe::default(); self.grid.count()];
        if !self.lamps.is_empty() {
            let lamps = room_bake::bake_lights_placed(
                &self.grid, &self.occluders, &placed, &self.lamps, &quality,
            )?;
            for lamp in lamps {
                for (sum, p) in emission.iter_mut().zip(lamp) {
                    for (axis, received) in sum.axes.iter_mut().zip(p.axes) { *axis += received; }
                }
            }
        }
        if cancelled() { return Ok(None); }
        let planes = room_bake::separation_planes_placed(&self.grid, &self.occluders, &placed);
        Ok(Some(BakedRoom { grid: self.grid, sky, emission, planes, elapsed_ms: started.elapsed().as_secs_f64() * 1000. }))
    }
}

struct Entry {
    cancelled: Arc<AtomicBool>,
    baked: Option<BakedRoom>,
}

impl Drop for Entry {
    fn drop(&mut self) { self.cancelled.store(true, Ordering::Relaxed); }
}

struct Working {
    id: u32,
    cancelled: Arc<AtomicBool>,
    result: mpsc::Receiver<Result<Option<BakedRoom>, String>>,
}

#[derive(Default)]
pub struct RoomLighting {
    entries: BTreeMap<u32, Entry>,
    waiting: VecDeque<Request>,
    working: Option<Working>,
}

impl RoomLighting {
    pub fn contains(&self, id: u32) -> bool { self.entries.contains_key(&id) }

    pub fn ready(&self) -> impl Iterator<Item = (u32, &BakedRoom)> {
        self.entries.iter().filter_map(|(&id, e)| Some((id, e.baked.as_ref()?)))
    }

    /// Remove stale results immediately. A queued request is dropped without
    /// baking. An already-running one checks cancellation between bake stages.
    pub fn retain(&mut self, ids: &[u32]) -> bool {
        let before = self.entries.len();
        self.entries.retain(|id, _| ids.contains(id));
        self.waiting.retain(|req| !req.cancelled.load(Ordering::Relaxed));
        before != self.entries.len()
    }

    pub fn request(
        &mut self, id: u32, min: Vec3, max: Vec3, occluders: Occluders, lights: Vec<LocalLight>, sun: Vec3,
    ) -> Result<(), String> {
        if self.contains(id) { return Ok(()); }
        if self.entries.len() == MAX_ROOMS { return Err("at most four streamed room bakes are active".into()); }
        let grid = grid(min, max)?;
        if !sun.is_finite() || !sun.length_squared().is_finite() || sun.length_squared() < 1e-8 {
            return Err("room lighting needs a finite sun direction".into());
        }
        let lamps = headlights::validate_local(&lights)?;
        let cancelled = Arc::new(AtomicBool::new(false));
        self.entries.insert(id, Entry { cancelled: cancelled.clone(), baked: None });
        // Browsers without threads keep the immediate direct-lit room. A CPU
        // bake in the browser's render loop would stall every player's frame.
        if !cfg!(target_arch = "wasm32") {
            self.waiting.push_back(Request { id, grid, occluders, lamps, sun: sun.normalize(), cancelled });
        }
        Ok(())
    }

    /// Cheap nonblocking render-thread poll. At most one room bake runs per
    /// game. Launch the next job only while its room is still requested.
    pub fn poll(&mut self) -> Result<bool, String> {
        let mut changed = false;
        if let Some(current) = self.working.as_ref() {
            let finished = match current.result.try_recv() {
                Ok(result) => result,
                Err(mpsc::TryRecvError::Empty) => return Ok(false),
                Err(mpsc::TryRecvError::Disconnected) => Err("room lighting worker stopped unexpectedly".into()),
            };
            let current = self.working.take().unwrap();
            // Re-entry may have reused the ID while an earlier bake was busy.
            if !current.cancelled.load(Ordering::Relaxed) {
                match finished {
                    Ok(Some(baked)) => {
                        if let Some(entry) = self.entries.get_mut(&current.id) {
                            entry.baked = Some(baked);
                            changed = true;
                        }
                    }
                    Ok(None) => {}
                    Err(error) => { self.entries.remove(&current.id); return Err(error); }
                }
            }
        }
        if let Some(req) = self.waiting.pop_front() {
            let id = req.id;
            let cancelled = req.cancelled.clone();
            let (send, result) = mpsc::channel();
            let started = std::thread::Builder::new().name("room-lighting".into()).spawn(move || {
                let baked = room_bake::with_worker_limit(2, || req.bake());
                let _ = send.send(baked);
            });
            if let Err(err) = started {
                self.entries.remove(&id);
                return Err(format!("could not start room lighting worker: {err}"));
            }
            self.working = Some(Working { id, cancelled, result });
        }
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Affine3A;

    #[test]
    fn varying_rooms_fit_the_atlas_with_padding() {
        for extent in [Vec3::new(36., 4.2, 36.), Vec3::new(12.7, 4.2, 25.3),
                Vec3::new(3., 12., 7.), Vec3::splat(0.2)] {
            let min = Vec3::new(-140., 0., 80.);
            let max = min + extent;
            let grid = grid(min, max).unwrap();
            let end = grid.position(grid.count() - 1);
            assert!(grid.count() <= MAX_ROOM_PROBES);
            assert!((min - grid.origin).min_element() >= grid.cell - 1e-4);
            assert!((end - max).min_element() >= grid.cell - 1e-4);
        }
        assert!(grid(Vec3::ONE, Vec3::ZERO).is_err());
    }

    #[test]
    fn released_or_replaced_rooms_never_receive_stale_lighting() {
        let mut manager = RoomLighting::default();
        let min = Vec3::ZERO;
        let max = Vec3::new(20., 4.2, 22.);
        let block = |pos, size| Affine3A::from_scale_rotation_translation(size, glam::Quat::IDENTITY, pos);
        let geometry = || Occluders::from_boxes(&[
            block(Vec3::new(10., -0.1, 11.), Vec3::new(20., 0.2, 22.)),
            block(Vec3::new(10., 4.1, 11.), Vec3::new(20., 0.2, 22.)),
            block(Vec3::new(0.1, 2.1, 11.), Vec3::new(0.2, 4.2, 22.)),
            block(Vec3::new(19.9, 2.1, 11.), Vec3::new(0.2, 4.2, 22.)),
            block(Vec3::new(10., 2.1, 0.1), Vec3::new(20., 4.2, 0.2)),
        ]);
        let lamp = LocalLight {
            position: Vec3::new(10., 3.6, 9.), direction: Vec3::NEG_Y, intensity_cd: Vec3::splat(1600.),
            range_m: 20., inner_cos: 0.6, outer_cos: 0.3, source_radius_m: 0.15, falloff: 2.,
        };
        manager.request(7, min, max, geometry(), vec![], Vec3::Y).unwrap();
        manager.poll().unwrap();
        manager.request(9, min, max, geometry(), vec![], Vec3::Y).unwrap();
        assert!(manager.retain(&[]));
        assert_eq!(manager.waiting.len(), 0);
        manager.request(7, min, max, geometry(), vec![lamp], Vec3::Y).unwrap();
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(60);
        while manager.ready().next().is_none() {
            manager.poll().unwrap();
            assert!(start.elapsed() < timeout, "room bake timed out");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let (id, room) = manager.ready().next().unwrap();
        eprintln!("{} room probes, sky and lamp bounce, {:.2}s", room.grid.count(), start.elapsed().as_secs_f32());
        assert_eq!(id, 7);
        let i = (0..room.grid.count()).min_by_key(|&i|
            (10000.0 * room.grid.position(i).distance_squared(Vec3::new(10., 2., 7.))) as u32).unwrap();
        assert!(room.sky[i].irradiance(Vec3::Y).x < 0.15);
        assert!(room.emission[i].irradiance(Vec3::Y).max_element() > 1.0, "must use the replacement lamp");
        assert_eq!(room.planes.len(), room.grid.count());
        assert!(manager.retain(&[]));
        assert_eq!(manager.ready().count(), 0);
    }
}
