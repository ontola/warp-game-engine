//! Explicit optical diagnostic, not a performance or optical-parity acceptance.
//! Run without a simultaneous game: --out FRESH_DIRECTORY [--max-mip 0..3].
//! Source coverage uses actual 4K owner pixels resolved 2x2 to a 1080p panel.
//! Volume coverage uses its raw RGBA16F layer, never a colour-threshold mask.

use glam::{Affine3A, Mat4, Quat, Vec2, Vec3};
use render::{Camera, Gpu, Lighting, Renderer, Scene, ViewRect};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::foliage_assembly::{self, Limits, Parameters};
use render::foliage_representation::Policy;
use render::foliage_voxel::{BakeConfig, ProjectionMeasure};
use render::scene::{GeometryCategory, Material};
use render::vegetation_accounting::Accounting;
use render::vegetation_asset::{AttachLimits, CookedPlant, PlantInstance, SceneVegetation};
use render::wind::WindState;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const W:u32=1920;
const H:u32=1080;
const ALBEDO_RADIANCE:f32=6000.;

fn base()->Scene {
    let mut scene=Scene::default();
    scene.sky=Some((16,8,vec![[0.,0.,0.,1.];128]));
    let material=scene.add_material(Material {tint:[0.18;3],layer:-1,roughness:1.,flags:0,scale:1.,metal:0.});
    let vertices=[[-500.,-0.02,500.],[500.,-0.02,500.],[-500.,-0.02,-500.],[500.,-0.02,-500.]]
        .map(|pos|Vertex {pos,normal:Vec3::Y.to_array(),uv:[0.;2]});
    let mesh=scene.add_mesh(&ClusterMesh::build(&[Part {vertices:&vertices,indices:&[0,1,2,1,3,2],material}]));
    scene.add_instance(mesh,Affine3A::IDENTITY);
    scene
}

fn scene(plants:&[CookedPlant],count:usize,forced_mip:Option<u32>)->Result<Scene,String> {
    let mut scene=base();
    let mut builder=SceneVegetation::new(&mut scene,AttachLimits::default())?;
    for i in 0..count {
        let position=if count==1 {Vec3::ZERO} else {Vec3::new((i%3)as f32-1.,0.,(i/3)as f32-1.)*0.45};
        let root=Mat4::from_scale_rotation_translation(Vec3::ONE,Quat::from_rotation_y(i as f32*0.71),position);
        builder.append(&plants[i%plants.len()],PlantInstance {id:i as u64,generation:1,root,category:GeometryCategory::Grass})?;
    }
    builder.finish(Policy::default())?;
    if let Some(mip)=forced_mip {
        // An error_radius of zero would lie about reconstruction support and
        // fail the engine validators. Keep the real conservative radius, but
        // explicitly relax this diagnostic's selection threshold. Retaining
        // exactly one candidate fixes its maximum mip without changing data.
        for binding in &mut scene.foliage_representations {
            binding.group.candidates.retain(|candidate|candidate.level==mip);
            if binding.group.candidates.len()!=1 {return Err("requested mip missing from cooked grass".into());}
        }
        scene.foliage_representation_policy=Policy {max_error_pixels:1_000_000.,
            hysteresis_fraction:0.,allow_unvalidated_optics:true,..Default::default()};
    }
    // This stays at the real footprint setting: the relaxed selection policy
    // must not also coarsen reconstruction in the volume integrator.
    scene.voxel_lod_pixels=1.;
    scene.voxel_active_tiles=true;
    render::foliage_representation::validate_bindings(&scene)?;
    Ok(scene)
}

