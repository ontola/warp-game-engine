//! Opt-in procedural spectator prototype. Uses the engine's shared baked
//! rigid-animation atlas and ordinary GPU instance/cluster culling. No CPU
//! per-frame actors, no simulation RNG consumption and no gameplay collision.

use glam::{Affine3A, Mat4, Quat, Vec2, Vec3};
use render::{baked_animation::{Actor, BakedAnimations, BonePose, Clip, Part as AnimatedPart},
    cluster::{ClusterMesh, Part, Vertex}, scene::{Material, Scene}};
use sideways::{builder::{Chunk, N}, stage::{CHUNK, GRID, Stage}};
use std::collections::HashMap;

const BONES:u32=15;
const FRAMES:u32=48;
const ACTOR_BUDGET:usize=256;
const ROAD_CLEARANCE:f32=4.5;
const FOOT_SPREAD:f32=0.16;

#[derive(Clone, Debug, PartialEq)]
struct Spectator {id:u32,group:u32,position:Vec3,yaw:f32,scale:f32,clip:u32,phase:f32,rate:f32}

fn hash(mut x:u32)->u32 {x^=x>>16;x=x.wrapping_mul(0x7feb352d);x^=x>>15;x=x.wrapping_mul(0x846ca68b);x^(x>>16)}
fn random(id:u32,salt:u32)->f32 {(hash(id^salt.wrapping_mul(0x9e3779b9))>>8) as f32/16777216.0}
fn vec(p:gdcompat::math::V3)->Vec3 {Vec3::new(p.x,p.y,p.z)}

struct Ground<'a> {chunks:HashMap<(i32,i32),&'a Chunk>,obstacles:HashMap<(i32,i32),Vec<(Vec2,f32)>>}
#[derive(Clone,Copy)]
struct Support {height:f32,road_d:f32,up:f32}
impl<'a> Ground<'a> {
    fn new(chunks:&'a [Chunk])->Self {
        let mut obstacles:HashMap<(i32,i32),Vec<(Vec2,f32)>>=HashMap::new();
        for chunk in chunks {
            for (spots,padding) in [(&chunk.trees,0.65),(&chunk.rocks,1.7),(&chunk.stumps,0.7)] {
                for spot in spots {
                    let p=Vec2::new(spot.pos.0,spot.pos.2);
                    let cell=((p.x/8.0).floor() as i32,(p.y/8.0).floor() as i32);
                    obstacles.entry(cell).or_default().push((p,padding*spot.scale+0.65));
                }
            }
        }
        Self {chunks:chunks.iter().map(|c|(c.cell,c)).collect(),obstacles}
    }
    fn clear(&self,p:Vec2)->bool {
        let c=((p.x/8.0).floor() as i32,(p.y/8.0).floor() as i32);
        for dz in -1..=1 {for dx in -1..=1 {
            if self.obstacles.get(&(c.0+dx,c.1+dz)).is_some_and(|items|items.iter().any(|(q,r)|p.distance_squared(*q)<r*r)) {return false;}
        }}true
    }
    fn sample(&self,p:Vec2)->Option<Support> {
        let cell=((p.x/CHUNK as f32).floor() as i32,(p.y/CHUNK as f32).floor() as i32);
        let chunk=self.chunks.get(&cell)?;
        let q=(p-Vec2::new(chunk.x0 as f32,chunk.z0 as f32))/GRID as f32;
        let i=q.x.floor() as usize;let j=q.y.floor() as usize;
        if i>=N-1||j>=N-1 {return None;}
        let x=q.x-i as f32;let z=q.y-j as f32;let a=j*N+i;
        let (idx,w,dx,dz)=if x+z<=1.0 {
            ([a,a+1,a+N],[1.0-x-z,x,z],chunk.heights[a+1]-chunk.heights[a],chunk.heights[a+N]-chunk.heights[a])
        } else {
            ([a+N+1,a+N,a+1],[x+z-1.0,1.0-x,1.0-z],chunk.heights[a+N+1]-chunk.heights[a+N],chunk.heights[a+N+1]-chunk.heights[a+1])
        };
        let height=(0..3).map(|k|chunk.heights[idx[k]]*w[k]).sum();
        let road_d=(0..3).map(|k|chunk.road_d[idx[k]]*w[k]).sum();
        Some(Support {height,road_d,up:1.0/(1.0+(dx*dx+dz*dz)/(GRID*GRID) as f32).sqrt()})
    }
}

