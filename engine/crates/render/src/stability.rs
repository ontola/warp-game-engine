//! World-anchored grids for shadows and canopy lighting.
//!
//! Snapping the target *after* a look-at centered on that target is a no-op:
//! its light-space x/y are already zero. Use a sun-only basis, then quantize
//! the absolute target coordinates in that basis instead.

use glam::{Mat4, Vec3, Vec4};

const MIN_EXTENT: f32 = 96.0;
const EXTENT_STEP: f32 = 16.0;

fn sun_basis(sun_dir: Vec3) -> (Vec3, Vec3, Vec3) {
    let z = if sun_dir.is_finite() { sun_dir.normalize_or(Vec3::Y) } else { Vec3::Y };
    // Avoid the singular look-at basis at noon or with an invalid zero sun.
    let up = if z.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
    let x = up.cross(z).normalize();
    let y = z.cross(x);
    (x, y, z)
}

/// A stable directional-light projection, with the existing 1..900 m depth
/// interval and eye 400 m toward the sun. `extent` is the HALF width in metres.
/// For a fixed sun and extent, translation changes x/y only by whole texels.
/// Depth may follow the target continuously: it does not change the x/y grid.
pub fn stable_shadow(centre: Vec3, sun_dir: Vec3, extent: f32, resolution: u32) -> Mat4 {
    let centre = if centre.is_finite() { centre } else { Vec3::ZERO };
    let extent = if extent.is_finite() && extent > 0.0 { extent } else { MIN_EXTENT };
    let texel = 2.0 * extent / resolution.max(1) as f32;
    let (x, y, z) = sun_basis(sun_dir);
    let cx = (x.dot(centre) / texel).round() * texel;
    let cy = (y.dot(centre) / texel).round() * texel;
    let cz = z.dot(centre) + 400.0;
    let view = Mat4::from_cols(
        Vec4::new(x.x, y.x, z.x, 0.0),
        Vec4::new(x.y, y.y, z.y, 0.0),
        Vec4::new(x.z, y.z, z.z, 0.0),
        Vec4::new(-cx, -cy, -cz, 1.0),
    );
    Mat4::orthographic_rh(-extent, extent, -extent, extent, 1.0, 900.0) * view
}

/// Sun-only diagnostic: absolute light-space caster depth, shared by all maps.
/// X/Y use the exact legacy grid; changing scene depth cannot move a texel.
pub fn stable_shadow_depth(centre: Vec3, sun_dir: Vec3, extent: f32, resolution: u32, range: [f32;2]) -> Mat4 {
    if !range.iter().all(|v|v.is_finite()) || range[1] <= range[0] {
        return stable_shadow(centre,sun_dir,extent,resolution);
    }
    let centre=if centre.is_finite(){centre}else{Vec3::ZERO};
    let extent=if extent.is_finite() && extent>0.0 {extent}else{MIN_EXTENT};
    let texel=2.0*extent/resolution.max(1) as f32;
    let (x,y,z)=sun_basis(sun_dir);
    let cx=(x.dot(centre)/texel).round()*texel;
    let cy=(y.dot(centre)/texel).round()*texel;
    let eye=range[1]+1.0;
    let view=Mat4::from_cols(
        Vec4::new(x.x,y.x,z.x,0.0),Vec4::new(x.y,y.y,z.y,0.0),
        Vec4::new(x.z,y.z,z.z,0.0),Vec4::new(-cx,-cy,-eye,1.0));
    let mut projection=Mat4::orthographic_rh(-extent,extent,-extent,extent,1.0,range[1]-range[0]+2.0)*view;
    // Canonical texel translations avoid cancellation roundoff from cx/extent.
    // Power-of-two map sizes then produce exact reusable depth permutations.
    projection.w_axis.x=-(x.dot(centre)/texel).round()*(2.0/resolution.max(1)as f32);
    projection.w_axis.y=-(y.dot(centre)/texel).round()*(2.0/resolution.max(1)as f32);
    projection
}

pub fn shadow_eye(focus:Vec3,sun_dir:Vec3,range:[f32;2])->Vec3 {
    let (_,_,z)=sun_basis(sun_dir);
    focus+z*(range[1]+1.0-z.dot(focus))
}

