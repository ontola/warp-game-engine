//! One closed leaf, exact source and two explicit optical conventions.
//! This diagnoses coverage; a mechanically valid report is not a visual pass.
//! Run only when the live GPU playtest is stopped:
//! cargo run --release -p cli --example foliage_leaf_optics_gpu -- --out FRESH_DIR
use glam::{Affine3A, Quat, Vec3};
use render::{Camera, Gpu, Lighting, Renderer, Scene, ViewRect, split_rects};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::foliage_voxel::{self, BakeConfig, ProjectionMeasure, VoxelAsset};
use render::foliage_volume_data::{self, ALL_MATERIALS, PackLimits, VolumeBinding};
use render::scene::{Material, THIN_FOLIAGE, TWO_SIDED};
use serde_json::{Value,json};
use std::path::PathBuf;

const W:u32=960;
const H:u32=540;
const SIZE:Vec3=Vec3::new(0.07,0.8,0.003);

fn leaf()->(Vec<Vertex>,Vec<u32>) {
    let p:Vec<_>=(0..8).map(|i|SIZE*Vec3::new(if i&1==0{-0.5}else{0.5},
        if i&2==0{-0.5}else{0.5},if i&4==0{-0.5}else{0.5})).collect();
    let mut vertices=Vec::new();let mut indices=Vec::new();
    for face in [[0,4,6,2],[1,3,7,5],[0,1,5,4],[2,6,7,3],[0,2,3,1],[4,5,7,6]] {
        for t in [[0,1,2],[0,2,3]] {
            let [a,b,c]=t.map(|j|p[face[j]]);let normal=(b-a).cross(c-a).normalize().to_array();
            for q in [a,b,c] {indices.push(vertices.len() as u32);vertices.push(Vertex {pos:q.to_array(),normal,uv:[0.0;2]});}
        }
    }
    (vertices,indices)
}

fn scene(vertices:&[Vertex],indices:&[u32],rotation:Quat,asset:Option<&VoxelAsset>,active_tiles:bool)->Result<Scene,String> {
    let mut scene=Scene::default();
    scene.voxel_active_tiles=active_tiles;
    let sky=PathBuf::from(env!("CARGO_MANIFEST_DIR")).ancestors().nth(3).unwrap().join("assets/sky/sky_2k.hdr");
    scene.load_sky(&sky)?;
    let material=scene.add_material(Material {tint:[0.11,0.22,0.04],layer:-1,roughness:0.9,
        flags:THIN_FOLIAGE|TWO_SIDED,scale:1.0,metal:0.0});
    let mesh=scene.add_mesh(&ClusterMesh::build(&[Part {vertices,indices,material}]));
    scene.add_instance(mesh,Affine3A::from_quat(rotation));
    if let Some(asset)=asset {
        scene.voxel_forest=Some(foliage_volume_data::pack(std::slice::from_ref(asset),
            &[VolumeBinding {owner:0,asset:0,crown_material:ALL_MATERIALS}],&scene.instances,
            PackLimits {levels_per_asset:4,first_level_bounds:false,..Default::default()})?);
        // Fixed finest level isolates optics from the independent LOD policy.
        scene.voxel_lod_pixels=0.0;
    }
    Ok(scene)
}

fn half(bits:u16)->f32 {
    let exponent=(bits>>10)&31;let fraction=bits&1023;
    if exponent==0 {let v=fraction as f32/16_777_216.0;return if bits&0x8000!=0{-v}else{v};}
    f32::from_bits(((bits as u32&0x8000)<<16)|((if exponent==31{255}else{exponent as u32+112})<<23)|((fraction as u32)<<13))
}
fn mask_image(alpha:&[f32])->Vec<u8> {alpha.iter().flat_map(|v|{let c=(v.clamp(0.0,1.0)*255.0).round() as u8;[c,c,c,255]}).collect()}

