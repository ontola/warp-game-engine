//! Connect the streamed entity sets declared by a Luau game to Warp's architectural
//! lighting. Static transport is independent of simulation state and frame rate.
use crate::room_lighting::{self, RoomLighting};
use crate::world::World;
use glam::Vec3;
use render::clustered_lights::ClusteredLight;
use render::headlights::LocalLight;
use render::irradiance_volume;
use render::light_face::LightFace;
use render::reflection_probes::ProbeBox;
use render::{Gpu, Renderer, Scene};

const ROOMS: usize = 4;

pub(crate) fn panel(pos: Vec3, rgb: Vec3, cd: f32, range: f32, width: f32, depth: f32) -> Result<(LocalLight, LightFace), String> {
    if !width.is_finite() || !depth.is_finite() || width <= 0.0 || depth <= 0.0 || !cd.is_finite() || cd < 0.0 {
        return Err("a ceiling panel needs positive dimensions and finite nonnegative candela".into());
    }
    let face = LightFace::Rect { width_m: width, depth_m: depth };
    let light = face.apply(LocalLight {
        position: pos, direction: Vec3::NEG_Y, intensity_cd: rgb * cd, range_m: range,
        inner_cos: 40_f32.to_radians().cos(), outer_cos: 59_f32.to_radians().cos(),
        source_radius_m: 0.1, falloff: 2.0,
    }).validated()?;
    Ok((light, face))
}

pub(crate) fn prepare(scene: &mut Scene) -> Result<(), String> {
    scene.graphics.clustered_lights = true;
    if scene.irradiance_atlas.is_none() {
        scene.reserve_irradiance_atlas(ROOMS, room_lighting::MAX_ROOM_PROBES)?;
    }
    Ok(())
}

struct Room {
    id: u32,
    min: Vec3,
    max: Vec3,
    entities: Vec<u32>,
    panels: Vec<(LocalLight, LightFace)>,
}

// Each slot keeps its number when another seat leaves. A renderer rebuild needs
// a fresh upload, but can use the results of the already-running bakes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Transport { Invalid, Preview, Baked, PreviousSun }

pub(crate) struct RoomScene {
    pub enabled: bool,
    pub check_geometry: bool,
    rooms: [Option<Room>; ROOMS],
    bakes: RoomLighting,
    uploaded: [Option<(u32, Transport)>; ROOMS],
    sun: Option<Vec3>,
    fixtures_dirty: bool,
    direct_fixtures: Option<bool>,
}

impl RoomScene {
    pub fn new(enabled: bool) -> Self {
        Self { enabled, check_geometry: false, rooms: Default::default(), bakes: Default::default(),
            uploaded: [None; ROOMS], sun: None, fixtures_dirty: false, direct_fixtures: None }
    }

    pub fn set(&mut self, id: u32, min: Vec3, max: Vec3, entities: Vec<u32>, panels: Vec<(LocalLight, LightFace)>) -> Result<(), String> {
        // An explicit replacement snapshots a new geometry/light state, even
        // when the caller reuses the ID before another frame is drawn.
        self.remove(id);
        let slot = self.rooms.iter().position(Option::is_none).ok_or("at most four lit rooms can be active")?;
        if let Some((_, stage)) = &mut self.uploaded[slot] { *stage = Transport::Invalid; }
        self.rooms[slot] = Some(Room {id, min, max, entities, panels});
        self.fixtures_dirty = true;
        Ok(())
    }

    pub fn remove(&mut self, id: u32) {
        if let Some(slot) = self.rooms.iter_mut().find(|s| s.as_ref().is_some_and(|r| r.id == id)) {
            *slot = None;
            let keep: Vec<_> = self.rooms.iter().flatten().map(|r| r.id).collect();
            self.bakes.retain(&keep);
            self.fixtures_dirty = true;
        }
    }

    pub fn clear(&mut self) {
        self.rooms = Default::default();
        self.bakes.retain(&[]);
        self.fixtures_dirty = true;
    }

    pub fn status(&self) -> serde_json::Value {
        let times: std::collections::BTreeMap<_, _> = self.bakes.ready().map(|(id, b)| (id, b.elapsed_ms)).collect();
        serde_json::json!({"active": self.rooms.iter().flatten().count(), "baked": times.len(), "bake_ms": times})
    }

    fn fixtures(&self, renderer: &Renderer) -> Result<(), String> {
        let mut lights = Vec::new();
        let mut reflections = Vec::new();
        for room in self.rooms.iter().flatten() {
            for (i, &(light, face)) in room.panels.iter().enumerate() {
                lights.push(ClusteredLight::shadowed(light, u64::from(room.id) * 8 + i as u64).with_face(face));
            }
            reflections.push(ProbeBox { capture: (room.min + room.max) * 0.5, min: room.min, max: room.max });
        }
        if renderer.clustered_lights_enabled() { renderer.set_clustered_lights(&lights)?; }
        renderer.set_reflection_probes(&reflections)
    }

