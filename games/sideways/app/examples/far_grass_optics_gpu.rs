//! Actual far-grass shader goldens; no gameplay/assets or desktop window.
//! cargo run --release -p cli --example far_grass_optics_gpu -- --out FRESH_DIR
//! The oracle clips independent convex triangle prisms, not the shader's DDA.
use glam::{Affine3A, DVec3, Mat4, Vec2, Vec3, Vec4};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::far_grass::FarGrassField;
use render::scene::{Heightfield, Material, TWO_SIDED};
use render::{Camera, Gpu, Lighting, Renderer, Scene};
use serde_json::{json, Value};
use std::{path::{Path, PathBuf}, time::Instant};

const ALPHA_TOLERANCE: f64 = 0.003;
type Triangle = [DVec3; 3];
#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind { Empty, Uniform, Positive, Boundary, Odd, Road, BlockedRoad, Hill, Half, Front, Subpixel, Sparse }
struct Case { name: &'static str, kind: Kind, size: [u32; 2] }
fn cases() -> Vec<Case> {
    [
        ("empty", Kind::Empty), ("uniform", Kind::Uniform),
        ("positive_z", Kind::Positive), ("negative_grid_boundary", Kind::Boundary),
        ("odd_viewport", Kind::Odd), ("partial_road", Kind::Road),
        ("blocked_road", Kind::BlockedRoad), ("native_diagonal_hill", Kind::Hill),
        ("opaque_half", Kind::Half), ("opaque_front", Kind::Front),
        ("opaque_one_of_four", Kind::Subpixel), ("sparse_tiles", Kind::Sparse),
    ].into_iter().map(|(name, kind)| Case { name, kind, size: match kind {
        Kind::Boundary => [130, 130], Kind::Odd => [129, 127], _ => [128, 128],
    }}).collect()
}
fn camera(case: &Case) -> Camera {
    Camera { pos: Vec3::new(0.0, 0.30, match case.kind { Kind::Positive => -70.0, Kind::Sparse => 100.0, _ => 70.0 }),
        target: Vec3::new(0.0, 0.30, 0.0), fov_y: 5f32.to_radians(), near: 0.1 }
}
fn inverse(camera: &Camera, size: [u32; 2]) -> Mat4 {
    (Mat4::perspective_infinite_reverse_rh(camera.fov_y, size[0] as f32 / size[1] as f32, camera.near)
        * Mat4::look_at_rh(camera.pos, camera.target, Vec3::Y)).inverse()
}
fn ray(camera: &Camera, size: [u32; 2], sample: [f32; 2]) -> DVec3 {
    let ndc = Vec2::new(sample[0] / size[0] as f32 * 2.0 - 1.0, 1.0 - sample[1] / size[1] as f32 * 2.0);
    let p = inverse(camera, size) * Vec4::new(ndc.x, ndc.y, 1e-4, 1.0);
    (p.truncate() / p.w - camera.pos).normalize().as_dvec3()
}
fn half_ray(camera: &Camera, size: [u32; 2], pixel: [u32; 2]) -> DVec3 {
    ray(camera, size, [(pixel[0] as f32 * 2.0 + 1.0).min(size[0] as f32 - 0.5),
        (pixel[1] as f32 * 2.0 + 1.0).min(size[1] as f32 - 0.5)])
}
fn field(case: &Case) -> FarGrassField {
    let n = if case.kind == Kind::Sparse { 25 } else { 9 };
    let extent = (n - 1) as f32;
    let mut f = FarGrassField { origin: [-extent; 2], cell: 2.0, width: n, height: n,
        heights: vec![0.0; (n*n) as usize], road: vec![5.0; (n*n) as usize],
        coverage: vec![255; ((n-1)*(n-1)) as usize], profile: [0.08; 4],
        max_height: 0.65, albedo: [0.12, 0.25, 0.07],
        shading_profile: std::array::from_fn(|i| {
            let height = (i as f32 + 0.5) / 4.0;
            let factor = 0.8 + 0.2 * height;
            [0.12 * factor, 0.25 * factor, 0.07 * factor, height]
        }) };
    for z in 0..n { for x in 0..n {
        let p = [f.origin[0] + x as f32 * f.cell, f.origin[1] + z as f32 * f.cell];
        let index = (z*n+x) as usize;
        if case.kind == Kind::Hill { f.heights[index] = p[0]*0.015 + p[1]*0.008 + ((x+z)%2) as f32*0.12; }
        if case.kind == Kind::Road { f.road[index] = 0.9 + p[1]*0.2; }
        if case.kind == Kind::BlockedRoad { f.road[index] = 0.2; }
    }}
    if case.kind == Kind::Empty { f.coverage.fill(0); }
    if case.kind == Kind::Sparse {
        for z in 0..n-1 { for x in 0..n-1 {
            f.coverage[(z*(n-1)+x) as usize] = if z < 4 || z >= 20 { 255 } else { 0 };
        }}
    }
    if case.kind == Kind::Hill { f.profile = [0.02, 0.04, 0.06, 0.08]; }
    f
}
fn terrain_triangles(field: &FarGrassField) -> Vec<(Triangle, [f64; 3], f64)> {
    let mut result = Vec::new();
    for z in 0..field.height-1 { for x in 0..field.width-1 {
        let a = (z*field.width+x) as usize;
        let corners = [a, a+1, a+field.width as usize, a+field.width as usize+1];
        let positions: [DVec3; 4] = std::array::from_fn(|i| DVec3::new(
            field.origin[0] as f64 + (x + (i as u32 & 1)) as f64*field.cell as f64,
            field.heights[corners[i]] as f64,
            field.origin[1] as f64 + (z + (i as u32 >> 1)) as f64*field.cell as f64));
        for ids in [[0,2,1], [3,1,2]] {
            result.push((ids.map(|i|positions[i]), ids.map(|i|field.road[corners[i]] as f64),
                field.coverage[(z*(field.width-1)+x) as usize] as f64/255.0));
        }
    }}
    result
}
fn plane_point(camera: &Camera, size: [u32; 2], sample: [f32; 2], z: f64) -> DVec3 {
    let d = ray(camera, size, sample); let p = camera.pos.as_dvec3(); p+d*((z-p.z)/d.z)
}
fn opaque(case: &Case, field: &FarGrassField, camera: &Camera) -> Vec<Triangle> {
    let mut result: Vec<_> = terrain_triangles(field).into_iter().map(|t|t.0).collect();
    if matches!(case.kind, Kind::Half | Kind::Front | Kind::Subpixel) {
        let z = if case.kind == Kind::Half { 0.0 } else { 10.0 };
        let bounds = if case.kind == Kind::Subpixel { [62.25, 62.25, 62.75, 63.75] }
            else { [-10.0, -10.0, case.size[0] as f32+10.0, case.size[1] as f32+10.0] };
        let p = [[bounds[0],bounds[1]], [bounds[2],bounds[1]], [bounds[0],bounds[3]], [bounds[2],bounds[3]]]
            .map(|s|plane_point(camera, case.size, s, z));
        result.extend([[p[0],p[1],p[2]], [p[2],p[1],p[3]]]);
    }
    result
}
fn fixture(field: &FarGrassField, opaque: &[Triangle]) -> Scene {
    let mut scene = Scene::default();
    let material = scene.add_material(Material { tint:[0.18;3], layer:-1, roughness:0.95,
        flags:TWO_SIDED, scale:1.0, metal:0.0 });
    let mut vertices = Vec::new();
    for triangle in opaque {
        let n = (triangle[1]-triangle[0]).cross(triangle[2]-triangle[0]).normalize().as_vec3().to_array();
        vertices.extend(triangle.map(|p|Vertex {pos:p.as_vec3().to_array(),normal:n,uv:[0.0;2]}));
    }
    let indices: Vec<u32> = (0..vertices.len() as u32).collect();
    let mesh = scene.add_mesh(&ClusterMesh::build(&[Part {vertices:&vertices,indices:&indices,material}]));
    scene.add_instance(mesh, Affine3A::IDENTITY); scene.mark_solid(); scene.mark_no_shadow();
    scene.heightfield = Heightfield { origin:field.origin, cell:field.cell, w:field.width,
        h:field.height, heights:field.heights.clone() };
    scene.far_grass = Some(field.clone());
    scene
}
// Independent closed convex-prism oracle. It enumerates all source triangles
// and height bins; it contains no grid stepping, minmax tiles or skip caches.
fn keep_halfspace(span: &mut [f64; 2], origin: DVec3, direction: DVec3, normal: DVec3, offset: f64) {
    let value = normal.dot(origin)+offset; let rate = normal.dot(direction);
    if rate.abs() < 1e-13 { if value < 0.0 {span[1]=span[0]-1.0;} return; }
    let t = -value/rate;
    if rate > 0.0 {span[0]=span[0].max(t);} else {span[1]=span[1].min(t);}
}
fn affine_height(triangle: Triangle, values: [f64; 3]) -> (DVec3, f64) {
    let p = std::array::from_fn::<_,3,_>(|i|DVec3::new(triangle[i].x,values[i],triangle[i].z));
    let n = (p[1]-p[0]).cross(p[2]-p[0]);
    let slope = DVec3::new(-n.x/n.y,0.0,-n.z/n.y);
    (slope, p[0].y-slope.dot(p[0]))
}
fn triangle_hit(origin: DVec3, direction: DVec3, triangle: Triangle) -> Option<f64> {
    let e1=triangle[1]-triangle[0]; let e2=triangle[2]-triangle[0];
    let p=direction.cross(e2); let det=e1.dot(p);
    if det.abs()<1e-12 {return None;}
    let s=origin-triangle[0]; let u=s.dot(p)/det;
    let q=s.cross(e1); let v=direction.dot(q)/det;
    if u < -1e-9 || v < -1e-9 || u+v > 1.0+1e-9 {return None;}
    let t=e2.dot(q)/det; (t>=0.1).then_some(t)
}
fn opaque_limit(case: &Case, camera: &Camera, opaque: &[Triangle], pixel: [u32; 2], centre: DVec3) -> f64 {
    let forward=(camera.target-camera.pos).normalize().as_dvec3(); let origin=camera.pos.as_dvec3();
    let mut minimum_view_depth=f64::INFINITY;
    for dy in 0..2 {for dx in 0..2 {
        let x=(pixel[0]*2+dx).min(case.size[0]-1); let y=(pixel[1]*2+dy).min(case.size[1]-1);
        let d=ray(camera,case.size,[x as f32+0.5,y as f32+0.5]);
        for &triangle in opaque {if let Some(t)=triangle_hit(origin,d,triangle) {
            minimum_view_depth=minimum_view_depth.min(t*d.dot(forward));
        }}
    }}
    minimum_view_depth/centre.dot(forward)
}
fn expected(case: &Case, field: &FarGrassField, camera: &Camera, opaque: &[Triangle], pixel:[u32;2]) -> (f64,f64) {
    let origin=camera.pos.as_dvec3(); let direction=half_ray(camera,case.size,pixel);
    let horizontal=direction.x.hypot(direction.z);
    let limit=opaque_limit(case,camera,opaque,pixel,direction);
    let range=[40.0/horizontal,(240.0/horizontal).min(limit)];
    let mut tau=0.0;
    for (triangle,roads,coverage) in terrain_triangles(field) {
        if coverage==0.0 {continue;}
        let centre=(triangle[0]+triangle[1]+triangle[2])/3.0;
        let (slope,intercept)=affine_height(triangle,triangle.map(|p|p.y));
        let (road_slope,road_intercept)=affine_height(triangle,roads);
        for bin in 0..4 {
            let mut span=range;
            for edge in 0..3 {
                let a=triangle[edge];let b=triangle[(edge+1)%3];
                let mut n=DVec3::new(-(b.z-a.z),0.0,b.x-a.x);
                if n.dot(centre-a)<0.0 {n=-n;}
                // Symbolic positive-X/Z perturbation owns a shared vertical
                // face exactly once. No finite epsilon changes ray lengths.
                // Otherwise a ray on x=0 would count both neighbouring cells.
                if n.dot(direction).abs()<1e-13 && n.dot(origin-a).abs()<1e-9
                    && n.dot(DVec3::new(1.0,0.0,0.37))<0.0 {
                    span[1]=span[0]-1.0;
                }
                keep_halfspace(&mut span,origin,direction,n,-n.dot(a));
            }
            let relative=DVec3::Y-slope;
            let bin_height=field.max_height as f64/4.0;
            // Parallel shared-bin planes use one half-open interval.
            let rate=relative.dot(direction);let value=relative.dot(origin)-intercept;
            if rate.abs()<1e-13 && !(value>=bin as f64*bin_height && value<(bin+1) as f64*bin_height) {continue;}
            keep_halfspace(&mut span,origin,direction,relative,-intercept-bin as f64*bin_height);
            keep_halfspace(&mut span,origin,direction,-relative,intercept+(bin+1) as f64*bin_height);
            keep_halfspace(&mut span,origin,direction,road_slope,road_intercept-0.9);
            if span[1]>span[0] {
                // All test media end before the outer160m fade starts.
                assert!(span[1]*horizontal<150.0,"fixture left the no-outer-fade region");
                tau+=(span[1]-span[0])*field.profile[bin] as f64*coverage*horizontal;
            }
        }
    }
    (1.0-(-tau).exp(),tau)
}
fn probes(case: &Case) -> Vec<[u32;2]> {
    let mut p=vec![[31,31],[32,31],[31,32],[32,32]];
    if case.kind==Kind::Boundary {p.push([32,32]);}
    if case.kind==Kind::Odd {p.extend([[64,31],[64,63],[0,63]]);}
    p.sort_unstable();p.dedup();p
}
fn half(bits:u16)->f32 {
    let exponent=(bits>>10)&31;let fraction=bits&1023;
    if exponent==0 {let value=fraction as f32/16_777_216.0;return if bits&0x8000!=0 {-value}else{value};}
    f32::from_bits(((bits as u32&0x8000)<<16)|((if exponent==31{255}else{exponent as u32+112})<<23)|((fraction as u32)<<13))
}
fn check(checks:&mut Vec<Value>,name:&str,pass:bool,evidence:Value) {checks.push(json!({"name":name,"pass":pass,"evidence":evidence}));}
fn render_case(gpu:&Gpu,out:&Path,case:&Case,light:&Lighting)->Result<Value,String> {
    let field=field(case);let camera=camera(case);let opaque=opaque(case,&field,&camera);
    let scene=fixture(&field,&opaque);let renderer=Renderer::new(gpu,&scene,light);
    let start=Instant::now();let (pixels,stats)=renderer.render(gpu,&camera,light,case.size[0],case.size[1]);
    let wall_ms=start.elapsed().as_secs_f64()*1000.0;
    render::write_png(&out.join(format!("{}-render.png",case.name)),case.size[0],case.size[1],&pixels)?;
    let mut layers=renderer.far_grass_layers_rgba16(gpu)?;
    if layers.len()!=1 {return Err(format!("{}: expected one far layer",case.name));}
    let (w,h,bytes)=layers.remove(0);
    if [w,h]!=case.size.map(|v|v.div_ceil(2)) || bytes.len()!=w as usize*h as usize*8 {return Err("invalid half-resolution readback size".into());}
    std::fs::write(out.join(format!("{}-rgba16f.bin",case.name)),&bytes).map_err(|e|e.to_string())?;
    let raw:Vec<[f32;4]>=bytes.chunks_exact(8).map(|p|std::array::from_fn(|k|half(u16::from_le_bytes([p[2*k],p[2*k+1]])))).collect();
    let finite=raw.iter().flatten().all(|v|v.is_finite());
    let alpha_png:Vec<u8>=raw.iter().flat_map(|p|{let a=(p[3].clamp(0.0,1.0)*255.0).round() as u8;[a,a,a,255]}).collect();
    render::write_png(&out.join(format!("{}-alpha.png",case.name)),w,h,&alpha_png)?;
    let mut checks=Vec::new();
    check(&mut checks,"finite_raw_rgba_and_valid_alpha",finite&&raw.iter().all(|p|(0.0..=1.0).contains(&p[3])),json!({"finite":finite}));
    let diagnostic=renderer.far_grass_diagnostics(gpu)?.ok_or("missing far grass diagnostics")?;
    if diagnostic.len()!=1 {return Err("expected one diagnostics view".into());}
    let d=&diagnostic[0];
    check(&mut checks,"bounded_complete_integration",d.overflow_rays==0&&d.max_iterations<=512,
        json!({"traced_rays":d.traced_rays,"overflow_rays":d.overflow_rays,"max_iterations":d.max_iterations}));
    if matches!(case.kind,Kind::Empty|Kind::BlockedRoad|Kind::Front) {
        let max=raw.iter().flatten().map(|x|x.abs()).fold(0.0f32,f32::max);
        check(&mut checks,"empty_or_fully_occluded_is_zero",finite&&max==0.0,json!({"maximum_abs_rgba":max}));
    }
    for pixel in probes(case) {
        let (expected,tau)=expected(case,&field,&camera,&opaque,pixel);
        let actual=raw[(pixel[1]*w+pixel[0]) as usize][3] as f64;
        check(&mut checks,&format!("analytic_alpha_{}_{}",pixel[0],pixel[1]),actual.is_finite()&&(actual-expected).abs()<=ALPHA_TOLERANCE,
            json!({"half_pixel":pixel,"expected_alpha":expected,"actual_alpha":actual,"tau":tau,"error":(actual-expected).abs(),"tolerance":ALPHA_TOLERANCE}));
    }
    let report=json!({"name":case.name,"pass":checks.iter().all(|c|c["pass"]==true),"checks":checks,
        "full_size":case.size,"raw_size":[w,h],"camera":{"position":camera.pos.to_array(),"target":camera.target.to_array(),"fov_y":camera.fov_y},
        "field":{"origin":field.origin,"cell":field.cell,"point_dimensions":[field.width,field.height],"profile":field.profile,"max_height":field.max_height},
        "first_frame_wall_ms":wall_ms,"gpu_ms":stats.gpu_ms});
    std::fs::write(out.join(format!("{}-report.json",case.name)),serde_json::to_vec_pretty(&report).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    println!("{}: {}",case.name,if report["pass"]==true{"PASS"}else{"FAIL"});Ok(report)
}
fn run(out:PathBuf)->Result<(),String> {
    if out.as_os_str().is_empty()||out.exists() {return Err("--out must name a fresh directory".into());}
    if let Some(parent)=out.parent(){std::fs::create_dir_all(parent).map_err(|e|e.to_string())?;}
    std::fs::create_dir(&out).map_err(|e|e.to_string())?;
    let start=Instant::now();let gpu=Gpu::new()?;
    let light=Lighting { debug:2.0,taa:0.0,sharpen:0.0,gi:0.0,contact_occlusion:0.0,reflection_occlusion:0.0,
        near_shadows:0.0,shadow_strength:0.0,fog_start:1e6,fog_end:2e6,fog_density:0.0,..Lighting::default() };
    let mut results=Vec::new();for case in cases(){results.push(render_case(&gpu,&out,&case,&light)?);}
    let pass=results.iter().all(|r|r["pass"]==true);
    let report=json!({"schema":1,"pass":pass,"adapter":gpu.info.name,"backend":format!("{:?}",gpu.info.backend),
        "purpose":"Real GPU far-grass optical checks, not gameplay performance or natural-grass visual parity",
        "oracle":"f64 convex-prism clipping for every native triangle and vertical bin, summed Beer-Lambert optical depth; independent of GPU DDA/minmax traversal. Closest opaque view-depth across the exact source2x2 footprint.",
        "limitations":["Selected raw half-resolution pixels have analytic alpha assertions; every output pixel is checked finite and retained.",
            "This validates far optics and conservative foreground clipping, not the near/far compositor or live habitat bake.",
            "All media lie inside160m so outer fade is intentionally outside this fixture's claims.",
            "First-frame timings include setup; no120fps or production-scale claim."],"runs":results,"elapsed_seconds":start.elapsed().as_secs_f64()});
    std::fs::write(out.join("report.json"),serde_json::to_vec_pretty(&report).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    if pass {println!("far grass optics GPU: PASS ({})",out.display());Ok(())}else{Err(format!("optical checks failed; evidence retained in {}",out.display()))}
}
fn main(){
    let args:Vec<String>=std::env::args().skip(1).collect();
    let result=if args.len()==2&&args[0]=="--out"{run(PathBuf::from(&args[1]))}else{Err("usage: far_grass_optics_gpu --out FRESH_DIRECTORY (12 bounded small cases)".into())};
    if let Err(e)=result{eprintln!("far_grass_optics_gpu: {e}");std::process::exit(1);}
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tau(kind:Kind,pixel:[u32;2])->f64 {
        let mut case=cases().into_iter().find(|c|c.kind==kind).unwrap();case.size=[130,130];
        let f=field(&case);let c=camera(&case);let o=opaque(&case,&f,&c);expected(&case,&f,&c,&o,pixel).1
    }
    #[test]
    fn prism_oracle_matches_closed_form_and_half_road_clip(){
        let total=tau(Kind::Uniform,[32,32]);
        assert!((total-16.0*0.08).abs()<1e-6);
        assert!((tau(Kind::Road,[32,32])*2.0-total).abs()<1e-5);
        assert!((tau(Kind::Half,[32,32])*2.0-total).abs()<1e-5);
        assert_eq!(tau(Kind::BlockedRoad,[32,32]),0.0);
        assert_eq!(tau(Kind::Front,[32,32]),0.0);
    }
    #[test]
    fn reverse_rays_and_empty_tiles_keep_mass(){
        assert!((tau(Kind::Positive,[32,32])-tau(Kind::Uniform,[32,32])).abs()<1e-6);
        assert!((tau(Kind::Sparse,[32,32])-16.0*0.08).abs()<1e-6);
        assert_eq!(tau(Kind::Empty,[32,32]),0.0);
    }
    #[test]
    fn opaque_quarter_pixel_clips_the_whole_half_pixel(){
        let case=cases().into_iter().find(|c|c.kind==Kind::Subpixel).unwrap();let f=field(&case);let c=camera(&case);let o=opaque(&case,&f,&c);
        let direction=half_ray(&c,case.size,[31,31]);
        // The half pixel's centre misses the thin quad, but a real full pixel
        // hits it. This proves the oracle exercises the conservative2x2 rule.
        assert!(o[o.len()-2..].iter().all(|t|triangle_hit(c.pos.as_dvec3(),direction,*t).is_none()));
        assert_eq!(expected(&case,&f,&c,&o,[31,31]).0,0.0);
        assert!(expected(&case,&f,&c,&o,[32,31]).0>0.0);
    }
    #[test]
    fn half_decode_keeps_nonfinite_visible(){assert_eq!(half(0x3c00),1.0);assert!(half(0x7c00).is_infinite());assert!(half(0x7e00).is_nan());}
}
