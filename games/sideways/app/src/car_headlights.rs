//! Bounded car-only night lights: two physical front lamps per human car.
//! Visual light intensity is a tuning value, not a homologated beam model.
use glam::Vec3;
use render::{Lighting,headlights::{LocalLight,SpotLight}};
use crate::visuals::CarPose;

const MAX_CARS:usize=4;
// Lens surfaces are part of the imported body. Move just outside the lens,
// rather than moving every lamp ahead of the car's furthest bumper vertex.
const LENS_CLEARANCE_M:f32=0.025;
const PAIR_INTENSITY_CD:Vec3=Vec3::new(80_000.0,76_000.0,68_000.0);
// The shared renderer fades over the final 20%: 88..110 m. Intensity,
// angular footprint and the existing two shadow maps per car are unchanged.
const RANGE_M:f32=110.0;

/// Measured lens centres in imported body space, before the body's ride
/// translation. The asymmetry is present in the source meshes. Asti uses
/// the outer round headlights; MV6 uses the factory lamps (not its rally
/// pods); Toyama uses the two front glass covers. Z is the lens triangle
/// intersection at that X/Y, not the bounding box centre of a curved lens.
fn model_lenses(model:&str)->Option<[Vec3;2]> {
    match model.rsplit(['/', '\\']).next().unwrap_or("") {
        "asti_stradale.glb"=>Some([Vec3::new(-0.587173,0.342272,-1.889431),Vec3::new(0.591817,0.334694,-1.889431)]),
        "mv6.glb"=>Some([Vec3::new(-0.573250,0.376079,-1.701265),Vec3::new(0.573032,0.376079,-1.701265)]),
        "toyama_xr.glb"=>Some([Vec3::new(-0.595698,0.350809,-1.973669),Vec3::new(0.585233,0.350809,-1.973669)]),
        _=>None,
    }
}

fn local_emitters(p:&CarPose)->[Vec3;2] {
    let lenses=model_lenses(&p.model).unwrap_or_else(|| {
        // Unknown/custom models have no authored hardpoints yet. This is
        // an explicit collision-bound approximation, not a measured lens.
        let front=p.boxes.iter().map(|b|(b[5]-b[2]*0.5)as f32).fold(f32::INFINITY,f32::min);
        let front=if front.is_finite(){front}else{-1.8};
        let (centre,half_width,height)=p.boxes.first()
            .map(|b|(b[3]as f32,b[0]as f32*0.3,(b[4]as f32).clamp(0.2,0.8)))
            .unwrap_or((0.0,0.54,0.35));
        [Vec3::new(centre-half_width,height,front-0.1),Vec3::new(centre+half_width,height,front-0.1)]
    });
    lenses.map(|lens|lens+Vec3::new(0.0,p.ride,-LENS_CLEARANCE_M))
}

