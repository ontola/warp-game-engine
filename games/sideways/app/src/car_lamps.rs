//! Visible original rear lenses. This does not add spotlights, geometry,
//! shadow passes or physics state. Radiance is an art tuning value in cd/m2,
//! not a measured or homologated lamp photometry model.
use crate::car_look::{LampRole,Slot};
use render::{Lighting,scene::GpuCarMaterial};
use sideways::race::Race;

const TAIL_RADIANCE:f32=1400.0;
const BRAKE_RADIANCE:f32=5500.0;
const REVERSE_RADIANCE:f32=2600.0;
const TAIL_CORE_RADIANCE:f32=30000.0;
const BRAKE_CORE_RADIANCE:f32=60000.0;

/// Authored body-local core centres/radii on the measured red-lens regions.
/// The body instance already includes ride height; these coordinates do not.
/// Lamp-only spare fields carry generic ellipses, not model IDs, to the shader.
fn core_profile(model:&str)->([f32;4],[f32;4]) {
    match model.rsplit(['/', '\\']).next().unwrap_or("").trim_end_matches(".glb") {
        "asti_stradale"=>([-0.705,0.320,0.710,0.310],[0.050,0.038,0.14,0.0]),
        "mv6"=>([-0.720,0.414,0.720,0.414],[0.045,0.024,0.09,0.0]),
        "toyama_xr"=>([-0.668736,0.524098,0.658270,0.524098],[0.090,0.060,0.447,0.0]),
        _=>([0.0;4],[0.0;4]),
    }
}

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
        let core=if self.brake {BRAKE_CORE_RADIANCE}else if self.tail {TAIL_CORE_RADIANCE}else{0.0};
        [if role==LampRole::Reverse{0.0}else{red},
            if self.reverse&&role!=LampRole::Red{REVERSE_RADIANCE}else{0.0},
            if role==LampRole::Reverse{0.0}else{core},role as u32 as f32]
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
        let (centres,profile)=core_profile(&slot.model);
        if m.second!=next {m.second=next;changed=true;}
        if m.flake!=centres {m.flake=centres;changed=true;}
        if m.wear!=profile {m.wear=profile;changed=true;}
    }
    Ok(changed)
}

