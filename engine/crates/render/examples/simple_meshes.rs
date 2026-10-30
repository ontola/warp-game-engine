//! GPU equivalence of shared low-profile culling and bounded direct submission.
//! Run alone on the GPU. Timestamps are disabled: this checks images/source IDs.
use glam::{Affine3A,Vec3};
use render::{Gpu,Scene,Renderer,Lighting,Camera,ViewRect};
fn main(){if let Err(e)=run(){panic!("{e}");}}
fn run()->Result<(),String>{
    let mut gpu=Gpu::new()?;
    gpu.features.remove(wgpu::Features::TIMESTAMP_QUERY|wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);
    let mut scene=Scene::default();
    scene.graphics.simple_lighting=true;
    let m=scene.add_material(render::scene::Material{tint:[0.6,0.3,0.1],layer:-1,roughness:1.,flags:0,scale:1.,metal:0.});
    let mesh=scene.add_mesh(&render::primitives::cube(m).mesh());
    let detailed=scene.add_mesh(&render::primitives::sphere(m,8,12).mesh());
    for i in 0..130 {
        scene.add_instance(if i%3==0 {detailed}else{mesh},Affine3A::from_translation(Vec3::new((i%13)as f32*1.5-9.,(i/13)as f32*1.5-6.,-25.)));
        if i%7==0 {scene.instances[i].cluster_count=0;}
        if i%9==0 {scene.instances[i].flags|=1<<8;}
    }
    scene.instances[1].flags|=render::scene::SKINNED; // instance bit 64 is not the material GRASS flag
    let light=Lighting{gi:0.,taa:0.,near_shadows:0.,fog_density:0.,..Default::default()};
    unsafe {std::env::set_var("KERN_BOUNDED_DRAWS","0");}
    let reference=Renderer::new(&gpu,&scene,&light);
    scene.graphics.simple_mesh_culling=true;
    unsafe {std::env::set_var("KERN_BOUNDED_DRAWS","1");}
    let candidate=Renderer::new(&gpu,&scene,&light);
    let views:Vec<_>=[-0.032,0.032].into_iter().enumerate().map(|(i,x)|ViewRect{
        cam:Camera{pos:Vec3::new(x,0.,0.),target:Vec3::new(x,0.,-1.),fov_y:1.2,near:0.05},x:i as u32*256,y:0,w:256,h:256}).collect();
    for phase in 0..3 {
        if phase==1 {scene.instances[0].cluster_count=scene.meshes[detailed].cluster_count;scene.instances[1].cluster_count=0;}
        if phase==2 {for i in &mut scene.instances {i.cluster_count=0;}}
        reference.update_instances(&gpu,0,&scene.instances)?;
        candidate.update_instances(&gpu,0,&scene.instances)?;
        let (a,sa)=reference.try_render_views(&gpu,&views,&light,512,256,&[])?;
        let (b,sb)=candidate.try_render_views(&gpu,&views,&light,512,256,&[])?;
        if phase<2 {assert!(sb.submitted_vertices<sa.submitted_vertices,"plain skinned flag must retain the small-cluster path");}
        assert_eq!((sa.clusters,sa.triangles),(sb.clusters,sb.triangles),"phase {phase} geometry");
        for view in 0..2 {
            let mut a=reference.debug_visible_clusters(&gpu,view,None)?.pairs;
            let mut b=candidate.debug_visible_clusters(&gpu,view,None)?.pairs;
            a.sort_unstable();b.sort_unstable();assert_eq!(a,b,"phase {phase} view {view}");
        }
        let error=a.iter().zip(&b).map(|(a,b)|a.abs_diff(*b)).max().unwrap();
        assert!(error<=1,"phase {phase} pixel error {error}");
        println!("phase {phase}: {} clusters, pixel error {error}",sa.clusters);
    }
    Ok(())
}
