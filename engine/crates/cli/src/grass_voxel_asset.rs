//! Source-clump foliage volume data, currently CPU/offline only.
//!
//! One photo/three-card asset is shared by all plants and all views. Sparse
//! 3D mips keep covered area, local color and normal moments; a plant only
//! stores its stable world-cell identity and the same rest transform as live
//! grass. This does not enable a new renderer or replace the live mean field.
//!
//! Rendering contract: voxel RGB is untinted linear photo RGB. Apply the
//! instance's seasonal tint once, then the SAME local-height color factor,
//! wetness and statistical leaf BRDF as near grass. Source vertex normals
//! are all UP; their stored six-lobe moments are an explicit proxy, not a
//! reconstruction of photographed blade orientation. Extinction uses the
//! geometric card normals independently. A consumer must not silently use
//! those UP shading moments instead of the shared grass material response.

use glam::{Affine3A, Vec2, Vec3};
use image::RgbaImage;
use render::{cluster::{Part, Vertex}, foliage_voxel::{self, BakeConfig, VoxelAsset},
    scene::{ALPHA_TEST, GRASS, Material, TWO_SIDED, TextureSource}};
use sideways::{builder::Chunk, vegetation::Vegetation};
use std::{collections::BTreeMap, path::Path};

pub(crate) const GROUP_CELLS: i32 = 8;