/// Keep the same order for emission and its metadata; invalid or duplicate
/// seat indices must not spend the four-car budget or duplicate brightness.
fn selected_cars<'a>(poses:&'a[CarPose],indices:&[usize])->[Option<(usize,&'a CarPose)>;MAX_CARS] {
    let mut selected=[None;MAX_CARS];let mut count=0;
    for &index in indices {
        let Some(pose)=poses.get(index) else {continue};
        if selected.iter().flatten().any(|(previous,_)|*previous==index){continue;}
        selected[count]=Some((index,pose));count+=1;
        if count==MAX_CARS {break;}
    }
    selected
}

pub fn active(light:&Lighting)->bool {
    !(light.sun_dir.y>5.0f32.to_radians().sin() && light.sun_lux>=5000.0)
}

pub fn from_poses(poses:&[CarPose],human_indices:&[usize],light:&Lighting)->Vec<SpotLight> {
    // Activate at twilight/night without raising ambient exposure or adding
    // scene lamps. Daytime has no headlight shadow-pass cost.
    if !active(light) {return Vec::new();}
    selected_cars(poses,human_indices).into_iter().flatten().flat_map(|(_,p)| {
        local_emitters(p).map(|local|SpotLight {position:p.pos+p.rot*local,
            direction:p.rot*Vec3::new(0.0,-0.045,-1.0).normalize(),up:p.rot*Vec3::Y,
            intensity_cd:PAIR_INTENSITY_CD*0.5,range_m:RANGE_M,
            inner_angle_radians:17.0f32.to_radians(),outer_angle_radians:32.0f32.to_radians()})
    }).collect()
}

/// Effective foot-brake amount and gear state, indexed by the original car
/// index. The caller applies automatic-reverse pedal mapping, just as it does
/// for the visible rear lenses. This adapter retains no light/event history.
#[derive(Clone,Copy,Debug,Default,PartialEq)]
pub struct RearLightState {pub brake:f32,pub reverse:bool}

#[derive(Clone,Copy)]
struct RearMounts {red:[Vec3;2],reverse:[Vec3;2]}

/// Points on the same imported triangles selected by car_model::rear_lens_role.
/// XY comes from each lens region; Z is its actual triangle intersection.
/// Atlas red/clear regions were identified from the original base-colour map.
/// Unknown models have no known emitting lenses, so they get no guessed spill.
fn model_rear_lenses(model:&str)->Option<RearMounts> {
    match model.rsplit(['/', '\\']).next().unwrap_or("") {
        "asti_stradale.glb"=>Some(RearMounts {
            red:[Vec3::new(-0.705000,0.320000,1.834089),Vec3::new(0.710000,0.310000,1.833807)],
            reverse:[Vec3::new(-0.602490,0.325074,1.855777),Vec3::new(0.606913,0.317301,1.855777)]}),
        "mv6.glb"=>Some(RearMounts {
            red:[Vec3::new(-0.720000,0.414000,1.856082),Vec3::new(0.720000,0.414000,1.855998)],
            reverse:[Vec3::new(-0.747866,0.584852,1.823039),Vec3::new(0.747649,0.584853,1.823039)]}),
        "toyama_xr.glb"=>Some(RearMounts {
            red:[Vec3::new(-0.668736,0.524098,2.032834),Vec3::new(0.658270,0.524098,2.032834)],
            reverse:[Vec3::new(-0.488722,0.411710,2.100906),Vec3::new(0.478256,0.411710,2.100906)]}),
        _=>None,
    }
}

const TAIL_INTENSITY_CD:f32=2.5;
const BRAKE_INTENSITY_CD:f32=32.0;
const REVERSE_PAIR_INTENSITY_CD:f32=100.0;
const TAIL_RANGE_M:f32=2.2;
const BRAKE_RANGE_M:f32=3.2;
const REVERSE_RANGE_M:f32=5.0;
const REAR_DOWN_RADIANS:f32=10.0*std::f32::consts::PI/180.0;
const REAR_OUTER_RADIANS:f32=78.0*std::f32::consts::PI/180.0;

/// Short, unshadowed direct spill: two red lenses and at most one merged
/// reverse-pair light per selected car. Four cars leave four of the sixteen
/// local-light slots available for actual exhaust flashes. No extra shadow
/// maps or GI are created. Finite rear-facing cones avoid illuminating ahead
/// through the car; nearby walls/terrain can still leak unshadowed light.
/// Intensities are art tuning in candela, not measured vehicle photometry.
pub fn local_from_poses(poses:&[CarPose],human_indices:&[usize],light:&Lighting,states:&[RearLightState])->Vec<LocalLight> {
    let tail=if active(light){TAIL_INTENSITY_CD}else{0.0};
    let mut result=Vec::with_capacity(MAX_CARS*3);
    for (index,p) in selected_cars(poses,human_indices).into_iter().flatten() {
        let Some(mounts)=model_rear_lenses(&p.model)else{continue};
        let state=states.get(index).copied().unwrap_or_default();
        let brake=if state.brake.is_finite()&&state.brake>0.02 {state.brake.clamp(0.0,1.0)}else{0.0};
        let red=tail+(BRAKE_INTENSITY_CD-tail)*brake;
        let direction=p.rot*Vec3::new(0.0,-REAR_DOWN_RADIANS.sin(),REAR_DOWN_RADIANS.cos());
        let make=|mount:Vec3,intensity_cd:Vec3,range_m:f32,source_radius_m:f32|LocalLight {
            position:p.pos+p.rot*(mount+Vec3::new(0.0,p.ride,LENS_CLEARANCE_M)),
            direction,intensity_cd,range_m,source_radius_m,
            inner_cos:40.0f32.to_radians().cos(),outer_cos:70.0f32.to_radians().cos(),
        };
        if red>0.0 {
            // A visible lamp is much brighter than its nearby spill. Preserve
            // a broad, soft rear wash rather than two headlight-like beams.
            let range=TAIL_RANGE_M+(BRAKE_RANGE_M-TAIL_RANGE_M)*brake;
            result.extend(mounts.red.map(|mount|LocalLight {
                inner_cos:30.0f32.to_radians().cos(),outer_cos:REAR_OUTER_RADIANS.cos(),
                ..make(mount,Vec3::new(1.0,0.008,0.003)*red,range,0.06)
            }));
        }
        if state.reverse {
            // One explicitly combined emitter bounds the cost of two clear
            // lenses. Its centre is between their measured source mounts.
            result.push(make((mounts.reverse[0]+mounts.reverse[1])*0.5,
                Vec3::new(1.0,0.94,0.82)*REVERSE_PAIR_INTENSITY_CD,REVERSE_RANGE_M,0.10));
        }
    }
    result
}

pub fn local_metadata()->serde_json::Value {serde_json::json!({
    "mode":"short unshadowed direct rear-lamp spill",
    "red_lamps_per_car":2,"reverse_lights_per_car":1,"reverse_mount":"merged pair midpoint",
    "tail_intensity_cd":TAIL_INTENSITY_CD,"brake_intensity_cd":BRAKE_INTENSITY_CD,
    "reverse_pair_intensity_cd":REVERSE_PAIR_INTENSITY_CD,"tail_range_m":TAIL_RANGE_M,"brake_range_m":BRAKE_RANGE_M,
    "red_cone_degrees":[30.0,78.0],"red_spill_policy":"faint tail wash, brighter brake wash; source lens emission is independent",
    "reverse_range_m":REVERSE_RANGE_M,"mount_source":"measured bundled rear-lens triangles; unknown models omitted",
    "limits":["no occlusion shadows or indirect bounce","nearby walls and terrain folds can leak light",
        "no atmospheric scattering","tuned intensity, not measured photometry"]})}
pub fn metadata(lights:&[SpotLight])->serde_json::Value {serde_json::json!({
    "mode":"two shadowed physical front lamps per human car","enabled":!lights.is_empty(),
    "lamps_per_car":2,"pair_intensity_cd":PAIR_INTENSITY_CD.to_array(),"lens_clearance_m":LENS_CLEARANCE_M,
    "origin_policy":"measured imported lens hardpoints for bundled models; collision-bound approximation for unknown models",
    "lights":lights.iter().enumerate().map(|(i,l)|serde_json::json!({"car_slot":i/2,"side":if i%2==0{"left"}else{"right"},"position":l.position.to_array(),"direction":l.direction.to_array(),
        "up":l.up.to_array(),"intensity_cd":l.intensity_cd.to_array(),"range_m":l.range_m,
        "range_fade_metres":[l.range_m*0.8,l.range_m],
        "inner_angle_radians":l.inner_angle_radians,"outer_angle_radians":l.outer_angle_radians})).collect::<Vec<_>>(),
    "shadow_size":render::headlights::SHADOW_SIZE,
    "limits":["four human cars maximum; eight real spot lights; no AI lamps","unknown models use approximate collision-bound mounts","existing low-grass no-shadow policy unchanged","no atmospheric beam volume or lens-emitter glow"]})}

/// Pose-aware capture metadata records fallback use for each actual lamp.
pub fn metadata_for_poses(lights:&[SpotLight],poses:&[CarPose],human_indices:&[usize])->serde_json::Value {
    let mut value=metadata(lights);
    let selected=selected_cars(poses,human_indices);
    if let Some(entries)=value["lights"].as_array_mut() {
        for (i,entry) in entries.iter_mut().enumerate() {
            if let Some((car,p))=selected.get(i/2).copied().flatten() {
                entry["car_index"]=serde_json::json!(car);
                entry["model"]=serde_json::json!(p.model);
                entry["mount_source"]=serde_json::json!(if model_lenses(&p.model).is_some(){"imported_lens_surface"}else{"collision_bound_fallback"});
                entry["body_local_origin_with_ride"]=serde_json::json!(local_emitters(p)[i%2].to_array());
            }
        }
    }
    value
}

#[cfg(test)]mod tests {
    use super::*;use glam::Quat;
    fn pose()->CarPose {CarPose {pos:Vec3::new(1.0,2.0,3.0),rot:Quat::IDENTITY,color:Vec3::ONE,
        boxes:vec![[1.8,0.6,4.0,0.0,0.4,0.0]],model:String::new(),ride:0.03,wheels:vec![]}}
    fn night()->Lighting {Lighting {sun_lux:0.0,sun_dir:Vec3::NEG_Y,..Default::default()}}
    #[test]fn lamp_pair_follows_full_body_pose_and_applies_ride_once() {
        for model in ["asti_stradale.glb","mv6.glb","toyama_xr.glb"] {
            let mut p=pose();p.model=format!("res://assets/cars/{model}");p.ride=0.17;
            let a=from_poses(&[p],&[0],&night());let lens=model_lenses(model).unwrap();
            assert_eq!(a.len(),2);assert!(a[0].position.x<a[1].position.x);
            let q=Quat::from_euler(glam::EulerRot::YXZ,0.7,0.2,-0.15);let mut p=pose();p.model=model.into();p.rot=q;p.ride=0.17;
            let b=from_poses(&[p],&[0],&night());
            for i in 0..2 {
                assert_eq!(a[i].range_m,110.0);
                assert_eq!(a[i].inner_angle_radians,17.0f32.to_radians());
                assert_eq!(a[i].outer_angle_radians,32.0f32.to_radians());
                let local=lens[i]+Vec3::new(0.0,0.17,-LENS_CLEARANCE_M);
                assert!(a[i].position.abs_diff_eq(Vec3::new(1.0,2.0,3.0)+local,1e-6));
                assert!(b[i].position.abs_diff_eq(Vec3::new(1.0,2.0,3.0)+q*local,1e-6));
                assert!(b[i].direction.abs_diff_eq(q*a[i].direction,1e-6));assert!(b[i].up.abs_diff_eq(q*Vec3::Y,1e-6));
            }
            assert!((a[0].intensity_cd+a[1].intensity_cd).abs_diff_eq(PAIR_INTENSITY_CD,0.01));
        }
    }
    #[test]fn four_human_cars_keep_two_lamps_each_and_skip_invalid_or_duplicate_indices() {
        assert!(render::headlights::MAX_LIGHTS>=MAX_CARS*2);
        assert!(from_poses(&[pose()],&[0],&Lighting::default()).is_empty());
        let poses:Vec<_>=(0..7).map(|i|{let mut p=pose();p.pos.x+=i as f32*10.0;p}).collect();
        let indices=[999,3,3,1,6,2,0];let lamps=from_poses(&poses,&indices,&night());
        assert_eq!(lamps.len(),8);
        let meta=metadata_for_poses(&lamps,&poses,&indices);
        for (pair,car) in [3,1,6,2].into_iter().enumerate() {
            assert_eq!(meta["lights"][pair*2]["car_index"],car);
            assert_eq!(meta["lights"][pair*2+1]["car_index"],car);
            assert!((lamps[pair*2].intensity_cd+lamps[pair*2+1].intensity_cd).abs_diff_eq(PAIR_INTENSITY_CD,0.01));
        }
    }
    #[test]fn unknown_models_use_two_explicit_fallback_mounts() {
        let mut p=pose();p.model="res://custom/new-car.glb".into();
        let local=local_emitters(&p);assert!(local[0].x<0.0&&local[1].x>0.0);
        assert!(local.iter().all(|v|v.z< -2.0 && (v.y-0.43).abs()<1e-6));
        let poses=[p];let lamps=from_poses(&poses,&[0],&night());let meta=metadata_for_poses(&lamps,&poses,&[0]);
        assert_eq!(meta["lights"][0]["mount_source"],"collision_bound_fallback");
        assert_eq!(meta["lights"][1]["side"],"right");
        let mut p=pose();p.boxes.clear();assert!(local_emitters(&p).iter().all(|v|v.is_finite()));
    }
    fn rear_pose()->CarPose {let mut p=pose();p.model="asti_stradale.glb".into();p}
    #[test]fn rear_spill_follows_current_brake_reverse_and_tail_without_lingering() {
        let poses=[rear_pose()];let day=Lighting::default();
        let lamps=|state,light:&Lighting|local_from_poses(&poses,&[0],light,&[state]);
        assert!(lamps(RearLightState::default(),&day).is_empty());
        let brake=lamps(RearLightState{brake:1.0,reverse:false},&day);
        assert_eq!(brake.len(),2);assert_eq!(brake[0].intensity_cd.x,BRAKE_INTENSITY_CD);
        assert_eq!(brake[0].range_m,BRAKE_RANGE_M);
        let half=lamps(RearLightState{brake:0.5,reverse:false},&day);
        assert_eq!(half[0].intensity_cd.x,BRAKE_INTENSITY_CD*0.5);
        let reverse=lamps(RearLightState{brake:0.0,reverse:true},&day);
        assert_eq!(reverse.len(),1);assert_eq!(reverse[0].range_m,REVERSE_RANGE_M);
        let tail=lamps(RearLightState::default(),&night());
        assert_eq!(tail[0].intensity_cd.x,TAIL_INTENSITY_CD);
        assert_eq!(tail[0].range_m,TAIL_RANGE_M);
        assert!(tail[0].range_m<brake[0].range_m);
        for invalid in [f32::NAN,f32::INFINITY,-1.0,0.02] {
            assert!(lamps(RearLightState{brake:invalid,reverse:false},&day).is_empty());
        }
        assert_eq!(lamps(RearLightState{brake:2.0,reverse:false},&day)[0].intensity_cd.x,BRAKE_INTENSITY_CD);
        assert!(lamps(RearLightState::default(),&day).is_empty(),"release must clear the next upload");
    }
    #[test]fn rear_spill_applies_full_body_pose_ride_and_rearward_cone_once() {
        let mut p=rear_pose();p.ride=0.19;
        let local=local_from_poses(&[p],&[0],&night(),&[RearLightState{brake:1.0,reverse:true}]);
        let q=Quat::from_euler(glam::EulerRot::YXZ,1.2,-0.31,0.2);
        let mut p=rear_pose();p.ride=0.19;p.rot=q;
        let world=local_from_poses(&[p],&[0],&night(),&[RearLightState{brake:1.0,reverse:true}]);
        let mounts=model_rear_lenses("asti_stradale.glb").unwrap();
        let origins=[mounts.red[0],mounts.red[1],(mounts.reverse[0]+mounts.reverse[1])*0.5];
        assert!(REAR_OUTER_RADIANS+REAR_DOWN_RADIANS<std::f32::consts::FRAC_PI_2,
            "the whole cone must remain behind the car, not just its centre");
        for i in 0..3 {
            let offset=origins[i]+Vec3::new(0.0,0.19,LENS_CLEARANCE_M);
            assert!(local[i].position.abs_diff_eq(Vec3::new(1.0,2.0,3.0)+offset,1e-6));
            assert!(world[i].position.abs_diff_eq(Vec3::new(1.0,2.0,3.0)+q*offset,1e-6));
            assert!(world[i].direction.abs_diff_eq(q*local[i].direction,1e-6));
            assert!((world[i].direction.length()-1.0).abs()<1e-6);
            assert!(world[i].source_radius_m>0.0&&world[i].range_m<=5.0);
        }
    }
    #[test]fn rear_spill_uses_original_car_indices_and_preserves_flash_budget() {
        let poses:Vec<_>=(0..6).map(|i|{let mut p=rear_pose();p.pos.x+=10.0*i as f32;p}).collect();
        let states=vec![RearLightState{brake:1.0,reverse:true};6];
        let lamps=local_from_poses(&poses,&[999,4,4,1,5,2,0],&night(),&states);
        assert_eq!(lamps.len(),12);assert!(lamps.len()+MAX_CARS<=16);
        for (slot,car) in [4,1,5,2].into_iter().enumerate() {
            assert!((lamps[slot*3].position.x-poses[car].pos.x).abs()<1.0);
        }
        let mut states=vec![RearLightState::default();6];states[4].reverse=true;
        assert_eq!(local_from_poses(&poses,&[4,1],&Lighting::default(),&states).len(),1);
        assert!(local_from_poses(&poses,&[4,1],&Lighting::default(),&[]).is_empty());
        assert!(local_from_poses(&[pose()],&[0],&night(),&states).is_empty(),"unknown lenses are not guessed");
    }
    #[test]fn bundled_rear_mounts_are_on_the_original_rear_lens_triangles() {
        for model in ["asti_stradale","mv6","toyama_xr"] {
            let bytes=std::fs::read(sideways::game_assets::asset("cars").join(format!("{model}.glb"))).unwrap();
            let g=gltf::Gltf::from_slice_without_validation(&bytes).unwrap();let blob=g.blob.as_deref().unwrap();
            let body=g.nodes().find(|n|n.name().is_some_and(|name|name.starts_with("body"))).unwrap();
            let(t,r,s)=body.transform().decomposed();
            let transform=glam::Affine3A::from_scale_rotation_translation(Vec3::from(s),Quat::from_array(r),Vec3::from(t));
            let mounts=model_rear_lenses(&format!("{model}.glb")).unwrap();
            for (reverse,lenses) in [(false,mounts.red),(true,mounts.reverse)] {
                let material=match (model,reverse) {
                    ("asti_stradale",_)=>"UCB_Lights_and_Glass.003",
                    ("mv6",_)=>"UCB_Lights_and_Glass.002",
                    ("toyama_xr",false)=>"Body_Glass_Red.001",
                    ("toyama_xr",true)=>"Body_Glass.001",_=>unreachable!(),
                };
                let mesh=body.mesh().unwrap();let prim=mesh.primitives().find(|p|p.material().name()==Some(material)).unwrap();
                let reader=prim.reader(|_|Some(blob));
                let vertices:Vec<_>=reader.read_positions().unwrap().map(|p|transform.transform_point3(Vec3::from(p))).collect();
                let indices:Vec<_>=reader.read_indices().unwrap().into_u32().collect();
                for lens in lenses {
                    let on_lens=indices.chunks_exact(3).any(|tri|{
                        let [a,b,c]=[vertices[tri[0]as usize],vertices[tri[1]as usize],vertices[tri[2]as usize]];
                        if [a,b,c].iter().any(|p|p.z<1.65||!(0.24..=0.65).contains(&p.y)){return false;}
                        let d=(b.y-c.y)*(a.x-c.x)+(c.x-b.x)*(a.y-c.y);
                        if d.abs()<1e-10{return false;}
                        let u=((b.y-c.y)*(lens.x-c.x)+(c.x-b.x)*(lens.y-c.y))/d;
                        let v=((c.y-a.y)*(lens.x-c.x)+(a.x-c.x)*(lens.y-c.y))/d;let w=1.0-u-v;
                        u.min(v).min(w)>=-1e-4&&(u*a.z+v*b.z+w*c.z-lens.z).abs()<0.001
                    });
                    assert!(on_lens,"{model}: {lens:?} is no longer on its imported rear lens");
                }
            }
        }
    }
    #[test]fn bundled_mounts_are_on_actual_imported_lens_triangles() {
        // This reads the same GLB node transforms as car_model::load. A model
        // re-export that moves/scales the lenses must update the hardpoints.
        for (model,material) in [("asti_stradale.glb","UCB_Lights_and_Glass.003"),("mv6.glb","UCB_Lights_and_Glass_Transperent.002"),("toyama_xr.glb","Body_Glass.001")] {
            let path=sideways::game_assets::asset("cars").join(model);
            let bytes=std::fs::read(path).unwrap();let g=gltf::Gltf::from_slice_without_validation(&bytes).unwrap();let blob=g.blob.as_deref().unwrap();
            let body=g.nodes().find(|n|n.name().is_some_and(|name|name.starts_with("body"))).unwrap();
            let (t,r,s)=body.transform().decomposed();let transform=glam::Affine3A::from_scale_rotation_translation(Vec3::from(s),Quat::from_array(r),Vec3::from(t));
            let mesh=body.mesh().unwrap();let primitive=mesh.primitives().find(|p|p.material().name()==Some(material)).unwrap();let reader=primitive.reader(|_|Some(blob));
            let vertices:Vec<_>=reader.read_positions().unwrap().map(|p|transform.transform_point3(Vec3::from(p))).collect();let indices:Vec<_>=reader.read_indices().unwrap().into_u32().collect();
            for lens in model_lenses(model).unwrap() {
                let on_lens=indices.chunks_exact(3).any(|tri|{
                    let [a,b,c]=[vertices[tri[0]as usize],vertices[tri[1]as usize],vertices[tri[2]as usize]];
                    let denominator=(b.y-c.y)*(a.x-c.x)+(c.x-b.x)*(a.y-c.y);
                    if denominator.abs()<1e-10{return false;}
                    let u=((b.y-c.y)*(lens.x-c.x)+(c.x-b.x)*(lens.y-c.y))/denominator;
                    let v=((c.y-a.y)*(lens.x-c.x)+(a.x-c.x)*(lens.y-c.y))/denominator;let w=1.0-u-v;
                    u.min(v).min(w)>=-1e-4 && (u*a.z+v*b.z+w*c.z-lens.z).abs()<0.001
                });
                assert!(on_lens,"{model}: {lens:?} is no longer on the imported lens");
            }
        }
    }
}