fn camera(distance:f32,angle:f32)->Camera {
    Camera {pos:Vec3::new(angle.sin()*distance,0.65+distance*0.07,angle.cos()*distance),
        target:Vec3::new(0.,0.30,0.),fov_y:50f32.to_radians(),near:0.05}
}
fn light(angle:f32,back:bool,albedo:bool)->Lighting {
    let towards_camera=Vec3::new(angle.sin(),0.,angle.cos());
    Lighting {sun_dir:(towards_camera*if back {-0.85}else{0.85}+Vec3::Y*0.55).normalize(),
        sun_lux:100_000.,sun_color:Vec3::ONE,sky_lux:0.,shadow_strength:0.,
        gi:0.,contact_occlusion:0.,reflection_occlusion:0.,near_shadows:0.,
        fog_density:0.,fog_start:10000.,fog_end:20000.,fog_culling:false,
        taa:0.,sharpen:0.,lod_pixels:0.,debug:if albedo {2.}else{0.},..Default::default()}
}
fn half(value:u16)->f32 {
    let sign=if value&0x8000==0 {1.}else{-1.};let exponent=(value>>10)&31;let fraction=(value&1023)as f32;
    match exponent {0=>sign*fraction*2f32.powi(-24),31=>if fraction==0. {sign*f32::INFINITY}else{f32::NAN},
        _=>sign*(1.+fraction/1024.)*2f32.powi(exponent as i32-15)}
}
fn linear(byte:u8)->f32 {let x=byte as f32/255.;if x<=0.04045 {x/12.92}else{((x+0.055)/1.055).powf(2.4)}}
fn display(value:f32)->u8 {let x=value.clamp(0.,1.);((if x<=0.0031308 {12.92*x}else{1.055*x.powf(1./2.4)-0.055})*255.).round()as u8}
fn luma(c:[f32;3])->f64 {c[0]as f64*0.2126+c[1]as f64*0.7152+c[2]as f64*0.0722}

struct Capture {
    /// Resolved display-linear composite (already tonemapped), not scene HDR.
    rgb:Vec<[f32;3]>,
    alpha:Vec<f32>,
    /// Source: visible material tint weighted by 2x2 coverage. Volume:
    /// raw premultiplied HDR; /6000 yields albedo only in debug=2 captures.
    premultiplied:Vec<[f32;3]>,
    selection:Value,
    source_plant_subpixels:Vec<u64>,
    raw_alpha_histogram:[u64;5],
}

