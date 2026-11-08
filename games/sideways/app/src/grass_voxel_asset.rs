//! Source-clump diagnostics plus an explicit applied procedural grass slice.
//!
//! The older CPU photo diagnostic below shares one three-card asset. Sparse
//! 3D mips keep covered area, local color and normal moments; a plant only
//! stores its stable world-cell identity and the same rest transform as live
//! grass. The explicitly selected procedural slice later in this module uses
//! opaque meadow-blade assemblies instead and is attached to the live scene
//! builder. Its native cell membership and 36-blade per-tuft budget are fixed;
//! the ordinary play path still uses photographed grass.
//!
//! Photo-diagnostic rendering contract: voxel RGB is untinted linear photo RGB. Apply the
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
        rgba: photo, normal: None, orm: None }];
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


/// Applied, explicitly selected native + DLSS procedural source/voxel slice.
/// The photo-clump diagnostics above remain separate. This uses the same
/// opaque meadow-blade assembly as the shared branch-wind renderer, not photo
/// cards. This replaces only the exact native cells reported below.
pub(crate) struct ProceduralPatchReport {
    pub excluded: crate::live_ground_cover::CellRect,
    pub summary: String,
    pub metadata: serde_json::Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProceduralMode { Source, Automatic }
fn procedural_mode(value:Option<&str>)->Result<Option<ProceduralMode>,String> {
    match value {
        None|Some("0")=>Ok(None),Some("source")=>Ok(Some(ProceduralMode::Source)),
        Some("1")|Some("auto")=>Ok(Some(ProceduralMode::Automatic)),
        Some(_)=>Err("SIDEWAYS_PROCEDURAL_GRASS must be 0, source, or auto (1)".into()),
    }
}
fn identity(id:[i32;2])->u64 {(id[0] as u32 as u64)<<32 | id[1] as u32 as u64}
fn mix(mut x:u64)->u64 {x=(x^(x>>30)).wrapping_mul(0xbf58476d1ce4e5b9);x=(x^(x>>27)).wrapping_mul(0x94d049bb133111eb);x^(x>>31)}
fn variant(id:[i32;2],seed:u64)->usize {(mix(identity(id)^seed)%3)as usize}

const GRASS_PATCH_WIDTH:i32=8;
const GRASS_PATCH_DEPTH:i32=4;
const MAX_PATCH_GRASS:usize=32;
const MAX_PATCH_SAPLINGS:usize=4;
const SAPLING_CELL:f32=2.0;
const SAPLING_GENERATION:u32=0x5341_504c;

struct ProceduralAsset {
    parameters:render::foliage_assembly::Parameters,
    cooked:render::vegetation_asset::CookedPlant,
}
impl ProceduralAsset {
    fn recipe(&self)->&'static str {
        use render::foliage_assembly::Archetype;
        match self.parameters.archetype {
            Archetype::GrassTuft=>"reference_grass_tuft",
            Archetype::MeadowGrass=>"meadow_grass_v1",
            Archetype::MeadowPatch=>"meadow_patch_v1",
            Archetype::ConiferTwig=>"conifer_twig",
        }
    }
}
fn procedural_assets(vegetation:Vegetation)->Result<Vec<ProceduralAsset>,String> {
    use render::{foliage_assembly::{self,Limits,Parameters},vegetation_asset::CookedPlant};
    // Three meadow layouts keep the exact reference density: 36 blades of
    // 22 triangles, three reusable shapes, four occupied branch groups. The
    // fuller silhouette comes from shape/lean, not more placements or a tint
    // compensation. The two sapling growth variants remain unchanged.
    [101u64,303,707,1103,1907].into_iter().enumerate().map(|(index,seed)| {
        let grass=index<3;
        let mut parameters=if grass {Parameters::meadow_grass(seed^vegetation.seed)}
            else {Parameters::conifer_twig(seed^vegetation.seed)};
        parameters.growth=if grass {1.0}else if index==3 {0.85}else{1.15};
        let mut source=foliage_assembly::generate(parameters,Limits::default())?;
        let tint=if grass {vegetation.grass_tint()}else{vegetation.pine_tint()};
        for material in &mut source.materials {
            // Wood retains its own measured material; seasonal foliage colour
            // enters both the primitive and its derived volume exactly once.
            if material.flags&render::scene::THIN_FOLIAGE!=0 {
                for k in 0..3 {material.tint[k]*=tint[k];}
            }
        }
        let cooked=CookedPlant::cook(&source,BakeConfig {
            projection_measure:foliage_voxel::ProjectionMeasure::ClosedSurfaceEntries,
            cell_size:if grass {0.025}else{0.018},levels:4,samples_per_cell:4,max_samples:4_000_000,
            max_voxels:131_072,max_texture_texels:0,..Default::default()
        })?;
        Ok(ProceduralAsset {parameters,cooked})
    }).collect()
}

