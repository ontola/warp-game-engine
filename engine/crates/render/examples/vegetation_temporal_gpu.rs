//! Explicit serial GPU/NGX acceptance, not a gameplay performance benchmark.
//! Checks the real volume integrator's opacity-weighted motion against an
//! independent affine oracle. Every occupied branch shares one moving root,
//! making the weighted centroid's reprojection exactly affine, even in overlap.
use glam::{Mat4, Vec2, Vec3, Vec4};
use render::{Camera, Gpu, Lighting, Renderer, Scene, ViewRect};
use render::foliage_assembly::{self, Limits, Parameters};
use render::foliage_representation::Policy;
use render::foliage_voxel::{BakeConfig, ProjectionMeasure};
use render::vegetation_asset::{AttachLimits, CookedPlant, PlantInstance, SceneVegetation};
use render::scene::GeometryCategory;
use render::upscale::UpscaleMode;
use render::wind::WindState;
use serde_json::json;

fn scene(volume:bool)->Result<Scene,String> {
    let mut scene=Scene::default();
    scene.sky=Some((16,8,vec![[0.6,0.7,0.9,1.];128]));
    let recipe=foliage_assembly::generate(Parameters::meadow_grass(101),Limits::default())?;
    let cooked=CookedPlant::cook(&recipe,BakeConfig {cell_size:0.025,levels:4,
        projection_measure:ProjectionMeasure::ClosedSurfaceEntries,max_samples:4_000_000,
        max_voxels:131072,max_texture_texels:0,..Default::default()})?;
    let mut builder=SceneVegetation::new(&mut scene,AttachLimits::default())?;
    builder.append(&cooked,PlantInstance {id:7,generation:1,root:Mat4::from_scale(Vec3::new(1.1,0.95,0.8)),category:GeometryCategory::Grass})?;
    builder.finish(Policy {max_error_pixels:100_000.,allow_unvalidated_optics:volume,..Default::default()})?;
    for branch in &mut scene.part_wind {branch.max_angle=if branch.parent.is_none(){0.24}else{0.};}
    // Finest mip only: this test isolates animation, not optical equivalence.
    for binding in &mut scene.foliage_representations {binding.group.candidates.truncate(1);binding.group.radius+=0.3;}
    Ok(scene)
}
fn radical(mut i:u32,base:u32)->f32 {let(mut f,mut r)=(1.,0.);while i>0 {f/=base as f32;r+=f*(i%base)as f32;i/=base;}r-0.5}
fn ray(camera:&Camera,w:u32,h:u32,frame:u32,x:u32,y:u32)->Vec3 {
    let mut projection=Mat4::perspective_infinite_reverse_rh(camera.fov_y,w as f32/h as f32,camera.near);
    projection.z_axis.x+=2.*radical(frame%32+1,2)/w as f32;
    projection.z_axis.y+=2.*radical(frame%32+1,3)/h as f32;
    let inv=(projection*Mat4::look_at_rh(camera.pos,camera.target,Vec3::Y)).inverse();
    let point=inv*Vec4::new(2.*(x as f32+0.5)/w as f32-1.,1.-2.*(y as f32+0.5)/h as f32,1e-4,1.);
    (point.truncate()/point.w-camera.pos).normalize()
}
fn views(count:usize)->Vec<ViewRect> {
    (0..count).map(|i| {let angle=i as f32*0.7;
        ViewRect {cam:Camera {pos:Vec3::new(angle.sin()*3.,1.,angle.cos()*3.),target:Vec3::new(0.,0.23,0.),fov_y:50f32.to_radians(),near:0.05},
            x:(i as u32%2)*1920,y:(i as u32/2)*1080,w:1920,h:1080}
    }).collect()
}
fn half(bits:u16)->f32 {
    let exponent=(bits>>10)&31;let fraction=bits&1023;
    if exponent==0 {return (if bits&0x8000==0 {1.}else{-1.})*fraction as f32/16_777_216.;}
    f32::from_bits(((bits as u32&0x8000)<<16)|((if exponent==31 {255}else{exponent as u32+112})<<23)|((fraction as u32)<<13))
}
fn run()->Result<(),String> {
    let args:Vec<_>=std::env::args().collect();
    let out=std::path::PathBuf::from(args.windows(2).find(|a|a[0]=="--out").ok_or("--out FRESH_DIR required")?[1].clone());
    if out.exists(){return Err("output must be fresh".into());}std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    let gpu=Gpu::new()?;
    let light=Lighting {gi:0.,near_shadows:0.,fog_culling:false,fog_start:1e6,fog_end:2e6,taa:0.,shadow_strength:0.,..Default::default()};
    let mut records=Vec::new();
    for volume in [false,true] {for count in [1usize,4] {
        let scene=scene(volume)?;let renderer=Renderer::new(&gpu,&scene,&light);
        renderer.set_upscale(&gpu,UpscaleMode::DlssQuality)?;
        let views=views(count);let output=if count==1 {(1920,1080)}else{(3840,2160)};
        let mut previous=Mat4::IDENTITY;
        for (frame,time) in [0.,0.25,0.5,0.5,0.75].into_iter().enumerate() {
            let wind=WindState {strength:0.8,time_seconds:time,direction:Vec2::new(0.8,0.6),..Default::default()};
            let current=render::part_wind::sample_poses(&scene.part_wind,&wind)?[0];
            renderer.set_wind(&gpu,&wind)?;
            let (rgba,stats)=renderer.try_render_views(&gpu,&views,&light,output.0,output.1,&[])?;
            let status=renderer.upscale_status();
            if status.mode!=UpscaleMode::DlssQuality||status.views.len()!=count {return Err("NGX was not evaluated for every view".into());}
            let layers=renderer.voxel_motion_layers_rgba32(&gpu)?;
            let colors=renderer.voxel_layers_rgba16(&gpu)?;
            if layers.len()!=count||colors.len()!=count {return Err("missing temporal volume target".into());}
            let mut valid=0usize;let mut maximum=0f32;let mut movement=0f32;let mut finite=true;
            let mut per_view=Vec::new();let mut views_pass=true;
            for view in 0..count {
                let(w,h,data)=&layers[view];let(cw,ch,color)=&colors[view];
                if (*w,*h)!=(*cw,*ch)||[*w,*h]!=status.views[view].input||data.len()!=(*w as usize**h as usize)*16 {return Err("invalid volume motion target dimensions".into());}
                let mut covered=0usize;let mut missing=0usize;let mut view_valid=0usize;let mut view_motion=0f32;
                for (pixel,bytes) in data.chunks_exact(16).enumerate() {
                    let value:[f32;4]=std::array::from_fn(|c|f32::from_le_bytes(bytes[c*4..c*4+4].try_into().unwrap()));
                    finite &= value.iter().all(|v|v.is_finite());
                    let alpha=half(u16::from_le_bytes([color[pixel*8+6],color[pixel*8+7]]));
                    if alpha<0.05 {continue;}covered+=1;
                    if frame==0 {continue;}
                    if value[3]<=0. {missing+=1;continue;}
                    let point=views[view].cam.pos+ray(&views[view].cam,*w,*h,frame as u32,pixel as u32%*w,pixel as u32/ *w)*value[3];
                    let expected=(previous*current.inverse()).transform_point3(point)-point;
                    let actual=Vec3::from_slice(&value[..3]);
                    maximum=maximum.max(actual.distance(expected));movement=movement.max(actual.length());valid+=1;
                    view_valid+=1;view_motion=view_motion.max(actual.length());
                }
                let view_pass=if volume {covered>20&&(frame==0||(missing==0&&view_valid==covered&&(frame==3||view_motion>0.001)))}else{covered==0};
                views_pass &= view_pass;
                per_view.push(json!({"view":view,"covered":covered,"missing_metadata":missing,"valid":view_valid,"max_motion_m":view_motion,"pass":view_pass}));
            }
            let pass=finite&&views_pass&&maximum<0.0005&&(!volume||frame==0||valid>20)
                &&(!volume||frame==0||frame==3||movement>0.001);
            records.push(json!({"volume":volume,"views":count,"frame":frame,"time":time,"input_sizes":status.views.iter().map(|v|v.input).collect::<Vec<_>>(),
                "output":output,"samples":valid,"max_affine_delta_error_m":maximum,"max_motion_m":movement,"finite":finite,"pass":pass,"per_view":per_view,
                "gpu_passes":stats.gpu_ms,"scope":"Actual fractional volume producer and real NGX execution; identical affine root motion across all overlapping branches. Not an optical/ghosting quality claim."}));
            if frame==4 {image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(output.0,output.1,rgba).ok_or("image shape")?)
                .resize(1600,900,image::imageops::FilterType::Triangle).to_rgb8()
                .save(out.join(format!("{}-{count}view.jpg",if volume{"volume"}else{"source"}))).map_err(|e|e.to_string())?;}
            previous=current;
        }
        renderer.set_upscale(&gpu,UpscaleMode::Native)?;
        renderer.try_render_views(&gpu,&views,&light,output.0,output.1,&[])?;
        if !renderer.voxel_motion_layers_rgba32(&gpu)?.is_empty(){return Err("native retained temporal output".into());}
    }}
    let passed=records.iter().all(|r|r["pass"]==true);
    std::fs::write(out.join("report.json"),serde_json::to_vec_pretty(&json!({"passed":passed,"cases":records})).unwrap()).map_err(|e|e.to_string())?;
    if passed {println!("vegetation temporal GPU/NGX PASS");Ok(())}else{Err("vegetation temporal producer oracle failed; see report.json".into())}
}
fn main(){if let Err(error)=run(){eprintln!("{error}");std::process::exit(1);}}