fn capture(gpu:&Gpu,renderer:&Renderer,scene:&Scene,accounting:&Accounting,cam:Camera,
    lighting:&Lighting,count:usize,volume:bool,max_mip:u32)->Result<Capture,String> {
    let scale=if volume {1}else{2};let width=W*scale;let height=H*scale;
    let view=ViewRect {cam,x:0,y:0,w:width,h:height};
    let (pixels,stats)=renderer.try_render_views(gpu,&[view],lighting,width,height,&[])?;
    let selected=renderer.debug_visible_clusters(gpu,0,None)?;
    let counts=accounting.view(&selected,stats.cluster_buckets)?;
    let grass=counts.categories.iter().find(|c|c.category==GeometryCategory::Grass).ok_or("grass accounting missing")?;
    let groups=(count*4)as u64;
    if volume {
        if grass.selected_source_groups!=0||grass.selected_volume_groups!=groups||grass.raster.triangles!=0
            ||grass.selected_mips.iter().any(|m|m.max_mip!=max_mip) {
            return Err(format!("forced volume did not select all {groups} groups: {grass:?}"));
        }
    } else if grass.selected_source_groups!=groups||grass.selected_volume_groups!=0||grass.raster.triangles!=(count*792)as u64 {
        return Err(format!("finest source selection mismatch: {grass:?}"));
    }
    let owner_pixels=renderer.debug_geometry_owner_pixels(gpu,0,Some(selected.render_sequence))?;
    if owner_pixels.input_size!=[width,height]||pixels.len()!=(width*height*4)as usize {
        return Err("unexpected native capture dimensions".into());
    }
    let mut owners=vec![None;scene.instances.len()];
    for binding in &scene.foliage_representations {
        let source=scene.foliage_source_groups.iter().find(|g|g.owner==binding.owner).ok_or("missing source membership")?;
        for &owner in &source.members {
            let instance=&scene.instances[owner as usize];
            let material=scene.materials[scene.clusters[instance.first_cluster as usize].material as usize];
            owners[owner as usize]=Some((binding.group.key.instance as usize,material.tint));
        }
    }
    let owner=|i:usize|owners.get(owner_pixels.owners[i]as usize).copied().flatten();
    let mut result=Capture {rgb:vec![[0.;3];(W*H)as usize],alpha:vec![0.;(W*H)as usize],
        premultiplied:vec![[0.;3];(W*H)as usize],selection:Value::Null,
        source_plant_subpixels:vec![0;count],raw_alpha_histogram:[0;5]};
    if volume {
        if (0..owner_pixels.owners.len()).any(|i|owner(i).is_some()) {return Err("source pixels survived forced volume selection".into());}
        let mut layers=renderer.voxel_layers_rgba16(gpu)?;
        if layers.len()!=1 {return Err("expected one raw volume layer".into());}
        let (lw,lh,bytes)=layers.remove(0);
        if [lw,lh]!=[W,H]||bytes.len()!=(W*H*8)as usize {return Err("raw volume resolution mismatch".into());}
        for i in 0..result.alpha.len() {
            let raw:[f32;4]=std::array::from_fn(|c|half(u16::from_le_bytes([bytes[i*8+c*2],bytes[i*8+c*2+1]])));
            if raw.iter().any(|v|!v.is_finite())||!(0.0..=1.0).contains(&raw[3]) {return Err("invalid raw volume colour/alpha".into());}
            result.alpha[i]=raw[3];result.premultiplied[i]=[raw[0],raw[1],raw[2]];
            result.rgb[i]=std::array::from_fn(|c|linear(pixels[i*4+c]));
        }
    } else {
        for y in 0..H {for x in 0..W {
            let dst=(y*W+x)as usize;let mut covered=0;
            for dy in 0..2 {for dx in 0..2 {
                let src=((y*2+dy)*width+x*2+dx)as usize;
                for channel in 0..3 {result.rgb[dst][channel]+=linear(pixels[src*4+channel])*0.25;}
                if let Some((plant,tint))=owner(src) {
                    covered+=1;result.source_plant_subpixels[plant]+=1;
                    for channel in 0..3 {result.premultiplied[dst][channel]+=tint[channel]*0.25;}
                }
            }}
            result.alpha[dst]=covered as f32*0.25;result.raw_alpha_histogram[covered]+=1;
        }}
    }
    let diagnostics=renderer.voxel_diagnostics(gpu)?.ok_or("missing voxel counters")?;
    if diagnostics.len()!=1 {return Err("unexpected voxel diagnostic view count".into());}
    let d=&diagnostics[0];
    if d.step_overflow_rays!=0 {return Err(format!("incomplete volume integration: {} rays",d.step_overflow_rays));}
    if !volume&&d.traced_rays!=0 {return Err("source-only view still traced volumes".into());}
    result.selection=json!({"render_sequence":selected.render_sequence,"input_size":[width,height],
        "source_groups":grass.selected_source_groups,"volume_groups":grass.selected_volume_groups,
        "colour_triangles":grass.raster.triangles,"clusters":grass.raster.clusters,
        "maximum_mips":grass.selected_mips.iter().map(|m|json!({"mip":m.max_mip,"instances":m.volume_instances,
            "occupied_cells":m.instance_occupied_cells})).collect::<Vec<_>>(),
        "volume_work":{"traced_rays":d.traced_rays,"total_steps":d.total_steps,"volume_samples":d.volume_samples,
            "occupied_cell_samples":d.occupied_cell_samples,"aabb_interval_tests":d.aabb_interval_tests,
            "active_tiles":d.active_tiles,"scheduled_pixels":d.scheduled_pixels,
            "candidate_overflow_tiles":d.candidate_overflow_tiles,"ray_candidate_overflow_rays":d.ray_candidate_overflow_rays,
            "active_cache_overflow_rays":d.active_cache_overflow_rays,"step_overflow_rays":d.step_overflow_rays}});
    Ok(result)
}

fn roi(source:&Capture,volume:&Capture)->(Vec<bool>,[u32;4]) {
    let mut roi=vec![false;(W*H)as usize];let mut min=[W,H];let mut max=[0,0];let mut found=false;
    for i in 0..roi.len() {if source.alpha[i]>0.||volume.alpha[i]>1./1024. {
        found=true;let x=i as u32%W;let y=i as u32/W;
        min[0]=min[0].min(x);min[1]=min[1].min(y);max[0]=max[0].max(x);max[1]=max[1].max(y);
        for yy in y.saturating_sub(2)..=(y+2).min(H-1) {for xx in x.saturating_sub(2)..=(x+2).min(W-1) {roi[(yy*W+xx)as usize]=true;}}
    }}
    if !found {return (roi,[W/2-16,H/2-16,32,32]);}
    let x=min[0].saturating_sub(12);let y=min[1].saturating_sub(12);
    (roi,[x,y,(max[0]+13).min(W)-x,(max[1]+13).min(H)-y])
}

