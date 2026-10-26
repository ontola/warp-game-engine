//! Sun-only GPU regression: remote terrain blockers, near/far agreement and
//! an unoccluded receiver. Uses real shadow raster/filtering, not gameplay FPS.
use glam::{Affine3A,Vec3};
use render::{Camera,Gpu,Lighting,Renderer,Scene};
use render::cluster::{ClusterMesh,Part,Vertex};
use render::scene::Material;
use std::path::PathBuf;
const W:u32=640;const H:u32=384;
fn quad(scene:&mut Scene,p:[[f32;3];4],n:Vec3)->usize {
    let material=scene.add_material(Material{tint:[0.5;3],layer:-1,roughness:1.,flags:0,scale:1.,metal:0.});
    let vertices:Vec<_>=p.into_iter().map(|pos|Vertex{pos,normal:n.to_array(),uv:[0.;2]}).collect();
    scene.add_mesh(&ClusterMesh::build(&[Part{vertices:&vertices,indices:&[0,1,2,0,2,3],material}]))
}
fn centre(pixels:&[u8])->f64 {
    let mut sum=0u32;for y in H/2-2..=H/2+2 {for x in W/2-2..=W/2+2 {sum+=pixels[((y*W+x)*4)as usize]as u32;}} sum as f64/25.
}
fn main(){if let Err(e)=run(){eprintln!("sun visibility fixture: {e}");std::process::exit(1);}}
fn run()->Result<(),String>{
    let out=PathBuf::from(std::env::args().nth(1).ok_or("FRESH_DIR required")?);
    if out.exists(){return Err("output directory must be fresh".into());}std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    let mut scene=Scene::default();scene.sky=Some((4,2,vec![[0.,0.,0.,1.];8]));scene.sky_physical=true;
    let floor=quad(&mut scene,[[-10.,0.,10.],[10.,0.,10.],[10.,0.,-10.],[-10.,0.,-10.]],Vec3::Y);scene.add_instance(floor,Affine3A::IDENTITY);
    // At two degrees the ray reaches y20.95m at x600m. The entire blocker
    // is beyond the old +399m caster plane, but well inside both XY frusta.
    let wall=quad(&mut scene,[[600.,16.,-20.],[600.,30.,-20.],[600.,30.,20.],[600.,16.,20.]],Vec3::NEG_X);scene.add_instance(wall,Affine3A::IDENTITY);
    let sun=Vec3::new(2f32.to_radians().cos(),2f32.to_radians().sin(),0.);
    let camera=Camera{pos:Vec3::new(-4.,5.,7.),target:Vec3::ZERO,fov_y:55f32.to_radians(),near:0.1};
    let mut light=Lighting{sun_dir:sun,sun_lux:80_000.,sky_lux:0.,ev100:11.,gi:0.,shadow_strength:1.,near_shadows:1.,
        contact_occlusion:0.,reflection_occlusion:0.,fog_density:0.,taa:0.,sharpen:0.,lod_pixels:0.,debug:9.,..Default::default()};
    let gpu=Gpu::new()?;let renderer=Renderer::new(&gpu,&scene,&light);let mut rows=Vec::new();let mut failures=Vec::new();
    for (name,focus,near) in [("near",Vec3::ZERO,1.),("far",Vec3::Z*40.,1.),("far-negative",Vec3::NEG_Z*40.,1.),("near-disabled",Vec3::ZERO,0.)]{
        renderer.set_lighting_focus(&[focus])?;light.near_shadows=near;
        let (pixels,stats)=renderer.render(&gpu,&camera,&light,W,H);let value=centre(&pixels);
        render::write_png(&out.join(format!("{name}.png")),W,H,&pixels)?;
        if value>2. {failures.push(format!("remote blocker leaks with {name}: {value}"));}
        rows.push(format!("{{\"case\":\"{name}\",\"visibility_rgb\":{value},\"shadow_vertices\":{}}}",stats.shadow_submitted_vertices));
    }
    scene.update_instance(1,wall,Affine3A::from_translation(Vec3::Z*1000.));renderer.update_instances(&gpu,1,&scene.instances[1..])?;
    renderer.set_lighting_focus(&[Vec3::ZERO])?;light.near_shadows=1.;
    let (pixels,_)=renderer.render(&gpu,&camera,&light,W,H);let open=centre(&pixels);render::write_png(&out.join("unblocked.png"),W,H,&pixels)?;
    if open<200. {failures.push(format!("unblocked plane self-shadows: {open}"));}
    // A concave crease under a vertical sun is fully visible on both sides.
    // Extrapolating either receiver plane across the join invents a blocker.
    let mut crease=Scene::default();crease.sky=Some((4,2,vec![[0.,0.,0.,1.];8]));crease.sky_physical=true;
    let left=quad(&mut crease,[[-10.,5.,10.],[0.,0.,10.],[0.,0.,-10.],[-10.,5.,-10.]],Vec3::new(0.5,1.,0.).normalize());
    crease.add_instance(left,Affine3A::IDENTITY);
    let right=quad(&mut crease,[[0.,0.,10.],[10.,5.,10.],[10.,5.,-10.],[0.,0.,-10.]],Vec3::new(-0.5,1.,0.).normalize());
    crease.add_instance(right,Affine3A::IDENTITY);
    light.sun_dir=Vec3::Y;
    let crease_renderer=Renderer::new(&gpu,&crease,&light);
    crease_renderer.set_lighting_focus(&[Vec3::ZERO])?;
    for (name,x) in [("crease-left",-0.003f32),("crease-right",0.003)] {
        let target=Vec3::new(x,x.abs()*0.5,0.);
        let view=Camera{pos:target+Vec3::new(0.,0.12,0.2),target,near:0.001,..camera};
        let (pixels,_)=crease_renderer.render(&gpu,&view,&light,W,H);let value=centre(&pixels);
        render::write_png(&out.join(format!("{name}.png")),W,H,&pixels)?;
        if value<225. {failures.push(format!("fully lit crease self-shadows with {name}: {value}"));}
        rows.push(format!("{{\"case\":\"{name}\",\"visibility_rgb\":{value}}}"));
    }
    let report=format!("{{\"scope\":\"native640x384 sun visibility fixture; not gameplay FPS\",\"passed\":{},\"rows\":[{}],\"unblocked_rgb\":{open},\"failures\":{:?}}}",failures.is_empty(),rows.join(","),failures);
    std::fs::write(out.join("report.json"),report).map_err(|e|e.to_string())?;
    if failures.is_empty(){println!("sun visibility fixture PASS");Ok(())}else{Err(failures.join("; "))}
}