fn check_reuse(gpu:&Gpu,renderer:&Renderer,camera:Camera,light:&Lighting,out:&std::path::Path)->Result<Vec<Value>,String> {
    let empty=Camera {pos:camera.pos+Vec3::X*30.0,target:camera.target+Vec3::X*30.0,..camera};
    let mut checks=Vec::new();
    for (label,width,height,occupied) in [("empty-after-occupied",W,H,vec![false]),
        ("odd-resize-occupied",W+1,H+1,vec![true]),("mixed-four-views",W+1,H+1,vec![true,false,true,false]),
        ("empty-after-mixed-four",W+1,H+1,vec![false;4])] {
        let rectangles=split_rects(occupied.len(),width,height);
        let views:Vec<_>=rectangles.iter().zip(&occupied).map(|(&(x,y,w,h),&filled)|ViewRect {
            cam:if filled{camera}else{empty},x,y,w,h}).collect();
        let (_,stats)=renderer.render_views(gpu,&views,light,width,height,&[]);
        let layers=renderer.voxel_layers_rgba16(gpu)?;
        if layers.len()!=occupied.len(){return Err(format!("{label}: stale view count"));}
        let mut layer_rows=Vec::new();
        for (index,((w,h,bytes),&filled)) in layers.iter().zip(&occupied).enumerate() {
            if (*w,*h)!=(views[index].w,views[index].h)||bytes.len()!=*w as usize**h as usize*8 {
                return Err(format!("{label}: stale view dimensions"));
            }
            let mut alpha=0.0f64;
            for p in bytes.chunks_exact(8) {
                let values:[f32;4]=std::array::from_fn(|i|half(u16::from_le_bytes([p[2*i],p[2*i+1]])));
                if values.iter().any(|v|!v.is_finite()){return Err(format!("{label}: non-finite raw layer"));}
                alpha+=values[3] as f64;
            }
            let all_zero=bytes.iter().all(|&b|b==0);
            if (!filled&&!all_zero)||(filled&&alpha<=0.0) {return Err(format!("{label} view {index}: stale or missing volume output"));}
            let file=format!("reuse-{label}-view{index}.rgba16f");
            std::fs::write(out.join(&file),bytes).map_err(|e|e.to_string())?;
            layer_rows.push(json!({"view":index,"size":[w,h],"expect_occupied":filled,"all_bytes_zero":all_zero,
                "alpha_sum":alpha,"raw_file":file}));
        }
        checks.push(json!({"case":label,"layers":layer_rows,"gpu_ms":stats.gpu_ms}));
    }
    Ok(checks)
}

