//! Bounded car-only night lights: two physical front lamps per human car.
//! Visual light intensity is a tuning value, not a homologated beam model.
use glam::Vec3;
use render::{Lighting,headlights::SpotLight};
use crate::visuals::CarPose;

const MAX_CARS:usize=4;
// Lens surfaces are part of the imported body. Move just outside the lens,
// rather than moving every lamp ahead of the car's furthest bumper vertex.
const LENS_CLEARANCE_M:f32=0.025;
const PAIR_INTENSITY_CD:Vec3=Vec3::new(80_000.0,76_000.0,68_000.0);

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
            intensity_cd:PAIR_INTENSITY_CD*0.5,range_m:90.0,
            inner_angle_radians:17.0f32.to_radians(),outer_angle_radians:32.0f32.to_radians()})
    }).collect()
}
pub fn metadata(lights:&[SpotLight])->serde_json::Value {serde_json::json!({
    "mode":"two shadowed physical front lamps per human car","enabled":!lights.is_empty(),
    "lamps_per_car":2,"pair_intensity_cd":PAIR_INTENSITY_CD.to_array(),"lens_clearance_m":LENS_CLEARANCE_M,
    "origin_policy":"measured imported lens hardpoints for bundled models; collision-bound approximation for unknown models",
    "lights":lights.iter().enumerate().map(|(i,l)|serde_json::json!({"car_slot":i/2,"side":if i%2==0{"left"}else{"right"},"position":l.position.to_array(),"direction":l.direction.to_array(),
        "up":l.up.to_array(),"intensity_cd":l.intensity_cd.to_array(),"range_m":l.range_m,
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
    #[test]fn bundled_mounts_are_on_actual_imported_lens_triangles() {
        // This reads the same GLB node transforms as car_model::load. A model
        // re-export that moves/scales the lenses must update the hardpoints.
        for (model,material) in [("asti_stradale.glb","UCB_Lights_and_Glass.003"),("mv6.glb","UCB_Lights_and_Glass_Transperent.002"),("toyama_xr.glb","Body_Glass.001")] {
            let path=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../assets/cars").join(model);
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