fn patch_bounds(stage:&sideways::stage::Stage,ground:&crate::live_ground_cover::Ground,cameras:&[Vec3],
    explicit:Option<&str>)->Result<(crate::live_ground_cover::CellRect,Vec<crate::live_ground_cover::Tuft>),String> {
    use crate::live_ground_cover::{CELL,CellRect,patch_candidates};
    let evaluate=|min:[i32;2]|->Result<_,String> {
        let max=[min[0].checked_add(GRASS_PATCH_WIDTH).ok_or("grass patch cell overflow")?,
            min[1].checked_add(GRASS_PATCH_DEPTH).ok_or("grass patch cell overflow")?];
        let bounds=CellRect {min,max};let candidates=patch_candidates(ground,min,max)?;Ok((bounds,candidates))
    };
    if let Some(text)=explicit {
        let values=text.split(',').map(str::trim).map(str::parse::<i32>).collect::<Result<Vec<_>,_>>()
            .map_err(|_|"SIDEWAYS_PROCEDURAL_GRASS_CELL must be integer x,z")?;
        if values.len()!=2{return Err("SIDEWAYS_PROCEDURAL_GRASS_CELL must be integer x,z".into());}
        let patch=evaluate([values[0],values[1]])?;
        if patch.1.is_empty(){return Err("explicit procedural grass patch has no accepted native plants".into());}
        return Ok(patch);
    }
    let camera=cameras.first().filter(|p|p.is_finite()).ok_or("procedural vegetation needs one finite camera")?;
    let along=stage.nearest_index(camera.x as f64,camera.z as f64).max(0)as f64*sideways::stage::STEP+70.0;
    let mut best=None;
    for advance in [0.0,8.0,-8.0,16.0,-16.0] {for side in [-1.0f32,1.0] {for offset in [4.0f32,7.0,11.0] {
        let along=(along+advance).clamp(4.0,(stage.finish-12.0).max(4.0));
        let point=stage.point_at(along);let dir=stage.dir_at(along);
        let right=Vec2::new(-dir.z,dir.x);let centre=Vec2::new(point.x,point.z)+right*side*(stage.width_at(along)as f32*0.5+offset);
        let min=[(centre.x/CELL).floor()as i32-GRASS_PATCH_WIDTH/2,(centre.y/CELL).floor()as i32-GRASS_PATCH_DEPTH/2];
        let patch=evaluate(min)?;
        if best.as_ref().is_none_or(|(_,tufts):&(CellRect,Vec<crate::live_ground_cover::Tuft>)|patch.1.len()>tufts.len()) {best=Some(patch);}
    }}}
    best.filter(|(_,tufts)|!tufts.is_empty()).ok_or("no native grass beside the route for bounded procedural patch".into())
}

