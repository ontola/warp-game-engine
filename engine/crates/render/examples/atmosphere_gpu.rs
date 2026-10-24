//! Four real-renderer sky/lighting captures. CPU LUT builds are configuration
//! work, not per-frame work. Run only when root owns the GPU window.
use glam::{Affine3A,Vec3};
use render::{Camera,Gpu,Lighting,Renderer,Scene};
use render::atmosphere::{Atmosphere,AtmosphereCache,BakeQuality,SolarState};
use render::cluster::{ClusterMesh,Part,Vertex};
use render::scene::Material;
use std::{path::PathBuf,time::Instant};

fn box_mesh(scene:&mut Scene,size:Vec3,tint:[f32;3])->usize {
    let material=scene.add_material(Material {tint,layer:-1,roughness:0.8,flags:0,scale:1.0,metal:0.0});
    let mut vertices=Vec::new();let mut indices=Vec::new();
    for (normal,u,v) in [(Vec3::X,Vec3::NEG_Z,Vec3::Y),(Vec3::NEG_X,Vec3::Z,Vec3::Y),
        (Vec3::Y,Vec3::X,Vec3::NEG_Z),(Vec3::NEG_Y,Vec3::X,Vec3::Z),
        (Vec3::Z,Vec3::X,Vec3::Y),(Vec3::NEG_Z,Vec3::NEG_X,Vec3::Y)] {
        let base=vertices.len()as u32;
        for (a,b) in [(-1.0,-1.0),(1.0,-1.0),(1.0,1.0),(-1.0,1.0)] {
            vertices.push(Vertex {pos:((normal+u*a+v*b)*size*0.5).to_array(),normal:normal.to_array(),uv:[a,b]});
        }
        indices.extend([base,base+1,base+2,base,base+2,base+3]);
    }
    scene.add_mesh(&ClusterMesh::build(&[Part {vertices:&vertices,indices:&indices,material}]))
}
fn scene()->Scene {
    let mut scene=Scene::default();
    let ground=box_mesh(&mut scene,Vec3::new(240.0,0.2,240.0),[0.22,0.24,0.20]);
    scene.add_instance(ground,Affine3A::from_translation(Vec3::new(0.0,-0.1,-35.0)));
    for (index,tint) in [[0.65,0.65,0.65],[0.30,0.025,0.01],[0.025,0.2,0.04],[0.015,0.03,0.25]].into_iter().enumerate() {
        let mesh=box_mesh(&mut scene,Vec3::new(2.5,3.8,2.5),tint);
        scene.add_instance(mesh,Affine3A::from_translation(Vec3::new(index as f32*5.0-7.5,1.9,-16.0)));
    }
    scene
}
fn run()->Result<(),String> {
    let args:Vec<_>=std::env::args().collect();
    let out=PathBuf::from(args.windows(2).find(|pair|pair[0]=="--out").ok_or("use --out FRESH_DIRECTORY")?[1].clone());
    if out.exists() {return Err("atmosphere output directory must be new".into());}
    std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    let gpu=Gpu::new()?;let mut cache=AtmosphereCache::default();let model=Atmosphere::default();
    let quality=BakeQuality::default();let mut records=Vec::new();
    let camera=Camera {pos:Vec3::new(15.0,6.0,12.0),target:Vec3::new(0.0,3.0,-25.0),fov_y:70f32.to_radians(),near:0.1};
    let mut renderer=None;
    let mut first_noon=None;
    let mut reused_images_equal=false;
    for (name,elevation) in [("morning",8.0),("noon",60.0),("sunset",1.0),("night",-20.0),("noon_return",60.0)] {
        // Look towards the changing solar horizon so the same view exposes
        // the scattering colour as well as light and shadows on the posts.
        let begin=Instant::now();let solar=SolarState::from_degrees(elevation,345.0)?;
        let sky=cache.get_or_bake(&model,solar,quality)?;let bake_ms=begin.elapsed().as_secs_f64()*1000.0;
        let mut scene=scene();
        let mut light=Lighting {taa:0.0,sharpen:0.0,ev100:14.0,fog_start:20_000.0,fog_end:30_000.0,
            gi:0.0,near_shadows:0.0,contact_occlusion:0.0,reflection_occlusion:0.0,..Default::default()};
        sky.apply(&mut scene,&mut light);
        if renderer.is_none() {renderer=Some(Renderer::new(&gpu,&scene,&light));}
        let renderer=renderer.as_mut().unwrap();
        renderer.update_sky(&gpu,&sky,&mut light)?;
        let (pixels,stats)=renderer.render(&gpu,&camera,&light,960,540);
        if name=="noon" {first_noon=Some(pixels.clone());}
        if name=="noon_return" {
            reused_images_equal=first_noon.as_ref()==Some(&pixels);
            if !reused_images_equal {return Err("live sky return did not reproduce original noon image".into());}
            // Rejected updates must leave both the texture and sunlight intact.
            let mut invalid=(*sky).clone();invalid.pixels[0][0]=f32::NAN;
            if renderer.update_sky(&gpu,&invalid,&mut light).is_ok(){return Err("accepted invalid sky".into());}
            let (unchanged,_)=renderer.render(&gpu,&camera,&light,960,540);
            if unchanged!=pixels {return Err("rejected sky update changed live state".into());}
            let views:Vec<_>=render::split_rects(4,960,540).iter().map(|r|render::ViewRect {
                cam:camera,x:r.0,y:r.1,w:r.2,h:r.3,
            }).collect();
            let (four,_)=renderer.try_render_views(&gpu,&views,&light,960,540,&[])?;
            render::write_png(&out.join("noon_four_views.png"),960,540,&four)?;
        }
        render::write_png(&out.join(format!("{name}.png")),960,540,&pixels)?;
        records.push(format!("{{\"name\":\"{name}\",\"solar_elevation_degrees\":{elevation},\"solar_azimuth_degrees\":345,\"ev100\":14,\"bake_ms\":{bake_ms},\"sun_rgb_lux\":{:?},\"sky_horizontal_rgb_lux\":{:?},\"retained_triangles\":{}}}",
            sky.sun_rgb_lux.to_array(),sky.sky_horizontal_rgb_lux.to_array(),stats.triangles));
        println!("{name}: bake {bake_ms:.2}ms, sun {:?}, sky {:?}",sky.sun_rgb_lux,sky.sky_horizontal_rgb_lux);
    }
    let report=format!("{{\"schema\":1,\"mode\":\"single_scattering_cpu_lut_foundation\",\"real_renderer\":true,\"native_size\":[960,540],\"shared_transmittance_bakes\":{},\"sky_bakes\":{},\"sky_lut\":[{},{}],\"transmittance_lut\":[{},{}],\"fixed_exposure\":true,\"limitations\":[\"no multiple scattering\",\"no volumetric clouds\",\"no aerial perspective LUT\",\"no visible solar disk, moon, stars\",\"no GPU timing or four-player performance claim\"],\"cases\":[{}]}}",
        cache.transmittance_bakes,cache.sky_bakes,quality.sky_width,quality.sky_height,quality.transmittance_width,quality.transmittance_height,records.join(","));
    std::fs::write(out.join("report.json"),report).map_err(|e|e.to_string())?;
    std::fs::write(out.join("live-update-checks.json"),format!("{{\"renderer_instances\":1,\"in_place_updates\":5,\"return_to_noon_pixel_equal\":{reused_images_equal},\"invalid_update_preserved_pixels\":true,\"four_views_rendered\":true}}"))
        .map_err(|e|e.to_string())?;Ok(())
}
fn main(){if let Err(error)=run(){eprintln!("{error}");std::process::exit(1);}}
