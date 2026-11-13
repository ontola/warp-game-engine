//! Fixed-tick input recording and headless replay for Sideways feedback captures.
use crate::{play, tree_tuning, visuals};
use kern::Timings;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sideways::{
    race::{Control, Race, Settings},
    spec::Cars,
};
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
};

const MAX_TICKS: u64 = 120 * 60 * 60;
const MAX_RUNS: usize = 32_768;
const WINDOW: u64 = 600;

#[derive(Serialize, Deserialize)]
struct Run {
    count: u64,
    controls: Vec<Option<Control>>,
    manual: bool,
    trees: tree_tuning::Settings,
    skip_countdown: bool,
}

#[derive(Default)]
pub struct Recorder {
    runs: Vec<Run>,
    stage: Option<(u32, u64)>,
    end: u64,
}

impl Recorder {
    pub fn record(
        &mut self,
        race: &Race,
        controls: &[Option<Control>],
        trees: tree_tuning::Settings,
        skip_countdown: bool,
    ) {
        let Some((_, _, start)) = &race.stage_origin else {
            return;
        };
        // F10's saved mesh-forest settings can carry across stages. A grown
        // biome races with its authored population and per-species trunks.
        let trees = if race.stage.land.as_ref().is_some_and(|l| l.trunk_profile.is_some()) {
            tree_tuning::Settings::default()
        } else { trees };
        let stage = (race.round, *start);
        if self.stage != Some(stage) {
            self.runs.clear();
            self.stage = Some(stage);
            self.end = *start;
        }
        if race.frame != self.end || race.frame - start >= MAX_TICKS {
            return;
        }
        let manual = race.settings.manual;
        if let Some(last) = self.runs.last_mut() {
            if last.controls == controls
                && last.trees == trees
                && last.manual == manual
                && last.skip_countdown == skip_countdown
            {
                self.end += 1;
                last.count += 1;
                return;
            }
        }
        if self.runs.len() >= MAX_RUNS {
            return;
        }
        self.end += 1;
        self.runs.push(Run {
            count: 1,
            controls: controls.to_vec(),
            manual,
            trees,
            skip_countdown,
        });
    }

    pub fn capture(&self, race: &Race) -> Value {
        let Some((stream, settings, start)) = &race.stage_origin else {
            return Value::Null;
        };
        if self.stage != Some((race.round, *start))
            || self.end != race.frame
            || self.runs.is_empty()
        {
            return json!({"unavailable":"no complete input history for this stage"});
        }
        json!({"version":1,"hz":120,"stream":stream.stream_state(),"settings":settings,"biome":settings.land.as_ref().map(|l| l.biome),
            "land_layout": settings.land.as_ref().map(|l| l.layout).unwrap_or_default(),
            "grown_trunks": settings.land.as_ref().is_some_and(|l| l.trunk_profile.is_some()),
            "round":race.round,"start":start,"end":race.frame,"stage_seeds":[race.stage.noise_seed,race.stage.detail_seed],
            "digest":kern::digest_of(race),"runs":self.runs})
    }
}

#[derive(Deserialize)]
struct Tape {
    version: u32,
    hz: u32,
    stream: u64,
    settings: Settings,
    biome: Option<String>,
    #[serde(default)]
    land_layout: sideways::stage::LandLayout,
    #[serde(default)]
    grown_trunks: bool,
    round: u32,
    start: u64,
    end: u64,
    stage_seeds: [i32; 2],
    digest: String,
    runs: Vec<Run>,
}

