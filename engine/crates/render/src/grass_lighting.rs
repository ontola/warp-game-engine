//! Shared source-density grass lighting. The medium is statistical, not a
//! per-blade shadow map. Sun tracing is local (8 cells / 16 m); unvisited space
//! remains unoccluded. Known native terrain also blocks the ray, independent
//! of grass habitat or road exclusion. This does not replace a distant terrain
//! horizon. The field and material are identical at every LOD.
use crate::{Gpu, far_grass::{FarGrassField, FarGrassRenderer}};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone,Copy,Pod,Zeroable)]
struct Uniform {grid:[f32;4],dims:[u32;4],profile:[f32;4],receiver:[f32;4]}

// The far renderer may skip tiles without grass. Sunlight must also intersect
// their bare terrain, so use separate conservative bounds at the SAME binding.
// -1e9 is the finite missing-height sentinel; no synthetic floor fills holes.
fn sunlight_bounds(field:&FarGrassField)->Vec<[f32;2]> {
    let width=(field.width-1).div_ceil(8);
    let height=(field.height-1).div_ceil(8);
    let mut result=vec![[1e30f32,-1e30];(width*height) as usize];
    for z in 0..field.height-1 {for x in 0..field.width-1 {
        let index=(z*field.width+x) as usize;
        let tile=&mut result[((z/8)*width+x/8) as usize];
        for i in [index,index+1,index+field.width as usize,index+field.width as usize+1] {
            let y=field.heights[i];
            if y<=-1e8 {continue;}
            tile[0]=tile[0].min(y);tile[1]=tile[1].max(y+field.max_height);
        }
    }}
    result
}
pub(crate) struct Resources {
    pub uniform:wgpu::Buffer,
    pub terrain:wgpu::TextureView,
    pub coverage:wgpu::TextureView,
    pub tiles:wgpu::TextureView,
}
impl Resources {
    pub fn new(gpu:&Gpu,field:Option<(&FarGrassField,&FarGrassRenderer)>,terrain_visual_offset_m:f32)->Self {
        let flags=u32::from(!crate::switches::GRASS_LEAF_BRDF.off())
            | (u32::from(!crate::switches::GRASS_SUN_OCCLUSION.off())<<1);
        let mut values=Uniform::zeroed();values.dims[3]=flags;
        let (terrain,coverage,tiles)=if let Some((field,renderer))=field {
            values=Uniform {grid:[field.origin[0],field.origin[1],field.cell,field.max_height],
                dims:[field.width,field.height,1,flags],profile:field.profile,receiver:[0.;4]};
            let (terrain,coverage,_)=renderer.surface_fields();
            let size=wgpu::Extent3d {width:(field.width-1).div_ceil(8),height:(field.height-1).div_ceil(8),depth_or_array_layers:1};
            let tiles=gpu.device.create_texture(&wgpu::TextureDescriptor {label:Some("grass sunlight terrain bounds"),size,mip_level_count:1,sample_count:1,
                dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rg32Float,
                usage:wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_DST,view_formats:&[]});
            gpu.queue.write_texture(wgpu::TexelCopyTextureInfo {texture:&tiles,mip_level:0,origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::All},
                bytemuck::cast_slice(&sunlight_bounds(field)),wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(size.width*8),rows_per_image:Some(size.height)},size);
            (terrain,coverage,tiles.create_view(&Default::default()))
        }else{
            let empty=|format|gpu.device.create_texture(&wgpu::TextureDescriptor {label:Some("empty grass lighting"),size:wgpu::Extent3d {width:1,height:1,depth_or_array_layers:1},
                mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format,usage:wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[]}).create_view(&Default::default());
            (empty(wgpu::TextureFormat::Rg32Float),empty(wgpu::TextureFormat::R8Unorm),empty(wgpu::TextureFormat::Rg32Float))
        };
        values.receiver[0]=if terrain_visual_offset_m.is_finite() {terrain_visual_offset_m.max(0.)}else{0.};
        let uniform=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("source grass optical field"),contents:bytemuck::bytes_of(&values),usage:wgpu::BufferUsages::UNIFORM});
        Self {uniform,terrain,coverage,tiles}
    }
}

