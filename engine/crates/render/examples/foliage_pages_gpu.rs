//! Real source-alpha paged-sun regression. Exclusive GPU, no game/window.
//! KERN_PAGED_FOLIAGE_SUN=1 foliage_pages_gpu --out FRESH_DIRECTORY
//! KERN_FOLIAGE_SUN_PAGE_METRES=64|128|256 selects matching group spacing.
//! Fixed native geometry/wind; diagnostic25 distinguishes missing pages.
//! Timings are fixture pass costs, not gameplay FPS or forest scalability.
use glam::{Affine3A, Mat4, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::scene::{Heightfield, Material, ALPHA_TEST, FOLIAGE, TWO_SIDED};
use render::{Camera, FrameStats, Gpu, Lighting, Renderer, Scene, ViewRect};
use std::path::{Path, PathBuf};

const W:u32=480;
const H:u32=320;
const MAX_WARMUP:usize=32;
const UPDATE_BUDGET:u64=8;
const PROBES:[(&str,f32,bool);3]=[("opaque-strip",0.0,true),("alpha-gap",8.0,false),("behind-caster",-12.0,false)];

fn page_metres(setting:Option<&str>)->f32 {
    // Match the backend's bounded setting and default exactly.
    match setting {Some("128")=>128.0,Some("256")=>256.0,_=>64.0}
}
fn group_origin(group:usize,page_world:f32)->Vec3 {Vec3::Z*(group as f32*2.0*page_world)}

fn mesh(scene:&mut Scene,positions:[[f32;3];4],normal:Vec3,material:u32)->usize {
    let uv=[[0.0,1.0],[0.0,0.0],[1.0,0.0],[1.0,1.0]];
    let vertices:Vec<_>=positions.into_iter().zip(uv).map(|(pos,uv)|Vertex{pos,normal:normal.to_array(),uv}).collect();
    scene.add_mesh(&ClusterMesh::build(&[Part{vertices:&vertices,indices:&[0,1,2,0,2,3],material}]))
}
fn scene(page_world:f32)->(Scene,[u32;4]) {
    let mut s=Scene::default();s.sky_physical=true;s.sky=Some((2,1,vec![[0.0,0.0,0.0,1.0];2]));
    // Optional world-terrain visibility sees a flat field, never the caster.
    s.heightfield=Heightfield{origin:[-1000.0,-1000.0],cell:2000.0+group_origin(3,page_world).z,w:2,h:2,heights:vec![0.0;4]};
    s.terrain_visibility_complete=true;
    let alpha=s.add_image("paged foliage alpha strip",image::RgbaImage::from_fn(128,128,|x,_|
        image::Rgba([255,255,255,if (52..76).contains(&x){255}else{0}])));
    let material=|layer,flags|Material{tint:[0.5;3],layer,roughness:1.0,flags,scale:1.0,metal:0.0};
    let floor_m=s.add_material(material(-1,TWO_SIDED));
    let alpha_m=s.add_material(material(alpha,ALPHA_TEST|TWO_SIDED));
    let solid_alpha_m=s.add_material(material(-1,ALPHA_TEST|TWO_SIDED));
    let floor=mesh(&mut s,[[-22.0,0.0,22.0],[22.0,0.0,22.0],[22.0,0.0,-22.0],[-22.0,0.0,-22.0]],Vec3::Y,floor_m);
    let wall=mesh(&mut s,[[600.0,16.0,-20.0],[600.0,30.0,-20.0],[600.0,30.0,20.0],[600.0,16.0,20.0]],Vec3::NEG_X,alpha_m);
    // Along the open receiver's backwards solar line, below its floor.
    // Shares the projected footprint but is farther from the sun: must not
    // shadow the receiver. This exercises depth ordering, not alpha coverage.
    let behind=mesh(&mut s,[[-60.0,-5.0,-15.0],[-60.0,-0.5,-15.0],[-60.0,-0.5,-9.0],[-60.0,-5.0,-9.0]],Vec3::X,solid_alpha_m);
    let mut owners=[0;4];
    for (i,owner) in owners.iter_mut().enumerate() {
        let pose=Affine3A::from_translation(group_origin(i,page_world));
        *owner=s.instances.len()as u32;s.add_instance(floor,pose);
        s.add_instance(wall,pose);s.instances.last_mut().unwrap().flags|=FOLIAGE;
        s.add_instance(behind,pose);s.instances.last_mut().unwrap().flags|=FOLIAGE;
    }
    (s,owners)
}
fn camera(group:usize,offset:Vec3,page_world:f32)->Camera {
    let target=group_origin(group,page_world);
    Camera{pos:target+offset,target,fov_y:16f32.to_radians(),near:0.1}
}
fn view(camera:Camera,index:usize)->ViewRect {ViewRect{cam:camera,x:index as u32%2*W,y:index as u32/2*H,w:W,h:H}}
fn project(view:&ViewRect,p:Vec3)->Result<(u32,u32),String> {
    let vp=Mat4::perspective_infinite_reverse_rh(view.cam.fov_y,view.w as f32/view.h as f32,view.cam.near)
        *Mat4::look_at_rh(view.cam.pos,view.cam.target,Vec3::Y);
    let q=vp.project_point3(p);let x=(q.x*0.5+0.5)*view.w as f32;let y=(0.5-q.y*0.5)*view.h as f32;
    if !q.is_finite()||x<3.0||y<3.0||x>=view.w as f32-3.0||y>=view.h as f32-3.0{return Err("probe outside camera".into());}
    Ok((view.x+x as u32,view.y+y as u32))
}
fn missing(rgb:&[u8])->bool {
    // Diagnostic25 still passes through AgX: at this fixture's EV11 the
    // linear (6000,0,6000) marker becomes approximately (234,139,234), not
    // display-primary magenta. Known visibility is strictly achromatic here.
    // Require both purple channels to agree and exceed green substantially;
    // no amount of grey PCF coverage can satisfy this chromatic predicate.
    rgb[0].abs_diff(rgb[2])<=8 && rgb[0]as u16>=rgb[1]as u16+48 && rgb[2]as u16>=rgb[1]as u16+48
}
fn magenta_count(image:&[u8])->usize {image.chunks_exact(4).filter(|c|missing(c)).count()}
fn dimensions(views:&[ViewRect])->(u32,u32){(views.iter().map(|v|v.x+v.w).max().unwrap(),views.iter().map(|v|v.y+v.h).max().unwrap())}
fn picks(views:&[ViewRect],groups:&[usize],page_world:f32)->Result<Vec<(u32,u32)>,String> {
    let mut out=Vec::new();
    for (v,&group) in views.iter().zip(groups) {for (_,z,_) in PROBES {
        let (x,y)=project(v,group_origin(group,page_world)+Vec3::Z*z)?;
        for dy in -2..=2 {for dx in -2..=2 {out.push(((x as i32+dx)as u32,(y as i32+dy)as u32));}}
    }}Ok(out)
}
fn record(name:&str,s:&FrameStats,image:&[u8])->String {
    let times=s.gpu_ms.iter().map(|(name,ms)|format!("[{name:?},{ms}]")).collect::<Vec<_>>().join(",");
    format!("{{\"name\":{name:?},\"requested\":{},\"resident_requested\":{},\"missing\":{},\"updated\":{},\"cache_bytes\":{},\"clusters\":{},\"triangles\":{},\"magenta_pixels\":{},\"gpu_ms\":[{times}]}}",
        s.foliage_sun_requested_pages,s.foliage_sun_resident_pages,s.foliage_sun_missing_pages,
        s.foliage_sun_updated_pages,s.foliage_sun_cache_bytes,s.foliage_sun_clusters,s.foliage_sun_triangles,magenta_count(image))
}
fn verify(name:&str,image:&[u8],stats:&FrameStats,width:u32,coords:&[(u32,u32)],groups:&[usize],owners:&[u32;4],diagnostic:bool,failures:&mut Vec<String>)->Vec<String> {
    let mut rows=Vec::new();
    for (vi,&group) in groups.iter().enumerate() {
        let mut means=[0.0;3];
        for (probe,(label,_,blocked)) in PROBES.into_iter().enumerate() {
            let offset=(vi*3+probe)*25;let mut unknown=0;let mut wrong_owner=0;let mut sum=[0u32;3];
            for k in offset..offset+25 {
                let (x,y)=coords[k];let c=&image[((y*width+x)*4)as usize..][..3];
                unknown+=usize::from(missing(c));
                wrong_owner+=usize::from(!stats.picks.get(k).is_some_and(|p|p.is_some_and(|p|p.0==owners[group])));
                for channel in 0..3 {sum[channel]+=c[channel]as u32;}
            }
            let rgb=sum.map(|v|v as f64/25.0);means[probe]=rgb[0];
            rows.push(format!("{{\"case\":{name:?},\"view\":{vi},\"group\":{group},\"probe\":{label:?},\"expected_blocked\":{blocked},\"mean_rgb\":{rgb:?},\"missing_pixels\":{unknown},\"wrong_owner_pixels\":{wrong_owner}}}"));
            if unknown!=0||wrong_owner!=0 {failures.push(format!("{name} view{vi} {label}: missing={unknown}, wrong owner={wrong_owner}"));}
            if diagnostic && ((blocked&&rgb[0]>3.0)||(!blocked&&rgb[0]<225.0)) {
                failures.push(format!("{name} view{vi} {label}: shadow visibility RGB={rgb:?}"));
            }
        }
        if !diagnostic && !(means[1]>means[0]+12.0&&means[2]>means[0]+12.0) {
            failures.push(format!("{name} view{vi}: lit alpha gap/depth control did not retain sunlight {means:?}"));
        }
    }rows
}
fn warm(renderer:&Renderer,gpu:&Gpu,views:&[ViewRect],light:&Lighting,coords:&[(u32,u32)],out:&Path,name:&str,rows:&mut Vec<String>,failures:&mut Vec<String>)->Result<(Vec<u8>,FrameStats,bool),String> {
    let (w,h)=dimensions(views);let mut saw_deferred=false;
    for frame in 0..MAX_WARMUP {
        let (pixels,stats)=renderer.try_render_views(gpu,views,light,w,h,coords).map_err(|error| {
            let partial=format!("{{\"case\":{name:?},\"frame\":{frame},\"error\":{error:?},\"rows\":[{}]}}",rows.join(","));
            let _=std::fs::write(out.join("render-error.json"),partial);error
        })?;
        if stats.foliage_sun_updated_pages as u64>UPDATE_BUDGET {failures.push(format!("{name}: exceeded8 shared page updates"));}
        if stats.foliage_sun_resident_pages>256 {failures.push(format!("{name}: exceeded256 resident page budget"));}
        if stats.foliage_sun_missing_pages>0&&magenta_count(&pixels)>0{saw_deferred=true;}
        if stats.foliage_sun_requested_pages!=stats.foliage_sun_resident_pages+stats.foliage_sun_missing_pages {
            failures.push(format!("{name}: requested != resident requested + missing"));
        }
        rows.push(record(&format!("{name}-warm{frame}"),&stats,&pixels));
        if frame==0 {render::write_png(&out.join(format!("{name}-cold.png")),w,h,&pixels)?;}
        let settled=stats.foliage_sun_missing_pages==0&&stats.foliage_sun_updated_pages==0;
        if settled||frame+1==MAX_WARMUP {
            render::write_png(&out.join(format!("{name}.png")),w,h,&pixels)?;
            if stats.foliage_sun_requested_pages==0||stats.foliage_sun_resident_pages==0||stats.foliage_sun_cache_bytes==0 {
                failures.push(format!("{name}: converged with empty page cache"));
            }
            if !settled {failures.push(format!("{name}: page requests did not converge in {MAX_WARMUP} frames; missing pages must not count as clear"));}
            return Ok((pixels,stats,saw_deferred));
        }
    }
    unreachable!("nonzero warmup budget returns its final frame")
}
fn panel(pixels:&[u8],index:usize)->Vec<u8> {
    (0..H).flat_map(|y|{let start=(((y+index as u32/2*H)*W*2+index as u32%2*W)*4)as usize;
        pixels[start..start+(W*4)as usize].iter().copied()}).collect()
}
fn run()->Result<(),String> {
    if std::env::var("KERN_PAGED_FOLIAGE_SUN").as_deref()!=Ok("1"){return Err("set KERN_PAGED_FOLIAGE_SUN=1 explicitly".into());}
    let args:Vec<_>=std::env::args().collect();
    let out=PathBuf::from(&args.windows(2).find(|v|v[0]=="--out").ok_or("--out FRESH_DIRECTORY required")?[1]);
    if out.exists(){return Err("output directory must be fresh".into());}std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    // Prevent the old shared far-map experiment from masking paged failures.
    unsafe {std::env::set_var("KERN_FAR_SUN_SHADOW","0");}
    let page_world=page_metres(std::env::var("KERN_FOLIAGE_SUN_PAGE_METRES").ok().as_deref());
    let group_spacing=2.0*page_world;let texel_m=page_world/504.0;
    let group_positions=std::array::from_fn::<_,4,_>(|i|group_origin(i,page_world).z);
    let (scene,owners)=scene(page_world);let sun_angle=2f32.to_radians();
    let light=Lighting{sun_dir:Vec3::new(sun_angle.cos(),sun_angle.sin(),0.0),sun_color:Vec3::ONE,sun_lux:80000.0,
        sky_lux:0.0,ev100:11.0,gi:0.0,shadow_strength:1.0,near_shadows:1.0,contact_occlusion:0.0,
        reflection_occlusion:0.0,fog_density:0.0,fog_start:10000.0,fog_end:20000.0,fog_culling:false,
        taa:0.0,sharpen:0.0,lod_pixels:0.0,debug:25.0,..Default::default()};
    let gpu=Gpu::new()?;let renderer=Renderer::new(&gpu,&scene,&light);
    renderer.set_wind(&gpu,&render::wind::WindState{strength:0.0,time_seconds:0.0,..Default::default()})?;
    renderer.set_animation_time(0.0)?;
    let mut rows=Vec::new();let mut measurements=Vec::new();let mut failures=Vec::new();
    let cameras:[Camera;4]=std::array::from_fn(|i|camera(i,Vec3::new(0.0,80.0,180.0),page_world));
    let split:Vec<_>=cameras.into_iter().enumerate().map(|(i,c)|view(c,i)).collect();
    renderer.set_lighting_focus(&cameras.map(|c|c.pos))?;
    let split_picks=picks(&split,&[0,1,2,3],page_world)?;
    let (split_reference,initial,saw_deferred)=warm(&renderer,&gpu,&split,&light,&split_picks,&out,"four-view",&mut rows,&mut failures)?;
    if !saw_deferred {failures.push("cold four-view request never demonstrated missing pages as magenta; deferred-page test not covered".into());}
    measurements.extend(verify("four-view",&split_reference,&initial,W*2,&split_picks,&[0,1,2,3],&owners,true,&mut failures));
    for (name,cam) in [("single",cameras[0]),("approach",camera(0,Vec3::new(0.0,60.0,130.0),page_world)),
        ("orbit",camera(0,Vec3::new(-110.0,70.0,130.0),page_world)), ("return",cameras[0])] {
        let views=[view(cam,0)];let coords=picks(&views,&[0],page_world)?;renderer.set_lighting_focus(&[cam.pos])?;
        let (pixels,stats,_)=warm(&renderer,&gpu,&views,&light,&coords,&out,name,&mut rows,&mut failures)?;
        measurements.extend(verify(name,&pixels,&stats,W,&coords,&[0],&owners,true,&mut failures));
        if (name=="single"||name=="return")&&pixels!=panel(&split_reference,0) {failures.push(format!("{name}: identical camera differs from four-view panel"));}
    }
    renderer.set_lighting_focus(&cameras.map(|c|c.pos))?;
    // Resident is scoped to the currently requested views. Cache persistence
    // must instead be established by returning to all four original views:
    // their pages must still be available without new raster work.
    let (held,held_stats)=renderer.try_render_views(&gpu,&split,&light,W*2,H*2,&split_picks)?;
    rows.push(record("four-view-return",&held_stats,&held));
    if held_stats.foliage_sun_missing_pages!=0||held_stats.foliage_sun_updated_pages!=0||held!=split_reference {
        failures.push("returning to four cameras changed a settled image or required page reconstruction".into());
    }
    let mut lit=light.clone();lit.debug=0.0;
    let (lit_pixels,lit_stats)=renderer.try_render_views(&gpu,&split,&lit,W*2,H*2,&split_picks)?;
    render::write_png(&out.join("four-view-lit.png"),W*2,H*2,&lit_pixels)?;
    rows.push(record("four-view-lit",&lit_stats,&lit_pixels));
    measurements.extend(verify("four-view-lit",&lit_pixels,&lit_stats,W*2,&split_picks,&[0,1,2,3],&owners,false,&mut failures));
    if lit_stats.foliage_sun_missing_pages!=0||lit_stats.foliage_sun_updated_pages!=0 {failures.push("lit consumer changed settled cache residency".into());}
    let report=format!("{{\"schema\":1,\"scope\":\"real source-alpha paged sun, not forest optical parity or gameplay FPS\",\"page_world_m\":{page_world},\"group_spacing_m\":{group_spacing},\"texel_m\":{texel_m},\"page_resolution\":512,\"usable_resolution\":504,\"page_budget\":256,\"updates_per_frame\":8,\"warmup_limit\":32,\"source_caster_sunward_m\":600,\"sun_elevation_degrees\":2,\"wind\":0,\"groups_cross_sun_m\":{group_positions:?},\"deferred_missing_observed\":{saw_deferred},\"rows\":[{}],\"receiver_measurements\":[{}],\"failures\":{:?},\"passed\":{}}}",rows.join(","),measurements.join(","),failures,failures.is_empty());
    std::fs::write(out.join("report.json"),report).map_err(|e|e.to_string())?;
    if failures.is_empty(){println!("foliage pages fixture PASS");Ok(())}else{Err(failures.join("\n"))}
}
fn main(){if let Err(e)=run(){eprintln!("foliage pages fixture: {e}");std::process::exit(1);}}

#[cfg(test)]mod tests {
    use super::*;
    #[test]fn missing_page_oracle_recognizes_agx_marker_but_never_grey_coverage() {
        // Actual real-renderer cold-frame palette with the fixed fixture look.
        assert!(missing(&[234,139,234]));
        assert!(missing(&[255,0,255]));
        for grey in 0..=255 {assert!(!missing(&[grey,grey,grey]));}
        assert!(!missing(&[234,139,139]));assert!(!missing(&[139,139,234]));
    }
    #[test]fn analytic_sun_rays_cover_alpha_strip_and_reject_backwards_depth() {
        let slope=2f32.to_radians().tan();assert!((16.0..30.0).contains(&(600.0*slope)));
        let opaque=|z:f32|{let x=((z+20.0)/40.0*128.0).floor()as i32;(52..76).contains(&x)};
        assert!(opaque(0.0));assert!(!opaque(8.0));assert!(!opaque(-12.0));
        assert!((-5.0..-0.5).contains(&(-60.0*slope)));
        // This geometry lies on the *negative* ray: it cannot intercept sun.
        let t=-60.0/2f32.to_radians().cos();assert!(t<0.0);
    }
    #[test]fn all_receiver_regions_remain_on_floor_for_camera_variants() {
        let page_world=page_metres(None);let (_,owners)=scene(page_world);assert_eq!(owners,[0,3,6,9]);
        for offset in [Vec3::new(0.0,80.0,180.0),Vec3::new(0.0,60.0,130.0),Vec3::new(-110.0,70.0,130.0)] {
            for group in 0..4 {let v=view(camera(group,offset,page_world),0);assert_eq!(picks(&[v],&[group],page_world).unwrap().len(),75);}
        }
    }
    #[test]fn page_width_changes_preserve_sixteen_distinct_visible_receiver_pages() {
        assert_eq!(page_metres(None),64.0);assert_eq!(page_metres(Some("64")),64.0);
        for invalid in ["0","65","512","NaN"] {assert_eq!(page_metres(Some(invalid)),64.0);}
        for (setting,width) in [("64",64.0),("128",128.0),("256",256.0)] {
            assert_eq!(page_metres(Some(setting)),width);
            let mut pages=std::collections::BTreeSet::new();
            for group in 0..4 {
                let origin=group_origin(group,width);assert_eq!(origin.z,group as f32*2.0*width);
                let v=view(camera(group,Vec3::new(0.0,80.0,180.0),width),0);
                assert_eq!(picks(&[v],&[group],width).unwrap().len(),75);
                // Interior floor points on both sides of each light-space
                // boundary are genuinely visible, not just sphere overreach.
                for x in [-10.0,10.0] {for z in [-10.0,10.0] {
                    let p=origin+Vec3::new(x,0.0,z);project(&v,p).unwrap();
                    pages.insert(((-p.z/width).floor()as i32,(-2f32.to_radians().sin()*p.x/width).floor()as i32));
                }}
            }
            assert_eq!(pages.len(),16);assert!(pages.len()>UPDATE_BUDGET as usize);
        }
    }
}
