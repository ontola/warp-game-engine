//! Static explosion scalability probe; real Effects recipes, native HDR, no window.
//! cargo run --locked -p render --example baked_smoke_scaling -- --size 3840x2160 --views 4 --out build/smoke-scale-4k
//! GPU timestamps exclude compilation, upload, allocation and readback. Not game FPS.
use glam::{Affine3A, Quat, Vec3};
use render::fx::Effects;
use render::{Camera, Gpu, Lighting, Renderer, Scene, ViewRect};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

const WARMUP: usize = 12;
const AGE: f32 = 0.85;
const BIRTH_INTERVAL: f32 = 0.0001;
// Metadata derived from the current Effects recipe, not a GPU counter.
const BAKED_CAP: usize = 16;

#[derive(Clone, Copy, PartialEq)]
enum Layout { Spread, Overlap, Inside }
impl Layout {
    fn name(self) -> &'static str { match self { Self::Spread=>"spread", Self::Overlap=>"overlap", Self::Inside=>"inside" } }
    fn radius(self) -> f32 { if self==Self::Inside {6.5} else {3.3} }
    fn position(self, index: usize, count: usize) -> Vec3 {
        if self!=Self::Spread {return Vec3::ZERO;}
        let cols=(count as f32).sqrt().ceil().max(1.0) as usize;
        let rows=count.div_ceil(cols);
        Vec3::new((index%cols) as f32-(cols-1) as f32*0.5,0.0,(index/cols) as f32-(rows-1) as f32*0.5)*5.0
    }
    fn cameras(self) -> [Camera;4] {
        let (distance,height,target)=match self {
            Self::Spread=>(36.0,18.0,Vec3::new(0.0,3.0,0.0)),
            Self::Overlap=>(18.0,7.0,Vec3::new(0.0,3.0,0.0)),
            Self::Inside=>(0.4,2.0,Vec3::new(0.0,3.0,0.0)),
        };
        std::array::from_fn(|i| {
            let angle=i as f32*std::f32::consts::FRAC_PI_2;
            let pos=Vec3::new(angle.sin()*distance,height,angle.cos()*distance);
            let target=if self==Self::Inside {pos+Vec3::new(-angle.sin(),0.15,-angle.cos())} else {target};
            Camera {pos,target,fov_y:55f32.to_radians(),near:0.05}
        })
    }
}

fn quantiles(mut values:Vec<f64>)->Value {
    values.sort_by(f64::total_cmp);
    let n=values.len();
    json!({"samples":n,"median_ms":values[n/2],"p95_ms":values[((n as f64*0.95).ceil() as usize-1).min(n-1)],
        "min_ms":values[0],"max_ms":values[n-1]})
}

fn measure(gpu:&Gpu,renderer:&Renderer,light:&Lighting,size:[u32;2],views:&[ViewRect],layout:Layout,
    radius:f32,count:usize,mode:&str,age:f32,samples:usize,out:&Path,captures:bool)->Result<Value,String> {
    let time=age+count.saturating_sub(1) as f32*BIRTH_INTERVAL;
    renderer.set_effects(gpu,time,Some(&[]))?;
    let mut fx=Effects::default();
    fx.set_baked_smoke(mode=="baked");
    for i in 0..count {
        fx.explosion(i as f32*BIRTH_INTERVAL,layout.position(i,count),radius,Vec3::new(1.0,0.7,0.2));
    }
    fx.flush(gpu,renderer,time)?;
    renderer.set_local_lights(&fx.lights(time))?;
    let mut by_pass:BTreeMap<&'static str,Vec<f64>>=BTreeMap::new();
    let mut instrumented=Vec::new();
    for frame in 0..samples+WARMUP {
        let (_,stats)=renderer.try_render_texture_profiled(gpu,views,light,size[0],size[1])?;
        if frame<WARMUP {continue;}
        if !stats.gpu_ms.iter().any(|(name,_)|*name=="fx") {return Err("FX timestamp scope missing; cannot report a valid measurement".into());}
        // FrameStats already aggregates every view's root scope under its name.
        // Sum only to report instrumented root work, never elapsed frame/union time.
        let mut sum=0.0;
        for (name,ms) in stats.gpu_ms {
            if !ms.is_finite() || ms<0.0 {return Err(format!("invalid {name} timestamp: {ms}"));}
            by_pass.entry(name).or_default().push(ms);
            sum+=ms;
        }
        instrumented.push(sum);
    }
    if by_pass.values().any(|values|values.len()!=samples) {return Err("incomplete per-pass sample coverage".into());}
    let passes:BTreeMap<_,_>=by_pass.into_iter().map(|(name,times)|(name,quantiles(times))).collect();
    let image=if captures && mode=="baked" && count==16 {
        let filename=format!("{}-{}-age{:.2}-radius{:.1}.png",layout.name(),mode,age,radius);
        let (pixels,_)=renderer.try_render_views(gpu,views,light,size[0],size[1],&[])?;
        render::write_png(&out.join(&filename),size[0],size[1],&pixels)?;
        Some(filename)
    } else {None};
    println!("{} {mode} count={count} radius={radius:.1} age={age:.2}: fx {} ms median / {} p95",
        layout.name(),passes["fx"]["median_ms"],passes["fx"]["p95_ms"]);
    Ok(json!({"layout":layout.name(),"mode":mode,"requested_explosions":count,"radius_m":radius,
        "newest_age_seconds":age,"oldest_age_seconds":time,"birth_interval_seconds":BIRTH_INTERVAL,
        "live_puffs_after_flush":fx.len(),
        "expected_baked_volumes_from_source_rule":if mode=="baked" {count.min(BAKED_CAP)} else {0},
        "volume_count_is_observed":false,"per_pass":passes,"instrumented_root_pass_sum":quantiles(instrumented),"capture":image}))
}

