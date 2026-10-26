//! Real-photo near/far grass comparison through the production GPU shaders.
//! cargo run --release -p cli --example grass_transition_gpu -- --out FRESH_DIR
//! Optional: --distances 50,80,100 --components lit --modes blend --size 960x540
//! Diagnostic only: one fixed resident patch, no playing window or simulation.
#![allow(dead_code)]
#[path = "../src/ground_cover.rs"] mod ground_cover;
#[path = "../src/live_ground_cover.rs"] mod live_ground_cover;
#[path = "../src/far_ground_cover.rs"] mod far_ground_cover;

use glam::{Affine3A, Mat4, Vec3, Vec4};
use render::{Camera, Gpu, Lighting, Renderer, Scene, ViewRect};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::scene::{Heightfield, Material, LIVE_GRASS};
use serde_json::{json, Value};
use sideways::builder::{Chunk, N};
use std::{path::{Path, PathBuf}, time::Instant};

const SIZE:[u32;2]=[640,360];
const DISTANCES:[f32;5]=[35.0,45.0,50.0,55.0,65.0];
const COMPONENTS:[(&str,f32);4]=[("lit",0.0),("albedo",2.0),("direct",3.0),("indirect",4.0)];
const MODES:[(&str,f32);3]=[("near",1.0),("far",2.0),("blend",0.0)];