fn run(out:PathBuf,active_tiles:bool)->Result<(),String> {
    if out.as_os_str().is_empty()||out.exists(){return Err("--out must name a fresh directory".into());}
    let (vertices,indices)=leaf();
    let materials=[Material {tint:[0.11,0.22,0.04],layer:-1,roughness:0.9,
        flags:THIN_FOLIAGE|TWO_SIDED,scale:1.0,metal:0.0}];
    let config=BakeConfig {cell_size:0.01,levels:4,samples_per_cell:2,
        max_samples:2_000_000,max_voxels:131_072,..Default::default()};
    let parts=[Part {vertices:&vertices,indices:&indices,material:0}];
    let sheets=foliage_voxel::bake(&parts,&materials,&[],config)?;
    let closed=foliage_voxel::bake(&parts,&materials,&[],BakeConfig {
        projection_measure:ProjectionMeasure::ClosedSurfaceEntries,..config})?;
    let mut oracle=Vec::new();
    for (mip,level) in closed.levels.iter().enumerate() {
        for (direction,d) in foliage_voxel::PROJECTION_DIRECTIONS.into_iter().enumerate() {
            let d=Vec3::from(d).abs();let exact=SIZE.y*SIZE.z*d.x+SIZE.x*SIZE.z*d.y+SIZE.x*SIZE.y*d.z;
            let actual:f32=level.voxels.iter().map(|v|v.directional_area[direction]).sum();
            if (exact-actual).abs()>exact*3e-5+1e-8{return Err("analytic single-leaf entry-area oracle failed".into());}
            oracle.push(json!({"mip":mip,"direction":direction,"expected_m2":exact,"actual_m2":actual}));
        }
    }
    std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    let gpu=Gpu::new()?;
    let light=Lighting {taa:0.0,sharpen:0.0,lod_pixels:0.0,gi:0.0,contact_occlusion:0.0,
        reflection_occlusion:0.0,fog_density:0.0,fog_start:10000.0,fog_end:20000.0,..Default::default()};
    let camera=Camera {pos:Vec3::new(0.0,0.0,1.5),target:Vec3::ZERO,fov_y:40.0f32.to_radians(),near:0.05};
    let picks:Vec<_>=(0..H).flat_map(|y|(0..W).map(move|x|(x,y))).collect();
    let mut rows=Vec::new();let mut reuse_checks=Vec::new();
    for degrees in [0.0f32,45.0,80.0] {
        let rotation=Quat::from_rotation_y(degrees.to_radians());let mut source_mask=Vec::new();
        for (mode,asset) in [("source",None),("sheet-crossings",Some(&sheets)),("closed-entries",Some(&closed))] {
            let scene=scene(&vertices,&indices,rotation,asset,active_tiles)?;let renderer=Renderer::new(&gpu,&scene,&light);
            let (pixels,stats)=renderer.render_picking(&gpu,&camera,&light,W,H,if asset.is_none(){&picks}else{&[]});
            let label=format!("leaf-{degrees:02.0}-{mode}");render::write_png(&out.join(format!("{label}.png")),W,H,&pixels)?;
            let alpha=if asset.is_none() {
                source_mask=stats.picks.iter().map(|p|if p.is_some(){1.0}else{0.0}).collect::<Vec<f32>>();source_mask.clone()
            }else {
                let layers=renderer.voxel_layers_rgba16(&gpu)?;let (w,h,bytes)=layers.first().ok_or("missing volume alpha")?;
                if (*w,*h)!=(W,H){return Err("wrong volume dimensions".into());}
                std::fs::write(out.join(format!("{label}-volume.rgba16f")),bytes).map_err(|e|e.to_string())?;
                let mut alpha=Vec::new();
                for p in bytes.chunks_exact(8) {
                    let rgba:[f32;4]=std::array::from_fn(|i|half(u16::from_le_bytes([p[2*i],p[2*i+1]])));
                    if rgba.iter().any(|v|!v.is_finite())||!(0.0..=1.001).contains(&rgba[3]) {return Err("invalid volume sample".into());}
                    alpha.push(rgba[3]);
                }
                for d in renderer.voxel_diagnostics(&gpu)?.unwrap_or_default() {
                    if d.step_overflow_rays!=0{return Err("incomplete single-leaf traversal".into());}
                }
                alpha
            };
            render::write_png(&out.join(format!("{label}-alpha.png")),W,H,&mask_image(&alpha))?;
            let sum=|a:&[f32]|a.iter().map(|&v|v as f64).sum::<f64>();
            let source_area=sum(&source_mask);let area=sum(&alpha);
            let outside:f64=alpha.iter().zip(&source_mask).map(|(a,b)|(a*(1.0-b)) as f64).sum();
            let missed:f64=alpha.iter().zip(&source_mask).map(|(a,b)|((1.0-a)*b) as f64).sum();
            let row=json!({"mode":mode,"angle_degrees":degrees,"source_area_pixels":source_area,"area_pixels":area,
                "area_ratio":area/source_area.max(1.0),"outside_source_alpha_sum":outside,
                "missing_inside_source_alpha_sum":missed,"gpu_ms":stats.gpu_ms});
            println!("{row}");rows.push(row);
            if degrees==0.0&&mode=="closed-entries" {reuse_checks=check_reuse(&gpu,&renderer,camera,&light,&out)?;}
        }
    }
    let report:Value=json!({"schema":1,"purpose":"one closed opaque leaf optical diagnostic; no visual-acceptance assertion",
        "adapter":gpu.info.name,"size":[W,H],"leaf_metres":SIZE.to_array(),"cell_metres":config.cell_size,
        "active_tiles":active_tiles,"raw_volume_format":"row-major RGBA little-endian IEEE754 binary16; premultiplied integrated RGB and alpha; 960x540",
        "analytic_projected_entry_area_passed":true,"closed_components":closed.stats.closed_components_checked,
        "analytic_oracle":oracle,"runs":rows,"reuse_checks":reuse_checks,"limitations":["Beer attenuation does not reconstruct opaque correlated leaf coverage",
            "native binary source silhouette has sampling error","projected-area conservation is necessary but not sufficient for image equivalence",
            "isolated leaf timings are not scene or gameplay performance"]});
    std::fs::write(out.join("report.json"),serde_json::to_vec_pretty(&report).map_err(|e|e.to_string())?).map_err(|e|e.to_string())
}
fn main(){let args:Vec<_>=std::env::args().skip(1).collect();let result=if args.len()==2&&args[0]=="--out" {
    run(PathBuf::from(&args[1]),false)}else if args.len()==4&&args[0]=="--out"&&args[2]=="--active-tiles"&&["true","false"].contains(&args[3].as_str()) {
        run(PathBuf::from(&args[1]),args[3]=="true")
    }else{Err("use --out FRESH_DIRECTORY [--active-tiles true|false]".into())};
    if let Err(e)=result{eprintln!("foliage_leaf_optics_gpu: {e}");std::process::exit(1);}}
