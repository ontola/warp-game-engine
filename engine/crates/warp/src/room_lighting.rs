//! Bounded, on-demand indoor irradiance bakes. The simulation supplies the room's
//! opaque geometry and lamps; only the renderer consumes the asynchronous result.
use glam::Vec3;
use render::headlights::{self, LocalLight};
use render::irradiance_volume::{Grid, Probe};
use render::room_bake::{self, Occluders, Placement, PreviewSampler, Settings};
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
    min: Vec3,
    max: Vec3,
    occluders: Occluders,
    lamps: Vec<LocalLight>,
    direct_fixtures: bool,
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
        let mut emission = if self.direct_fixtures {
            placed.positions.iter().map(|&p| room_bake::direct_lamp_probe(&self.occluders, p, &self.lamps)).collect()
        } else { vec![Probe::default(); self.grid.count()] };
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

impl Request {
    fn preview(&self, sampler: &PreviewSampler) -> BakedRoom {
        let started = web_time::Instant::now();
        let center = (self.min + self.max) * 0.5;
        let cell = (self.max - self.min).max_element().max(0.5);
        let grid = Grid { origin: center - Vec3::splat(cell * 0.5), cell, dims: [2; 3] };
        let (sky, emission, _) = sampler.trace(&self.occluders, center, self.sun, &self.lamps, cell, self.direct_fixtures);
        BakedRoom { grid, sky: vec![sky; 8], emission: vec![emission; 8], planes: Vec::new(),
            elapsed_ms: started.elapsed().as_secs_f64() * 1000. }
    }
}

/// The browser has no worker pool. Keep one small spatial grid per visited
/// room and interleave individual probes across all cameras' rooms.
struct Refinement {
    request: Request,
    result: BakedRoom,
}

impl Refinement {
    fn new(request: Request) -> Self {
        let span = request.max - request.min;
        let cell = (span.x / 6.).max(span.y / 2.).max(span.z / 6.).max(0.5);
        let steps = (span / cell).ceil().clamp(Vec3::ONE, Vec3::new(6., 2., 6.));
        let grid = Grid { origin: request.min + (span - steps * cell) * 0.5, cell,
            dims: steps.to_array().map(|n| n as u32 + 1) };
        let result = BakedRoom { grid, sky: Vec::with_capacity(grid.count()), emission: Vec::with_capacity(grid.count()),
            planes: Vec::with_capacity(grid.count()), elapsed_ms: 0. };
        Self { request, result }
    }

    fn advance(&mut self, sampler: &PreviewSampler) -> bool {
        let started = web_time::Instant::now();
        let r = &self.request;
        let b = &mut self.result;
        // The atlas is sampled within the room's bounds. Trace its outside
        // rows just inside the shell, clear of the wall and ceiling lining.
        let margin = ((r.max - r.min) * 0.16).min(Vec3::splat(0.40));
        let p = b.grid.position(b.sky.len()).clamp(r.min + margin, r.max - margin);
        let (sky, emission, plane) = sampler.trace(&r.occluders, p, r.sun, &r.lamps, b.grid.cell, r.direct_fixtures);
        b.sky.push(sky);
        b.emission.push(emission);
        b.planes.push(plane);
        b.elapsed_ms += started.elapsed().as_secs_f64() * 1000.;
        b.sky.len() == b.grid.count()
    }
}

struct Entry {
    preview: BakedRoom,
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
    refinements: VecDeque<Refinement>,
    sampler: PreviewSampler,
}

impl RoomLighting {
    pub fn contains(&self, id: u32) -> bool { self.entries.contains_key(&id) }

    pub fn pending(&self) -> bool { self.entries.values().any(|entry| entry.baked.is_none()) }

    pub fn current(&self, id: u32) -> Option<(&BakedRoom, bool)> {
        let entry = self.entries.get(&id)?;
        Some((entry.baked.as_ref().unwrap_or(&entry.preview), entry.baked.is_some()))
    }

    pub fn ready(&self) -> impl Iterator<Item = (u32, &BakedRoom)> {
        self.entries.iter().filter_map(|(&id, e)| Some((id, e.baked.as_ref()?)))
    }

    /// Remove stale results immediately. A queued request is dropped without
    /// baking. An already-running one checks cancellation between bake stages.
    pub fn retain(&mut self, ids: &[u32]) -> bool {
        let before = self.entries.len();
        self.entries.retain(|id, _| ids.contains(id));
        self.waiting.retain(|req| !req.cancelled.load(Ordering::Relaxed));
        self.refinements.retain(|job| !job.request.cancelled.load(Ordering::Relaxed));
        before != self.entries.len()
    }

    pub fn request(
        &mut self, id: u32, min: Vec3, max: Vec3, occluders: Occluders, lights: Vec<LocalLight>, sun: Vec3, direct_fixtures: bool,
    ) -> Result<(), String> {
        self.request_for(id, min, max, occluders, lights, sun, direct_fixtures, cfg!(target_arch = "wasm32"))
    }