/// Build once, then expand for CPU-updated instances. This deliberately never
/// shrinks mid-scene: moving actors cannot make shadow depth precision flicker.
/// GPU animation bounds cover the entire clip / every allowed wind angle.
pub(crate) struct SunSceneBounds { min:Vec3, max:Vec3 }
impl Default for SunSceneBounds {
    fn default()->Self {Self{min:Vec3::splat(f32::INFINITY),max:Vec3::splat(f32::NEG_INFINITY)}}
}
impl SunSceneBounds {
    pub(crate) fn sphere(&mut self,p:Vec3,r:f32) {
        if p.is_finite() && r.is_finite() && r>=0.0 {
            self.min=self.min.min(p-Vec3::splat(r));self.max=self.max.max(p+Vec3::splat(r));
        }
    }
    pub(crate) fn extend_instances(&mut self,instances:&[crate::scene::GpuInstance]) {
        use crate::scene::{NO_SHADOW,FOLIAGE,LIVE_GRASS,WIND_GRASS,PART_WIND};
        for i in instances {
            if i.cluster_count==0 || i.flags&NO_SHADOW!=0 {continue;}
            let mut radius=i.sphere[3];
            if i.flags&PART_WIND==0 && i.flags&(FOLIAGE|LIVE_GRASS|WIND_GRASS)!=0 {
                // Bounds the shader's vector waveform (<1), at max permitted
                // strength and shear, regardless of subsequent wind edits.
                let shear=if i.flags&(LIVE_GRASS|WIND_GRASS)!=0 {0.6}else{0.08};
                radius+=(i.sphere[1]-i.rows[1][3]).abs().mul_add(shear,i.sphere[3]*shear);
            }
            self.sphere(Vec3::from_slice(&i.sphere),radius);
        }
    }
    pub(crate) fn from_scene(scene:&crate::scene::Scene)->Self {
        let mut bounds=Self::default();bounds.extend_instances(&scene.instances);
        if let Some(forest)=&scene.gpu_forest {
            for &root in &forest.roots {
                if let Some(s)=crate::foliage_runtime::root_shadow_sphere(root,forest.nodes[root.info[0]as usize].sphere) {
                    bounds.sphere(Vec3::from_slice(&s),s[3]);
                }
            }
        }
        // Rigid child rotations preserve each local translation length. The
        // sum of root-to-part lengths bounds EVERY possible articulated pose.
        let mut bind:Vec<Mat4>=Vec::with_capacity(scene.part_wind.len());
        let mut paths:Vec<(Vec3,f32,f32)>=Vec::with_capacity(scene.part_wind.len());
        for (index,part) in scene.part_wind.iter().enumerate() {
            let (world,root,scale,path)=if let Some(parent)=part.parent {
                let (root,scale,path)=paths[parent as usize];
                (bind[parent as usize]*part.rest,root,scale,path+part.rest.w_axis.truncate().length()*scale)
            }else{
                let scale=[part.rest.x_axis,part.rest.y_axis,part.rest.z_axis].iter().map(|v|v.truncate().length()).fold(0.0,f32::max);
                (part.rest,part.rest.w_axis.truncate(),scale,0.0)
            };
            bind.push(world);paths.push((root,scale,path));
            if let Some(owner)=part.owner {
                let source=&scene.instances[owner as usize];
                if source.flags&crate::scene::NO_SHADOW==0 && source.cluster_count>0 {
                    let local=bind[index].inverse().transform_point3(Vec3::from_slice(&source.sphere));
                    let radius=path+local.length()*scale+source.sphere[3]*scale.max(source.scale)/source.scale;
                    bounds.sphere(root,radius);
                }
            }
        }
        // Shared primitive attachments may have no branch-proxy source mesh.
        // Bound every animated attachment around its assembly root, allowing
        // every child rotation and nonuniform root/primitive transform. The
        // inverse Frobenius norm deliberately overbounds the rest sphere.
        for attachment in &scene.part_attachments {
            let source=&scene.instances[attachment.owner as usize];
            if source.cluster_count==0 || source.flags&crate::scene::NO_SHADOW!=0 {continue;}
            let pose=attachment.pose as usize;
            let (root,scale,path)=paths[pose];
            let inverse=bind[pose].inverse();
            let centre=inverse.transform_point3(Vec3::from_slice(&source.sphere));
            let inverse_stretch=(inverse.x_axis.truncate().length_squared()+inverse.y_axis.truncate().length_squared()
                +inverse.z_axis.truncate().length_squared()).sqrt();
            let margin=1.0+crate::part_wind::MAX_ASSEMBLY_PARTS as f32*16.0*f32::EPSILON;
            bounds.sphere(root,(path+scale*(centre.length()+source.sphere[3]*inverse_stretch))*margin);
        }
        if let Some(animation)=&scene.baked_animations {
            for part in &animation.parts {
                let source=&scene.instances[part.owner as usize];
                if source.flags&crate::scene::NO_SHADOW!=0 || source.cluster_count==0 {continue;}
                let actor=&animation.actors[part.actor as usize];
                let clip=&animation.clips[actor.clip as usize];
                let reach=clip.poses.iter().skip(part.bone as usize).step_by(clip.bones as usize)
                    .map(|p|Vec3::from_slice(&p.translation).length()).fold(0.0,f32::max);
                let scale=actor.root.x_axis.truncate().length();
                bounds.sphere(actor.root.w_axis.truncate(),scale*(reach+Vec3::from_slice(&part.local_sphere).length()+part.local_sphere[3]));
            }
        }
        bounds
    }
    pub(crate) fn depth_range(&self,sun:Vec3,receivers:&[Vec3])->[f32;2] {
        let (_,_,z)=sun_basis(sun);
        let mut low=f32::INFINITY;let mut high=f32::NEG_INFINITY;
        if self.min.is_finite() && self.max.is_finite() {
            for bits in 0..8 {
                let p=Vec3::new(if bits&1==0{self.min.x}else{self.max.x},if bits&2==0{self.min.y}else{self.max.y},if bits&4==0{self.min.z}else{self.max.z});
                low=low.min(z.dot(p));high=high.max(z.dot(p));
            }
        }
        for p in receivers.iter().filter(|p|p.is_finite()) {let d=z.dot(*p);low=low.min(d);high=high.max(d);}
        if !low.is_finite() || !high.is_finite() {return [-512.0,512.0];}
        // Round outward, leaving a numeric safety margin. No fixed caster
        // distance: a low-sun mountain remains eligible beyond 400 metres.
        let margin=1.0+8.0*f32::EPSILON*low.abs().max(high.abs());
        [((low-margin)/16.0).floor()*16.0,((high+margin)/16.0).ceil()*16.0]
    }
}