/// A filtered opaque surface is not a homogeneous participating medium.
/// Coverage c transmits 1-c at an intersection; using exp(-c) incorrectly
/// makes a fully covered opaque blade transmit 36.8 percent of the light.
/// These original CPU data retain the exact three source planes plus a
/// coverage/premultiplied-color mip hierarchy. They are a diagnostic hybrid
/// candidate, not a new runtime path or an arbitrary extinction multiplier.
pub(crate) struct SurfaceClump {
    pub planes: Vec<SurfacePlane>,
    pub mips: Vec<CoverageMip>,
}
#[derive(Clone, Copy)]
pub(crate) struct SurfacePlane {
    pub origin: Vec3,
    pub u: Vec3,
    pub v: Vec3,
}
pub(crate) struct CoverageMip {
    pub width: usize,
    pub height: usize,
    /// Linear RGB times covered fraction, followed by covered fraction.
    pub texels: Vec<[f32; 4]>,
}
impl SurfacePlane {
    /// Exact rest-surface intersection. Wind uses the same inverse affine
    /// shear as the near cards; do not move a separate far billboard.
    pub fn intersect(self, origin: Vec3, direction: Vec3) -> Option<(f32, Vec2)> {
        let normal = self.u.cross(self.v);
        let denominator = direction.dot(normal);
        if denominator.abs() < 1e-8 { return None; }
        let distance = (self.origin-origin).dot(normal)/denominator;
        if distance < 0.0 { return None; }
        let q = origin + direction * distance - self.origin;
        Some((distance, Vec2::new(q.dot(self.u)/self.u.length_squared(), 1.0-q.dot(self.v)/self.v.length_squared())))
    }
}
impl SurfaceClump {
    pub fn from_source(vertices: &[Vertex], photo: &RgbaImage) -> Result<Self, String> {
        if vertices.len() != 12 || photo.width()==0 || photo.height()==0 { return Err("surface clump requires the shared three-card source".into()); }
        let planes = vertices.chunks_exact(4).map(|v|SurfacePlane {
            origin: Vec3::from(v[0].pos), u: Vec3::from(v[1].pos)-Vec3::from(v[0].pos),
            v: Vec3::from(v[2].pos)-Vec3::from(v[0].pos),
        }).collect();
        let linear = |v:u8| {let x=v as f32/255.0;if x<=0.04045{x/12.92}else{((x+0.055)/1.055).powf(2.4)}};
        let texels=photo.pixels().map(|p| {let a=if p[3]>=128{1.0}else{0.0};[linear(p[0])*a,linear(p[1])*a,linear(p[2])*a,a]}).collect();
        let mut mips=vec![CoverageMip{width:photo.width()as usize,height:photo.height()as usize,texels}];
        while mips.last().unwrap().width>1 || mips.last().unwrap().height>1 {
            let previous=mips.last().unwrap();let width=previous.width.div_ceil(2);let height=previous.height.div_ceil(2);
            let mut texels=Vec::with_capacity(width*height);
            for y in 0..height {for x in 0..width {
                let x0=x as f64*previous.width as f64/width as f64;let x1=(x+1)as f64*previous.width as f64/width as f64;
                let y0=y as f64*previous.height as f64/height as f64;let y1=(y+1)as f64*previous.height as f64/height as f64;
                let mut sum=[0.0f64;4];
                for sy in y0.floor()as usize..y1.ceil()as usize {for sx in x0.floor()as usize..x1.ceil()as usize {
                    let weight=(x1.min((sx+1)as f64)-x0.max(sx as f64))*(y1.min((sy+1)as f64)-y0.max(sy as f64));
                    let texel=previous.texels[sy.min(previous.height-1)*previous.width+sx.min(previous.width-1)];
                    for k in 0..4 {sum[k]+=texel[k]as f64*weight;}
                }}
                texels.push(sum.map(|v|(v/((x1-x0)*(y1-y0)))as f32));
            }}
            mips.push(CoverageMip{width,height,texels});
        }
        Ok(Self{planes,mips})
    }
    fn bilinear(&self,uv:Vec2,level:usize)->[f32;4] {
        let mip=&self.mips[level];let p=uv*Vec2::new(mip.width as f32,mip.height as f32)-Vec2::splat(0.5);
        let b=p.floor();let f=p-b;let mut result=[0.0;4];
        for y in 0..2 {for x in 0..2 {
            let ix=(b.x as i64+x).clamp(0,mip.width as i64-1)as usize;let iy=(b.y as i64+y).clamp(0,mip.height as i64-1)as usize;
            let weight=(if x==0{1.0-f.x}else{f.x})*(if y==0{1.0-f.y}else{f.y});
            for k in 0..4 {result[k]+=mip.texels[iy*mip.width+ix][k]*weight;}
        }} result
    }
    fn trilinear(&self,uv:Vec2,lod:f32)->[f32;4] {
        let lod=lod.clamp(0.0,(self.mips.len()-1)as f32);let lower=lod.floor()as usize;
        let a=self.bilinear(uv,lower);let b=self.bilinear(uv,(lower+1).min(self.mips.len()-1));
        std::array::from_fn(|k|a[k]+(b[k]-a[k])*lod.fract())
    }
    /// Bounded eight-tap anisotropic coverage filtering. No alpha rescaling
    /// or threshold after filtering: the result is mean surface coverage.
    pub fn sample(&self,uv:Vec2,dx:Vec2,dy:Vec2)->[f32;4] {
        let dimensions=Vec2::new(self.mips[0].width as f32,self.mips[0].height as f32);
        let ax=(dx*dimensions).length();let ay=(dy*dimensions).length();
        let (major,major_uv,minor)=if ax>=ay{(ax,dx,ay)}else{(ay,dy,ax)};
        let footprint=minor.max(major/8.0).max(1.0);let taps=(major/footprint).ceil().clamp(1.0,8.0)as u32;
        let mut result=[0.0;4];
        for i in 0..taps {
            let offset=(i as f32+0.5)/taps as f32-0.5;
            let p=self.trilinear(uv+major_uv*offset,footprint.log2());
            for k in 0..4 {result[k]+=p[k]/taps as f32;}
        } result
    }
    /// Front-to-back composition of intersected surfaces, with filtered
    /// footprint per plane. Overlapping subpixel masks are still assumed
    /// independent inside a sample; the CPU fixture measures this residual
    /// against a finer source integration instead of calling it exact.
    pub fn ray(&self,origin:Vec3,direction:Vec3,dx:Vec3,dy:Vec3)->[f32;4] {
        let mut hits=Vec::with_capacity(3);
        for plane in &self.planes {
            let Some((distance,uv))=plane.intersect(origin,direction)else{continue;};
            if uv.min_element()<0.0||uv.max_element()>1.0{continue;}
            let Some((_,ux))=plane.intersect(origin+dx,direction)else{continue;};
            let Some((_,uy))=plane.intersect(origin+dy,direction)else{continue;};
            hits.push((distance,self.sample(uv,ux-uv,uy-uv)));
        }
        hits.sort_by(|a,b|a.0.total_cmp(&b.0));let mut output=[0.0;4];let mut t=1.0;
        for (_,sample) in hits {for k in 0..3 {output[k]+=t*sample[k];}t*=1.0-sample[3].clamp(0.0,1.0);}
        output[3]=1.0-t;output
    }
}