#[derive(Clone,Copy,Debug,PartialEq)]
struct Sapling {cell:[i32;2],root:Vec3,scale:f32,yaw:f32,asset:usize}
// Grass IDs retain their existing signed x/z encoding. Its accepted x range
// is +/-20,000,000 cells. Saplings use a disjoint high-word namespace and
// signed24-bit 2m cell x (well beyond this patch's possible +/-4,500,000).
fn sapling_identity(cell:[i32;2])->u64 {
    ((0x4000_0000u64|(cell[0]as u32 as u64&0x00ff_ffff))<<32)|cell[1]as u32 as u64
}
impl Sapling {
    fn transform(self)->glam::Mat4 {
        glam::Mat4::from_scale_rotation_translation(Vec3::splat(self.scale),glam::Quat::from_rotation_y(self.yaw),self.root)
    }
}
// Exact native triangle support, including the same diagonal as live grass
// and collision. This only chooses new decorative plants, never edits terrain.
fn sapling_ground(chunks:&[Chunk],point:Vec2)->Option<(f32,f32,f32)> {
    use sideways::{builder::N,stage::GRID};
    for c in chunks {
        let q=(point-Vec2::new(c.x0 as f32,c.z0 as f32))/GRID as f32;
        if !(0.0..=(N-1)as f32).contains(&q.x)||!(0.0..=(N-1)as f32).contains(&q.y){continue;}
        let i=(q.x.floor()as usize).min(N-2);let j=(q.y.floor()as usize).min(N-2);
        let x=q.x-i as f32;let z=q.y-j as f32;let a=j*N+i;
        let (ids,w,dx,dz)=if x+z<=1.0 {
            ([a,a+1,a+N],[1.0-x-z,x,z],c.heights[a+1]-c.heights[a],c.heights[a+N]-c.heights[a])
        }else {
            ([a+N+1,a+N,a+1],[x+z-1.0,1.0-x,1.0-z],c.heights[a+N+1]-c.heights[a+N],c.heights[a+N+1]-c.heights[a+1])
        };
        let sample=|v:&[f32]|(0..3).map(|k|v[ids[k]]*w[k]).sum::<f32>();
        let slope=(dx*dx+dz*dz)/(GRID as f32).powi(2);
        return Some((sample(&c.heights),sample(&c.road_d),1.0/(1.0+slope).sqrt()));
    }
    None
}
fn saplings(chunks:&[Chunk],excluded:crate::live_ground_cover::CellRect,vegetation:Vegetation,generation:u32)->Vec<Sapling> {
    let centre=Vec2::new((excluded.min[0]+excluded.max[0])as f32,(excluded.min[1]+excluded.max[1])as f32)
        *(crate::live_ground_cover::CELL*0.5);
    let base=(centre/SAPLING_CELL).floor();
    let seed=vegetation.seed^u64::from(generation).rotate_left(27)^0x7a3d_485b_209c_166d;
    let unit=|h:u64|(mix(h)>>40)as f32/16_777_216.0;
    let mut candidates=Vec::new();
    for x in -5..=5 {for z in -5..=5 {
        let cell=[base.x as i32+x,base.y as i32+z];let h=mix(identity(cell)^seed);
        let p=(Vec2::new(cell[0]as f32,cell[1]as f32)+Vec2::new(0.5+(unit(h)-0.5)*0.6,0.5+(unit(h^19)-0.5)*0.6))*SAPLING_CELL;
        if !(2.0..=10.0).contains(&p.distance(centre)){continue;}
        let Some((height,road,up))=sapling_ground(chunks,p)else{continue;};
        if road<3.0||up<0.88{continue;}
        // The entire small root footprint must lie on supported, gentle ground
        // outside the drive corridor, including the biggest growth variant.
        let supported=(0..8).all(|i| {
            let a=i as f32*std::f32::consts::TAU/8.0;
            sapling_ground(chunks,p+Vec2::new(a.cos(),a.sin())*0.65)
                .is_some_and(|(y,d,n)|d>=2.5&&n>=0.80&&(y-height).abs()<=0.30)
        });
        if !supported{continue;}
        if chunks.iter().any(|c|c.trees.iter().chain(c.rocks.iter()).any(|spot|
            p.distance(Vec2::new(spot.pos.0,spot.pos.2))<1.5)){continue;}
        // Structural growth is cooked once per asset; uniform instance scale
        // is a separate, deterministic age variation. No layout RNG consumed.
        let scale=(1.25+unit(h^71)*0.65)*(0.65+0.35*vegetation.growth());
        candidates.push((h,Sapling {cell,root:Vec3::new(p.x,height-0.02,p.y),scale,
            yaw:unit(h^173)*std::f32::consts::TAU,asset:3+(mix(h^817)&1)as usize}));
    }}
    candidates.sort_by_key(|(h,_)|*h);
    let mut result:Vec<Sapling>=Vec::new();
    for (_,plant) in candidates {
        if result.iter().all(|p|Vec2::new(p.root.x,p.root.z).distance(Vec2::new(plant.root.x,plant.root.z))>=2.0) {
            result.push(plant);if result.len()==MAX_PATCH_SAPLINGS{break;}
        }
    }
    result
}

/// Existing opt-in, now one shared engine pipeline for grass and saplings.
/// No global tree, physics, photo grass outside the exact cells, or default
/// rendering changes. Native and DLSS are supported; a source/volume or mip
/// change resets the affected view's history. Optical parity remains unvalidated.
pub(crate) fn attach_procedural_patch(scene:&mut render::Scene,stage:&sideways::stage::Stage,
    chunks:&[Chunk],cameras:&[Vec3],mesh_names:&mut Vec<String>)->Result<Option<ProceduralPatchReport>,String> {
    let value=std::env::var("SIDEWAYS_PROCEDURAL_GRASS").ok();
    let Some(mode)=procedural_mode(value.as_deref())?else{return Ok(None);};
    if std::env::var("SIDEWAYS_FAR_GRASS").as_deref()==Ok("1") {
        return Err("procedural vegetation requires SIDEWAYS_FAR_GRASS=0: the statistical field cannot subtract exact source coverage".into());
    }
    let explicit=std::env::var("SIDEWAYS_PROCEDURAL_GRASS_CELL").ok();
    attach_procedural_patch_with_mode(scene,stage,chunks,cameras,mesh_names,mode,explicit.as_deref()).map(Some)
}

