//! The shared vehicle solver from Luau. The static collision set is captured at
//! reset so each wheel queries Rapier's spatial index instead of scanning the scene.
use crate::api::{Shared, lock, sim, vec3};
use physics::vehicle::{CarSpec, fleet::{Fleet, Pedals}};
use script::mlua::{self, Lua, Table, Value, Vector};

fn error(msg: &str) -> mlua::Error { mlua::Error::runtime(msg) }
fn vec(v: glam::Vec3) -> Vector { Vector::new(v.x, v.y, v.z) }

pub fn register(lua: &Lua, api: &Table, core: Shared) -> mlua::Result<()> {
    let t = lua.create_table()?;
    let c = core.clone();
    t.set("reset", lua.create_function(move |_, tag: Option<String>| {
        let mut c = lock(&c);
        sim(&c, "vehicles.reset")?;
        let fleet = Fleet::new(c.world.entities.values()
            .filter(|e| e.solid && tag.as_ref().is_none_or(|name| name == &e.tag)).map(|e| e.bounds()));
        c.world.fleet = Some(fleet);
        Ok(())
    })?)?;
    let c = core.clone();
    t.set("spawn", lua.create_function(move |_, (pos, yaw): (Value, Option<f32>)| {
        let p = vec3(&pos, "vehicles.spawn position")?;
        let yaw = yaw.unwrap_or(0.);
        if !p.is_finite() || !yaw.is_finite() { return Err(error("vehicles.spawn needs a finite pose")); }
        let mut c = lock(&c);
        sim(&c, "vehicles.spawn")?;
        let capacity = c.world.capacity();
        let fleet = c.world.fleet.as_mut().ok_or_else(|| error("call vehicles.reset before spawn"))?;
        if fleet.count() >= capacity { return Err(error("vehicles.spawn exceeds max_entities")); }
        Ok(fleet.spawn(CarSpec::compact(), p, yaw))
    })?)?;
    let c = core.clone();
    t.set("drive", lua.create_function(move |_, (id, controls): (u32, Option<Table>)| {
        let controls = controls.map(|t| {
            let axis = |name: &str| -> mlua::Result<f64> {
                let value = t.raw_get::<Option<f64>>(name)?.unwrap_or(0.);
                if !value.is_finite() { return Err(error("vehicles.drive needs finite inputs")); }
                Ok(value)
            };
            Ok::<_, mlua::Error>(Pedals { throttle: axis("throttle")?, brake: axis("brake")?, steer: axis("steer")?, handbrake: axis("handbrake")? })
        }).transpose()?;
        let mut c = lock(&c);
        sim(&c, "vehicles.drive")?;
        if !c.world.fleet.as_mut().is_some_and(|f| f.drive(id, controls)) { return Err(error("unknown vehicle")); }
        Ok(())
    })?)?;
    let c = core.clone();
    t.set("step", lua.create_function(move |_, dt: f64| {
        if !(0.0..=1.0).contains(&dt) { return Err(error("vehicles.step needs 0..1 seconds")); }
        let mut c = lock(&c);
        sim(&c, "vehicles.step")?;
        c.world.fleet.as_mut().ok_or_else(|| error("call vehicles.reset before step"))?.step(dt);
        Ok(())
    })?)?;
    t.set("get", lua.create_function(move |lua, id: u32| {
        let c = lock(&core);
        let Some(p) = c.world.fleet.as_ref().and_then(|f| f.pose(id)) else { return Ok(None) };
        let t = lua.create_table()?;
        t.set("pos", vec(p.pos))?; t.set("rot", vec(p.rot))?; t.set("speed", p.speed)?;
        t.set("gear", p.gear)?; t.set("rpm", p.rpm)?; t.set("torque", p.torque)?;
        t.set("slip", p.slip)?; t.set("cut", p.cut)?; t.set("parked", p.parked)?;
        let wheels = lua.create_table()?;
        for (i, (height, steer, spin)) in p.wheels.into_iter().enumerate() {
            wheels.set(i + 1, Vector::new(height as f32, steer as f32, spin as f32))?;
        }
        t.set("wheels", wheels)?;
        Ok(Some(t))
    })?)?;
    t.set_readonly(true);
    api.raw_set("vehicles", t)
}