pub(crate) struct ClumpAsset {
    pub volume: VoxelAsset,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub photo: RgbaImage,
}

pub(crate) fn bake_clump(root: &Path, cell_size: f32) -> Result<ClumpAsset, String> {
    bake_photo(crate::ground_cover::photo(root)?, cell_size)
}

fn bake_photo(photo: RgbaImage, cell_size: f32) -> Result<ClumpAsset, String> {
    if !cell_size.is_finite() || !(0.01..=0.08).contains(&cell_size) {
        return Err("grass clump base cell must be finite and within 0.01..=0.08 metres".into());
    }
    let (vertices, indices) = crate::ground_cover::source_geometry();
    let materials = [Material { tint: [1.0; 3], layer: 0, roughness: 0.85,
        flags: ALPHA_TEST | TWO_SIDED | GRASS, scale: 1.0, metal: 0.0 }];
    let textures = [TextureSource { name: "Poly Haven grass_medium_01 selected CC0 clump".into(),
        rgba: photo, normal: None }];
    let volume = foliage_voxel::bake(&[Part { vertices: &vertices, indices: &indices, material: 0 }],
        &materials, &textures, BakeConfig { cell_size, levels: 7, samples_per_cell: 4, alpha_cut: 0.5,
            max_samples: 4_000_000, max_voxels: 65_536, max_texture_texels: 262_144,
            max_axis_cells: 128, ..Default::default() })?;
    let [texture] = textures;
    Ok(ClumpAsset { volume, vertices, indices, photo: texture.rgba })
}