fn supported(stage:&Stage,ground:&Ground,p:Vec2,side_axis:Vec2,scale:f32)->Option<f32> {
    if !ground.clear(p) {return None;}
    let mut lo=f32::INFINITY;let mut hi=f32::NEG_INFINITY;
    // Both feet and the front/back of their footprints must have support.
    // A narrow foothold over a ravine is rejected, rather than posing by IK.
    let forward=Vec2::new(-side_axis.y,side_axis.x);
    for sideways in [-FOOT_SPREAD,FOOT_SPREAD] {for depth in [-0.13,0.18] {
        let foot=p+(side_axis*sideways+forward*depth)*scale;
        let s=ground.sample(foot)?;
        if s.up<0.985 || s.road_d<ROAD_CLEARANCE || stage.water_level(foot.x as f64,foot.y as f64)>s.height as f64-0.15 {return None;}
        lo=lo.min(s.height);hi=hi.max(s.height);
    }}
    if hi-lo>0.055 {return None;}
    // Keep the collision corridor clear even where interpolated road_d could
    // refer to another close hairpin. No spectators on the road ribbon.
    let near=stage.locate(gdcompat::math::v3(p.x,0.0,p.y),-1);
    if near.index<0 || near.offset.abs()<stage.width_at(near.along)+ROAD_CLEARANCE as f64 {return None;}
    Some((hi+lo)*0.5-0.012)
}

fn placements(stage:&Stage,chunks:&[Chunk])->Vec<Spectator> {
    let ground=Ground::new(chunks);let seed=hash(stage.detail_seed as u32^stage.noise_seed as u32);
    let mut result:Vec<Spectator>=Vec::new();let mut group=0u32;
    let mut at=70.0;
    while at<stage.finish-35.0 && result.len()<ACTOR_BUDGET {
        let group_seed=hash(seed^group.wrapping_mul(0x63d83595));
        let large=group%4==2;
        let desired=if large {32+(random(group_seed,1)*17.0) as usize} else {6+(random(group_seed,1)*7.0) as usize};
        let mut best=Vec::new();
        // Search nearby safe verges instead of forcing a crowd on a cliff.
        // Curves are preferred within the same small route interval.
        let mut anchors=[at-20.0,at,at+20.0,at+40.0];
        anchors.sort_by(|a,b|stage.curvature_at(*b).abs().total_cmp(&stage.curvature_at(*a).abs()));
        for anchor in anchors {for sign in [-1.0f32,1.0] {
            let mut candidate=Vec::new();
            for slot in 0..desired*3 {
                if candidate.len()>=desired {break;}
                let id=hash(group_seed^slot as u32);
                let along=anchor+(random(id,2) as f64-0.5)*if large {24.0}else{12.0};
                let center=vec(stage.point_at(along));let direction=vec(stage.dir_at(along)).normalize();
                let right=Vec3::new(-direction.z,0.0,direction.x).normalize();
                let away=stage.width_at(along) as f32+5.2+random(id,3)*if large {5.0}else{2.5};
                let p=center+right*away*sign;let xz=Vec2::new(p.x,p.z);
                if result.iter().chain(candidate.iter()).any(|other:&Spectator|Vec2::new(other.position.x,other.position.z).distance_squared(xz)<0.82*0.82) {continue;}
                let scale=0.87+random(id,5)*0.25;
                let facing=(center-p).normalize();let yaw=facing.x.atan2(facing.z)+(random(id,6)-0.5)*0.3;
                let side=Vec2::new(yaw.cos(),-yaw.sin());
                let Some(height)=supported(stage,&ground,xz,side,scale) else {continue;};
                candidate.push(Spectator {id,group,position:Vec3::new(p.x,height,p.z),yaw,scale,
                    clip:u32::from(random(id,7)>0.4),phase:random(id,8),rate:0.85+random(id,9)*0.3});
            }
            if candidate.len()>best.len() {best=candidate;}
            if best.len()==desired {break;}
        }if best.len()==desired {break;}}
        if best.len()>=3 {result.extend(best.into_iter().take(ACTOR_BUDGET-result.len()));}
        group+=1;at+=150.0+random(group_seed,10) as f64*70.0;
    }
    result
}