struct Options {
    out:PathBuf, root:PathBuf, size:[u32;2], distances:Vec<f32>,
    components:Vec<(&'static str,f32)>, modes:Vec<(&'static str,f32)>,
}
fn named_selection(value:&str,available:&[(&'static str,f32)])->Result<Vec<(&'static str,f32)>,String> {
    let mut selected=Vec::new();
    for name in value.split(',').map(str::trim) {
        let entry=available.iter().find(|entry|entry.0==name).copied()
            .ok_or_else(||format!("unknown selection {name:?}; expected one of {}",available.iter().map(|v|v.0).collect::<Vec<_>>().join(",")))?;
        if selected.contains(&entry) {return Err(format!("duplicate selection {name:?}"));}
        selected.push(entry);
    }
    Ok(selected)
}
fn parse_options(args:&[String])->Result<Options,String> {
    let mut out=None;
    let mut options=Options {out:PathBuf::new(),root:sideways::game_assets::root(),
        size:SIZE,distances:DISTANCES.to_vec(),components:COMPONENTS.to_vec(),modes:MODES.to_vec()};
    let mut seen=std::collections::HashSet::new();
    for pair in args.chunks(2) {
        let key=pair[0].as_str();let value=pair.get(1).ok_or_else(||format!("missing value after {key}"))?;
        if !seen.insert(key) {return Err(format!("duplicate option {key}"));}
        match key {
            "--out"=>out=Some(PathBuf::from(value)),
            "--assets"=>options.root=sideways::game_assets::diagnostic_root(value),
            "--distances"=>{
                let distances=value.split(',').map(|v|v.trim().parse::<f32>().map_err(|_|format!("invalid distance {v:?}"))).collect::<Result<Vec<_>,_>>()?;
                if distances.is_empty()||distances.len()>64||distances.iter().any(|v|!v.is_finite()||*v<=0.0) {
                    return Err("distances require 1..64 finite positive metres".into());
                }
                let mut unique=std::collections::HashSet::new();
                if distances.iter().any(|v|!unique.insert(v.to_bits())) {return Err("duplicate distances would overwrite captures".into());}
                options.distances=distances;
            },
            "--components"=>options.components=named_selection(value,&COMPONENTS)?,
            "--modes"=>options.modes=named_selection(value,&MODES)?,
            "--size"=>{
                let (w,h)=value.split_once('x').ok_or("size must be WIDTHxHEIGHT")?;
                let size=[w.parse::<u32>().map_err(|_|"invalid size width")?,h.parse::<u32>().map_err(|_|"invalid size height")?];
                if size.contains(&0)||size.iter().any(|v|*v>8192)||u64::from(size[0])*u64::from(size[1])>16_777_216 {
                    return Err("size must be positive, at most 8192 per axis and 16777216 pixels".into());
                }
                options.size=size;
            },
            _=>return Err(format!("unknown option {key}")),
        }
    }
    options.out=out.ok_or("usage: grass_transition_gpu --out FRESH_DIR [--assets ROOT] [--distances 50,80,100] [--components lit,albedo,direct,indirect] [--modes near,far,blend] [--size WIDTHxHEIGHT]")?;
    Ok(options)
}

fn fixture(root:&Path)->Result<(Scene,Value),String> {
    // Exactly one native-sized chunk; the production live scatter is evaluated
    // once at its centre. Both supported profile radii cover this chunk.
    // No later camera move can add, remove or replace a source grass plant.
    let chunk=Chunk {cell:(0,0),x0:0.0,z0:0.0,heights:vec![0.0;N*N],road_d:vec![5.0;N*N],
        trees:vec![],rocks:vec![],grass:vec![],ferns:vec![],stumps:vec![]};
    let mut scene=Scene::default();
    let material=scene.add_material(Material {tint:[0.05,0.05,0.05],layer:-1,
        roughness:1.0,flags:0,scale:1.0,metal:0.0});
    let vertices:Vec<_>=(0..N).flat_map(|z|(0..N).map(move|x|Vertex {
        pos:[x as f32*2.0,0.0,z as f32*2.0],normal:[0.0,1.0,0.0],uv:[0.0;2]})).collect();
    let mut indices=Vec::new();
    for z in 0..N-1 {for x in 0..N-1 {let a=(z*N+x) as u32;
        indices.extend([a,a+N as u32,a+1,a+N as u32+1,a+1,a+N as u32]);}}
    let mesh=scene.add_mesh(&ClusterMesh::build(&[Part {vertices:&vertices,indices:&indices,material}]));
    scene.add_instance(mesh,Affine3A::IDENTITY);scene.mark_no_shadow();
    scene.heightfield=Heightfield {origin:[0.0;2],cell:2.0,w:N as u32,h:N as u32,heights:chunk.heights.clone()};
    let chunks=[chunk];let mut names=Vec::new();
    let live=live_ground_cover::LiveGroundCover::reserve(&mut scene,&chunks,&[Vec3::new(32.0,4.0,32.0)],root,&mut names)?;
    let stats=live.stats();
    if stats.budget_clipped!=0 || stats.visible_budget_clipped!=0 {return Err("reference patch exceeded the live grass capacity".into());}
    let (field,summary)=far_ground_cover::bake(&chunks,&ground_cover::photo(root)?)?;
    let field_metadata=json!({"origin":field.origin,"size":[field.width,field.height],"cell":field.cell,
        "profile":field.profile,"shading_profile":field.shading_profile,"max_height":field.max_height});
    scene.far_grass=Some(field);
    let metadata=json!({"source":"same cropped Poly Haven CC0 grass_medium_01 photo, production three-card mesh and live scatter",
        "asset_root":root,"plants":stats.active,"capacity":stats.capacity,"budget_clipped":stats.budget_clipped,
        "grass_detail":{"profile":scene.grass_detail.label(),"views":1,"fade_start_m":stats.fade_start,
            "fade_end_m":stats.fade_end,"residency_m":stats.residency,"shared_pool_capacity":stats.capacity},
        "resident_anchor":[32.0,4.0,32.0],"world_patch":[0.0,0.0,64.0,64.0],"roi_world_xz":[16.0,16.0,48.0,48.0],
        "native_grid_metres":2.0,"terrain_height":0.0,"road_distance":5.0,"field":field_metadata,"bake":summary});
    Ok((scene,metadata))
}

fn camera(distance:f32)->Camera {
    Camera {pos:Vec3::new(32.0,4.0,32.0+distance),target:Vec3::new(32.0,0.25,32.0),
        fov_y:30f32.to_radians(),near:0.1}
}
fn roi(camera:&Camera,size:[u32;2])->Vec<(u32,u32)> {
    let inverse=(Mat4::perspective_infinite_reverse_rh(camera.fov_y,size[0] as f32/size[1] as f32,camera.near)
        *Mat4::look_at_rh(camera.pos,camera.target,Vec3::Y)).inverse();
    let mut pixels=Vec::new();
    for y in 0..size[1] {for x in 0..size[0] {
        let ndc=Vec4::new((x as f32+0.5)/size[0] as f32*2.0-1.0,1.0-(y as f32+0.5)/size[1] as f32*2.0,1e-4,1.0);
        let p=inverse*ndc;let direction=(p.truncate()/p.w-camera.pos).normalize();
        if direction.y>=0.0 {continue;}
        let hit=camera.pos+direction*(-camera.pos.y/direction.y);
        if (16.0..48.0).contains(&hit.x)&&(16.0..48.0).contains(&hit.z) {pixels.push((x,y));}
    }}
    pixels
}
fn half(bits:u16)->f32 {
    let exponent=(bits>>10)&31;let fraction=bits&1023;
    if exponent==0 {let value=fraction as f32/16_777_216.0;return if bits&0x8000!=0 {-value}else{value};}
    f32::from_bits(((bits as u32&0x8000)<<16)|((if exponent==31 {255}else{exponent as u32+112})<<23)|((fraction as u32)<<13))
}
fn write_json(path:&Path,value:&Value)->Result<(),String> {
    std::fs::write(path,serde_json::to_vec_pretty(value).map_err(|e|e.to_string())?).map_err(|e|e.to_string())
}
fn comparison(distance:f32,component:&str,samples:&[Value],near_coverage:Option<f64>)->Value {
    let sample=|mode:&str|samples.iter().find(|sample|sample["reference"].as_str()==Some(mode));
    let luma=|mode:&str|sample(mode).and_then(|v|v["display_luma_mean"].as_f64());
    let difference=|a:&str,b:&str|luma(a).zip(luma(b)).map(|(a,b)|a-b);
    // Every captured mode includes the raw far layer, even when only blend
    // was requested. Missing reference images cannot imply a comparison.
    let raw=sample("far").or_else(||samples.first());
    let alpha=raw.and_then(|v|v["far_raw_premul_linear_rgba_mean"][3].as_f64());
    json!({"distance_m":distance,"component":component,
        "references_present":samples.iter().filter_map(|v|v["reference"].as_str()).collect::<Vec<_>>(),
        "far_minus_near_display_luma":difference("far","near"),
        "blend_minus_near_display_luma":difference("blend","near"),"far_alpha_mean":alpha,
        "far_alpha_source_reference":raw.and_then(|v|v["reference"].as_str()),
        "far_to_actual_near_coverage_ratio":if component=="albedo" {
            alpha.zip(near_coverage.filter(|v|*v>0.0)).map(|(a,n)|a/n)
        }else{None}})
}

fn run()->Result<(),String> {
    let options=parse_options(&std::env::args().skip(1).collect::<Vec<_>>())?;
    let out=options.out;let size=options.size;
    if out.exists(){return Err("output directory must be fresh; previous evidence is preserved".into());}
    let (scene,source)=fixture(&options.root)?;
    std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    let gpu=Gpu::new()?;
    let base=Lighting {taa:0.0,sharpen:0.0,fog_density:0.0,fog_start:1000.0,fog_end:2000.0,
        ..Default::default()};
    let renderer=Renderer::new(&gpu,&scene,&base);
    let mut captures=Vec::new();let mut comparisons=Vec::new();
    for &distance in &options.distances {
        let camera=camera(distance);let roi=roi(&camera,size);
        if roi.len()<200 {return Err(format!("too few world-patch pixels at {distance}m: {}",roi.len()));}
        let views=[ViewRect {cam:camera,x:0,y:0,w:size[0],h:size[1]}];
        let mut near_coverage=None;
        for &(component,debug) in &options.components {
            let mut mode_samples=Vec::new();
            for &(mode,reference) in &options.modes {
                let name=format!("{distance}m-{component}-{mode}");
                let light=Lighting {debug,grass_reference:reference,..base.clone()};
                renderer.reset_history();
                let start=Instant::now();
                let picks=if mode=="near"&&component=="albedo" {roi.as_slice()}else{&[]};
                let (pixels,stats)=renderer.try_render_views(&gpu,&views,&light,size[0],size[1],picks)?;
                let wall_ms=start.elapsed().as_secs_f64()*1000.0;
                render::write_png(&out.join(format!("{name}.png")),size[0],size[1],&pixels)?;
                let mut mean=[0.0f64;3];
                for &(x,y) in &roi {let index=((y*size[0]+x)*4) as usize;
                    for channel in 0..3 {mean[channel]+=pixels[index+channel] as f64/255.0/roi.len() as f64;}}
                if !picks.is_empty() {
                    if stats.picks.len()!=roi.len() {return Err("near coverage picking count mismatch".into());}
                    let mut mask=vec![0u8;size[0] as usize*size[1] as usize*4];let mut hits=0usize;
                    for (&(x,y),pick) in roi.iter().zip(&stats.picks) {
                        let covered=pick.as_ref().is_some_and(|(owner,_,_)|scene.instances[*owner as usize].flags&LIVE_GRASS!=0);
                        hits+=usize::from(covered);let a=if covered{255}else{0};
                        mask[((y*size[0]+x)*4) as usize..((y*size[0]+x)*4+4) as usize].copy_from_slice(&[a,a,a,255]);
                    }
                    near_coverage=Some(hits as f64/roi.len() as f64);
                    render::write_png(&out.join(format!("{distance}m-near-coverage.png")),size[0],size[1],&mask)?;
                }
                let layers=renderer.far_grass_layers_rgba16(&gpu)?;
                let (w,h,bytes)=layers.first().ok_or("missing far grass raw layer")?;
                if [*w,*h]!=size.map(|v|v.div_ceil(2))||bytes.len()!=*w as usize**h as usize*8 {return Err("raw far layer extent mismatch".into());}
                let raw:Vec<[f32;4]>=bytes.chunks_exact(8).map(|p|std::array::from_fn(|c|half(u16::from_le_bytes([p[c*2],p[c*2+1]])))).collect();
                if raw.iter().flatten().any(|v|!v.is_finite())||raw.iter().any(|p|!(0.0..=1.0).contains(&p[3])) {return Err(format!("{name}: invalid raw HDR/alpha"));}
                std::fs::write(out.join(format!("{name}-far-rgba16f.bin")),bytes).map_err(|e|e.to_string())?;
                let mut raw_mean=[0.0f64;4];
                for &(x,y) in &roi {let p=raw[((y/2)*w+x/2) as usize];
                    for c in 0..4 {raw_mean[c]+=p[c] as f64/roi.len() as f64;}}
                let alpha_image:Vec<u8>=raw.iter().flat_map(|p|{let a=(p[3]*255.0).round() as u8;[a,a,a,255]}).collect();
                render::write_png(&out.join(format!("{name}-far-alpha.png")),*w,*h,&alpha_image)?;
                let diagnostics=renderer.far_grass_diagnostics(&gpu)?.ok_or("missing far diagnostics")?;
                let d=diagnostics.first().ok_or("missing far diagnostics view")?;
                if d.overflow_rays!=0 {return Err(format!("{name}: {} incomplete far rays",d.overflow_rays));}
                let entry=json!({"name":name,"distance_horizontal_m":distance,"component":component,"reference":mode,
                    "camera":{"position":camera.pos.to_array(),"target":camera.target.to_array(),"fov_y":camera.fov_y},
                    "size":size,"grass_detail":source["grass_detail"],"roi_pixels":roi.len(),"display_rgb_mean":mean,
                    "display_luma_mean":mean[0]*0.2126+mean[1]*0.7152+mean[2]*0.0722,
                    "far_raw_size":[w,h],"far_raw_premul_linear_rgba_mean":raw_mean,
                    "far_raw_unpremultiplied_mean_rgb":if raw_mean[3]>0.0 {Some([raw_mean[0]/raw_mean[3],raw_mean[1]/raw_mean[3],raw_mean[2]/raw_mean[3]])}else{None},
                    "near_coverage":if mode=="near"&&component=="albedo" {near_coverage}else{None},
                    "far_traced_rays":d.traced_rays,"far_overflow_rays":d.overflow_rays,"far_max_iterations":d.max_iterations,
                    "gpu_ms":stats.gpu_ms,"wall_with_readback_ms":wall_ms});
                write_json(&out.join(format!("{name}.json")),&entry)?;
                mode_samples.push(entry.clone());captures.push(entry);
            }
            comparisons.push(comparison(distance,component,&mode_samples,near_coverage));
        }
    }
    let report=json!({"schema":1,"status":"diagnostic_completed_not_quality_pass","adapter":gpu.info.name,
        "requested":{"distances_m":options.distances,"size":size,
            "components":options.components.iter().map(|v|v.0).collect::<Vec<_>>(),
            "modes":options.modes.iter().map(|v|v.0).collect::<Vec<_>>()},
        "fixture":source,"captures":captures,"comparisons":comparisons,
        "limits":["Actual current engine source-photo cards and statistical field; no image generation or Godot comparison.",
            "Same fixed world scene/instances/materials for all requested modes. When requested, near-only intentionally draws the whole resident patch beyond normal fade; timings are not a runtime streaming or production cost comparison.",
            "Flat sunny patch isolates representation and coverage. It does not prove forest shadow equivalence or fix a gameplay screenshot.",
            "PNG means are display-space values after tonemapping. Only saved RGBA16F far layers are linear premultiplied HDR with real alpha.",
            "Near coverage is recorded only when the albedo near-only reference is requested; it is actual opaque visibility ownership. Blend transition ownership is not inferred from the main depth buffer.",
            "Far raw layers are before the representation compositor, while blend PNG includes the actual distance handoff.",
            "Wind/TAA/DLSS disabled; no desktop window, input or gameplay FPS claim."]});
    write_json(&out.join("report.json"),&report)?;
    println!("grass transition diagnostic: {} actual GPU captures; {}",captures.len(),out.display());
    Ok(())
}
fn main(){if let Err(error)=run(){eprintln!("grass transition diagnostic: {error}");std::process::exit(1);}}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn all_reference_cameras_have_a_real_world_patch_roi(){for d in DISTANCES {
        let pixels=roi(&camera(d),SIZE);assert!(pixels.len()>=200);assert!(pixels.iter().all(|&(x,y)|x<SIZE[0]&&y<SIZE[1]));
    }}
    #[test] fn options_preserve_full_defaults_and_allow_bounded_subsets() {
        let args=|values:&[&str]|values.iter().map(|v|v.to_string()).collect::<Vec<_>>();
        let defaults=parse_options(&args(&["--out","unused"])).unwrap();
        assert_eq!(defaults.size,SIZE);assert_eq!(defaults.distances,DISTANCES);
        assert_eq!(defaults.components,COMPONENTS);assert_eq!(defaults.modes,MODES);
        let selected=parse_options(&args(&["--out","unused","--distances","50,80,100.5","--components","lit",
            "--modes","blend","--size","960x540"])).unwrap();
        assert_eq!(selected.distances,[50.0,80.0,100.5]);assert_eq!(selected.components,[("lit",0.0)]);
        assert_eq!(selected.modes,[("blend",0.0)]);assert_eq!(selected.size,[960,540]);
        for distance in selected.distances {
            let pixels=roi(&camera(distance),selected.size);assert!(pixels.len()>=200);
            assert!(pixels.iter().all(|&(x,y)|x<selected.size[0]&&y<selected.size[1]));
        }
        for extra in [["--distances","NaN"],["--distances","50,50.0"],["--components","lit,lit"],
            ["--components","unknown"],["--modes",""],["--size","0x360"],["--size","999999x1"]] {
            assert!(parse_options(&args(&["--out","unused",extra[0],extra[1]])).is_err());
        }
    }
    #[test] fn comparisons_require_requested_references_and_ignore_mode_order() {
        let entry=|name:&str,luma:f64|json!({"reference":name,"display_luma_mean":luma,"far_raw_premul_linear_rgba_mean":[0.1,0.2,0.3,0.75]});
        let blend=comparison(80.0,"lit",&[entry("blend",0.5)],None);
        assert!(blend["far_minus_near_display_luma"].is_null());assert!(blend["blend_minus_near_display_luma"].is_null());
        assert_eq!(blend["far_alpha_mean"],0.75);assert!(blend["far_to_actual_near_coverage_ratio"].is_null());
        let reordered=comparison(50.0,"albedo",&[entry("blend",0.5),entry("far",0.8),entry("near",0.2)],Some(0.5));
        assert!((reordered["far_minus_near_display_luma"].as_f64().unwrap()-0.6).abs()<1e-12);
        assert!((reordered["blend_minus_near_display_luma"].as_f64().unwrap()-0.3).abs()<1e-12);
        assert_eq!(reordered["far_to_actual_near_coverage_ratio"],1.5);
    }
    #[test] fn decode_does_not_hide_nonfinite_gpu_values(){assert_eq!(half(0x3c00),1.0);assert!(half(0x7c00).is_infinite());assert!(half(0x7e00).is_nan());}
}