pub fn metadata(states:&[State],slots:&[Slot])->serde_json::Value {serde_json::json!({
    "mode":"emission on original imported rear-lens triangles",
    "tail_radiance_cd_m2":TAIL_RADIANCE,"brake_radiance_cd_m2":BRAKE_RADIANCE,"reverse_radiance_cd_m2":REVERSE_RADIANCE,
    "core":{"tail_radiance_cd_m2":TAIL_CORE_RADIANCE,"brake_radiance_cd_m2":BRAKE_CORE_RADIANCE,
        "channel_cap":60000.0,"appearance":"authored body-local red-lens cores modulated by source texture; existing AgX desaturates bright highlights"},
    "cars":states.iter().enumerate().map(|(car,s)|serde_json::json!({"car":car,"tail":s.tail,"brake":s.brake,"reverse":s.reverse,
        "lens_slots":slots.iter().filter(|slot|slot.car==car&&LampRole::from_part(&slot.part).is_some()).map(|slot|serde_json::json!({"model":slot.model,"role":slot.part})).collect::<Vec<_>>()
    })).collect::<Vec<_>>(),
    "limits":["this material emits only on its lens; environment spill is controlled separately","this material does not add bloom or change exposure; display bloom is a separate renderer effect","authored bundled GLB lens mappings only; unknown models retain texture-only emission","rear atlas colour masks preserve unlit amber sections and dark surrounds","tuned peak radiance, not measured vehicle photometry"]})}

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
        assert_eq!(s.levels(LampRole::Red),[BRAKE_RADIANCE,0.0,BRAKE_CORE_RADIANCE,1.0]);
        assert_eq!(s.levels(LampRole::Reverse),[0.0,REVERSE_RADIANCE,0.0,2.0]);
    }
    #[test]fn core_profiles_preserve_material_and_are_stable_across_lamp_states() {
        let slots=vec![slot(0,LampRole::Red)];let mut original=GpuCarMaterial::default();
        original.colour=[0.4,0.2,0.1,7.0];original.surface=[0.3,0.0,0.0,0.04];original.body=[-0.2,1.0,-2.0,2.0];
        let mut materials=vec![original];
        assert!(apply(&mut materials,&slots,&[State{tail:true,..Default::default()}]).unwrap());
        let profile=core_profile("asti_stradale");
        assert_eq!((materials[0].flake,materials[0].wear),profile);
        assert_eq!(materials[0].colour,original.colour);assert_eq!(materials[0].surface,original.surface);
        assert_eq!(materials[0].body,original.body);
        assert!(!apply(&mut materials,&slots,&[State{tail:true,..Default::default()}]).unwrap());
        assert!(apply(&mut materials,&slots,&[State{brake:true,..Default::default()}]).unwrap());
        assert_eq!((materials[0].flake,materials[0].wear),profile);
        assert!(apply(&mut materials,&slots,&[State::default()]).unwrap());
        assert_eq!(&materials[0].second[..3],&[0.0;3]);
        for model in ["asti_stradale","mv6","toyama_xr"] {
            let (c,p)=core_profile(model);assert!(c[0]<0.0&&c[2]>0.0);
            assert!(p[0]>0.0&&p[1]>0.0&&p[2]>0.0);
            assert!(c.iter().chain(p.iter()).all(|v|v.is_finite()));
        }
        assert_eq!(core_profile("unknown"),([0.0;4],[0.0;4]));
    }
    #[test]fn authored_core_centres_hit_bright_red_original_lens_texels() {
        // A geometric lens centroid can lie on amber glass or dark trim. Check
        // the original tagged material and UVs, not just its bounding box.
        use glam::{Affine3A,Quat,Vec2,Vec3};
        for (model,material) in [("asti_stradale","UCB_Lights_and_Glass.003"),
            ("mv6","UCB_Lights_and_Glass.002"),("toyama_xr","Body_Glass_Red.001")] {
            let bytes=std::fs::read(sideways::game_assets::asset("cars").join(format!("{model}.glb"))).unwrap();
            let g=gltf::Gltf::from_slice_without_validation(&bytes).unwrap();let blob=g.blob.as_deref().unwrap();
            let body=g.nodes().find(|n|n.name().is_some_and(|n|n.starts_with("body"))).unwrap();
            let (t,r,s)=body.transform().decomposed();
            let transform=Affine3A::from_scale_rotation_translation(Vec3::from(s),Quat::from_array(r),Vec3::from(t));
            let mesh=body.mesh().unwrap();let primitive=mesh.primitives().find(|p|p.material().name()==Some(material)).unwrap();
            let reader=primitive.reader(|_|Some(blob));
            let vertices:Vec<_>=reader.read_positions().unwrap().map(|p|transform.transform_point3(Vec3::from(p))).collect();
            let uvs:Vec<_>=reader.read_tex_coords(0).map(|u|u.into_f32().map(Vec2::from).collect()).unwrap_or_default();
            let indices:Vec<_>=reader.read_indices().unwrap().into_u32().collect();
            let mat=primitive.material();let pbr=mat.pbr_metallic_roughness();let factor=pbr.base_color_factor();
            let texture=pbr.base_color_texture().map(|info| {
                let tex=info.texture();let image_index=tex.extension_value("EXT_texture_webp")
                    .and_then(|v|v.get("source")).and_then(|v|v.as_u64()).map(|v|v as usize)
                    .unwrap_or_else(||tex.source().expect("rear lens texture image").index());
                let image=g.images().nth(image_index).unwrap();
                let gltf::image::Source::View{view,..}=image.source() else {panic!("embedded car image required")};
                image::load_from_memory(&blob[view.offset()..view.offset()+view.length()]).unwrap().to_rgb8()
            });
            let (centres,profile)=core_profile(model);
            for xy in centres.chunks_exact(2) {
                let point=Vec2::new(xy[0],xy[1]);let mut nearest:Option<(f32,Vec2)>=None;
                for tri in indices.chunks_exact(3) {
                    let [a,b,c]=[vertices[tri[0]as usize],vertices[tri[1]as usize],vertices[tri[2]as usize]];
                    if [a,b,c].iter().any(|p|p.z<1.65||!(0.24..=0.65).contains(&p.y)){continue;}
                    let d=(b.y-c.y)*(a.x-c.x)+(c.x-b.x)*(a.y-c.y);if d.abs()<1e-10{continue;}
                    let u=((b.y-c.y)*(point.x-c.x)+(c.x-b.x)*(point.y-c.y))/d;
                    let v=((c.y-a.y)*(point.x-c.x)+(a.x-c.x)*(point.y-c.y))/d;let w=1.0-u-v;
                    if u.min(v).min(w)< -1e-5{continue;}
                    let z=u*a.z+v*b.z+w*c.z;
                    let uv=if texture.is_some(){uvs[tri[0]as usize]*u+uvs[tri[1]as usize]*v+uvs[tri[2]as usize]*w}else{Vec2::ZERO};
                    if nearest.is_none_or(|(old,_)|z>old){nearest=Some((z,uv));}
                }
                let (_,uv)=nearest.expect("authored core must intersect its tagged rear lens");
                let mut rgb=Vec3::new(factor[0],factor[1],factor[2]);
                if let Some(image)=&texture {
                    let x=((uv.x.rem_euclid(1.0)*image.width()as f32)as u32).min(image.width()-1);
                    let y=((uv.y.rem_euclid(1.0)*image.height()as f32)as u32).min(image.height()-1);
                    let linear=image.get_pixel(x,y).0.map(|v|{let s=v as f32/255.0;if s<=0.04045{s/12.92}else{((s+0.055)/1.055).powf(2.4)}});
                    rgb*=Vec3::from(linear);
                }
                assert!(rgb.x>=profile[2]*0.9,"{model} core at {point:?} is dark: {rgb:?}");
                assert!(rgb.y/rgb.x<0.08&&rgb.z/rgb.x<0.10,"{model} core at {point:?} is not inside the red source mask: {rgb:?}");
            }
        }
    }
}