fn pose(rotation:Quat,translation:Vec3)->BonePose {BonePose::new(rotation,translation)}
fn bake_clip(cheer:bool)->Clip {
    let mut poses=Vec::with_capacity((FRAMES*BONES) as usize);
    for frame in 0..FRAMES {
        let t=frame as f32/FRAMES as f32*std::f32::consts::TAU;
        let sway=Quat::from_rotation_z(t.sin()*if cheer {0.035}else{0.014});
        let hip=Vec3::new(0.0,0.96,0.0);
        poses.push(pose(sway,hip+sway*Vec3::new(0.0,0.28,0.0)));
        poses.push(pose(sway*Quat::from_rotation_y((t*0.5).sin().powi(2)*0.18-0.09),hip+sway*Vec3::new(0.0,0.72,0.0)));
        poses.push(pose(Quat::IDENTITY,Vec3::new(0.0,0.90,0.0)));
        for sign in [-1.0f32,1.0] {
            let lift=if cheer {2.25+0.23*(t+sign*0.7).sin()} else {0.12+0.03*(t+sign).sin()};
            let upper=sway*Quat::from_rotation_z(sign*lift)*Quat::from_rotation_x(if cheer {-0.12}else{-0.04});
            let shoulder=hip+sway*Vec3::new(sign*0.255,0.48,0.0);
            let forearm=upper*Quat::from_rotation_x(if cheer {-0.48-0.2*t.sin()}else{-0.18});
            let elbow=shoulder+upper*Vec3::new(0.0,-0.29,0.0);
            poses.push(pose(upper,shoulder));poses.push(pose(forearm,elbow));
            poses.push(pose(forearm,elbow+forearm*Vec3::new(0.0,-0.27,0.0)));
        }
        for sign in [-1.0f32,1.0] {
            poses.push(pose(Quat::IDENTITY,Vec3::new(sign*0.13,0.89,0.0)));
            poses.push(pose(Quat::IDENTITY,Vec3::new(sign*0.13,0.50,0.0)));
            poses.push(pose(Quat::IDENTITY,Vec3::new(sign*0.13,0.065,0.045)));
        }
    }
    Clip {name:if cheer {"cheer"} else {"idle"}.into(),bones:BONES,frames:FRAMES,duration:if cheer {2.8}else{4.0},poses}
}

fn material(scene:&mut Scene,color:[f32;3])->u32 {
    scene.add_material(Material {tint:color,layer:-1,roughness:0.94,flags:0,scale:1.0,metal:0.0})
}

