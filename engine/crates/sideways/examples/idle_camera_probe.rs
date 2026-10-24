//! CPU-only raw pose noise diagnosis; no controller, renderer or window.
use sideways::{race::{Race, Settings, Control}, spec::Cars};
fn main() {
    for brake in [0.0,1.0] {
        let mut race=Race::new(1,Settings{bots:1,rivals:0,stage_km:3.0,..Default::default()},Cars::load(None).unwrap());
        let mut poses=Vec::new();
        for tick in 0..2400 {
            race.step_with_inputs(&mut kern::Timings::default(),&[Some(Control{brake,..Default::default()})]);
            if tick>=1800 {
                let car=&race.players[0].car;let pose=car.pose(&race.phys);let f=pose.rotation*rapier3d::glamx::Vec3::NEG_Z;
                let p=pose.translation;
                poses.push([p.x,p.y,p.z,f.x.atan2(-f.z),car.linvel(&race.phys).length(),car.angvel(&race.phys).length()]);
            }
        }
        let lo:Vec<_>=(0..6).map(|i|poses.iter().map(|p|p[i]).fold(f32::INFINITY,f32::min)).collect();
        let hi:Vec<_>=(0..6).map(|i|poses.iter().map(|p|p[i]).fold(f32::NEG_INFINITY,f32::max)).collect();
        println!("brake={brake} lo={lo:?} hi={hi:?} delta={:?}",(0..6).map(|i|hi[i]-lo[i]).collect::<Vec<_>>());
    }
}
