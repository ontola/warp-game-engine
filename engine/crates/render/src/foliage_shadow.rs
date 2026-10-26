//! Bounded, CPU-only directional visibility-tile prototype. No live renderer
//! uses this yet. Unlike mean extinction, it averages TRANSMITTANCE after
//! resolving each fixed subpixel ray, retaining correlated holes through depth.
//! A GPU source or voxel tracer can supply the same sampled visibility layout.

use crate::{cluster::Part, scene::{Material, TextureSource, ALPHA_TEST, BLEND, GRASS}};
use glam::{Mat4, Vec2, Vec3};

const MAX_TEXELS: usize = 256 * 256;
const MAX_DEPTHS: usize = 64;
const MAX_RASTER_SAMPLES: u64 = 32_000_000;
const MAX_TRIANGLES: usize = 262_144;

/// Absolute coordinates in a sun-only orthonormal basis. X/Y origin must lie
/// on the world pixel grid; it does not follow a camera or a per-view jitter.
#[derive(Clone, Copy, Debug)]
pub struct TileLayout {
    /// Direction from a receiver towards the sun. Depth increases away from it.
    pub sun: Vec3,
    pub origin: Vec2,
    pub pixel_size: f32,
    pub size: [u32; 2],
    /// Absolute light-space depths, including both ends, strictly increasing.
    pub depth: [f32; 2],
    pub depth_samples: u32,
}

impl TileLayout {
    pub fn validate(&self) -> Result<(), String> {
        if !self.sun.is_finite() || self.sun.length_squared() < 1e-12 || !self.origin.is_finite()
            || !self.pixel_size.is_finite() || self.pixel_size <= 0.0
            || self.size.contains(&0) || self.size[0] as u64 * self.size[1] as u64 > MAX_TEXELS as u64
            || !self.depth.iter().all(|x| x.is_finite()) || self.depth[1] <= self.depth[0]
            || !(2..=MAX_DEPTHS as u32).contains(&self.depth_samples) {
            return Err("invalid or oversized directional visibility tile".into());
        }
        let q = self.origin / self.pixel_size;
        if (q - q.round()).abs().max_element() > 1e-3 {
            return Err("visibility tile origin must be world-grid aligned".into());
        }
        Ok(())
    }

    pub fn basis(&self) -> [Vec3; 3] {
        let z = -self.sun.normalize();
        let up = if z.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
        let x = up.cross(z).normalize(); [x, z.cross(x), z]
    }

    pub fn project(&self, world: Vec3) -> Vec3 {
        let [x, y, z] = self.basis(); Vec3::new(x.dot(world), y.dot(world), z.dot(world))
    }

    pub fn depth_at(&self, i: u32) -> f32 {
        self.depth[0] + (self.depth[1] - self.depth[0]) * i as f32 / (self.depth_samples - 1) as f32
    }
}

pub struct SurfaceInstance<'a> {
    pub parts: &'a [Part<'a>],
    pub transform: Mat4,
}

#[derive(Clone, Debug)]
pub struct VisibilityTile {
    pub layout: TileLayout,
    /// Layer-major U16 UNORM transmission, never optical density or colour.
    pub visibility: Vec<u16>,
    pub samples_per_pixel: u32,
    pub raster_samples: u64,
}

impl VisibilityTile {
    /// Input is pixel-major, then fixed subpixel ray, then increasing depth.
    /// Source/voxel producers must integrate each ray BEFORE this reduction.
    /// Sampling must use the same world-fixed grid as the surface producer.
    pub fn from_ray_samples(layout: TileLayout, samples_per_pixel: u32, values: &[f32]) -> Result<Self, String> {
        layout.validate()?;
        if !(1..=16).contains(&samples_per_pixel) { return Err("visibility tile supports 1..16 fixed rays per pixel".into()); }
        let pixels = layout.size[0] as usize * layout.size[1] as usize;
        let depths = layout.depth_samples as usize;
        if values.len() != pixels * samples_per_pixel as usize * depths { return Err("visibility sample count mismatch".into()); }
        let mut out = vec![0u16; pixels * depths];
        for p in 0..pixels {
            for s in 0..samples_per_pixel as usize {
                let ray = &values[(p * samples_per_pixel as usize + s) * depths..][..depths];
                if ray.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                    || ray.windows(2).any(|a| a[1] > a[0] + 1e-6) { return Err("visibility must be finite, bounded and nonincreasing through depth".into()); }
            }
            for d in 0..depths {
                let t: f64 = (0..samples_per_pixel as usize).map(|s| f64::from(values[(p * samples_per_pixel as usize + s) * depths + d])).sum();
                out[d * pixels + p] = (t / f64::from(samples_per_pixel) * 65535.0).round() as u16;
            }
        }
        Ok(Self { layout, visibility: out, samples_per_pixel, raster_samples: 0 })
    }