// Small closed smooth shapes are baked once into the same indexed mesh/CLOD
// format as other scene objects. Dimensions live in vertices, not sheared
// animated matrices. A palette is shared; no material or mesh per spectator.
fn ellipsoid(center:Vec3,size:Vec3,rings:u32,sides:u32)->(Vec<Vertex>,Vec<u32>) {
    let mut vertices=Vec::new();let mut indices=Vec::new();
    for y in 0..=rings {for x in 0..=sides {
        let theta=y as f32/rings as f32*std::f32::consts::PI;
        let phi=x as f32/sides as f32*std::f32::consts::TAU;
        let n=Vec3::new(theta.sin()*phi.cos(),theta.cos(),theta.sin()*phi.sin());
        vertices.push(Vertex {pos:(center+n*size).to_array(),normal:(n/size).normalize().to_array(),uv:[x as f32/sides as f32,y as f32/rings as f32]});
    }}
    for y in 0..rings {for x in 0..sides {
        let a=y*(sides+1)+x;let b=a+sides+1;
        if y>0 {indices.extend([a,a+1,b]);}
        if y+1<rings {indices.extend([a+1,b+1,b]);}
    }}
    (vertices,indices)
}

fn add_mesh(scene:&mut Scene,names:&mut Vec<String>,name:&str,shapes:&[(Vec3,Vec3,u32)])->usize {
    let buffers:Vec<_>=shapes.iter().map(|(center,size,_)|ellipsoid(*center,*size,5,8)).collect();
    let parts:Vec<_>=buffers.iter().zip(shapes).map(|((vertices,indices),(_,_,material))|Part {vertices,indices,material:*material}).collect();
    let mesh=ClusterMesh::build(&parts);let id=scene.add_mesh(&mesh);names.push(format!("crowd {name}"));id
}

pub(crate) fn append(scene:&mut Scene,stage:&Stage,chunks:&[Chunk],names:&mut Vec<String>)->Result<Option<String>,String> {
    if std::env::var("SIDEWAYS_CROWDS").as_deref()!=Ok("1") {return Ok(None);}
    append_enabled(scene,stage,chunks,names).map(Some)
}