pub(crate) fn attach_procedural_patch_with_mode(scene:&mut render::Scene,stage:&sideways::stage::Stage,
    chunks:&[Chunk],cameras:&[Vec3],mesh_names:&mut Vec<String>,mode:ProceduralMode,explicit:Option<&str>)
    ->Result<ProceduralPatchReport,String> {
    if scene.far_grass.is_some(){return Err("procedural vegetation cannot overlap the statistical far-grass field".into());}
    if scene.voxel_forest.is_some()||!scene.part_wind.is_empty()||!scene.foliage_representations.is_empty()
        ||!scene.foliage_source_groups.is_empty()||!scene.part_attachments.is_empty() {
        return Err("procedural vegetation requires an unused experimental source/volume pool".into());
    }
    let started=std::time::Instant::now();
    let ground=crate::live_ground_cover::Ground::new(chunks);
    let (excluded,tufts)=patch_bounds(stage,&ground,cameras,explicit)?;
    let generation=stage.noise_seed as u32^(stage.detail_seed as u32).rotate_left(13);
    let saplings=saplings(chunks,excluded,stage.vegetation,generation);
    let assets=procedural_assets(stage.vegetation)?;
    let bake_ms=started.elapsed().as_secs_f64()*1000.0;
    attach_procedural(scene,&assets,&tufts,&saplings,excluded,stage.vegetation,generation,mode,mesh_names,bake_ms)
}