fn metrics(source:&Capture,volume:&Capture,albedo:bool)->Value {
    let (roi,bounds)=roi(source,volume);let roi_pixels=roi.iter().filter(|&&inside|inside).count();
    let mut source_coverage=0.;let mut volume_coverage=0.;let mut missing=0.;let mut excess=0.;
    let mut abs_alpha=0.;let mut sq_alpha=0.;let mut rgb_error=0.;let mut srgb_error=0.;
    let mut source_luma=0.;let mut volume_luma=0.;let mut source_colour=[0.;3];let mut volume_colour=[0.;3];
    let mut source_centroid=[0.;2];let mut volume_centroid=[0.;2];
    let mut alpha_counts=[0u64;5];let thresholds=[0.,0.01,0.1,0.5,0.9];
    for i in 0..source.alpha.len() {
        let a=source.alpha[i]as f64;let b=volume.alpha[i]as f64;
        source_coverage+=a;volume_coverage+=b;missing+=(a-b).max(0.);excess+=(b-a).max(0.);
        for (n,&threshold) in thresholds.iter().enumerate() {alpha_counts[n]+=u64::from(b>threshold);}
        let position=[(i as u32%W)as f64+0.5,(i as u32/W)as f64+0.5];
        for k in 0..2 {source_centroid[k]+=position[k]*a;volume_centroid[k]+=position[k]*b;}
        for k in 0..3 {source_colour[k]+=source.premultiplied[i][k]as f64;
            volume_colour[k]+=volume.premultiplied[i][k]as f64/if albedo {ALBEDO_RADIANCE as f64}else{1.};}
        if roi[i] {
            abs_alpha+=(a-b).abs();sq_alpha+=(a-b).powi(2);
            source_luma+=luma(source.rgb[i]);volume_luma+=luma(volume.rgb[i]);
            for k in 0..3 {rgb_error+=(source.rgb[i][k]-volume.rgb[i][k]).abs()as f64;
                srgb_error+=display(source.rgb[i][k]).abs_diff(display(volume.rgb[i][k]))as f64;}
        }
    }
    let n=roi_pixels.max(1)as f64;
    json!({"roi_pixels":roi_pixels,"crop_xywh":bounds,"source_coverage_pixels":source_coverage,
        "volume_alpha_sum_pixels":volume_coverage,"coverage_ratio":if source_coverage>0. {Some(volume_coverage/source_coverage)}else{None},
        "missing_alpha_sum_pixels":missing,"excess_alpha_sum_pixels":excess,
        "roi_alpha_mae":abs_alpha/n,"roi_alpha_rmse":(sq_alpha/n).sqrt(),
        "source_subpixel_histogram_0_to_4":source.raw_alpha_histogram,
        "source_visible_4k_subpixels_by_plant":source.source_plant_subpixels,
        "volume_alpha_thresholds_strictly_greater":thresholds,"volume_pixels_above_threshold":alpha_counts,
        "source_centroid_pixels":source_centroid.map(|v|v/source_coverage.max(1e-30)),
        "volume_centroid_pixels":volume_centroid.map(|v|v/volume_coverage.max(1e-30)),
        "roi_display_linear_rgb_mae":rgb_error/(n*3.),"roi_srgb8_rgb_mae":srgb_error/(n*3.),
        "roi_source_display_linear_luma":source_luma/n,"roi_volume_display_linear_luma":volume_luma/n,
        "roi_display_linear_luma_bias":(volume_luma-source_luma)/n,
        "source_coverage_weighted_material_albedo":source_colour.map(|v|v/source_coverage.max(1e-30)),
        "volume_coverage_weighted_value":volume_colour.map(|v|v/volume_coverage.max(1e-30)),
        "volume_value_units":if albedo {"linear albedo, raw premultiplied debug HDR divided by6000 and accumulated alpha"}
            else {"linear HDR radiance per unit alpha; NOT directly comparable to source material albedo or tonemapped RGB"},
        "metric_scope":"ROI is union of source coverage and volume alpha>1/1024, dilated2px. Coverage sums include every panel pixel. Patch metrics are the visible union, not isolated per-plant volume contributions."})
}