impl Tape {
    fn restore_land(&mut self) -> Result<(), String> {
        self.settings.land = self.biome.as_deref()
            .map(|name| crate::biome_land::land_for_layout(name, self.land_layout)).transpose()?;
        if !self.grown_trunks {
            if let Some(land) = &mut self.settings.land { land.trunk_profile = None; }
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), String> {
        let ticks = self
            .end
            .checked_sub(self.start)
            .ok_or("reversed tick range")?;
        if self.version != 1
            || self.hz != 120
            || self.round == 0
            || ticks == 0
            || ticks > MAX_TICKS
            || self.runs.len() > MAX_RUNS
            || self.end == u64::MAX
            || self
                .runs
                .iter()
                .try_fold(0u64, |a, r| a.checked_add(r.count))
                != Some(ticks)
            || !(1..=4).contains(&self.settings.bots)
            || self.settings.rivals > 3
            || !(0.05..=8.0).contains(&self.settings.stage_km)
        {
            return Err("invalid drive capture header".into());
        }
        for r in &self.runs {
            r.trees.validate()?;
            if r.count == 0
                || r.controls.len() > 4
                || r.controls.iter().flatten().any(|c| c.sanitized() != *c)
            {
                return Err("invalid drive capture controls".into());
            }
        }
        Ok(())
    }
}

/// Recordings from the first biome-flora stages used a common native trunk
/// cylinder and the mesh editor's subset. Replay their original collision
/// geometry without coupling it to the renderer's current plant assets.
struct RecordedTrunks {
    colliders: sideways::physics::TreeColliderBaseline,
    ranks: Vec<f64>,
}

impl RecordedTrunks {
    fn capture(race: &Race) -> Result<Self, String> {
        use sideways::{builder, physics::{TreeColliderBaseline, TreeColliderSource}};
        let mut sources = Vec::new();
        for cell in builder::chunk_cells(&race.stage) {
            let chunk = builder::terrain_chunk(&race.stage, cell);
            sources.extend(chunk.trees.iter().map(|spot| TreeColliderSource {
                root: [spot.pos.0, spot.pos.1, spot.pos.2], scale: [spot.scale, spot.scale_y],
            }));
        }
        let colliders = TreeColliderBaseline::capture(&race.phys, &sources)?;
        let ranks = sources.iter().map(|s| tree_tuning::root_rank(s.root)).collect();
        Ok(Self { colliders, ranks })
    }