    /// Reference lookup. Return None outside the baked domain: a caller must
    /// select another covered representation, never silently assume sunlight.
    pub fn sample(&self, world: Vec3) -> Option<f32> {
        if !world.is_finite() { return None; }
        let p = self.layout.project(world);
        let xy = (p.truncate() - self.layout.origin) / self.layout.pixel_size;
        if xy.x < 0.0 || xy.y < 0.0 || xy.x >= self.layout.size[0] as f32 || xy.y >= self.layout.size[1] as f32
            || p.z < self.layout.depth[0] || p.z > self.layout.depth[1] { return None; }
        let depth = (p.z - self.layout.depth[0]) / (self.layout.depth[1] - self.layout.depth[0]) * (self.layout.depth_samples - 1) as f32;
        let z0 = depth.floor() as usize; let z1 = (z0 + 1).min(self.layout.depth_samples as usize - 1);
        let centre = (xy - Vec2::splat(0.5)).clamp(Vec2::ZERO,Vec2::new(self.layout.size[0] as f32 - 1.0,self.layout.size[1] as f32 - 1.0));
        let base = centre.floor(); let frac = centre - base;
        let w = self.layout.size[0] as usize; let h = self.layout.size[1] as usize;
        let layer = |z:usize| {
            let mut t=0.0;
            for j in 0..2 {for i in 0..2 {
                let x=(base.x as usize+i).min(w-1);let y=(base.y as usize+j).min(h-1);
                let weight=if i==0 {1.0-frac.x}else{frac.x} * if j==0 {1.0-frac.y}else{frac.y};
                t+=self.visibility[z*w*h+y*w+x] as f32/65535.0*weight;
            }} t
        };
        Some(layer(z0)+(layer(z1)-layer(z0))*depth.fract())
    }
}

fn alpha(texture: &TextureSource, uv: Vec2, clamp: bool) -> f32 {
    let image = &texture.rgba; let w=image.width() as i32; let h=image.height() as i32;
    if w == 0 || h == 0 { return 0.0; }
    let uv=if clamp {uv.clamp(Vec2::ZERO,Vec2::ONE)} else {uv-uv.floor()};
    let p=uv*Vec2::new(w as f32,h as f32)-Vec2::splat(0.5); let b=p.floor();let f=p-b;
    let at=|x:i32,y:i32| {
        let x=if clamp{x.clamp(0,w-1)}else{x.rem_euclid(w)};
        let y=if clamp{y.clamp(0,h-1)}else{y.rem_euclid(h)};
        image.get_pixel(x as u32,y as u32)[3] as f32/255.0
    };
    let a=at(b.x as i32,b.y as i32)*(1.0-f.x)+at(b.x as i32+1,b.y as i32)*f.x;
    let c=at(b.x as i32,b.y as i32+1)*(1.0-f.x)+at(b.x as i32+1,b.y as i32+1)*f.x;
    a*(1.0-f.y)+c*f.y
}