fn jpeg_pair(path:&Path,source:&Capture,volume:&Capture,bounds:[u32;4],coverage:bool)->Result<(),String> {
    let [x,y,w,h]=bounds;let mut image=image::RgbImage::new(w*2,h);
    for row in 0..h {for col in 0..w {let i=((y+row)*W+x+col)as usize;
        for (side,capture) in [source,volume].into_iter().enumerate() {
            let pixel=if coverage {[(capture.alpha[i].clamp(0.,1.)*255.).round()as u8;3]}
                else {capture.rgb[i].map(display)};
            image.put_pixel(col+side as u32*w,row,image::Rgb(pixel));
        }
    }}
    // Nearest-neighbour enlargement preserves the pixel structure at long
    // range; JPEG is a preview only, never the source of numerical metrics.
    let zoom=(512/w.max(h).max(1)).clamp(1,8);
    let image=image::imageops::resize(&image,w*2*zoom,h*zoom,image::imageops::FilterType::Nearest);
    let file=std::fs::File::create(path).map_err(|e|e.to_string())?;
    image::codecs::jpeg::JpegEncoder::new_with_quality(file,95).encode_image(&image).map_err(|e|e.to_string())
}

fn save(out:&Path,max_mip:u32,cases:&[Value],complete:bool)->Result<(),String> {
    let value=json!({"complete":complete,"technical_checks_passed":complete,"optical_parity":"not_asserted",
        "max_mip":max_mip,"source_resolution":[W*2,H*2],"volume_resolution":[W,H],
        "source_coverage_resolve":"mean of four actual native visibility owners; a finite2x2 estimate, not exact continuous coverage",
        "display_rgb_resolve":"average four source pixels in display-linear space after tonemapping, then encode sRGB for previews",
        "recipe":"meadow_grass_v1","wind":{"strength":0.8,"time_seconds":1.9,"direction":[0.8,0.6]},
        "lighting":"directional sun only; black sky, no GI/contact/fog/shadows; same fixed pose in both representations",
        "forced_volume":"one real conservative candidate; max_error_pixels1000000; allow_unvalidated_optics=true; volume footprint remains1px",
        "previews":"JPEG pairs: source left, volume right; coverage pairs show alpha; numerical comparisons precede JPEG encoding",
        "scope":"Bounded native optical diagnosis. No DLSS, no motion acceptance, no gameplay/FPS claim. Failures of optical equivalence are measurements, not suppressed test failures.",
        "cases":cases});
    std::fs::write(out.join("report.json"),serde_json::to_vec_pretty(&value).map_err(|e|e.to_string())?).map_err(|e|e.to_string())
}