    fn request_for(&mut self, id: u32, min: Vec3, max: Vec3, occluders: Occluders, lights: Vec<LocalLight>,
        sun: Vec3, direct_fixtures: bool, cooperative: bool) -> Result<(), String> {
        if self.contains(id) { return Ok(()); }
        if self.entries.len() == MAX_ROOMS { return Err("at most four streamed room bakes are active".into()); }
        let grid = grid(min, max)?;
        if !sun.is_finite() || !sun.length_squared().is_finite() || sun.length_squared() < 1e-8 {
            return Err("room lighting needs a finite sun direction".into());
        }
        let lamps = headlights::validate_local(&lights)?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let request = Request { id, grid, min, max, occluders, lamps, direct_fixtures, sun: sun.normalize(), cancelled: cancelled.clone() };
        let preview = request.preview(&self.sampler);
        self.entries.insert(id, Entry { cancelled, preview, baked: None });
        if cooperative { self.refinements.push_back(Refinement::new(request)); }
        else { self.waiting.push_back(request); }
        Ok(())
    }

    /// Cheap nonblocking render-thread poll. At most one room bake runs per
    /// game. Launch the next job only while its room is still requested.
    pub fn poll(&mut self) -> Result<bool, String> {
        let mut changed = false;
        // A single probe is the smallest indivisible unit. Check the deadline
        // after each and cap count as well on browsers with coarse clocks.
        if !self.refinements.is_empty() {
            let frame = web_time::Instant::now();
            for _ in 0..4 {
                let Some(mut job) = self.refinements.pop_front() else { break; };
                if job.advance(&self.sampler) {
                    if let Some(entry) = self.entries.get_mut(&job.request.id) {
                        entry.baked = Some(job.result);
                        changed = true;
                    }
                } else { self.refinements.push_back(job); }
                if frame.elapsed().as_secs_f64() >= 0.00075 { break; }
            }
        }
        if let Some(current) = self.working.as_ref() {
            let finished = match current.result.try_recv() {
                Ok(result) => result,
                Err(mpsc::TryRecvError::Empty) => return Ok(changed),
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
    fn browser_rooms_start_lit_then_refine_fairly_and_cancel_cleanly() {
        let block = |pos, size| Affine3A::from_scale_rotation_translation(size, glam::Quat::IDENTITY, pos);
        // One open face. The side of a shirt facing it must receive more
        // daylight near the entrance than at the back of the room.
        let geometry = || Occluders::from_boxes(&[
            block(Vec3::new(0., -0.1, 0.), Vec3::new(12., 0.2, 12.)),
            block(Vec3::new(0., 3.9, 0.), Vec3::new(12., 0.2, 12.)),
            block(Vec3::new(-5.9, 2., 0.), Vec3::new(0.2, 4., 12.)),
            block(Vec3::new(5.9, 2., 0.), Vec3::new(0.2, 4., 12.)),
            block(Vec3::new(0., 2., -5.9), Vec3::new(12., 4., 0.2)),
        ]);
        let lamp = LocalLight { position: Vec3::new(0., 3.4, -1.), direction: Vec3::NEG_Y,
            intensity_cd: Vec3::new(1500., 1000., 600.), range_m: 16., inner_cos: 0.6, outer_cos: 0.3,
            source_radius_m: 0.1, falloff: 2. };
        let min = Vec3::new(-6., 0., -6.);
        let max = Vec3::new(6., 4., 6.);
        let mut manager = RoomLighting::default();
        for id in 0..4 {
            manager.request_for(id, min, max, geometry(), vec![lamp], Vec3::Y, false, true).unwrap();
            let (preview, complete) = manager.current(id).unwrap();
            assert!(!complete && preview.grid.count() == 8);
            let s = preview.sky[0].irradiance(Vec3::Z);
            let e = preview.emission[0].irradiance(Vec3::Z);
            assert!(s.x > 0.05 && s.y > 0. && e.x > e.z && e.z > 1., "preview {s}, {e}");
        }
        manager.poll().unwrap();
        let completed: usize = manager.refinements.iter().map(|r| r.result.sky.len()).sum();
        assert!((1..=4).contains(&completed));
        assert!(manager.refinements.iter().all(|r| r.result.sky.len() <= 1));
        assert!(manager.retain(&[2]));
        assert_eq!(manager.refinements.len(), 1);
        let n = manager.refinements[0].result.grid.count();
        for _ in 0..n { manager.poll().unwrap(); }
        let (b, finished) = manager.current(2).unwrap();
        assert!(finished && b.grid.count() <= 7 * 3 * 7);
        let daylight = |z| render::irradiance_volume::sample(&b.grid, &b.sky, Vec3::new(0., 1.5, z), Vec3::Z);
        let near = daylight(5.2);
        let deep = daylight(-4.);
        let warm = render::irradiance_volume::sample(&b.grid, &b.emission, Vec3::new(0., 1.5, -4.), Vec3::Z);
        assert!(near.x > deep.x + 0.25 && warm.x > warm.z && warm.z > 1., "{near}, {deep}, {warm}");
        assert!(b.planes.iter().any(|p| p[..3] != [0.; 3]));
        eprintln!("browser room {} probes in {:.2}ms CPU; entrance {near}, back {deep}, panel return {warm} lux", b.grid.count(), b.elapsed_ms);
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
        manager.request(7, min, max, geometry(), vec![], Vec3::Y, false).unwrap();
        manager.poll().unwrap();
        manager.request(9, min, max, geometry(), vec![], Vec3::Y, false).unwrap();
        assert!(manager.retain(&[]));
        assert_eq!(manager.waiting.len(), 0);
        manager.request(7, min, max, geometry(), vec![lamp], Vec3::Y, false).unwrap();
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