#[cfg(test)]
mod tests {
    use glam::{Vec2,Vec3};
    fn field(width:u32,height:u32)->crate::far_grass::FarGrassField {
        crate::far_grass::FarGrassField {origin:[0.;2],cell:2.,width,height,
            heights:vec![0.;(width*height) as usize],road:vec![0.;(width*height) as usize],
            coverage:vec![0;((width-1)*(height-1)) as usize],profile:[2.;4],max_height:0.65,
            albedo:[0.2;3],shading_profile:[[0.2,0.2,0.2,0.5];4]}
    }
    #[test]
    fn sunlight_bounds_include_bare_terrain_and_skip_unknown_space() {
        let mut f=field(18,10);
        f.heights[4*18+7]=12.;
        let bounds=super::sunlight_bounds(&f);
        assert_eq!(bounds.len(),6);
        assert_eq!(bounds[0],[0.,12.65],"zero habitat / road mask cannot remove the occluding hill");
        assert_eq!(bounds[1],[0.,0.65]);
        let original=bounds;
        f.coverage.fill(255);f.road.fill(5.);
        assert_eq!(super::sunlight_bounds(&f),original,"terrain bounds are independent of plant density");
        f.heights.fill(-1e9);
        assert!(super::sunlight_bounds(&f).iter().all(|b|b[0]>b[1]));
        f.heights[0]=-50.;
        assert_eq!(super::sunlight_bounds(&f)[0],[-50.,-49.35],"valid terrain below sea level remains solid");
    }

    // Independent ray/triangle intersection oracle, rather than a second
    // implementation of height-bin clipping. All rays start above the mesh.
    fn ray_triangle(o:glam::DVec3,d:glam::DVec3,triangle:[glam::DVec3;3],length:f64)->bool {
        let edge1=triangle[1]-triangle[0];let edge2=triangle[2]-triangle[0];
        let p=d.cross(edge2);let determinant=edge1.dot(p);
        if determinant.abs()<1e-12 {return false;}
        let t=o-triangle[0];let u=t.dot(p)/determinant;
        let q=t.cross(edge1);let v=d.dot(q)/determinant;
        let distance=edge2.dot(q)/determinant;
        u>=0. && v>=0. && u+v<=1. && distance>0. && distance<length
    }
    #[test]
    fn native_diagonal_solid_hit_matches_triangle_ray_oracle_at_low_sun() {
        use glam::DVec3;
        // Ridge exists at the b--c diagonal; both ends of the ray are above
        // ground. Testing only the whole cell's endpoints would miss it.
        let points=[DVec3::new(0.,0.,0.),DVec3::new(2.,2.,0.),DVec3::new(0.,2.,2.),DVec3::new(2.,0.,2.)];
        let triangles=[[points[0],points[2],points[1]],[points[1],points[2],points[3]]];
        for elevation in [0.001f64,2.,15.,45.,80.] {
            let elevation=elevation.to_radians();
            let direction=DVec3::new(elevation.cos()/2f64.sqrt(),elevation.sin(),elevation.cos()/2f64.sqrt());
            let length=1.6/direction.x;let diagonal=0.8/direction.x;
            for initial_height in [0.5,0.8,1.,1.5,2.5,4.] {
                let origin=DVec3::new(0.2,initial_height,0.2);
                let expected=triangles.into_iter().any(|tri|ray_triangle(origin,direction,tri,length));
                let clearance=|t:f64| {
                    let p=origin+direction*t;let x=p.x/2.;let z=p.z/2.;
                    let ground=if x+z<=1. {2.*(x+z)} else {4.-2.*(x+z)};
                    p.y-ground
                };
                let begin=clearance(0.);let middle=clearance(diagonal);let end=clearance(length);
                let analytic=begin.min(middle) < -0.001 || middle.min(end) < -0.001;
                assert_eq!(analytic,expected,"elevation {elevation}, height {initial_height}");
            }
        }
        let sun=2f64.to_radians();
        let distance=1.6*2f64.sqrt()/sun.cos();
        assert!(1.0+sun.sin()*distance>0.4,"ray endpoint is above the second triangle");
        assert!(1.0+sun.sin()*distance*0.5<2.,"ridge still blocks this 2-degree sun ray");
    }