fn run()->Result<(),String> {
    let args:Vec<_>=std::env::args().skip(1).collect();
    let arg=|key:&str|args.windows(2).find(|a|a[0]==key).map(|a|a[1].as_str());
    if args.iter().any(|a|a=="--help") {
        println!("baked_smoke_scaling --out DIR --size WIDTHxHEIGHT --views 1|4 --samples 24 --layout all|spread|overlap|inside [--radius METRES] [--counts 0,1,4,8,16,32] [--captures]");
        return Ok(());
    }
    let size=match arg("--size").unwrap_or("1920x1080").split_once('x') {
        Some((w,h))=>[w.parse::<u32>().map_err(|_|"invalid width")?,h.parse::<u32>().map_err(|_|"invalid height")?],
        None=>return Err("size must be WIDTHxHEIGHT".into()),
    };
    if size.iter().any(|v|*v<128||*v>4096||v%2!=0) {return Err("size must be even, 128..4096 on each axis".into());}
    let view_count:usize=arg("--views").unwrap_or("4").parse().map_err(|_|"invalid view count")?;
    if ![1,4].contains(&view_count) {return Err("views must be 1 or 4".into());}
    let samples:usize=arg("--samples").unwrap_or("24").parse().map_err(|_|"invalid samples")?;
    if !(8..=240).contains(&samples) {return Err("samples must be 8..240".into());}
    let layouts=match arg("--layout").unwrap_or("all") {
        "all"=>vec![Layout::Spread,Layout::Overlap,Layout::Inside],"spread"=>vec![Layout::Spread],
        "overlap"=>vec![Layout::Overlap],"inside"=>vec![Layout::Inside],_=>return Err("invalid layout".into()),
    };
    let radius_override=arg("--radius").map(|v|v.parse::<f32>().map_err(|_|"invalid radius")).transpose()?;
    if radius_override.is_some_and(|r|!r.is_finite()||!(0.5..=100.0).contains(&r)) {return Err("radius must be finite, 0.5..100 metres".into());}
    let counts:Vec<usize>=arg("--counts").unwrap_or("0,1,4,8,16,32").split(',').map(|v|v.parse().map_err(|_|"invalid count")).collect::<Result<_,_>>()?;
    if counts.is_empty() || counts.iter().any(|n|*n>64) {return Err("counts must be 0..64".into());}
    let out=Path::new(arg("--out").unwrap_or("build/smoke-scaling"));
    std::fs::create_dir_all(out).map_err(|e|e.to_string())?;
    let captures=args.iter().any(|a|a=="--captures");
    let gpu=Gpu::new()?;
    if !gpu.features.contains(wgpu::Features::TIMESTAMP_QUERY|wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS) {
        return Err("adapter lacks required GPU timestamp queries; no wall-time or zero-value substitute".into());
    }
    let mut scene=Scene::default();
    scene.sky=Some((16,8,vec![[0.4,0.5,0.65,1.0];128]));
    let mat=scene.add_material(render::scene::Material{tint:[0.45,0.48,0.52],layer:-1,roughness:1.0,flags:0,scale:1.0,metal:0.0});
    let mesh=scene.add_mesh(&render::primitives::cube(mat).mesh());
    scene.add_instance(mesh,Affine3A::from_scale_rotation_translation(Vec3::new(100.0,0.2,100.0),Quat::IDENTITY,Vec3::new(0.0,-0.2,0.0)));
    let light=Lighting{taa:0.0,ev100:10.0,sun_lux:800.0,sky_lux:600.0,gi:0.0,shadow_strength:0.0,near_shadows:0.0,
        contact_occlusion:0.0,reflection_occlusion:0.0,fog_density:0.0,sharpen:0.0,lod_pixels:0.0,time:1.0,..Default::default()};
    let renderer=Renderer::new(&gpu,&scene,&light);
    // Initialize the FX slot and shared density asset before even the zero-count
    // baseline, so every case has the same available scopes and warm resources.
    let mut prime=Effects::default();
    prime.set_baked_smoke(true);
    prime.explosion(0.0,Vec3::ZERO,3.3,Vec3::ONE);
    prime.flush(&gpu,&renderer,AGE)?;
    let mut results=Vec::new();
    let mut layout_views=Vec::new();
    for layout in layouts {
        let cameras=layout.cameras();
        let views:Vec<_>=render::split_rects(view_count,size[0],size[1]).iter().zip(cameras).map(|(r,cam)|ViewRect{cam,x:r.0,y:r.1,w:r.2,h:r.3}).collect();
        layout_views.push(json!({"layout":layout.name(),"views":views.iter().map(|v|json!({"rect":[v.x,v.y,v.w,v.h],
            "camera_position":v.cam.pos.to_array(),"camera_target":v.cam.target.to_array(),"vertical_fov_degrees":55,"near_m":0.05})).collect::<Vec<_>>()}));
        let radius=radius_override.unwrap_or_else(||layout.radius());
        for (index,&count) in counts.iter().enumerate() {
            // Alternate paired recipe order instead of always timing baked second.
            let modes=if index%2==0 {["legacy","baked"]} else {["baked","legacy"]};
            for mode in modes {results.push(measure(&gpu,&renderer,&light,size,&views,layout,radius,count,mode,AGE,samples,out,captures)?);}
        }
        if layout!=Layout::Spread {
            for age in [3.2,4.3] {for mode in ["legacy","baked"] {
                results.push(measure(&gpu,&renderer,&light,size,&views,layout,radius,16,mode,age,samples,out,captures)?);
            }}
        }
    }
    let report=json!({"adapter":{"name":gpu.info.name,"backend":format!("{:?}",gpu.info.backend),"driver":gpu.info.driver},
        "output_size":size,"view_count":view_count,"layout_views":layout_views,
        "reconstruction":"native HDR, no TAA or spatial upscaler","warmup_frames_per_case":WARMUP,"measured_frames_per_case":samples,
        "scope":"GPU root-pass timestamps in static floor-only fixture using full Effects.explosion recipe and its local lights; excludes gameplay simulation, destruction, crowd, UI, startup, upload and image readback. Not gameplay FPS.",
        "timing_aggregation":"Per-pass values already sum all views. instrumented_root_pass_sum is root-scope work, not covered elapsed GPU time or FPS.",
        "allocation":"Shared 6.75 MiB smoke asset is primed before measurements; cold first-explosion upload is excluded.",
        "baked_volume_cap_from_source":BAKED_CAP,"baked_volume_counts":"Expected from current source rule, not observed/read back. Counts above16 retain other explosion effects but recycle old smoke volumes.",
        "layout_notes":{"spread":"Centres are 5m apart on a centred grid; cameras stay outside default-radius volumes.",
            "overlap":"All explosions share one centre; cameras outside default-radius volumes.",
            "inside":"All explosions share one centre; four cameras sit within the default 6.5m-radius smoke box. Larger --radius overrides may also contain cameras in other layouts."},
        "cases":results});
    std::fs::write(out.join("report.json"),serde_json::to_vec_pretty(&report).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    println!("Saved {} cases to {}",report["cases"].as_array().unwrap().len(),out.join("report.json").display());
    Ok(())
}
fn main(){if let Err(error)=run(){eprintln!("smoke scaling: {error}");std::process::exit(1);}}
