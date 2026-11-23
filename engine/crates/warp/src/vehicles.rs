//! The shared vehicle solver from Luau. The static collision set is captured at
//! reset so each wheel queries Rapier's spatial index instead of scanning the scene.
use crate::api::{Shared, lock, sim, vec3};
use physics::vehicle::{CarSpec, fleet::{Fleet, HullSection, Pedals}};
use script::mlua::{self, Lua, Table, Value, Vector};

fn error(msg: &str) -> mlua::Error { mlua::Error::runtime(msg) }
fn vec(v: glam::Vec3) -> Vector { Vector::new(v.x, v.y, v.z) }

pub(crate) fn read_dents(rows: Table) -> mlua::Result<Vec<kern::crush::Dent>> {
    if rows.raw_len()>kern::crush::MAX_DENTS { return Err(error("too many dent fields")); }
    rows.sequence_values::<Table>().map(|row| {
        let row=row?;
        let pos=vec3(&row.raw_get("center")?,"dent center")?;
        let axis=vec3(&row.raw_get("inward")?,"dent direction")?;
        kern::crush::Dent::new(pos,axis,row.raw_get("radius")?,row.raw_get("depth")?).ok_or_else(||error("invalid dent radius, direction or depth"))
    }).collect()
}

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
    t.set("contact_profile", lua.create_function(move |_, (name, bottom, list): (String, f32, Table)| {
        let mut c = lock(&c);
        sim(&c, "vehicles.contact_profile")?;
        if !(2..=20).contains(&list.raw_len()) { return Err(error("vehicles.contact_profile needs 2..20 sections")); }
        let mut sections = Vec::with_capacity(list.raw_len());
        for row in list.sequence_values::<Table>() {
            let row = row?;
            let width = row.raw_get("width")?;
            sections.push(HullSection { z: row.raw_get("z")?, width, top: row.raw_get("top")?,
                roof_width: row.raw_get::<Option<f32>>("roofWidth")?.unwrap_or(width) });
        }
        c.world.fleet.as_mut().ok_or_else(|| error("call vehicles.reset before contact_profile"))?
            .set_contact_profile(name, bottom, &sections).map_err(|e| error(&e))
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
    t.set("remove", lua.create_function(move |_, id: u32| {
        let mut c = lock(&c);
        sim(&c, "vehicles.remove")?;
        Ok(c.world.fleet.as_mut().is_some_and(|f| f.remove(id)))
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
    let c = core.clone();
    t.set("dent_query", lua.create_function(move |_, (id,name,list,body,mask): (u32,String,Table,Option<u32>,Option<u32>)| {
        let fields=read_dents(list)?;
        let mut c=lock(&c);sim(&c,"vehicles.dent_query")?;
        let mask=mask.unwrap_or(0);
        if let Some(body)=body && mask!=0 {
            return c.world.prepare_panel_query(id,&name,body,&fields,mask).map_err(|e|error(&e));
        }
        c.world.fleet.as_mut().ok_or_else(||error("call vehicles.reset first"))?.dent_query(id,&name,&fields).map_err(|e|error(&e))
    })?)?;
    let c = core.clone();
    t.set("crush_query", lua.create_function(move |_, (id, name, lo, hi, side, end): (u32, String, Value, Value, Value, Value)| {
        let lo=vec3(&lo,"low grid hinge")?;
        let hi=vec3(&hi,"high grid hinge")?;
        let side=vec3(&side,"side scales")?;
        let end=vec3(&end,"end scales")?;
        let mut c=lock(&c);
        sim(&c,"vehicles.crush_query")?;
        c.world.fleet.as_mut().ok_or_else(|| error("call vehicles.reset first"))?
            .crush_query(id, &name, kern::crush::Grid([lo.x,hi.x,lo.z,hi.z]), [side.x,side.z,end.x,end.z]).map_err(|e| error(&e))
    })?)?;
    let c = core.clone();
    t.set("raycast", lua.create_function(move |_, (id, name, o, d, limit): (u32, String, Value, Value, f32)| {
        let o = vec3(&o, "ray origin")?;
        let d = vec3(&d, "ray direction")?;
        if !o.is_finite() || !d.is_finite() || !limit.is_finite() || limit < 0. { return Err(error("vehicles.raycast needs a finite ray and nonnegative range")); }
        Ok(lock(&c).world.fleet.as_ref().and_then(|f| f.query_contact(id, &name)).and_then(|h| h.raycast(o, d, limit)))
    })?)?;
    let c = core.clone();
    t.set("closest_point", lua.create_function(move |_, (id, name, p): (u32, String, Value)| {
        let p = vec3(&p, "query point")?;
        if !p.is_finite() { return Err(error("vehicles.closest_point needs a finite point")); }
        Ok(lock(&c).world.fleet.as_ref().and_then(|f| f.query_contact(id, &name)).map(|h| vec(h.closest_point(p))))
    })?)?;
    let c = core.clone();
    t.set("impacts", lua.create_function(move |lua, ()| {
        let c = lock(&c);
        let out = lua.create_table()?;
        if let Some(fleet) = &c.world.fleet {
            for (&id, &(point, delta_v)) in fleet.impacts() {
                let row = lua.create_table()?;
                row.set("point", vec(point))?;
                row.set("deltaV", delta_v)?;
                out.raw_set(id, row)?;
            }
        }
        Ok(out)
    })?)?;
    t.set("get", lua.create_function(move |lua, id: u32| {
        let c = lock(&core);
        let Some(p) = c.world.fleet.as_ref().and_then(|f| f.pose(id)) else { return Ok(None) };
        let t = lua.create_table()?;
        t.set("pos", vec(p.pos))?; t.set("rot", vec(p.rot))?; t.set("speed", p.speed)?;
        t.set("gear", p.gear)?; t.set("rpm", p.rpm)?; t.set("torque", p.torque)?;
        t.set("load", p.road.load)?; t.set("slip", p.road.slip)?; t.set("contact", p.road.contact)?;
        t.set("loose", p.road.loose)?; t.set("soft", p.road.soft)?;
        t.set("cut", p.cut)?; t.set("parked", p.parked)?;
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