    // CPU semantic oracle for the shader's per-triangle receiver state. The
    // independent 3D ray oracle below supplies the geometric expectations;
    // the GPU replay separately checks actual production shading.
    fn receiver_segments(cells:&[Vec<(f64,f64,f64)>],bound:f64,terrain:bool)->bool {
        let mut grace=terrain&&bound>0.;
        // Native2m grid, unit horizontal ray. The shader divides this same
        // cell diagonal by the actual horizontal ray speed.
        let limit=2.*2f64.sqrt();let mut begin=0.;
        for cell in cells {
            for &(height,rise,length) in cell {
                let error=0.001;
                let end=height+rise*length;
                let mut pending=false;
                if height.min(end) < -error {
                    if !grace||height < -bound-error||rise<=0.||begin>=limit {return true;}
                    if begin+length>=limit&&height+rise*(limit-begin) < -error {return true;}
                    pending=end < -error;
                }
                grace=pending;
                begin+=length;
            }
            if grace&&begin>=limit {return true;}
        }
        grace
    }
    #[test]
    fn refined_rock_receiver_can_leave_only_its_bounded_native_support() {
        use glam::DVec3;
        // The visible cliff is 20 cm below its 2m native approximation.
        // A ray from that real surface escapes toward the morning sun.
        let surface=[DVec3::new(0.,-0.2,0.),DVec3::new(0.,-0.2,2.),DVec3::new(2.,-8.2,0.)];
        let direction=DVec3::new(0.9,0.4,0.).normalize();
        let origin=DVec3::new(0.2,-1.,0.2)+direction*0.005;
        assert!(!ray_triangle(origin,direction,surface,16.),"real rock does not block the outgoing sun");
        let height=origin.y+4.*origin.x;
        let rise=direction.y+4.*direction.x;
        let cell=vec![(height,rise,(2.-origin.x)/direction.x)];
        assert!(receiver_segments(&[cell.clone()],0.,true),"strict native approximation reproduces the false black patch");
        assert!(!receiver_segments(&[cell.clone()],1.1,true),"bounded refined receiver escapes");
        assert!(receiver_segments(&[cell],1.1,false),"grass/road cannot borrow rock's escape contract");
        // Morning capture #4: every black measured ray starts inside native
        // ground; its neighbouring lit ray starts outside. Source rays are
        // clear in both cases. Both remain clear with receiver escape.
        for initial in [-0.201,-0.031,-0.124,-0.188,-0.091,-0.085,-0.069,-0.154,0.007,0.170] {
            assert!(!receiver_segments(&[vec![(initial,2.0,0.5)]],1.1,true));
        }
    }
    #[test]
    fn receiver_escape_keeps_later_hills_and_rejects_underground_leaks() {
        use glam::DVec3;
        // A following hill still intersects the same outgoing ray. No
        // receiver state re-arms after escaping or at a new native cell.
        let origin=DVec3::new(0.,0.,0.);let direction=DVec3::new(1.,0.1,0.).normalize();
        let wall=[DVec3::new(2.,-1.,-1.),DVec3::new(2.,3.,-1.),DVec3::new(2.,-1.,1.)];
        assert!(ray_triangle(origin,direction,wall,16.));
        assert!(receiver_segments(&[vec![(-0.2,2.,0.5)],vec![(0.8,-2.,1.)]],1.1,true));
        assert!(receiver_segments(&[vec![(-0.2,2.,0.5),(0.8,-2.,1.)]],1.1,true),"hill in second triangle of same cell remains opaque");
        assert!(receiver_segments(&[vec![(-0.2,-0.1,1.)]],1.1,true),"ray enters deeper ground");
        assert!(receiver_segments(&[vec![(-1.2,2.,1.)]],1.1,true),"burial exceeds declared visual geometry bound");
        assert!(receiver_segments(&[vec![(-0.2,0.01,1.)],vec![(-0.19,0.01,2.)]],1.1,true),"cannot carry escape beyond the receiver-centred cell diagonal");
        assert!(!receiver_segments(&[vec![(-0.2,0.3,0.5),(-0.05,0.3,0.5)]],1.1,true),"continuous ascent across native diagonal is allowed");
        assert!(receiver_segments(&[vec![(-0.2,0.3,0.5),(-0.05,-0.3,0.5)]],1.1,true),"native diagonal cannot turn escape into downhill tunnelling");
    }
    #[test]
    fn rock_receiver_near_cell_edge_keeps_escape_and_following_hill_occlusion() {
        // Exact morning#4 failure pattern: initial native clearance -15.4cm,
        // receiver only5cm before cell edge, ascending out just beyond it.
        // Translate the grid boundary while keeping the physical ray fixed:
        // neither placement may produce a dark polygon.
        let height=-0.154;let rise=2.8;
        for edge in [0.01,0.05,0.25,0.5,1.] {
            let cells=[vec![(height,rise,edge)],vec![(height+rise*edge,rise,1.)]];
            assert!(!receiver_segments(&cells,1.1,true),"grid edge at{edge}m changed an exposed receiver");
        }
        // Clear native ground6cm from the receiver, then immediately meet
        // another hill. The old grace must not re-arm, even within its radius.
        let first_end=height+rise*0.05;
        let escaped=first_end+rise*0.01;
        assert!(escaped>0.);
        assert!(receiver_segments(&[vec![(height,rise,0.05)],vec![(first_end,rise,0.01)],
            vec![(escaped,-2.8,0.05)]],1.1,true),"hill immediately after escape must block");
        // Escape after the support radius is not allowed, even if the ray's
        // endpoint is eventually above the hill in the same affine segment.
        assert!(receiver_segments(&[vec![(-0.3,0.1,4.)]],1.1,true));
    }
    // Independent numerical oracle for a planar piecewise-constant medium.
    fn tau(profile:[f64;4],height:f64,start:f64,rate:f64,horizontal:f64,coverage:f64,road:(f64,f64),length:f64)->f64 {
        let dt=length/200_000.;(0..200_000).map(|i|{
            let t=(i as f64+0.5)*dt;let y=start+rate*t;
            if y<0. || y>=height || road.0+road.1*t<0.9 {0.}
            else {profile[(y/height*4.)as usize]*horizontal*coverage*dt}
        }).sum()
    }
    #[test] fn source_optics_empty_height_and_road_cutouts() {
        let profile=[2.,1.5,0.4,0.1];let h=0.6;let rate=0.5;let horizontal=(1f64-rate*rate).sqrt();
        assert_eq!(tau([0.;4],h,0.,rate,horizontal,1.,(3.,0.),4.),0.);
        assert_eq!(tau(profile,h,h,rate,horizontal,1.,(3.,0.),4.),0.);
        assert_eq!(tau(profile,h,0.,rate,horizontal,0.,(3.,0.),4.),0.);
        assert_eq!(tau(profile,h,0.,rate,horizontal,1.,(0.,0.),4.),0.);
        let ground=tau(profile,h,0.,rate,horizontal,1.,(3.,0.),4.);
        assert!((ground-profile.iter().sum::<f64>()*h/4.*horizontal/rate).abs()<1e-4);
        let tip=tau(profile,h,0.45,rate,horizontal,1.,(3.,0.),4.);
        assert!(tip<ground && (-ground).exp()>0. && (-ground).exp()<1.);
        let edge=tau(profile,h,0.,rate,horizontal,1.,(0.4,1.),4.);
        assert!(edge>0. && edge<ground);
    }
    fn normals(v:Vec3)->[(Vec3,f32);4] {
        let a=Vec2::new(v.x,v.z).try_normalize().unwrap_or(Vec2::X);let b=Vec2::new(-a.y,a.x);
        let mut samples=[(Vec3::ZERO,0.);4];let mut sum=0.;
        for (i,sample) in samples.iter_mut().enumerate(){let angle=(i as f32+0.5)*std::f32::consts::FRAC_PI_2;
            let xz=a*angle.cos()+b*angle.sin();let mut n=Vec3::new(xz.x,0.45,xz.y).normalize();
            let weight=n.dot(v).abs();if n.dot(v)<0. {n=-n;}*sample=(n,weight);sum+=weight;}
        samples.map(|(n,w)|(n,w/sum.max(1e-6)))
    }
    #[test] fn projected_leaf_distribution_is_finite_and_has_no_world_cardinal_axis() {
        for elevation in [0.001f32,0.1,0.4,0.8,1.55] {for azimuth in [0f32,0.2,0.7,1.3,2.9] {
            let v=Vec3::new(elevation.cos(),elevation.sin(),0.);let sun=Vec3::new(0.5,0.6,0.7).normalize();
            let rotation=glam::Quat::from_rotation_y(azimuth);
            let a=normals(v);let b=normals(rotation*v);
            let eval=|data:[(Vec3,f32);4],l:Vec3|data.iter().map(|(n,w)|w*n.dot(l).max(0.)).sum::<f32>();
            let value=eval(a,sun);assert!(value.is_finite() && (0.0..=1.000001).contains(&value));
            assert!((value-eval(b,rotation*sun)).abs()<1e-5);
            assert!((a.iter().map(|(_,w)|w).sum::<f32>()-1.).abs()<1e-5);
            assert!(a.iter().all(|(n,w)|n.is_finite() && *w>=0. && n.dot(v)>=-1e-6));
        }}
    }
    // Independent angular/transport oracle: integrate the diffuse reflection
    // and existing transmission coefficients over the visible blade samples.
    // Shadow/cone/range multiply incident light, never the material's normal.
    fn diffuse_blades(v:Vec3,l:Vec3,base:Vec3)->Vec3 {
        normals(v).iter().map(|(n,w)| {
            let cosine=n.dot(l);
            (base*cosine.max(0.)+Vec3::new(0.12,0.16,0.05)*(-cosine).max(0.))
                *(*w/std::f32::consts::PI)
        }).sum()
    }
    #[test] fn grass_headlamp_below_tuft_lights_visible_blades_without_world_up_gate() {
        let v=Vec3::new(0.,0.1,1.).normalize();
        let l=Vec3::new(0.,-0.2,1.).normalize();
        let base=Vec3::new(0.12,0.2,0.06);
        assert_eq!(Vec3::Y.dot(l).max(0.),0.,"the former horizontal-sheet gate rejects the entire tuft");
        let response=diffuse_blades(v,l,base);
        assert!(response.min_element()>0.005,"visible inclined blades still face the lamp: {response:?}");
        let rotate=glam::Quat::from_rotation_y(1.37);
        assert!(response.abs_diff_eq(diffuse_blades(rotate*v,rotate*l,base),1e-6));
        let back=diffuse_blades(v,-v,base);
        assert!(back.is_finite()&&back.min_element()>0.,"opposed lamp/view retains only finite transmission");
    }
    #[test] fn grass_spot_energy_is_incident_only_and_obeys_shadow_range_and_inverse_square() {
        let v=Vec3::new(0.,0.1,1.).normalize();let l=Vec3::new(0.,-0.2,1.).normalize();
        let response=diffuse_blades(v,l,Vec3::new(0.12,0.2,0.06));
        let beam=|distance:f32,cone:f32,visibility:f32,intensity:Vec3| {
            if distance<0.05||distance>=90. {return Vec3::ZERO;}
            let x=((distance-72.)/18.).clamp(0.,1.);let fade=1.-x*x*(3.-2.*x);
            response*intensity*(cone*visibility*fade/distance.powi(2).max(0.04))
        };
        let intensity=Vec3::new(40_000.,38_000.,34_000.);
        let near=beam(10.,1.,1.,intensity);
        assert!(near.abs_diff_eq(beam(20.,1.,1.,intensity)*4.,1e-5));
        assert!(near.abs_diff_eq(beam(10.,1.,1.,intensity*0.5)*2.,1e-5));
        assert_eq!(beam(10.,0.,1.,intensity),Vec3::ZERO);
        assert_eq!(beam(10.,1.,0.,intensity),Vec3::ZERO);
        assert_eq!(beam(10.,1.,1.,Vec3::ZERO),Vec3::ZERO);
        assert_eq!(beam(90.,1.,1.,intensity),Vec3::ZERO);
        assert!(beam(89.99,1.,1.,intensity).max_element()<1e-6);
    }
    #[test] fn field_uniform_matches_shader() {
        assert_eq!(std::mem::size_of::<super::Uniform>(),64);
        assert_eq!(std::mem::offset_of!(super::Uniform,receiver),48);
        assert_eq!(crate::scene::Scene::default().terrain_visual_offset_m,0.);
    }
}