/// Stable within one stage/field epoch. Never use a GPU pool slot as identity.
/// Rebuilding a stage or reusing a slot requires the existing motion-history
/// reset. Wind must sample the exact root (including ROOT_OFFSET) with
/// WindState::sample_shear(root, true), shared across geometry/volume/views.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ClumpInstance {
    pub id: [i32; 2],
    pub asset: u32,
    pub root: [f32; 3],
    pub scale: f32,
    pub yaw: f32,
    pub tint: [f32; 3],
}
impl ClumpInstance {
    pub fn transform(self) -> Affine3A {
        Affine3A::from_scale_rotation_translation(Vec3::splat(self.scale),
            glam::Quat::from_rotation_y(self.yaw), Vec3::from(self.root))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ClumpGroup {
    pub id: [i32; 2],
    pub first: u32,
    pub count: u32,
    /// Rest-pose bounds of actual source cards, not a flat terrain mean.
    /// GPU filtering/wind must expand these by their reconstruction support
    /// and the existing conservative wind displacement bound.
    pub bounds: [[f32; 3]; 2],
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ClumpPatch {
    pub instances: Vec<ClumpInstance>,
    pub groups: Vec<ClumpGroup>,
}

/// Group exact live candidates, with no camera-dependent membership/LOD.
/// Current grass identity has no extra seed: the stage terrain/habitat and
/// fixed integer world-cell hash are authoritative. Do not add independent
/// far color/placement hashes that would cause different plants at distance.
pub(crate) fn build_patch(chunks: &[Chunk], min: [i32; 2], max: [i32; 2],
    vegetation: Vegetation) -> Result<ClumpPatch, String> {
    let ground = crate::live_ground_cover::Ground::new(chunks);
    let candidates = crate::live_ground_cover::patch_candidates(&ground, min, max)?;
    let mut grouped: BTreeMap<[i32; 2], Vec<ClumpInstance>> = BTreeMap::new();
    for plant in candidates {
        let group = [plant.id.0.div_euclid(GROUP_CELLS), plant.id.1.div_euclid(GROUP_CELLS)];
        grouped.entry(group).or_default().push(ClumpInstance { id: [plant.id.0, plant.id.1],
            asset: 0, root: plant.position.to_array(), scale: plant.scale * vegetation.grass_scale(),
            yaw: plant.yaw, tint: vegetation.grass_tint() });
    }
    let (vertices, _) = crate::ground_cover::source_geometry();
    let mut patch = ClumpPatch { instances: Vec::new(), groups: Vec::new() };
    for (id, instances) in grouped {
        let first = patch.instances.len() as u32;
        let mut lo = Vec3::splat(f32::INFINITY); let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for instance in &instances { for vertex in &vertices {
            let point = instance.transform().transform_point3(Vec3::from(vertex.pos));
            lo = lo.min(point); hi = hi.max(point);
        }}
        patch.groups.push(ClumpGroup { id, first, count: instances.len() as u32,
            bounds: [lo.to_array(), hi.to_array()] });
        patch.instances.extend(instances);
    }
    Ok(patch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;
    use sideways::builder::N;
    fn source() -> RgbaImage {
        RgbaImage::from_fn(32, 16, |x, y| Rgba([if x < 16 { 220 } else { 30 },
            if x < 16 { 40 } else { 200 }, 20, if (x / 4 + y / 4) % 2 == 0 { 255 } else { 0 }]))
    }
    #[test]
    fn opaque_surface_does_not_become_a_translucent_medium() {
        let (vertices,_) = crate::ground_cover::source_geometry();
        let mut surface=SurfaceClump::from_source(&vertices,&RgbaImage::from_pixel(8,8,Rgba([120,200,20,255]))).unwrap();
        surface.planes.truncate(1);
        let output=surface.ray(Vec3::new(0.0,0.25,2.0),-Vec3::Z,Vec3::X*0.05,Vec3::Y*0.05);
        assert_eq!(output[3],1.0);
        assert!((1.0-(-1.0f32).exp()-output[3]).abs()>0.36);
        let half=RgbaImage::from_fn(8,8,|x,_|Rgba([120,200,20,if x<4{255}else{0}]));
        let surface=SurfaceClump::from_source(&vertices,&half).unwrap();
        let sample=surface.sample(Vec2::splat(0.5),Vec2::X,Vec2::Y);
        assert!((sample[3]-0.5).abs()<1e-6);
        assert!((1.0-(-0.5f32).exp()-sample[3]).abs()>0.1);
    }
    #[test]
    fn surface_mips_preserve_coverage_and_color_without_global_alpha_boost() {
        let (vertices,_) = crate::ground_cover::source_geometry();
        let image=RgbaImage::from_fn(17,11,|x,y|Rgba([if x<8{240}else{10},120,20,if(x+y)%3==0{255}else{0}]));
        let surface=SurfaceClump::from_source(&vertices,&image).unwrap();
        let mean=|mip:&CoverageMip|std::array::from_fn::<_,4,_>(|k|mip.texels.iter().map(|p|p[k]as f64).sum::<f64>()/mip.texels.len()as f64);
        let expected=mean(&surface.mips[0]);
        for mip in &surface.mips {for (value,target) in mean(mip).into_iter().zip(expected){assert!((value-target).abs()<1e-6);}}
        let blank=SurfaceClump::from_source(&vertices,&RgbaImage::new(4,4)).unwrap();
        assert_eq!(blank.ray(Vec3::new(0.0,0.25,2.0),-Vec3::Z,Vec3::X,Vec3::Y),[0.0;4]);
    }
    fn moments(level: &foliage_voxel::VoxelLevel) -> [f64; 23] {
        let mut sum = [0.0; 23];
        for v in &level.voxels {
            sum[0] += v.surface_area as f64;
            for k in 0..13 { sum[1 + k] += v.directional_area[k] as f64; }
            for k in 0..3 { sum[14 + k] += (v.surface_area * v.albedo[k]) as f64; }
            for k in 0..6 { sum[17 + k] += (v.surface_area * v.normal_weights[k]) as f64; }
        }
        sum
    }
    #[test]
    fn clump_mips_preserve_spatial_color_voids_and_all_optical_mass() {
        let a = bake_photo(source(), 0.02).unwrap();
        let b = bake_photo(source(), 0.02).unwrap();
        assert_eq!(a.volume, b.volume);
        assert_eq!(a.volume.stats.triangles, 6);
        let fine = &a.volume.levels[0];
        let dense: usize = fine.dimensions.iter().map(|n| *n as usize).product();
        assert!(fine.voxels.len() * 4 < dense, "empty space must remain empty");
        assert!(fine.voxels.iter().any(|v| v.albedo[0] > v.albedo[1] * 2.0));
        assert!(fine.voxels.iter().any(|v| v.albedo[1] > v.albedo[0] * 2.0));
        let reference = moments(fine);
        for level in &a.volume.levels {
            for (actual, expected) in moments(level).into_iter().zip(reference) {
                assert!((actual - expected).abs() <= expected.abs().max(1.0) * 1e-6);
            }
            for cell in &level.voxels {
                assert_eq!(cell.normal_weights, [0.0, 0.0, 0.5, 0.5, 0.0, 0.0]);
            }
        }
        assert_eq!(reference[2], 0.0, "vertical ray sees edge-on source cards");
        let empty = bake_photo(RgbaImage::new(4, 4), 0.02).unwrap();
        assert!(empty.volume.levels.iter().all(|l| l.voxels.is_empty()));
    }
    #[test]
    fn patch_reuses_exact_live_roots_membership_and_season_transforms() {
        let chunks = [Chunk { cell: (0, 0), x0: 0.0, z0: 0.0,
            heights: (0..N * N).map(|i| (i % N) as f32 * 0.1 + (i / N) as f32 * 0.2).collect(),
            road_d: (0..N * N).map(|i| ((i % N) as f32 * 2.0 - 12.0).abs() - 3.0).collect(),
            trees: vec![], rocks: vec![], grass: vec![], ferns: vec![], stumps: vec![] }];
        let min = [8, 12]; let max = [48, 36];
        let ground = crate::live_ground_cover::Ground::new(&chunks);
        let expected = crate::live_ground_cover::patch_candidates(&ground, min, max).unwrap();
        assert!(expected.len() > 100);
        for vegetation in [Vegetation::default(), Vegetation::new(14, 0.3, sideways::vegetation::Season::Winter).unwrap()] {
            let patch = build_patch(&chunks, min, max, vegetation).unwrap();
            assert_eq!(patch.instances.len(), expected.len());
            for p in &patch.instances {
                let source = expected.iter().find(|v| [v.id.0, v.id.1] == p.id).unwrap();
                assert_eq!(p.transform(), source.transform(vegetation.grass_scale()));
                assert_eq!(p.tint, vegetation.grass_tint());
                assert!((p.root[0] - 12.0).abs() >= 3.9);
                assert!((p.root[1] - (p.root[0] * 0.05 + p.root[2] * 0.1 - 0.025)).abs() < 1e-5);
            }
            for group in &patch.groups {
                assert!(group.count <= 64);
                for p in &patch.instances[group.first as usize..(group.first + group.count) as usize] {
                    assert_eq!(group.id, [p.id[0].div_euclid(8), p.id[1].div_euclid(8)]);
                }
            }
        }
        assert!(crate::live_ground_cover::patch_candidates(&ground, [i32::MIN; 2], [i32::MAX; 2]).is_err());
    }
}
