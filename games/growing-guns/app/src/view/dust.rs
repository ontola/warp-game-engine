//! Grounded blast dust. A small number of spawn-time rays avoids dust over
//! lava or flying through nearby walls; there is no per-frame CPU simulation.
use glam::Vec3;
use growing_guns::{World,shape};
pub(super) fn emit(fx:&mut render::fx::Effects,world:&World,time:f32,pos:Vec3,radius:f32) {
    if radius<1.5 {return;}
    let count=((radius*14.0) as usize).clamp(24,100).min(fx.dust_capacity(time));
    if count==0 {return;}
    let mut rng=0x9e3779b9u32 ^ pos.x.to_bits() ^ pos.z.to_bits().rotate_left(9) ^ time.to_bits();
    let mut random=|| {rng^=rng<<13;rng^=rng>>17;rng^=rng<<5;(rng as f64/u32::MAX as f64) as f32};
    let phase=(pos.x*3.17+pos.z*1.29+time*2.3).sin()*3.0;
    let reach=radius.min(12.0);
    for _ in 0..count {
        let angle=phase+random()*std::f32::consts::TAU;
        let dir=Vec3::new(angle.cos(),0.0,angle.sin());
        let start=pos+dir*reach*(0.3+random()*0.9);
        let travel=dir*reach*(0.7+random()*1.2);
        let ground=|at:Vec3| {
            let from=at+Vec3::Y*0.5;
            let (d,n,_)=shape::raycast(&world.arena.grid,from,Vec3::NEG_Y,reach*0.65+1.0)?;
            let y=from.y-d;
            (n.y>0.7 && y>world.arena.lava_surface()+0.15).then_some(y)
        };
        let (Some(y),Some(end_y))=(ground(start),ground(start+travel)) else {continue;};
        if (y-end_y).abs()>0.5 {continue;}
        let at=Vec3::new(start.x,y+0.55,start.z);
        if shape::raycast(&world.arena.grid,at,dir,travel.length()).is_some() {continue;}
        fx.ground_dust(time,at,travel,(reach*0.3).clamp(0.8,2.8)*(0.65+random()*0.5));
    }
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn dust_spawns_on_ground_but_not_in_air() {
        let world=World::new(3,&[false]);
        let mut fx=render::fx::Effects::default();
        emit(&mut fx,&world,0.0,Vec3::new(0.0,1.0,0.0),7.0);
        assert!(fx.len()>0,"ground explosion must raise dust");
        let count=fx.len();
        emit(&mut fx,&world,0.0,Vec3::new(0.0,100.0,0.0),7.0);
        assert_eq!(fx.len(),count);
    }
}