    pub fn update(&mut self, world: &World, gpu: &Gpu, renderer: &Renderer, sun: Vec3, rebuild: bool) -> Result<(), String> {
        if std::mem::take(&mut self.check_geometry) {
            let missing = self.rooms.each_ref().map(|s| s.as_ref().and_then(|r|
                r.entities.iter().any(|id| world.get(*id).is_none()).then_some(r.id)));
            for id in missing.into_iter().flatten() { self.remove(id); }
        }
        let direct_fixtures = !renderer.clustered_lights_enabled();
        if self.direct_fixtures.replace(direct_fixtures).is_some_and(|old| old != direct_fixtures) {
            self.bakes.retain(&[]);
            for (_, stage) in self.uploaded.iter_mut().flatten() { *stage = Transport::Invalid; }
        }
        if rebuild {
            renderer.set_reflection_probe_interval(30);
            self.uploaded.fill(None);
            self.fixtures_dirty = true;
        }
        if self.fixtures_dirty {
            self.fixtures(renderer)?;
            self.fixtures_dirty = false;
        }
        // A moving sun invalidates only its old transport. Time-of-day changes
        // in illuminance and colour already modulate the bake in the shader.
        // Finish the current generation before rebaking for a moving sun.
        // Four simultaneous visitors must still converge on slower devices.
        if !self.bakes.pending() && self.sun.is_none_or(|old| old.dot(sun) < 0.996) {
            self.bakes.retain(&[]);
            for (_, stage) in self.uploaded.iter_mut().flatten() {
                if *stage == Transport::Baked { *stage = Transport::PreviousSun; }
            }
            self.sun = Some(sun);
        }
        self.bakes.poll()?;
        for (slot, room) in self.rooms.iter().enumerate() {
            let Some(room) = room else {
                if self.uploaded[slot].take().is_some() { renderer.clear_irradiance_slot(gpu, slot)?; }
                continue;
            };
            if !self.bakes.contains(room.id) {
                let occluders = world.room_occluders(&room.entities)?;
                let lights = room.panels.iter().map(|p| p.0).collect();
                self.bakes.request(room.id, room.min, room.max, occluders, lights, sun, direct_fixtures)?;
            }
            let (b, finished) = self.bakes.current(room.id).expect("requested room has lighting");
            if !finished && self.uploaded[slot] == Some((room.id, Transport::PreviousSun)) {
                continue;
            }
            let stamp = (room.id, if finished { Transport::Baked } else { Transport::Preview });
            if self.uploaded[slot] == Some(stamp) { continue; }
            let packed = irradiance_volume::pack_sky_emission(&b.grid, &b.sky, &b.emission, &b.planes)?;
            renderer.set_irradiance_slot(gpu, slot, room.min, room.max, &packed)?;
            self.uploaded[slot] = Some(stamp);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use app::{Controls, Game};
    use crate::{LuaGame, Overrides};

    #[test]
    fn furnished_office_lights_a_shirt_across_the_actual_doorway() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
        let mut game = LuaGame::load(&path, &Overrides { players: Some(1), seed: Some(1948), ..Default::default() }, None).unwrap();
        game.echo = false;
        for _ in 0..12 { game.tick(&[Controls::default()]); }
        game.luau(r#"
            for _, b in state.city.buildings do
                if b.kind == "office" and b.x == 28 then
                    world.place(state.fleet.players[1].body, b.door + vector.create(0,.85,1.6))
                    return
                end
            end
            error("expected an office on the east avenue")
        "#).unwrap();
        for _ in 0..12 { game.tick(&[Controls::default()]); }
        assert!(game.errors().is_empty(), "{:?}", game.logs());
        let core = game.core();
        let room = core.rooms.rooms.iter().flatten().next().expect("streamed office");
        let geometry = core.world.room_occluders(&room.entities).unwrap();
        let lamps = room.panels.iter().map(|p| p.0).collect::<Vec<_>>();
        assert_eq!(lamps.len(), 4);
        let sampler = render::room_bake::PreviewSampler::default();
        let x = (room.min.x + room.max.x) * 0.5;
        let sun = Vec3::new(0.2, 0.9, 0.4).normalize();
        let shirt = |inset: f32| {
            let p = Vec3::new(x, 1.25, room.max.z - inset);
            let (sky, emission, _) = sampler.trace(&geometry, p, sun, &lamps, 2., false);
            (sky.irradiance(Vec3::Z), emission.irradiance(Vec3::Z))
        };
        let (near, _) = shirt(0.15);
        let (inside, lux) = shirt(2.4);
        eprintln!("office shirt front: entrance {near}, 2.4m inside {inside}, bounced fixture RGB {lux} lux");
        assert!(near.x > inside.x && inside.x > 0.05, "{near}, {inside}");
        // The shirt faces the street. Its weak warm fixture return is
        // separate from the much stronger directional daylight through the door.
        assert!(inside.y > 0. && inside.z > 0. && lux.min_element() > 0.1 && lux.x > lux.z, "{inside}, {lux}");
    }
}