fn attach_procedural(scene:&mut render::Scene,assets:&[ProceduralAsset],tufts:&[crate::live_ground_cover::Tuft],
    saplings:&[Sapling],excluded:crate::live_ground_cover::CellRect,vegetation:Vegetation,generation:u32,mode:ProceduralMode,
    mesh_names:&mut Vec<String>,bake_ms:f64)->Result<ProceduralPatchReport,String> {
    use render::{foliage_representation::Policy,scene::GeometryCategory,
        vegetation_asset::{AttachLimits,PlantInstance,SceneVegetation}};
    if assets.len()!=5||tufts.is_empty()||tufts.len()>MAX_PATCH_GRASS||saplings.len()>MAX_PATCH_SAPLINGS {
        return Err("procedural vegetation exceeds its 32 grass / 4 sapling bound".into());
    }
    if tufts.iter().any(|p|!excluded.contains(p.id))||saplings.iter().any(|p|!(3..5).contains(&p.asset)) {
        return Err("procedural vegetation placement does not match its source identity".into());
    }
    let started=std::time::Instant::now();let first_mesh=scene.meshes.len();
    let mut builder=SceneVegetation::new(scene,AttachLimits::default())?;
    let mut provenance=Vec::new();let mut expanded_vertices=0usize;let mut expanded_triangles=0usize;
    let mut growth_counts=[0usize;5];
    for (grass,index,id,root,asset,generation) in tufts.iter().enumerate().map(|(i,p)| {
        let id=[p.id.0,p.id.1];(true,i,id,glam::Mat4::from(p.transform(vegetation.grass_scale())),variant(id,vegetation.seed),generation)
    }).chain(saplings.iter().enumerate().map(|(i,p)| (false,i,p.cell,p.transform(),p.asset,generation^SAPLING_GENERATION))) {
        let cooked=&assets[asset].cooked;let stats=cooked.stats();
        expanded_vertices+=stats.expanded_source_vertices;expanded_triangles+=stats.expanded_source_triangles;growth_counts[asset]+=1;
        let plant_id=if grass {identity(id)}else{sapling_identity(id)};
        let handle=builder.append(cooked,PlantInstance {id:plant_id,generation,root,
            category:if grass {GeometryCategory::Grass}else{GeometryCategory::Trees}})?;
        provenance.push(serde_json::json!({"species":if grass {"grass"}else{"conifer_sapling"},"recipe":assets[asset].recipe(),"cell":id,
            "plant_id":plant_id.to_string(),"generation":generation,"variant":asset,
            "root":root.w_axis.truncate().to_array(),"rest_transform":root.to_cols_array(),
            "source_index":index,"pose_range":[handle.pose_range.start,handle.pose_range.end],
            "instance_range":[handle.instance_range.start,handle.instance_range.end],
            "volume_range":[handle.volume_range.start,handle.volume_range.end],"branch_owners":handle.branch_owners}));
    }
    let report=builder.finish(Policy {max_error_pixels:1.0,allow_unvalidated_optics:mode==ProceduralMode::Automatic,..Default::default()})?;
    for mesh in first_mesh..scene.meshes.len() {mesh_names.push(format!("shared procedural vegetation primitive {}",mesh-first_mesh));}
    scene.voxel_lod_pixels=1.0;scene.voxel_active_tiles=true;
    let prepare_ms=bake_ms+started.elapsed().as_secs_f64()*1000.0;
    let expanded_bytes=expanded_vertices*std::mem::size_of::<Vertex>()+expanded_triangles*3*std::mem::size_of::<u32>();
    let grass_stats=assets[0].cooked.stats();
    let summary=format!("unified procedural vegetation: {} meadow grass tufts (36 blades each) + {} conifer saplings, {} shared primitive meshes / {} primitive instances; {} poses / {} volume owners; {} shared geometry bytes vs {} expanded source bytes; {} packed volume bytes; {:.1} ms preparation; {:?}, native + DLSS, view history resets on representation/mip change, optical parity unvalidated; fixed 8x4-cell patch",
        tufts.len(),saplings.len(),report.primitive_meshes,report.primitive_instances,report.pose_nodes,report.volume_instances,
        report.shared_geometry_bytes,expanded_bytes,report.packed_volume_bytes,prepare_ms,mode);
    // Keep independently meaningful objects outside the parent json! macro.
    // Deep token recursion here must not require a crate-wide recursion limit.
    let grass_per_plant=serde_json::json!({"blades":assets[0].parameters.density,
        "primitive_placements":grass_stats.placements,"finest_source_triangles":grass_stats.expanded_source_triangles,
        "pose_nodes":grass_stats.branches,"volume_owners":grass_stats.volume_assets});
    let species_counts=serde_json::json!({"grass":tufts.len(),"conifer_sapling":saplings.len()});
    let growth_variants=assets.iter().enumerate().map(|(i,a)|serde_json::json!({"variant":i,"growth":a.parameters.growth,
        "species":if i<3 {"grass"}else{"conifer_sapling"},"recipe":a.recipe(),"density":a.parameters.density,
        "layout_seed":a.parameters.seed,"plants":growth_counts[i]})).collect::<Vec<_>>();
    let cell_dimensions=[excluded.max[0]-excluded.min[0],excluded.max[1]-excluded.min[1]];
    let extent_metres=cell_dimensions.map(|cells|cells as f32*crate::live_ground_cover::CELL);
    let mut metadata=serde_json::json!({"kind":"unified_shared_primitive_vegetation_slice","mode":format!("{mode:?}"),
        "cells_min":excluded.min,"cells_max":excluded.max,"plants":report.plants,
        "grass_recipe":assets[0].recipe(),"native_roots_preserved":true,
        "replacement_cell_size_m":crate::live_ground_cover::CELL,
        "replacement_cell_dimensions":cell_dimensions,"replacement_extent_metres":extent_metres,
        "grass_per_plant":grass_per_plant,"species_counts":species_counts,"assets":assets.len(),
        "season":vegetation.season,"stage_growth":vegetation.growth(),"growth_variants":growth_variants,
        "plants_rest":provenance,"collision":"decorative only; unchanged physics",
        "coverage_scope":"fixed 8x4 native grass cells (3.6x1.8 metres) plus at most four supported saplings; photographed grass remains outside; not global vegetation replacement"});
    let geometry_metadata=serde_json::json!({"primitive_meshes":report.primitive_meshes,"primitive_instances":report.primitive_instances,
        "branch_proxies":report.branch_proxies,"pose_nodes":report.pose_nodes,"volume_owners":report.volume_instances,
        "volume_assets":report.volume_assets,"source_vertices_shared":report.source_vertices_shared,"resident_vertices_shared":report.resident_vertices_shared,
        "source_triangles_shared":report.source_triangles_shared,"source_geometry_bytes":report.shared_geometry_bytes,
        "source_geometry_bytes_scope":"actual scene vertex/index/cluster payload; excludes instance/pose/volume buffers",
        "expanded_oracle_vertices":expanded_vertices,"expanded_oracle_triangles":expanded_triangles,"expanded_oracle_geometry_bytes":expanded_bytes,
        "expanded_oracle_bytes_scope":"per-placement expanded source vertices and u32 triangle indices, excludes CLOD metadata; not allocated for runtime",
        "packed_bytes":report.packed_volume_bytes,"cpu_prepare_ms":prepare_ms,"optical_equivalence_validated":false,
        "native_only":false,"render_modes":["native","dlss"],
        "temporal_history_policy":"source/volume or selected-mip change resets the whole affected view; temporal image quality remains unvalidated"});
    let serde_json::Value::Object(geometry_fields)=geometry_metadata else {unreachable!("json object literal");};
    metadata.as_object_mut().expect("json object literal").extend(geometry_fields);
    Ok(ProceduralPatchReport {excluded,summary,metadata})
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
    fn procedural_slice_is_explicit_and_cell_identity_has_no_signed_alias() {
        assert_eq!(procedural_mode(None).unwrap(),None);
        assert_eq!(procedural_mode(Some("0")).unwrap(),None);
        assert_eq!(procedural_mode(Some("source")).unwrap(),Some(ProceduralMode::Source));
        assert_eq!(procedural_mode(Some("auto")).unwrap(),Some(ProceduralMode::Automatic));
        assert!(procedural_mode(Some("dlss")).is_err());
        let ids=[[-1,-1],[0,-1],[-1,0],[0,0],[i32::MAX,i32::MIN],[i32::MIN,i32::MAX]];
        assert_eq!(ids.map(identity).into_iter().collect::<std::collections::BTreeSet<_>>().len(),ids.len());
    }

    #[test]
    fn procedural_slice_shares_primitives_across_species_and_keeps_exact_live_roots() {
        use render::{foliage_representation::{self,Representation,View,ViewPlanner},scene::{GeometryCategory,THIN_FOLIAGE}};
        use glam::Mat4;
        let vegetation=Vegetation::new(41,0.6,sideways::vegetation::Season::Autumn).unwrap();
        let assets=procedural_assets(vegetation).unwrap();
        assert_eq!(assets.len(),5);
        assert!(assets.iter().all(|asset|asset.cooked.stats().branches==5));
        for asset in &assets[..3] {
            assert_eq!(asset.parameters.archetype,render::foliage_assembly::Archetype::MeadowGrass);
            assert_eq!(asset.parameters.density,36);
            assert_eq!(asset.parameters.growth,1.0);
            assert_eq!(asset.cooked.stats().placements,36);
            assert_eq!(asset.cooked.stats().expanded_source_triangles,792);
            assert_eq!(asset.cooked.stats().volume_assets,4);
            assert!(asset.cooked.stats().primitive_meshes<=9,"three shapes times three shared material tones");
        }
        for (index,asset) in assets[3..].iter().enumerate() {
            assert_eq!(asset.parameters.archetype,render::foliage_assembly::Archetype::ConiferTwig);
            assert_eq!(asset.parameters.density,64);
            assert_eq!(asset.parameters.growth,if index==0 {0.85}else{1.15});
            assert_eq!(asset.cooked.stats().placements,69);
            assert_eq!(asset.cooked.stats().expanded_source_triangles,1_528);
        }
        let excluded=crate::live_ground_cover::CellRect {min:[-2,3],max:[6,7]};
        let tufts:Vec<_>=(0..4).map(|i|crate::live_ground_cover::Tuft {id:(i-2,3),
            position:Vec3::new(i as f32*0.45-0.9,-0.025,-70.0),scale:0.8+i as f32*0.13,yaw:i as f32*0.71}).collect();
        let saplings=[Sapling {cell:[4,-34],root:Vec3::new(8.1,-0.02,-68.0),scale:1.4,yaw:0.7,asset:3},
            Sapling {cell:[6,-34],root:Vec3::new(12.1,-0.02,-68.0),scale:1.7,yaw:1.2,asset:4}];
        let mut scene=render::Scene::default();let mut names=Vec::new();
        let report=attach_procedural(&mut scene,&assets,&tufts,&saplings,excluded,vegetation,19,
            ProceduralMode::Automatic,&mut names,0.0).unwrap();
        assert_eq!(report.metadata["plants"],6);assert_eq!(scene.part_wind.len(),30);
        assert_eq!(report.metadata["grass_recipe"],"meadow_grass_v1");
        assert_eq!(report.metadata["native_roots_preserved"],true);
        assert_eq!(report.metadata["native_only"],false);
        assert_eq!(report.metadata["render_modes"],serde_json::json!(["native","dlss"]));
        assert_eq!(report.metadata["optical_equivalence_validated"],false);
        assert_eq!(report.metadata["grass_per_plant"]["blades"],36);
        assert_eq!(report.metadata["grass_per_plant"]["finest_source_triangles"],792);
        assert_eq!(report.metadata["grass_per_plant"]["pose_nodes"],5);
        assert_eq!(report.metadata["grass_per_plant"]["volume_owners"],4);
        assert_eq!(report.metadata["replacement_cell_dimensions"],serde_json::json!([8,4]));
        assert_eq!(scene.foliage_representations.len(),26);
        assert_eq!(scene.foliage_source_groups.len(),26);
        foliage_representation::validate_bindings(&scene).unwrap();
        for (i,tuft) in tufts.iter().enumerate() {
            assert!(scene.part_wind[i*5].rest.abs_diff_eq(Mat4::from(tuft.transform(vegetation.grass_scale())),1e-6));
            assert_eq!(report.metadata["plants_rest"][i]["plant_id"],identity([tuft.id.0,tuft.id.1]).to_string());
            assert_eq!(report.metadata["plants_rest"][i]["recipe"],"meadow_grass_v1");
        }
        for (i,plant) in saplings.iter().enumerate() {
            assert!(scene.part_wind[(tufts.len()+i)*5].rest.abs_diff_eq(plant.transform(),1e-6));
        }
        assert!(scene.materials.iter().any(|m|m.flags&THIN_FOLIAGE!=0));
        assert!(scene.materials.iter().all(|m|m.flags&(GRASS|ALPHA_TEST)==0));
        let mut member_counts=[0usize;5];
        for group in &scene.foliage_source_groups {
            let proxy=&scene.instances[group.owner as usize];
            assert_eq!(proxy.cluster_count,0,"branch owners must not contain expanded source meshes");
            for &member in &group.members {
                let instance=&scene.instances[member as usize];assert!(instance.cluster_count>0);
                assert_eq!(instance.category(),proxy.category());member_counts[instance.category()as usize]+=1;
            }
        }
        assert_eq!(member_counts[GeometryCategory::Grass as usize],tufts.len()*36);
        assert_eq!(member_counts[GeometryCategory::Trees as usize],saplings.len()*69);
        assert_eq!(scene.part_attachments.len(),tufts.len()*36+saplings.len()*69);
        assert!(report.metadata["source_geometry_bytes"].as_u64().unwrap()*2<report.metadata["expanded_oracle_geometry_bytes"].as_u64().unwrap());
        assert_eq!(names.len(),scene.meshes.len());
        let view=View {view_projection:Mat4::perspective_rh(60f32.to_radians(),16.0/9.0,0.1,1000.0),width:1920,height:1080};
        let mut planner=ViewPlanner::default();
        assert!(planner.plan(&scene.foliage_representations,view,Default::default()).unwrap().iter()
            .all(|p|p.representation==Representation::Source),"unvalidated optics remains source by default");
        assert!(planner.plan(&scene.foliage_representations,view,scene.foliage_representation_policy).unwrap().iter()
            .all(|p|p.projected_error_pixels<=1.0));
        let mut source_scene=render::Scene::default();let mut source_names=Vec::new();
        attach_procedural(&mut source_scene,&assets,&tufts,&saplings,excluded,vegetation,19,ProceduralMode::Source,&mut source_names,0.0).unwrap();
        assert_eq!(scene.vertices.len(),source_scene.vertices.len());
        for (a,b)in scene.vertices.iter().zip(&source_scene.vertices){assert_eq!((a.pos,a.normal,a.uv),(b.pos,b.normal,b.uv));}
        assert_eq!(scene.instances.len(),source_scene.instances.len());
        for (a,b)in scene.instances.iter().zip(&source_scene.instances){assert_eq!((a.rows,a.sphere,a.first_cluster,a.cluster_count,a.flags),(b.rows,b.sphere,b.first_cluster,b.cluster_count,b.flags));}
        assert_eq!(scene.part_attachments.len(),source_scene.part_attachments.len());
        assert!(!source_scene.foliage_representation_policy.allow_unvalidated_optics);
        // One shared pool serves every view; filling the maximum patch must
        // not quietly truncate grass, saplings, branch poses or volume owners.
        let max_tufts:Vec<_>=(excluded.min[0]..excluded.max[0]).flat_map(|x|
            (excluded.min[1]..excluded.max[1]).map(move|z|crate::live_ground_cover::Tuft {
                id:(x,z),position:Vec3::new(x as f32*0.45,-0.025,z as f32*0.45),scale:1.0,yaw:0.0})).collect();
        let max_saplings:Vec<_>=(0..4).map(|i|Sapling {cell:[30+i,-34],
            root:Vec3::new(60.0+i as f32*2.0,-0.02,-68.0),..saplings[i as usize%2]}).collect();
        let mut full=render::Scene::default();
        let full_report=attach_procedural(&mut full,&assets,&max_tufts,&max_saplings,excluded,vegetation,19,
            ProceduralMode::Source,&mut Vec::new(),0.0).unwrap();
        assert_eq!(full_report.metadata["plants"],36);assert_eq!(full.part_wind.len(),180);
        assert_eq!(full.foliage_representations.len(),148);
        assert!(full.part_wind.len()<=render::part_wind::MAX_PARTS);
        assert_eq!(full.part_attachments.len(),32*36+4*69);
        foliage_representation::validate_bindings(&full).unwrap();
        let accounting=render::vegetation_accounting::Accounting::new(&full).unwrap();
        let inventory=accounting.inventory();
        let source_triangles:u64=inventory.categories.iter().map(|c|c.source_triangles_full_detail).sum();
        let primitive_placements:u64=inventory.categories.iter().map(|c|c.source_primitive_placements).sum();
        let volume_owners:u64=inventory.categories.iter().map(|c|c.branch_proxies).sum();
        // Reviewed all-visible source maximum for one view, counting every
        // placement; shared asset triangles are not this draw budget.
        // Four views may draw the same source four times; shadows are separate.
        assert!(source_triangles<=31_456,"patch source triangle budget exceeded: {source_triangles}");
        assert!(primitive_placements<=1_428,"patch primitive placement budget exceeded: {primitive_placements}");
        assert!(full.part_wind.len()<=180,"patch pose budget exceeded");
        assert!(volume_owners<=148,"patch volume owner budget exceeded: {volume_owners}");
        assert!(full.voxel_forest.as_ref().unwrap().instances.len()<=148,"patch volume instance budget exceeded");
        assert_eq!(source_triangles,32*792+4*1_528,"meadow preserves the reviewed finest source budget");
        assert_eq!(primitive_placements,32*36+4*69,"no dropped or extra grass blades");
    }
    fn placement_chunk()->Chunk {
        Chunk {cell:(0,0),x0:0.0,z0:0.0,heights:(0..N*N).map(|i|(i%N)as f32*0.02).collect(),
            road_d:(0..N*N).map(|i|((i%N)as f32*2.0-12.0).abs()-3.0).collect(),
            trees:vec![],rocks:vec![],grass:vec![],ferns:vec![],stumps:vec![]}
    }
    #[test]
    fn procedural_saplings_have_separate_stable_seed_and_supported_offroad_footprints() {
        let chunks=[placement_chunk()];let bounds=crate::live_ground_cover::CellRect {min:[45,45],max:[53,49]};
        let vegetation=Vegetation::default();
        let first=saplings(&chunks,bounds,vegetation,73);assert_eq!(first.len(),4);
        for p in &first {assert_ne!(identity(p.cell),sapling_identity(p.cell));}
        assert_eq!([[-4_500_000,-4_500_000],[4_500_000,4_500_000],[-1,0],[0,0]].map(sapling_identity).into_iter().collect::<std::collections::BTreeSet<_>>().len(),4);
        assert_eq!(first,saplings(&chunks,bounds,vegetation,73));
        assert_ne!(first,saplings(&chunks,bounds,vegetation,74));
        let winter=saplings(&chunks,bounds,Vegetation::new(0,0.3,sideways::vegetation::Season::Winter).unwrap(),73);
        assert_eq!(first.iter().map(|p|(p.cell,p.root,p.yaw,p.asset)).collect::<Vec<_>>(),winter.iter().map(|p|(p.cell,p.root,p.yaw,p.asset)).collect::<Vec<_>>());
        for p in &first {
            let (height,road,up)=sapling_ground(&chunks,Vec2::new(p.root.x,p.root.z)).unwrap();
            assert!((p.root.y-(height-0.02)).abs()<1e-6&&road>=3.0&&up>=0.88);
        }
        let mut steep=placement_chunk();for (i,h)in steep.heights.iter_mut().enumerate(){*h=(i%N)as f32*3.0;}
        assert!(saplings(&[steep],bounds,vegetation,73).is_empty());
        let mut road=placement_chunk();road.road_d.fill(-1.0);
        assert!(saplings(&[road],bounds,vegetation,73).is_empty());
        assert!(saplings(&[],bounds,vegetation,73).is_empty());
    }
    #[test]
    fn procedural_stage_membership_reuses_all_and_only_excluded_native_grass_cells() {
        use sideways::{builder,stage::{Stage,CHUNK}};
        let stage=Stage::generate(&mut gdcompat::Pcg::new(1),1600.0);
        let p=stage.point_at(70.0);let cell=((p.x/CHUNK as f32).floor()as i32,(p.z/CHUNK as f32).floor()as i32);
        let chunks:Vec<_>=(-1..=1).flat_map(|x|(-1..=1).map(move|z|(cell.0+x,cell.1+z))).map(|c|builder::terrain_chunk(&stage,c)).collect();
        let ground=crate::live_ground_cover::Ground::new(&chunks);
        let camera=stage.point_at(0.0);let camera=Vec3::new(camera.x,camera.y+2.0,camera.z);
        let before=(stage.noise_seed,stage.detail_seed,stage.points.clone());
        let (bounds,tufts)=patch_bounds(&stage,&ground,&[camera],None).unwrap();
        assert!(!tufts.is_empty()&&tufts.len()<=MAX_PATCH_GRASS);
        assert_eq!(tufts,crate::live_ground_cover::patch_candidates(&ground,bounds.min,bounds.max).unwrap());
        let explicit=format!("{},{}",bounds.min[0],bounds.min[1]);
        let (again,repeated)=patch_bounds(&stage,&ground,&[camera+Vec3::splat(500.0)],Some(&explicit)).unwrap();
        assert_eq!(bounds,again);assert_eq!(tufts,repeated,"explicit world cells are independent of camera/view count");
        assert_eq!(before,(stage.noise_seed,stage.detail_seed,stage.points.clone()));
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