fn append_enabled(scene:&mut Scene,stage:&Stage,chunks:&[Chunk],names:&mut Vec<String>)->Result<String,String> {
    if scene.baked_animations.is_some() {return Err("crowd prototype expects an unused animation atlas".into());}
    let spectators=placements(stage,chunks);
    if spectators.is_empty() {return Ok("crowds prototype: no safe supported sites, 0 actors".into());}
    let shirts:Vec<_>=[[0.55,0.035,0.025],[0.025,0.12,0.38],[0.55,0.34,0.025],[0.04,0.21,0.10],
        [0.62,0.61,0.53],[0.08,0.035,0.15],[0.06,0.065,0.07],[0.12,0.34,0.35]].map(|c|material(scene,c)).to_vec();
    let trousers:Vec<_>=[[0.025,0.04,0.07],[0.09,0.055,0.025],[0.035,0.04,0.035],[0.19,0.16,0.09]].map(|c|material(scene,c)).to_vec();
    let skins:Vec<_>=[[0.52,0.30,0.17],[0.30,0.14,0.065],[0.12,0.055,0.025],[0.65,0.44,0.30]].map(|c|material(scene,c)).to_vec();
    let hair=material(scene,[0.035,0.022,0.012]);let shoes=material(scene,[0.018,0.021,0.025]);
    let torso:Vec<_>=shirts.iter().flat_map(|&m|[0.9,1.0,1.13].map(|width|add_mesh(scene,names,"shirt",&[(Vec3::ZERO,Vec3::new(0.23*width,0.28,0.135),m)]))).collect();
    let upper:Vec<_>=shirts.iter().map(|&m|add_mesh(scene,names,"sleeve",&[(Vec3::new(0.0,-0.145,0.0),Vec3::new(0.08,0.165,0.08),m)])).collect();
    let forearms:Vec<_>=skins.iter().map(|&m|add_mesh(scene,names,"forearm",&[(Vec3::new(0.0,-0.135,0.0),Vec3::new(0.06,0.15,0.06),m)])).collect();
    let hands:Vec<_>=skins.iter().map(|&m|add_mesh(scene,names,"hand",&[(Vec3::new(0.0,-0.06,0.0),Vec3::new(0.052,0.075,0.038),m)])).collect();
    let heads:Vec<_>=skins.iter().map(|&m|add_mesh(scene,names,"head",&[
        (Vec3::ZERO,Vec3::new(0.105,0.145,0.10),m),(Vec3::new(0.0,-0.15,0.0),Vec3::new(0.06,0.07,0.06),m),
        (Vec3::new(0.0,0.095,-0.015),Vec3::new(0.106,0.075,0.09),hair),
        (Vec3::new(0.0,0.01,0.10),Vec3::new(0.025,0.025,0.035),m),
        (Vec3::new(-0.04,0.04,0.086),Vec3::new(0.011,0.009,0.016),hair),
        (Vec3::new(0.04,0.04,0.086),Vec3::new(0.011,0.009,0.016),hair)])).collect();
    let pelvis:Vec<_>=trousers.iter().map(|&m|add_mesh(scene,names,"hips",&[(Vec3::ZERO,Vec3::new(0.19,0.15,0.125),m)])).collect();
    let thighs:Vec<_>=trousers.iter().map(|&m|add_mesh(scene,names,"thigh",&[(Vec3::new(0.0,-0.19,0.0),Vec3::new(0.10,0.215,0.105),m)])).collect();
    let shins:Vec<_>=trousers.iter().map(|&m|add_mesh(scene,names,"shin",&[(Vec3::new(0.0,-0.19,0.0),Vec3::new(0.075,0.215,0.08),m)])).collect();
    let foot=add_mesh(scene,names,"shoe",&[(Vec3::ZERO,Vec3::new(0.085,0.065,0.15),shoes)]);
    let mut animation=BakedAnimations {clips:vec![bake_clip(false),bake_clip(true)],..Default::default()};
    let groups=spectators.iter().map(|p|p.group).collect::<std::collections::BTreeSet<_>>().len();
    for s in &spectators {
        let shirt=(hash(s.id^33)%shirts.len() as u32) as usize;let pants=(hash(s.id^61)%trousers.len() as u32) as usize;
        let skin=(hash(s.id^92)%skins.len() as u32) as usize;let width=(hash(s.id^113)%3) as usize;
        let meshes=[torso[shirt*3+width],heads[skin],pelvis[pants],upper[shirt],forearms[skin],hands[skin],
            upper[shirt],forearms[skin],hands[skin],thighs[pants],shins[pants],foot,thighs[pants],shins[pants],foot];
        let actor=animation.actors.len();
        animation.actors.push(Actor {root:Mat4::from_scale_rotation_translation(Vec3::splat(s.scale),Quat::from_rotation_y(s.yaw),s.position),
            clip:s.clip,phase:s.phase,rate:s.rate});
        for (bone,mesh) in meshes.into_iter().enumerate() {
            let owner=scene.instances.len();let matrix=animation.sample(actor,bone as u32,0.0);
            scene.add_instance(mesh,Affine3A::from_mat4(matrix));
            animation.parts.push(AnimatedPart {actor:actor as u32,bone:bone as u32,owner:owner as u32,local_sphere:scene.meshes[mesh].sphere});
        }
    }
    animation.validate(&scene.instances)?;
    let summary=format!("crowds prototype: {} actors / {} groups / {} rigid parts, {} shared clip bytes; cap {} world actors, ordinary per-view culling; no gameplay collision",
        spectators.len(),groups,animation.parts.len(),animation.pose_bytes(),ACTOR_BUDGET);
    scene.baked_animations=Some(animation);Ok(summary)
}