    fn apply(&self, race: &mut Race, settings: tree_tuning::Settings) -> Result<(), String> {
        let enabled: Vec<_> = self.ranks.iter().map(|rank| *rank < f64::from(settings.density)).collect();
        self.colliders.apply(&mut race.phys, settings.size_scale, &enabled)
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    if !(args.len() == 2 || (args.len() == 3 && args[2] == "--state-only")) {
        return Err("usage: sideways replay-drive F12.json FRESH_DIR [--state-only]".into());
    }
    #[derive(Deserialize)]
    struct Capture {
        drive: Tape,
    }
    let capture: Capture = serde_json::from_slice(&fs::read(&args[0]).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let mut tape = capture.drive;
    tape.validate()?;
    tape.restore_land()?;
    let out = Path::new(&args[1]);
    fs::create_dir(out).map_err(|e| format!("fresh replay directory {}: {e}", out.display()))?;
    let mut race = Race::from_stage_stream(
        gdcompat::Pcg::from_stream_state(tape.stream),
        tape.settings,
        Cars::load(None).map_err(|e| e.to_string())?,
        tape.start,
        tape.round - 1,
    );
    if tape.stage_seeds != [race.stage.noise_seed, race.stage.detail_seed] {
        return Err("reconstructed stage seeds differ".into());
    }
    let views = race.settings.bots;
    let mut rest = Default::default();
    // Biome trunks are part of the simulation. A state-only replay does not
    // need to cook the visual plant assets to reconstruct them.
    let mut built = if args.len() == 2 || tape.land_layout != sideways::stage::LandLayout::BiomeFlora {
        let poses = play::poses(&race);
        let cams = play::cameras(&race, &poses, &[0.0; 4], &[false; 4], &mut rest);
        let looks = crate::car_look::Looks::load(None, vec![])?;
        Some(visuals::build_live(&race.stage, &poses,
            &cams.iter().take(views).map(|c| c.pos).collect::<Vec<_>>(), &looks)?)
    } else { None };
    let mut trees = tape.runs[0].trees;
    let old_biome_trunks = if tape.land_layout == sideways::stage::LandLayout::BiomeFlora && !tape.grown_trunks {
        let previous = RecordedTrunks::capture(&race)?;
        previous.apply(&mut race, trees)?;
        Some(previous)
    } else { None };
    let baseline = if let (None, Some(visuals)) = (&old_biome_trunks, &mut built) {
        tree_tuning::Baseline::prepare_optional_stage(&mut visuals.scene, &mut race.phys, trees)?
    } else { None };
    let gpu = if args.len() == 2 {
        Some(render::Gpu::new()?)
    } else {
        None
    };
    let light = play::lighting(race.weather);
    let mut renderer = None;
    let mut samples = Vec::new();
    let mut frames = Vec::new();
    let mut timings = Timings::default();
    for run in tape.runs {
        if run.trees != trees {
            if let Some(previous) = &old_biome_trunks {
                previous.apply(&mut race, run.trees)?;
            } else {
                let base = baseline.as_ref().ok_or("unsupported tree edit in recorded stage")?;
                base.apply_scene(&mut built.as_mut().ok_or("missing source mesh forest")?.scene, run.trees)?;
                base.apply_physics(&mut race.phys, run.trees)?;
            }
            trees = run.trees;
            renderer = None;
        }
        race.settings.manual = run.manual;
        for p in &mut race.players {
            p.car.manual = run.manual;
        }
        for _ in 0..run.count {
            if run.skip_countdown {
                race.phase = "racing";
                race.clock = race.clock.max(0.0);
            }
            race.step_with_inputs(&mut timings, &run.controls);
            if tape.end - race.frame >= WINDOW {
                continue;
            }
            let players = race
                .players
                .iter()
                .map(|p| {
                    json!({"position":p.car.pose(&race.phys).translation.to_array(),
                "velocity":p.car.linvel(&race.phys).to_array(),"gear":p.car.gear,"rpm":p.car.rpm()})
                })
                .collect::<Vec<_>>();
            samples
                .push(json!({"tick":race.frame,"digest":kern::digest_of(&race),"players":players}));
            if let Some(gpu) = &gpu {
                if (tape.end - race.frame) % 8 != 0 {
                    continue;
                }
                let built = built.as_mut().ok_or("missing replay scene")?;
                let poses = play::poses(&race);
                let cams = play::cameras(&race, &poses, &[0.0; 4], &[false; 4], &mut rest);
                let positions = cams.iter().take(views).map(|c| c.pos).collect::<Vec<_>>();
                built.update_cars(&poses);
                let dirty = built.update_ground_cover(&positions);
                let renderer = renderer
                    .get_or_insert_with(|| render::Renderer::new(gpu, &built.scene, &light));
                if let Some(range) = dirty {
                    renderer.update_instances(gpu, range.start, &built.scene.instances[range])?;
                }
                built.update_trees(renderer, gpu, &positions)?;
                if let Some(car) = built.cars.first() {
                    renderer.update_instances(
                        gpu,
                        car.first,
                        &built.scene.instances[car.first..],
                    )?;
                }
                let rects = crate::shot::view_rects(&cams, views, 640, 360);
                let started = std::time::Instant::now();
                let (rgba, stats) =
                    renderer.try_render_views(gpu, &rects, &light, 640, 360, &[])?;
                let render_readback_ms = started.elapsed().as_secs_f64() * 1000.0;
                let file = format!("frame-{:03}.png", frames.len());
                render::write_png(&out.join(&file), 640, 360, &rgba)?;
                frames.push(json!({"tick":race.frame,"image":file,"gpu_pass_ms":stats.gpu_ms,"render_readback_ms":render_readback_ms}));
            }
        }
    }
    let report = json!({"start":tape.start,"end":tape.end,"matches_capture":kern::digest_of(&race)==tape.digest,
        "expected_digest":tape.digest,"samples":samples,"frames":frames,"adapter":gpu.as_ref().map(|g|&g.info.name)});
    fs::write(
        out.join("drive.json"),
        serde_json::to_vec(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

pub fn save_visibility(
    png: &Path,
    meta: &mut Value,
    stats: &render::FrameStats,
    built: &visuals::Built,
) -> Result<(), String> {
    let source = VisibilitySource::snapshot(built);
    write_visibility(png, meta, &stats.visibility, &source)
}

/// Raw frame-owned data for a queued F12. Only the writer expands visible
/// owners and triangles into JSON, after the GPU copy has completed.
pub struct VisibilitySource {
    meshes: BTreeMap<u32, String>,
    instances: Vec<render::scene::GpuInstance>,
    clusters: Vec<render::cluster::GpuCluster>,
    flags: Vec<u32>,
    car_parts: BTreeMap<u32, Value>,
    poses: BTreeMap<u32, u32>,
    parents: Vec<Option<u32>>,
    actor_parts: BTreeMap<u32, (u32, u32)>,
}

/// Shared GPU branch owners encode their primitive member in the low 12
/// bits (render/shaders/virtual_member.wgsl). Their posed transform lives
/// on the GPU; indexing the static scene with that encoded ID is invalid.
pub(crate) fn virtual_plant_member(id: u32) -> Option<(u32,u32)> {
    (id & 0x8000_0000 != 0).then_some(((id >> 12) & 0x7ffff, id & 0xfff))
}

impl VisibilitySource {
    pub fn snapshot(built: &visuals::Built) -> Self {
        let meshes: BTreeMap<_, _> = built
            .scene
            .meshes
            .iter()
            .zip(&built.mesh_names)
            .map(|(m, n)| (m.first_cluster, n.clone()))
            .collect();
        // Owner indices come from this frame's visibility buffer. Model slot labels
        // come from allocation order, including glTFs with absent wheel nodes.
        let mut car_parts = BTreeMap::new();
        for (car_index, car) in built.cars.iter().enumerate() {
            for (offset, name) in car.part_names.iter().enumerate() {
                car_parts.insert(
                    (car.first + offset) as u32,
                    json!({"kind":"car",
                "car_index":car_index,"model":car.model,"part":name,"body_instance":car.first}),
                );
            }
        }
        let mut poses = BTreeMap::new();
        for (index, part) in built.scene.part_wind.iter().enumerate() {
            if let Some(owner) = part.owner {
                poses.insert(owner, index as u32);
            }
        }
        for attachment in &built.scene.part_attachments {
            poses.insert(attachment.owner, attachment.pose);
        }
        let mut actor_parts = BTreeMap::new();
        if let Some(animation) = &built.scene.baked_animations {
            for part in &animation.parts {
                actor_parts.insert(part.owner, (part.actor, part.bone));
            }
        }
        Self {
            meshes,
            car_parts,
            poses,
            actor_parts,
            parents: built.scene.part_wind.iter().map(|p| p.parent).collect(),
            instances: built.scene.instances.clone(),
            clusters: built.scene.clusters.clone(),
            flags: built.scene.materials.iter().map(|m| m.flags).collect(),
        }
    }

    fn describe(&self, captures: &[render::frame::VisibilityCapture]) -> Value {
        let mut owners = BTreeMap::new();
        let mut clusters = BTreeMap::new();
        for v in captures {
            for &[owner, cluster] in &v.visible {
                if let Some((slot, member)) = virtual_plant_member(owner) {
                    owners.entry(owner).or_insert_with(|| json!({"category":"Trees",
                        "hierarchy":{"kind":"gpu_plant_member","branch_owner":slot,"member":member}}));
                } else if let Some(i) = self.instances.get(owner as usize) {
                    owners.entry(owner).or_insert_with(|| {
                        let mut info = json!({"mesh":self.meshes.get(&i.first_cluster),
                        "category":format!("{:?}",i.category()),"transform":i.rows});
                        if let Some(car) = self.car_parts.get(&owner) {
                            info["hierarchy"] = car.clone();
                        } else if let Some(&pose) = self.poses.get(&owner) {
                            let path: Vec<_> =
                                kern::parenting::ancestors(pose, |p| self.parents[p as usize])
                                    .collect();
                            info["hierarchy"] =
                                json!({"kind":"plant","pose_to_root":path,"root_pose":path.last()});
                        } else if let Some(&(actor, bone)) = self.actor_parts.get(&owner) {
                            info["hierarchy"] =
                                json!({"kind":"animated_actor","actor":actor,"bone":bone});
                        }
                        info
                    });
                }
                if let Some(c) = self.clusters.get(cluster as usize) {
                    clusters.entry(cluster).or_insert_with(|| {
                        json!({"lod_depth":c.depth,"material":c.material,"error":c.own_error,
                    "flags":self.flags.get(c.material as usize),"shader":"shade.wgsl"})
                    });
                }
            }
        }
        json!({"version":1,"owners":owners,"clusters":clusters})
    }
}

pub fn write_visibility(
    png: &Path,
    meta: &mut Value,
    captures: &[render::frame::VisibilityCapture],
    source: &VisibilitySource,
) -> Result<(), String> {
    app::feedback::write_visibility(png,meta,captures,source.describe(captures))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn later_stage_replays_driver_edges_and_ai_from_original_stream() {
        let settings = Settings {
            stage_km: 0.2,
            bots: 2,
            rivals: 0,
            land: Some(crate::biome_land::land("desert").unwrap()),
            ..Default::default()
        };
        let mut race = Race::new(17, settings, Cars::load(None).unwrap());
        let mut timings = Timings::default();
        race.step_with_inputs(&mut timings, &[]);
        race.new_stage(&mut timings);
        let mut recorder = Recorder::default();
        for n in 0..610 {
            let controls = [
                Some(Control {
                    throttle: 0.85,
                    steer: 0.17,
                    handbrake: if n > 570 { 1.0 } else { 0.0 },
                    recover: n == 500,
                    shift: if n == 435 { 1 } else { 0 },
                    ..Default::default()
                }),
                None,
            ];
            recorder.record(&race, &controls, tree_tuning::Settings{density:0.45,..Default::default()}, false);
            race.step_with_inputs(&mut timings, &controls);
        }
        let mut capture = recorder.capture(&race);
        let mut tape: Tape = serde_json::from_value(capture.clone()).unwrap();
        tape.restore_land().unwrap();
        assert!(tape.grown_trunks);
        assert!(tape.runs.iter().all(|r|r.trees==tree_tuning::Settings::default()));
        assert!(!tape.settings.land.as_ref().unwrap().flowing_water);
        capture.as_object_mut().unwrap().remove("grown_trunks");
        let mut recorded: Tape = serde_json::from_value(capture.clone()).unwrap();
        recorded.restore_land().unwrap();
        assert!(recorded.settings.land.unwrap().trunk_profile.is_none());
        capture.as_object_mut().unwrap().remove("land_layout");
        let mut old: Tape = serde_json::from_value(capture).unwrap();
        old.restore_land().unwrap();
        assert!(old.settings.land.unwrap().flowing_water);
        tape.validate().unwrap();
        assert!(tape.runs.len() < 10);
        let mut replay = Race::from_stage_stream(
            gdcompat::Pcg::from_stream_state(tape.stream),
            tape.settings,
            Cars::load(None).unwrap(),
            tape.start,
            tape.round - 1,
        );
        for run in tape.runs {
            for _ in 0..run.count {
                replay.step_with_inputs(&mut timings, &run.controls);
            }
        }
        assert_eq!(kern::digest_of(&race), kern::digest_of(&replay));
    }
}
