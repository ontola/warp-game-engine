//! Visible original rear lenses. This does not add spotlights, geometry,
//! shadow passes or physics state. Radiance is an art tuning value in cd/m2,
//! not a measured or homologated lamp photometry model.
use crate::car_look::{LampRole,Slot};
use render::{Lighting,scene::GpuCarMaterial};
use sideways::race::Race;

const TAIL_RADIANCE:f32=600.0;
const BRAKE_RADIANCE:f32=4000.0;
const REVERSE_RADIANCE:f32=1800.0;

#[derive(Clone,Copy,Debug,Default,PartialEq,Eq)]
pub struct State {pub tail:bool,pub brake:bool,pub reverse:bool}
impl State {
    fn controls(tail:bool,gear:i32,manual:bool,hold:bool,brake:f64,throttle:f64)->Self {
        // Exactly the foot-brake choice in Car::drive: in automatic reverse
        // LT powers reverse and RT brakes. The handbrake alone is not a pedal.
        let foot=if hold {1.0}else if gear>0||manual {brake}else{throttle};
        Self {tail,brake:foot.is_finite()&&foot>0.02,reverse:gear<0}
    }
    fn levels(self,role:LampRole)->[f32;4] {
        let red=if self.brake {BRAKE_RADIANCE}else if self.tail {TAIL_RADIANCE}else{0.0};
        [if role==LampRole::Reverse{0.0}else{red},
            if self.reverse&&role!=LampRole::Red{REVERSE_RADIANCE}else{0.0},0.0,role as u32 as f32]
    }
}

pub fn from_race(race:&Race,light:&Lighting,count:usize)->Vec<State> {
    let tail=crate::car_headlights::active(light);
    (0..count).map(|i|race.players.get(i).or_else(||race.players.first()).map_or(State::default(),|p|{
        let c=&p.car;State::controls(tail,c.gear,c.manual,c.hold,c.brake,c.throttle)
    })).collect()
}

/// Slots belong to individual cars even when their GLB and texture are shared.
/// Only a state change needs an upload; steady cruising writes no material data.
pub fn apply(materials:&mut[GpuCarMaterial],slots:&[Slot],states:&[State])->Result<bool,String> {
    if materials.len()!=slots.len(){return Err("rear lamp material/slot count mismatch".into());}
    let mut changed=false;
    for (m,slot) in materials.iter_mut().zip(slots) {
        let Some(role)=LampRole::from_part(&slot.part)else{continue};
        let next=states.get(slot.car).copied().unwrap_or_default().levels(role);
        if m.second!=next {m.second=next;changed=true;}
    }
    Ok(changed)
}

pub fn metadata(states:&[State],slots:&[Slot])->serde_json::Value {serde_json::json!({
    "mode":"emission on original imported rear-lens triangles",
    "tail_radiance_cd_m2":TAIL_RADIANCE,"brake_radiance_cd_m2":BRAKE_RADIANCE,"reverse_radiance_cd_m2":REVERSE_RADIANCE,
    "cars":states.iter().enumerate().map(|(car,s)|serde_json::json!({"car":car,"tail":s.tail,"brake":s.brake,"reverse":s.reverse,
        "lens_slots":slots.iter().filter(|slot|slot.car==car&&LampRole::from_part(&slot.part).is_some()).map(|slot|serde_json::json!({"model":slot.model,"role":slot.part})).collect::<Vec<_>>()
    })).collect::<Vec<_>>(),
    "limits":["emissive lenses only; no rear ground lighting or bloom","authored bundled GLB lens mappings only; unknown models stay unchanged","rear atlas colour masks preserve unlit amber sections and dark surrounds","tuned peak radiance, not measured vehicle photometry"]})}

#[cfg(test)]mod tests {
    use super::*;
    #[test]fn brake_follows_effective_pedal_in_forward_manual_reverse_and_hold() {
        assert!(State::controls(false,1,false,false,1.0,0.0).brake);
        let reversing=State::controls(false,-1,false,false,1.0,0.0);
        assert!(reversing.reverse);assert!(!reversing.brake);
        assert!(State::controls(false,-1,false,false,0.0,1.0).brake);
        assert!(State::controls(false,-1,true,false,1.0,0.0).brake);
        assert!(!State::controls(false,-1,true,false,0.0,1.0).brake);
        assert!(State::controls(false,1,false,true,0.0,0.0).brake);
        assert!(!State::controls(false,1,false,false,f64::NAN,0.0).brake);
    }
    fn slot(car:usize,role:LampRole)->Slot {Slot {car,model:"asti_stradale".into(),game_colour:[1.0;3],
        part:role.part().into(),gltf:[1.0,1.0,1.0,0.3,0.0],body:[0.0;4],axles:[0.0;4]}}
    #[test]fn four_same_model_cars_keep_independent_brake_reverse_and_tail() {
        let slots:Vec<_>=(0..4).map(|i|slot(i,LampRole::RearAtlas)).collect();
        let states=[State::default(),State{tail:true,..Default::default()},State{brake:true,..Default::default()},State{reverse:true,..Default::default()}];
        let mut materials=vec![GpuCarMaterial::default();4];
        assert!(apply(&mut materials,&slots,&states).unwrap());
        assert_eq!(materials[0].second,[0.0;4]);
        assert_eq!(materials[1].second[0],TAIL_RADIANCE);
        assert_eq!(materials[2].second[0],BRAKE_RADIANCE);
        assert_eq!(materials[3].second[1],REVERSE_RADIANCE);
        assert!(!apply(&mut materials,&slots,&states).unwrap());
        assert!(apply(&mut materials,&slots,&[State::default();4]).unwrap());
        assert!(materials.iter().all(|m|m.second==[0.0;4]));
    }
    #[test]fn red_and_clear_lenses_do_not_cross_emit() {
        let s=State{tail:true,brake:true,reverse:true};
        assert_eq!(s.levels(LampRole::Red),[BRAKE_RADIANCE,0.0,0.0,1.0]);
        assert_eq!(s.levels(LampRole::Reverse),[0.0,REVERSE_RADIANCE,0.0,2.0]);
    }
}