/// Round outward, never cropping the requested half-width. Keeping the scale
/// constant within a bucket also keeps orthographic shadow mesh LOD constant.
pub fn quantized_shadow_extent(required: f32) -> f32 {
    let required = if required.is_finite() { required.max(MIN_EXTENT) } else { MIN_EXTENT };
    (required / EXTENT_STEP).ceil() * EXTENT_STEP
}

/// Retain the largest bucket for this scene so four players close to a bucket
/// boundary cannot make all shadows alternate size/LOD. Reset `previous` when
/// rebuilding the scene. Wide player separation trades shadow detail for
/// coverage; retaining this value separately from shadow-cache validity is
/// essential because moving cars invalidate shadow CONTENT every frame.
pub fn stable_shadow_extent(required: f32, previous: f32) -> f32 {
    quantized_shadow_extent(required).max(quantized_shadow_extent(previous))
}

/// Scroll a volume in complete world cells. The cell centres at overlapping
/// world positions then remain identical when the volume moves a whole cell.
/// `desired` is its lower corner, not its centre; floor also handles negatives.
pub fn world_grid_origin(desired: Vec3, voxel: f32) -> Vec3 {
    let voxel = if voxel.is_finite() && voxel > 0.0 { voxel } else { 1.0 };
    let desired = if desired.is_finite() { desired } else { Vec3::ZERO };
    (desired / voxel).floor() * voxel
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receiver_plane_texel_depth_matches_independent_ray_plane_intersection() {
        use glam::DVec3;
        // Oracle intersects rays through actual texel centres with a plane;
        // the tested expression is the WGSL projected-depth gradient.
        for elevation in [0.2_f64,2.0,15.0,45.0,80.0] {
            let el=elevation.to_radians();let az=75_f64.to_radians();
            let sun=DVec3::new(el.cos()*az.sin(),el.sin(),-el.cos()*az.cos());
            let right=DVec3::Y.cross(sun).normalize();let up=sun.cross(right);
            for n in [DVec3::Y,DVec3::new(0.2,0.9,0.3).normalize(),DVec3::new(-0.3,0.4,0.8).normalize()] {
                for texel in [32.0/2048.0,192.0/4096.0] {
                    for fx in [0.0,0.17,0.5,0.99] {for fy in [0.0,0.31,0.83] {
                        for y in -1..=2 {for x in -1..=2 {
                            let dx=x as f64-fx;let dy=y as f64-fy;
                            let p=DVec3::new(54.436972,-2.871241,-2251.465536);
                            let origin=p+right*(dx*texel)-up*(dy*texel)+sun*200.0;
                            let t=n.dot(p-origin)/n.dot(-sun);
                            let hit=origin-sun*t;
                            let oracle=-sun.dot(hit-p)/899.0;
                            let gradient=(n.dot(right)*dx-n.dot(up)*dy)*texel/(899.0*n.dot(sun));
                            assert!((oracle-gradient).abs()<2e-12,"{elevation}: {oracle} vs {gradient}");
                        }}
                    }}
                }
            }
        }
    }

    #[test]
    fn receiver_plane_sixteen_loads_preserve_nine_bilinear_pcf_weights() {
        for fx in [0.0_f32,0.13,0.5,0.999] {for fy in [0.0_f32,0.37,0.92] {
            // Arbitrary discontinuous comparison outcomes, not a flat shadow.
            let value=|x:i32,y:i32| if ((x*7+y*13)&3)==0 {0.0}else{1.0};
            let mut old=0.0;
            for y in -1..=1 {for x in -1..=1 {
                old+=((1.0-fx)*value(x,y)+fx*value(x+1,y))*(1.0-fy)
                    +((1.0-fx)*value(x,y+1)+fx*value(x+1,y+1))*fy;
            }}
            let wx=[1.0-fx,1.0,1.0,fx];let wy=[1.0-fy,1.0,1.0,fy];let mut explicit=0.0;
            for y in 0..4 {for x in 0..4 {explicit+=wx[x]*wy[y]*value(x as i32-1,y as i32-1)/9.0;}}
            assert!((explicit-old/9.0).abs()<2e-7);
            assert!((wx.iter().sum::<f32>()*wy.iter().sum::<f32>()/9.0-1.0).abs()<2e-7);
        }}
    }

    #[test]
    fn receiver_plane_kernel_stays_inside_actual_triangle_at_unoccluded_concave_crease() {
        use glam::Vec2;
        // Vertical sun, h(x)=0.5*abs(x): BOTH faces are fully sun-facing.
        // Extrapolating the left triangle plane h=-0.5*x into x>0 falsely
        // puts that sample below the neighbour and produces a dark edge.
        let a=Vec2::new(-4.0,-4.0);let b=Vec2::new(0.0,-4.0);let c=Vec2::new(0.0,4.0);
        let inside=|p:Vec2|(b-a).perp_dot(p-a)>=0.0 && (c-b).perp_dot(p-b)>=0.0 && (a-c).perp_dot(p-c)>=0.0;
        for q in [Vec2::new(-0.9,0.2),Vec2::new(-0.5,0.2),Vec2::new(-0.1,0.2)] {
            let base=q.floor();let f=q-base;
            let wx=[1.0-f.x,1.0,1.0,f.x];let wy=[1.0-f.y,1.0,1.0,f.y];
            let mut unbounded=0.0;let mut clipped=0.0;let mut sum=0.0;
            for y in 0..4 {for x in 0..4 {
                let p=base+Vec2::new(x as f32-1.0,y as f32-1.0);
                let weight=wx[x]*wy[y]/9.0;
                let visible=if -0.5*p.x+1e-6>=0.5*p.x.abs(){1.0}else{0.0};
                unbounded+=weight*visible;
                if inside(p) {clipped+=weight*visible;sum+=weight;}
            }}
            assert!(unbounded<0.99,"old extrapolation must reproduce false crease shadow");
            assert!(sum>0.0);assert!((clipped/sum-1.0_f32).abs()<1e-6);
        }
    }

    #[test]
    fn receiver_plane_subtexel_fallback_keeps_real_occluders_and_bounded_sample_distance() {
        use glam::Vec2;
        let a=Vec2::new(0.1,0.1);let b=Vec2::new(0.4,0.1);let c=Vec2::new(0.1,0.4);
        let inside=|p:Vec2|(b-a).perp_dot(p-a)>=0.0 && (c-b).perp_dot(p-b)>=0.0 && (a-c).perp_dot(p-c)>=0.0;
        for y in -1..=2 {for x in -1..=2 {assert!(!inside(Vec2::new(x as f32,y as f32)));}}
        let receiver=Vec2::new(0.2,0.2);let nearest=(receiver+Vec2::splat(0.5)).floor();
        assert!((nearest-receiver).abs().max_element()<=0.5);
        // Compare the actual receiver depth, never a plane at an invalid tap
        // or an unconditional lit result. Both possible outcomes remain.
        let compare=|stored:f32|if 0.6-1e-6<=stored {1.0}else{0.0};
        assert_eq!(compare(0.5),0.0);assert_eq!(compare(0.8),1.0);
    }

    #[test]
    fn scene_depth_covers_distant_casters_without_changing_xy_or_near_far_depth() {
        let sun=Vec3::new(0.9653374,0.0348995,-0.2586614).normalize();
        let focus=Vec3::new(57.0,-2.3,-2253.0);
        let caster=focus+sun*600.0;
        let mut bounds=SunSceneBounds::default();bounds.sphere(caster,20.0);bounds.sphere(focus,2.0);
        let range=bounds.depth_range(sun,&[focus]);
        let old=stable_shadow(focus,sun,96.0,4096);
        assert!(old.project_point3(caster).z<0.0,"old projection clips the sunward mountain");
        let far=stable_shadow_depth(focus,sun,96.0,4096,range);
        let near=stable_shadow_depth(focus,sun,16.0,2048,range);
        for p in [focus,caster,caster+sun*20.0,caster-sun*20.0] {
            let a=far.project_point3(p);let b=near.project_point3(p);
            assert!(a.z>0.0 && a.z<1.0);assert_eq!(a.z,b.z);
            assert_eq!(a.truncate(),old.project_point3(p).truncate());
        }
        assert!((sun.dot(shadow_eye(focus,sun,range))-range[1]-1.0).abs()<0.001);
    }

    #[test]
    fn scene_depth_bounds_cover_maximum_affine_wind_and_expand_dynamic_updates() {
        let mut instance=crate::scene::GpuInstance {
            rows:[Vec4::X.to_array(),Vec4::Y.to_array(),Vec4::Z.to_array()],
            sphere:[0.0,20.0,0.0,12.0],first_cluster:0,cluster_count:1,scale:1.0,flags:crate::scene::FOLIAGE,
        };
        let mut bounds=SunSceneBounds::default();bounds.extend_instances(&[instance]);
        let before=bounds.depth_range(Vec3::X,&[]);
        for sign in [-1.0,1.0] {
            let point=Vec3::new(sign*12.0,32.0,0.0)+Vec3::X*(sign*32.0*0.08);
            assert!(point.cmpge(bounds.min).all() && point.cmple(bounds.max).all());
        }
        instance.sphere[0]=3000.0;instance.flags=0;bounds.extend_instances(&[instance]);
        let after=bounds.depth_range(Vec3::X,&[]);
        assert!(after[0]<=before[0] && after[1]>=3012.0);
        instance.sphere[0]=-10000.0;instance.flags=crate::scene::NO_SHADOW;
        bounds.extend_instances(&[instance]);assert_eq!(after,bounds.depth_range(Vec3::X,&[]));
    }

    #[test]
    fn scene_depth_bounds_cover_baked_clip_extremes_and_articulated_branches() {
        use crate::baked_animation::{Actor,BonePose,BakedAnimations,Clip,Part};
        let source=crate::scene::GpuInstance {
            rows:[Vec4::X.to_array(),Vec4::Y.to_array(),Vec4::Z.to_array()],
            sphere:[0.0,0.0,0.0,1.0],first_cluster:0,cluster_count:1,scale:1.0,flags:0,
        };
        let mut scene=crate::scene::Scene::default();scene.instances.push(source);
        scene.baked_animations=Some(BakedAnimations {
            clips:vec![Clip{name:"reach".into(),bones:1,frames:2,duration:1.0,poses:vec![
                BonePose::new(glam::Quat::IDENTITY,Vec3::X*12.0),
                BonePose::new(glam::Quat::from_rotation_y(2.0),Vec3::NEG_Z*19.0)]}],
            actors:vec![Actor{root:Mat4::from_scale_rotation_translation(Vec3::splat(2.0),glam::Quat::IDENTITY,Vec3::X*1000.0),clip:0,phase:0.0,rate:1.0}],
            parts:vec![Part{actor:0,bone:0,owner:0,local_sphere:[3.0,0.0,0.0,1.0]}],
        });
        let bounds=SunSceneBounds::from_scene(&scene);
        for time in (0..100).map(|i|i as f32/100.0) {
            let matrix=scene.baked_animations.as_ref().unwrap().sample(0,0,time);
            let p=matrix.transform_point3(Vec3::X*3.0);
            assert!((p-Vec3::splat(2.0)).cmpge(bounds.min).all());
            assert!((p+Vec3::splat(2.0)).cmple(bounds.max).all());
        }
        scene.baked_animations=None;
        scene.part_wind=vec![crate::part_wind::PartWind{rest:Mat4::from_scale_rotation_translation(Vec3::new(2.0,3.0,1.0),glam::Quat::IDENTITY,Vec3::X*100.0),..Default::default()},
            crate::part_wind::PartWind{parent:Some(0),owner:Some(0),rest:Mat4::from_translation(Vec3::Y*10.0),..Default::default()}];
        let world=scene.part_wind[0].rest*scene.part_wind[1].rest;
        scene.instances[0].sphere=world.transform_point3(Vec3::Y*4.0).extend(3.0).to_array();
        scene.instances[0].scale=3.0;scene.instances[0].flags=crate::scene::PART_WIND;
        let bounds=SunSceneBounds::from_scene(&scene);
        // Even a 90-degree articulation exceeds allowed wind and is covered.
        for a in [-1.57_f32,-0.785,0.0,0.785,1.57] {
            let m=scene.part_wind[0].rest*Mat4::from_rotation_z(a)*scene.part_wind[1].rest*Mat4::from_rotation_z(a);
            let p=m.transform_point3(Vec3::Y*4.0);
            assert!((p-Vec3::splat(3.0)).cmpge(bounds.min).all());
            assert!((p+Vec3::splat(3.0)).cmple(bounds.max).all());
        }
    }

    #[test]
    fn scene_depth_covers_animated_attachments_without_branch_proxy_geometry() {
        use crate::part_wind::{PartWind,PartAttachment};
        use crate::scene::{Scene,GpuInstance,PART_WIND};
        let mut scene=Scene::default();
        scene.part_wind=vec![PartWind {rest:Mat4::from_scale_rotation_translation(Vec3::new(3.,0.5,2.),
            glam::Quat::from_rotation_y(0.7),Vec3::new(120.,30.,-600.)),max_angle:0.4,..Default::default()},
            PartWind {parent:Some(0),rest:Mat4::from_translation(Vec3::new(1.,7.,2.)),max_angle:0.7,..Default::default()}];
        let local=Mat4::from_scale_rotation_translation(Vec3::new(0.1,2.,0.3),glam::Quat::from_rotation_x(0.5),Vec3::new(0.,4.,2.));
        let world=scene.part_wind[0].rest*scene.part_wind[1].rest*local;
        let centre=world.transform_point3(Vec3::Y);
        let radius=(world.x_axis.truncate().length_squared()+world.y_axis.truncate().length_squared()
            +world.z_axis.truncate().length_squared()).sqrt();
        scene.instances.push(GpuInstance {rows:[world.row(0).to_array(),world.row(1).to_array(),world.row(2).to_array()],
            sphere:centre.extend(radius).to_array(),first_cluster:0,cluster_count:1,scale:radius,flags:PART_WIND});
        scene.part_attachments.push(PartAttachment {pose:1,owner:0,rest:local});
        let bounds=SunSceneBounds::from_scene(&scene);
        for time in (0..64).map(|i|i as f64*1.9) {
            let poses=crate::part_wind::sample_poses(&scene.part_wind,&crate::wind::WindState {
                time_seconds:time,strength:1.,..Default::default()}).unwrap();
            for axis in [Vec3::X,Vec3::Y,Vec3::Z] {for sign in [-1.,1.] {
                let p=(poses[1]*local).transform_point3(Vec3::Y+axis*sign);
                assert!(p.cmpge(bounds.min).all()&&p.cmple(bounds.max).all());
                for sun in [Vec3::X,Vec3::Y,Vec3::new(0.5,0.02,-0.7).normalize()] {
                    let range=bounds.depth_range(sun,&[Vec3::ZERO]);
                    let m=stable_shadow_depth(Vec3::ZERO,sun,2048.,4096,range);
                    let z=m.project_point3(p).z;
                    assert!(z>0.&&z<1.,"animated caster escaped conservative sun Z: {z}");
                }
            }}
        }
    }

    fn pixel_xy(m: Mat4, point: Vec3, resolution: u32) -> [f32; 2] {
        let p = m.project_point3(point);
        [p.x * resolution as f32 * 0.5, p.y * resolution as f32 * 0.5]
    }

    #[test]
    fn sub_texel_camera_motion_does_not_move_world_shadow_xy() {
        let sun = Vec3::new(0.4, 0.8, -0.3).normalize();
        let (x, y, z) = sun_basis(sun);
        let texel = 192.0 / 4096.0;
        let point = Vec3::new(7.0, 2.0, -9.0);
        let reference = pixel_xy(stable_shadow(Vec3::ZERO, sun, 96.0, 4096), point, 4096);
        // Includes suspension/depth motion: only x/y grid stability matters.
        for t in [-0.49, -0.2, 0.0, 0.2, 0.49] {
            let centre = x * (texel * t) + y * (texel * t) + z * (t * 0.03);
            let p = pixel_xy(stable_shadow(centre, sun, 96.0, 4096), point, 4096);
            assert!((p[0] - reference[0]).abs() < 0.001);
            assert!((p[1] - reference[1]).abs() < 0.001);
        }
    }

    #[test]
    fn moving_or_separated_cameras_change_projection_by_integer_texels() {
        let sun = Vec3::new(-0.6, 0.7, 0.2).normalize();
        let point = Vec3::new(2007.0, 11.0, -1900.0);
        let initial = Vec3::new(2000.0, 10.0, -1890.0);
        let extent = quantized_shadow_extent(90.0 + 80.0);
        let first = pixel_xy(stable_shadow(initial, sun, extent, 4096), point, 4096);
        for shift in [Vec3::new(0.01, 0.02, -0.03), Vec3::new(1.2, -0.1, 2.0), Vec3::new(15.0, 0.0, -9.0)] {
            let p = pixel_xy(stable_shadow(initial + shift, sun, extent, 4096), point, 4096);
            for a in 0..2 {
                let delta = p[a] - first[a];
                assert!((delta - delta.round()).abs() < 0.02, "fractional shadow shift: {delta}");
            }
        }
    }

    #[test]
    fn vertical_sun_and_invalid_direction_stay_finite_and_cover_target() {
        for sun in [Vec3::Y, Vec3::NEG_Y, Vec3::ZERO, Vec3::new(f32::NAN, 0.0, 0.0)] {
            let centre = Vec3::new(1300.0, 40.0, -2000.0);
            let m = stable_shadow(centre, sun, 96.0, 4096);
            assert!(m.is_finite());
            let p = m.project_point3(centre);
            assert!(p.x.abs() <= 1.0 / 4096.0 + 0.00001);
            assert!(p.y.abs() <= 1.0 / 4096.0 + 0.00001);
            assert!(p.z > 0.0 && p.z < 1.0);
        }
    }

    #[test]
    fn extent_does_not_oscillate_or_crop_at_a_split_screen_bucket_boundary() {
        assert_eq!(quantized_shadow_extent(90.0), 96.0);
        let mut extent = 96.0;
        for required in [111.999, 112.001, 111.999, 112.001, 95.0, 250.0, 90.0] {
            let next = stable_shadow_extent(required, extent);
            assert!(next >= required && next >= extent);
            assert_eq!(next % EXTENT_STEP, 0.0);
            extent = next;
        }
        assert_eq!(extent, 256.0);
    }

    #[test]
    fn scrolling_volume_keeps_absolute_cell_centres_including_negative_space() {
        let a = world_grid_origin(Vec3::new(-0.01, 0.74, 3.2), 1.5);
        let b = world_grid_origin(Vec3::new(0.02, 0.76, 3.7), 1.5);
        assert_eq!(a, Vec3::new(-1.5, 0.0, 3.0));
        assert_eq!(b, Vec3::new(0.0, 0.0, 3.0));
        // The same centre shifts one local index when the grid scrolls.
        let before = a + (Vec3::new(4.0, 3.0, 2.0) + Vec3::splat(0.5)) * 1.5;
        let after = b + (Vec3::new(3.0, 3.0, 2.0) + Vec3::splat(0.5)) * 1.5;
        assert_eq!(before, after);
        assert_eq!(world_grid_origin(Vec3::new(0.1, 0.1, 3.1), 1.5), b);
    }
}