/// Exact opaque/cutout surface union on a deterministic stratified grid.
/// No frame randomization, tint, synthetic holes, density scale or camera.
/// Depth discretization and finite spatial sampling remain explicit errors.
pub fn bake_surfaces(layout: TileLayout, instances: &[SurfaceInstance<'_>], materials: &[Material], textures: &[TextureSource], samples_axis: u32, alpha_cut: f32) -> Result<VisibilityTile, String> {
    layout.validate()?;
    if !(1..=4).contains(&samples_axis) || !alpha_cut.is_finite() || !(0.0..=1.0).contains(&alpha_cut) { return Err("invalid visibility sample grid/alpha threshold".into()); }
    let triangles=instances.iter().flat_map(|i|i.parts).try_fold(0usize,|sum,p|sum.checked_add(p.indices.len()/3)).ok_or("visibility triangle count overflow")?;
    if triangles>MAX_TRIANGLES {return Err("directional visibility prototype exceeds 262144 source triangles".into());}
    let sw=layout.size[0]*samples_axis;let sh=layout.size[1]*samples_axis;
    let mut first=vec![f32::INFINITY;sw as usize*sh as usize];let mut visits=0u64;
    for instance in instances {
        if !instance.transform.is_finite() || instance.transform.determinant().abs()<1e-12 { return Err("invalid visibility instance transform".into()); }
        for part in instance.parts {
            let material=materials.get(part.material as usize).ok_or("visibility material index out of bounds")?;
            if material.flags&BLEND!=0 { return Err("surface tile requires opaque/cutout inputs; volume/transparent inputs supply ray transmission".into()); }
            if part.indices.len()%3!=0 { return Err("visibility indices are not triangles".into()); }
            for tri in part.indices.chunks_exact(3) {
                let vertices=tri.iter().map(|i|part.vertices.get(*i as usize).ok_or("visibility vertex index out of bounds")).collect::<Result<Vec<_>,_>>()?;
                let mut q=[Vec3::ZERO;3];let mut uv=[Vec2::ZERO;3];
                for k in 0..3 {
                    let p=Vec3::from(vertices[k].pos);uv[k]=Vec2::from(vertices[k].uv);
                    if !p.is_finite() || !uv[k].is_finite() { return Err("nonfinite visibility source vertex".into()); }
                    let projected=layout.project(instance.transform.transform_point3(p));
                    let xy=(projected.truncate()-layout.origin)/layout.pixel_size*samples_axis as f32;
                    q[k]=Vec3::new(xy.x,xy.y,projected.z);
                    if !q[k].is_finite() {return Err("visibility projection overflow".into());}
                }
                let a=q[0].truncate();let b=q[1].truncate();let c=q[2].truncate();
                let area=(b-a).perp_dot(c-a);if area.abs()<1e-9 {continue;}
                let low=a.min(b).min(c).floor().max(Vec2::ZERO);let high=a.max(b).max(c).ceil().min(Vec2::new(sw as f32,sh as f32));
                let nx=(high.x-low.x).max(0.0) as u64;let ny=(high.y-low.y).max(0.0) as u64;
                visits=visits.checked_add(nx*ny).ok_or("visibility work counter overflow")?;
                if visits>MAX_RASTER_SAMPLES {return Err("directional visibility prototype exceeds 32M raster samples".into());}
                for y in low.y as u32..high.y.max(low.y) as u32 {for x in low.x as u32..high.x.max(low.x) as u32 {
                    let p=Vec2::new(x as f32+0.5,y as f32+0.5);
                    let weights=[(b-p).perp_dot(c-p)/area,(c-p).perp_dot(a-p)/area,(a-p).perp_dot(b-p)/area];
                    if weights.iter().any(|w|*w < -1e-6) {continue;}
                    if material.flags&ALPHA_TEST!=0 && material.layer>=0 {
                        let texture=textures.get(material.layer as usize).ok_or("visibility texture index out of bounds")?;
                        let texcoord=uv[0]*weights[0]+uv[1]*weights[1]+uv[2]*weights[2];
                        if alpha(texture,texcoord,material.flags&GRASS!=0)<alpha_cut {continue;}
                    }
                    let depth=q[0].z*weights[0]+q[1].z*weights[1]+q[2].z*weights[2];
                    let hit=&mut first[y as usize*sw as usize+x as usize];*hit=hit.min(depth);
                }}
            }
        }
    }
    let pixels=layout.size[0] as usize*layout.size[1] as usize;let mut visibility=vec![0u16;pixels*layout.depth_samples as usize];
    for d in 0..layout.depth_samples {
        let depth=layout.depth_at(d);
        for y in 0..layout.size[1] {for x in 0..layout.size[0] {
            let mut lit=0u32;
            for j in 0..samples_axis {for i in 0..samples_axis {
                lit+=u32::from(first[((y*samples_axis+j)*sw+x*samples_axis+i) as usize]>depth);
            }}
            visibility[d as usize*pixels+(y*layout.size[0]+x) as usize]=(lit as f64/f64::from(samples_axis*samples_axis)*65535.0).round() as u16;
        }}
    }
    Ok(VisibilityTile {layout,visibility,samples_per_pixel:samples_axis*samples_axis,raster_samples:visits})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn layout()->TileLayout {TileLayout{sun:Vec3::Z,origin:Vec2::ZERO,pixel_size:1.0,size:[1,1],depth:[-2.0,2.0],depth_samples:5}}

    #[test]
    fn correlated_holes_average_visibility_not_extinction() {
        let tile=VisibilityTile::from_ray_samples(layout(),2,&[1.0,1.0,0.0,0.0,0.0, 1.0,1.0,1.0,1.0,1.0]).unwrap();
        assert_eq!(tile.visibility[..2],[65535,65535]);
        assert!((tile.visibility[4] as f32/65535.0-0.5).abs()<1e-5);
        assert!((-5.0f32).exp()<0.01,"averaged optical depth would incorrectly close the clear half");
        assert!(tile.sample(Vec3::new(9.0,9.0,0.0)).is_none());
    }

    #[test]
    fn source_cutout_and_overlapping_surfaces_keep_same_clear_rays() {
        use crate::cluster::Vertex;
        let vertices=[[-1.0,0.0,0.0],[0.0,0.0,0.0],[-1.0,1.0,0.0],[0.0,1.0,0.0]].into_iter().zip([[0.0,0.0],[1.0,0.0],[0.0,1.0],[1.0,1.0]])
            .map(|(pos,uv)|Vertex{pos,uv,normal:[0.0,0.0,1.0]}).collect::<Vec<_>>();
        let part=[Part{vertices:&vertices,indices:&[0,1,2,1,3,2],material:0}];
        let mat=[Material{flags:ALPHA_TEST|GRASS,tint:[1.0;3],layer:0,roughness:1.0,scale:1.0,metal:0.0}];
        let mut mask=image::RgbaImage::from_pixel(2,2,image::Rgba([255,255,255,255]));for y in 0..2{mask.put_pixel(0,y,image::Rgba([0,0,0,0]));}
        let texture=[TextureSource{name:"half clear".into(),rgba:mask,normal:None}];
        let one=[SurfaceInstance{parts:&part,transform:Mat4::IDENTITY}];
        let two=[SurfaceInstance{parts:&part,transform:Mat4::IDENTITY},SurfaceInstance{parts:&part,transform:Mat4::from_translation(Vec3::new(0.0,0.0,-0.5))}];
        let a=bake_surfaces(layout(),&one,&mat,&texture,4,0.5).unwrap();let b=bake_surfaces(layout(),&two,&mat,&texture,4,0.5).unwrap();
        assert_eq!(a.visibility,b.visibility,"duplicate aligned crown layers must not erase the same clear rays twice");
        assert_eq!(a.visibility,[65535,65535,32768,32768,32768]);
        let again=bake_surfaces(layout(),&one,&mat,&texture,4,0.5).unwrap();assert_eq!(a.visibility,again.visibility);
        let rotation=Mat4::from_cols(glam::Vec4::new(0.0,0.0,-1.0,0.0),glam::Vec4::Y,glam::Vec4::new(1.0,0.0,0.0,0.0),glam::Vec4::W);
        let turned=[SurfaceInstance{parts:&part,transform:rotation}];
        let mut turned_layout=layout();turned_layout.sun=rotation.transform_vector3(turned_layout.sun);
        let rotated=bake_surfaces(turned_layout,&turned,&mat,&texture,4,0.5).unwrap();
        assert_eq!(a.visibility,rotated.visibility,"rotating source and sun together must preserve visibility; no baked sun direction");
        assert!((a.sample(Vec3::new(-0.5,0.5,-1.0)).unwrap()-0.5).abs()<1e-5);
        assert_eq!(a.sample(Vec3::new(-0.5,0.5,1.0)),Some(1.0));
    }

    #[test]
    fn ray_input_validation_rejects_light_creation_and_invalid_domain() {
        assert!(VisibilityTile::from_ray_samples(layout(),1,&[1.0,0.1,0.2,0.0,0.0]).is_err());
        let mut invalid=layout();invalid.origin.x=0.3;assert!(invalid.validate().is_err());
        invalid=layout();invalid.size=[u32::MAX,u32::MAX];assert!(invalid.validate().is_err());
    }

    #[test]
    fn retained_source_crown_preserves_clear_rays_and_bare_stem_space() {
        let fixtures=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        let source=crate::cluster::read_kmesh(&std::fs::read(fixtures.join("fir_a_near.kmesh")).unwrap()).unwrap();
        let (name,vertices,indices)=source.iter().find(|(name,_,_)|name.is_empty()).unwrap();
        assert!(name.is_empty());
        let texture=[TextureSource{name:"authored pine source".into(),rgba:image::open(fixtures.join("foliage-shadow-fixture/authored_pine_albedo.png")).unwrap().to_rgba8(),normal:None}];
        let materials=[Material{flags:ALPHA_TEST,tint:[1.0;3],layer:0,roughness:1.0,scale:1.0,metal:0.0}];
        let parts=[Part{vertices,indices,material:0}];
        let instances=[SurfaceInstance{parts:&parts,transform:Mat4::IDENTITY}];
        let layout=TileLayout{sun:Vec3::Z,origin:Vec2::new(-8.0,0.0),pixel_size:0.5,size:[32,40],depth:[-8.0,8.0],depth_samples:17};
        let tile=bake_surfaces(layout,&instances,&materials,&texture,4,0.42).unwrap();
        let last=&tile.visibility[16*32*40..];
        assert!(last[..14*32].iter().all(|t|*t==65535),"no leaf shadow below actual 7.2m crown base");
        let fractional=last.iter().filter(|t|**t>0 && **t<65535).count();
        let blocked=last.iter().filter(|t|**t<65535).count();
        assert!(fractional>0 && blocked>0 && blocked<last.len());
        let projected_area=last.iter().map(|t|1.0-f64::from(*t)/65535.0).sum::<f64>()*0.25;
        println!("source crown directional tile: projected opaque area={projected_area:.6}m2; fractional pixels={fractional}; stored={} bytes; raster samples={}",tile.visibility.len()*2,tile.raster_samples);
    }
}
