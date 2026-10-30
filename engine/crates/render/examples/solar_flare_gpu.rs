//! Real GPU regression: clear, partially covered and hidden solar disc, plus
//! independent split cameras. No CPU visibility readback drives the effect.
use glam::{Affine3A, Quat, Vec3};
use render::{Camera, Gpu, Lighting, Renderer, Scene, ViewRect};
use render::atmosphere::{Atmosphere, AtmosphereCache, BakeQuality, SolarState};
use std::path::PathBuf;
fn run() -> Result<(),String> {
    let out=PathBuf::from(std::env::args().nth(1).ok_or("use solar_flare_gpu OUTPUT_DIRECTORY")?);
    std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    let mut gpu=Gpu::new()?;
    // Correctness images do not need Metal's occasionally invalid timestamps.
    gpu.features.remove(wgpu::Features::TIMESTAMP_QUERY);
    let sky=AtmosphereCache::default().get_or_bake(&Atmosphere::default(),SolarState::from_degrees(10.0,0.0)?,BakeQuality::default())?;
    let mut scene=Scene::default();
    let material=scene.add_material(render::scene::Material {tint:[0.35,0.37,0.4],layer:-1,roughness:0.8,flags:0,scale:1.0,metal:0.0});
    let cube=scene.add_mesh(&render::primitives::cube(material).mesh());
    for (size,pos) in [(Vec3::new(3.0,8.0,1.0),Vec3::new(0.0,4.0,-12.0)),(Vec3::new(200.0,0.2,200.0),Vec3::new(0.0,-0.1,0.0))] {
        scene.add_instance(cube,Affine3A::from_scale_rotation_translation(size,Quat::IDENTITY,pos));
    }
    let mut light=Lighting {taa:0.0,sharpen:0.0,ev100:12.0,gi:0.0,shadow_strength:0.0,contact_occlusion:0.0,reflection_occlusion:0.0,fog_density:0.0,fog_start:20000.0,fog_end:30000.0,..Default::default()};
    sky.apply(&mut scene,&mut light);
    let renderer=Renderer::new(&gpu,&scene,&light);
    let cam=|x:f32| { let pos=Vec3::new(x,2.0,10.0); Camera {pos,target:pos+(sky.sun_dir+Vec3::new(0.25,-0.06,0.0))*50.0,fov_y:65f32.to_radians(),near:0.1} };
    let (w,h)=(960,540);
    let mut clear=0u64;
    let mut partial=0u64;
    for (name,camera) in [("clear",cam(6.0)),("half",cam(1.5)),("blocked",cam(0.0))] {
        let (with,_)=renderer.render(&gpu,&camera,&light,w,h);
        let mut off=light.clone();off.lens_flare=0.0;
        let (without,_)=renderer.render(&gpu,&camera,&off,w,h);
        let delta:u64=with.iter().zip(&without).map(|(a,b)|a.abs_diff(*b) as u64).sum();
        // Compare the same unobstructed sky region: whole-frame display
        // deltas depend on the dark wall's placement under the optical glare.
        let sky_delta:u64=with[..(w*h/3*4) as usize].iter().zip(&without).map(|(a,b)|a.abs_diff(*b) as u64).sum();
        match name {"clear"=>{clear=sky_delta;assert!(delta>10000,"visible solar flare: {delta}");},"half"=>partial=sky_delta,_=>assert_eq!(delta,0,"building must fully hide flare")}
        if name=="blocked" { off.sun_disc=false;let (hidden,_)=renderer.render(&gpu,&camera,&off,w,h); assert_eq!(hidden,without,"building must hide solar disc too"); }
        render::write_png(&out.join(format!("{name}.png")),w,h,&with)?;
        render::write_png(&out.join(format!("{name}-no-flare.png")),w,h,&without)?;
        println!("{name}: flare pixel delta {delta}");
    }
    assert!(partial>0 && partial<clear,"partial coverage: {partial}, clear {clear}");
    let cameras=[cam(6.0),cam(0.0),cam(1.5),Camera {target:cam(6.0).pos-sky.sun_dir*50.0,..cam(6.0)}];
    let views:Vec<_>=render::split_rects(4,w,h).iter().zip(cameras).map(|(r,cam)|ViewRect {cam,x:r.0,y:r.1,w:r.2,h:r.3}).collect();
    let (four,_)=renderer.try_render_views(&gpu,&views,&light,w,h,&[])?;
    for v in &views {
        let (single,_)=renderer.render(&gpu,&v.cam,&light,v.w,v.h);
        for y in 0..v.h {
            let dst=(((v.y+y)*w+v.x)*4) as usize;
            let src=(y*v.w*4) as usize;
            assert_eq!(&four[dst..dst+(v.w*4) as usize],&single[src..src+(v.w*4) as usize],"split camera must match independent rendering");
        }
    }
    render::write_png(&out.join("four-views.png"),w,h,&four)?;
    std::fs::write(out.join("checks.json"),format!("{{\"comparison_region\":\"top_third_sky\",\"clear_pixel_delta\":{clear},\"partial_pixel_delta\":{partial},\"blocked_pixel_delta\":0,\"split_views_pixel_equal\":true}}\n")).map_err(|e|e.to_string())?;
    Ok(())
}
fn main() {if let Err(e)=run() {eprintln!("{e}");std::process::exit(1);}}