pub(crate) fn metadata(scene:&Scene)->serde_json::Value {
    scene.baked_animations.as_ref().map(|a|serde_json::json!({"enabled":true,"quality":"procedural_rigid_placeholder",
        "actors":a.actors.len(),"parts":a.parts.len(),"clips":a.clips.iter().map(|c|&c.name).collect::<Vec<_>>(),
        "pose_bytes":a.pose_bytes(),"actor_capacity":ACTOR_BUDGET,"scope":"one shared world pool, per-view culling",
        "placement":"supported native terrain triangles, road/water/obstacle rejection","collidable":false
    })).unwrap_or(serde_json::json!({"enabled":false}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stage_and_chunks()->(Stage,Vec<Chunk>) {
        let stage=Stage::generate(&mut gdcompat::Pcg::new(1),1600.0);
        let mut cells=std::collections::BTreeSet::new();
        for along in (0..1600).step_by(20) {
            let p=stage.point_at(along as f64);let cell=((p.x/CHUNK as f32).floor() as i32,(p.z/CHUNK as f32).floor() as i32);
            for dx in -1..=1 {for dz in -1..=1 {cells.insert((cell.0+dx,cell.1+dz));}}
        }
        let chunks=cells.into_iter().map(|cell|sideways::builder::terrain_chunk(&stage,cell)).collect();(stage,chunks)
    }
    #[test]
    fn crowd_placement_is_deterministic_supported_and_bounded_for_four_views() {
        let (stage,chunks)=stage_and_chunks();let before=(stage.noise_seed,stage.detail_seed,stage.points.clone());
        let a=placements(&stage,&chunks);let b=placements(&stage,&chunks);assert_eq!(a,b);assert!(!a.is_empty());assert!(a.len()<=ACTOR_BUDGET);
        let ground=Ground::new(&chunks);let mut group_sizes=HashMap::new();
        for p in &a {
            *group_sizes.entry(p.group).or_insert(0)+=1;
            assert_eq!(supported(&stage,&ground,Vec2::new(p.position.x,p.position.z),Vec2::new(p.yaw.cos(),-p.yaw.sin()),p.scale),Some(p.position.y));
            let near=stage.locate(gdcompat::math::v3(p.position.x,p.position.y,p.position.z),-1);
            assert!(near.offset.abs()>stage.width_at(near.along)+ROAD_CLEARANCE as f64);
        }
        assert!(group_sizes.values().any(|n|*n>=16),"expected an occasional larger group, got {group_sizes:?}");
        assert_eq!(before.0,stage.noise_seed);assert_eq!(before.1,stage.detail_seed);assert_eq!(before.2,stage.points);
        // The actor pool has no camera argument: four views reuse it, not 4x.
        let total_parts=a.len()*BONES as usize;assert!(total_parts<=render::baked_animation::MAX_PARTS);
        eprintln!("crowd fixture: {} actors, groups {group_sizes:?}, shared parts {total_parts}",a.len());
    }
    #[test]
    fn clips_keep_feet_planted_and_are_continuous_at_wrap() {
        for cheer in [false,true] {
            let clip=bake_clip(cheer);
            assert_eq!(clip.poses.len(),(FRAMES*BONES) as usize);
            for frame in 0..FRAMES {for bone in [9,10,11,12,13,14] {
                assert_eq!(clip.poses[(frame*BONES+bone) as usize].translation,clip.poses[bone as usize].translation);
            }}
            let a=BakedAnimations {clips:vec![clip],actors:vec![Actor {root:Mat4::IDENTITY,clip:0,phase:0.0,rate:1.0}],parts:Vec::new()};
            for bone in 0..BONES {assert!(a.sample(0,bone,0.0).abs_diff_eq(a.sample(0,bone,a.clips[0].duration-1e-5),0.0001));}
        }
    }
    #[test]
    fn body_mesh_winding_matches_normals_and_shapes_are_closed() {
        let (v,i)=ellipsoid(Vec3::ZERO,Vec3::new(0.1,0.3,0.2),5,8);
        for triangle in i.chunks_exact(3) {
            let a=Vec3::from(v[triangle[0] as usize].pos);let b=Vec3::from(v[triangle[1] as usize].pos);let c=Vec3::from(v[triangle[2] as usize].pos);
            let n=(b-a).cross(c-a);
            assert!(n.length()>1e-7);assert!(n.dot((a+b+c)/3.0)>0.0,"inward spectator body face");
        }
    }
}