fn run()->Result<(),String> {
    let args:Vec<_>=std::env::args().collect();
    let out=PathBuf::from(&args.windows(2).find(|a|a[0]=="--out").ok_or("--out FRESH_DIRECTORY required")?[1]);
    let max_mip:u32=args.windows(2).find(|a|a[0]=="--max-mip").map(|a|a[1].parse()).transpose()
        .map_err(|_|"invalid --max-mip")?.unwrap_or(0);
    if max_mip>3 {return Err("--max-mip must be0..3".into());}
    if out.exists() {return Err("output directory already exists".into());}
    std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    // Process-local fixture configuration before any device/thread creation.
    unsafe {
        std::env::set_var("KERN_DIAGNOSTIC_HIDE","none");
        for key in ["KERN_WORLD_TERRAIN_VISIBILITY","KERN_PAGED_FOLIAGE_SUN","KERN_FAR_SUN_SHADOW"] {std::env::set_var(key,"0");}
    }
    let config=BakeConfig {projection_measure:ProjectionMeasure::ClosedSurfaceEntries,cell_size:0.025,levels:4,
        samples_per_cell:4,max_samples:4_000_000,max_voxels:131_072,max_texture_texels:0,..Default::default()};
    let plants=[101,303,707].into_iter().map(|seed|
        CookedPlant::cook(&foliage_assembly::generate(Parameters::meadow_grass(seed),Limits::default())?,config))
        .collect::<Result<Vec<_>,String>>()?;
    let gpu=Gpu::new()?;let wind=WindState {strength:0.8,time_seconds:1.9,direction:Vec2::new(0.8,0.6),..Default::default()};
    let mut cases=Vec::new();save(&out,max_mip,&cases,false)?;
    for count in [1,9] {
        let source=scene(&plants,count,None)?;let volume=scene(&plants,count,Some(max_mip))?;
        let source_account=Accounting::new(&source)?;let volume_account=Accounting::new(&volume)?;
        let source_renderer=Renderer::new(&gpu,&source,&light(0.,false,false));
        let volume_renderer=Renderer::new(&gpu,&volume,&light(0.,false,false));
        source_renderer.set_wind(&gpu,&wind)?;volume_renderer.set_wind(&gpu,&wind)?;
        let mut configurations=Vec::new();
        for distance in [50.,75.,100.] {for angle in if count==1 {vec![0.]}else{vec![0.,0.8]} {for back in [false,true] {
            configurations.push((distance,angle,back));
        }}}
        if count==1 {configurations.insert(0,(5.,0.,false));}
        for (distance,angle,back) in configurations {
            let name=format!("{}-{}m-angle{}-{}",if count==1 {"clump"}else{"patch3x3"},distance,angle,if back {"back"}else{"front"});
            let cam=camera(distance,angle);let mut captures=Vec::new();
            for albedo in [false,true] {
                let lighting=light(angle,back,albedo);
                let a=capture(&gpu,&source_renderer,&source,&source_account,cam,&lighting,count,false,max_mip)?;
                let b=capture(&gpu,&volume_renderer,&volume,&volume_account,cam,&lighting,count,true,max_mip)?;
                if a.source_plant_subpixels.iter().sum::<u64>()==0 {return Err(format!("source grass absent in {name}"));}
                let measurements=metrics(&a,&b,albedo);let (_,bounds)=roi(&a,&b);
                let suffix=if albedo {"albedo"}else{"sun"};
                jpeg_pair(&out.join(format!("{name}-{suffix}.jpg")),&a,&b,bounds,false)?;
                if !albedo {jpeg_pair(&out.join(format!("{name}-coverage.jpg")),&a,&b,bounds,true)?;}
                captures.push(json!({"mode":suffix,"metrics":measurements,"source_selection":a.selection,"volume_selection":b.selection,
                    "preview":format!("{name}-{suffix}.jpg")}));
            }
            println!("grass transition {name}: coverage_ratio={}, display_luma_bias={}",
                captures[0]["metrics"]["coverage_ratio"],captures[0]["metrics"]["roi_display_linear_luma_bias"]);
            cases.push(json!({"name":name,"plants":count,"distance_parameter_m":distance,"angle_radians":angle,
                "sun":if back {"behind plants, towards camera"}else{"front-lit from camera side"},
                "regime":if distance<10. {"near shape preview; forced volume not ordinary subpixel selection"}else{"transition diagnosis"},
                "camera":{"position":cam.pos.to_array(),"target":cam.target.to_array(),"fov_y_degrees":50},
                "coverage_preview":format!("{name}-coverage.jpg"),"captures":captures}));
            save(&out,max_mip,&cases,false)?;
        }
    }
    save(&out,max_mip,&cases,true)?;println!("Completed {} optical diagnostic cases; parity is not asserted.",cases.len());Ok(())
}
fn main() {if let Err(error)=run() {eprintln!("grass transition probe: {error}");std::process::exit(1);}}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_half_and_display_conversion_keep_zero_fraction_and_one() {
        assert_eq!(half(0),0.);assert_eq!(half(0x3400),0.25);assert_eq!(half(0x3c00),1.);
        assert_eq!(half(1),2f32.powi(-24));assert!(half(0x7c00).is_infinite());assert!(half(0x7e00).is_nan());
        for byte in 0..=255 {assert_eq!(display(linear(byte)),byte);}
    }
}
