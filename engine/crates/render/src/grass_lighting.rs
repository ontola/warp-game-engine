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
struct Uniform {grid:[f32;4],dims:[u32;4],profile:[f32;4]}

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
    pub fn new(gpu:&Gpu,field:Option<(&FarGrassField,&FarGrassRenderer)>)->Self {
        let flags=u32::from(std::env::var("KERN_GRASS_LEAF_BRDF").as_deref()!=Ok("0"))
            | (u32::from(std::env::var("KERN_GRASS_SUN_OCCLUSION").as_deref()!=Ok("0"))<<1);
        let mut values=Uniform::zeroed();values.dims[3]=flags;
        let (terrain,coverage,tiles)=if let Some((field,renderer))=field {
            values=Uniform {grid:[field.origin[0],field.origin[1],field.cell,field.max_height],
                dims:[field.width,field.height,1,flags],profile:field.profile};
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
    #[test] fn field_uniform_matches_shader() {assert_eq!(std::mem::size_of::<super::Uniform>(),48);}
}
