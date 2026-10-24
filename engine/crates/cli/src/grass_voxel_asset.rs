//! Source-clump diagnostics plus an explicit applied procedural grass slice.
//!
//! The older CPU photo diagnostic below shares one three-card asset. Sparse
//! 3D mips keep covered area, local color and normal moments; a plant only
//! stores its stable world-cell identity and the same rest transform as live
//! grass. The explicitly selected procedural slice later in this module uses
//! opaque blade assemblies instead and is attached to the live scene builder.
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


/// Applied, explicitly selected native-only procedural source/voxel slice.
/// The photo-clump diagnostics above remain separate. This uses the same
/// opaque blade assembly as the shared branch-wind renderer, not photo cards.
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

struct ProceduralAsset {
    source: render::foliage_assembly::Assembly,
    branches: Vec<render::foliage_assembly::BakedBranch>,
}
fn procedural_assets(vegetation:Vegetation)->Result<Vec<ProceduralAsset>,String> {
    use render::foliage_assembly::{self,Limits,Parameters};
    [101u64,303,707].into_iter().map(|seed| {
        // Growth stays in the common root transform; colour is baked exactly
        // once into both source materials and derived spatial voxel moments.
        let mut source=foliage_assembly::generate(Parameters::grass(seed^vegetation.seed),Limits::default())?;
        let tint=vegetation.grass_tint();
        for material in &mut source.materials {for k in 0..3 {material.tint[k]*=tint[k];}}
        let branches=source.bake_branches(BakeConfig {
            projection_measure:foliage_voxel::ProjectionMeasure::ClosedSurfaceEntries,
            cell_size:0.025,levels:4,samples_per_cell:4,max_samples:4_000_000,
            max_voxels:131_072,max_texture_texels:0,..Default::default()
        })?;
        Ok(ProceduralAsset {source,branches})
    }).collect()
}

fn patch_bounds(stage:&sideways::stage::Stage,ground:&crate::live_ground_cover::Ground,cameras:&[Vec3],
    explicit:Option<&str>)->Result<(crate::live_ground_cover::CellRect,Vec<crate::live_ground_cover::Tuft>),String> {
    use crate::live_ground_cover::{CELL,CellRect,patch_candidates};
    let evaluate=|min:[i32;2]|->Result<_,String> {
        let max=[min[0].checked_add(8).ok_or("grass patch cell overflow")?,min[1].checked_add(6).ok_or("grass patch cell overflow")?];
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
    let camera=cameras.first().filter(|p|p.is_finite()).ok_or("procedural grass needs one finite camera")?;
    let along=stage.nearest_index(camera.x as f64,camera.z as f64).max(0)as f64*sideways::stage::STEP+70.0;
    let mut best=None;
    for advance in [0.0,8.0,-8.0,16.0,-16.0] {for side in [-1.0f32,1.0] {for offset in [4.0f32,7.0,11.0] {
        let along=(along+advance).clamp(4.0,(stage.finish-12.0).max(4.0));
        let point=stage.point_at(along);let dir=stage.dir_at(along);
        let right=Vec2::new(-dir.z,dir.x);let centre=Vec2::new(point.x,point.z)+right*side*(stage.width_at(along)as f32*0.5+offset);
        let min=[(centre.x/CELL).floor()as i32-4,(centre.y/CELL).floor()as i32-3];
        let patch=evaluate(min)?;
        if best.as_ref().is_none_or(|(_,tufts):&(CellRect,Vec<crate::live_ground_cover::Tuft>)|patch.1.len()>tufts.len()) {best=Some(patch);}
    }}}
    best.filter(|(_,tufts)|!tufts.is_empty()).ok_or("no native grass beside the route for bounded procedural patch".into())
}

/// Conservative displacement of this one rigid child around its fixed pivot.
/// Includes every filtered voxel level, so switching mips cannot escape the
/// source/volume selection bounds. Only uniform instance roots are used here.
fn branch_wind_margin(asset:&VoxelAsset,scale:f32,max_angle:f32)->f32 {
    let mut radius=Vec3::from(asset.source_bounds[0]).abs().max(Vec3::from(asset.source_bounds[1]).abs()).length();
    for mip in &asset.levels {
        let b=mip.local_bounds(asset.origin);let lo=Vec3::from(b[0])-Vec3::splat(mip.cell_size*0.5);
        let hi=Vec3::from(b[1])+Vec3::splat(mip.cell_size*0.5);
        radius=radius.max(lo.abs().max(hi.abs()).length());
    }
    2.0*(max_angle*0.5).sin()*radius*scale
}

/// Called once while assembling a live/shot scene, before the photo-grass
/// pool and before cars. Runtime view selection owns the representation;
/// this does not remove the source mesh unconditionally.
pub(crate) fn attach_procedural_patch(scene:&mut render::Scene,stage:&sideways::stage::Stage,
    chunks:&[Chunk],cameras:&[Vec3],mesh_names:&mut Vec<String>)->Result<Option<ProceduralPatchReport>,String> {
    let value=std::env::var("SIDEWAYS_PROCEDURAL_GRASS").ok();
    let Some(mode)=procedural_mode(value.as_deref())?else{return Ok(None);};
    if scene.voxel_forest.is_some() || !scene.part_wind.is_empty() {
        return Err("procedural grass slice cannot share the bounded volume/part pool with another experimental patch".into());
    }
    if std::env::var("SIDEWAYS_FAR_GRASS").as_deref()==Ok("1") {
        return Err("procedural grass slice requires SIDEWAYS_FAR_GRASS=0: the old statistical field cannot subtract exact source plant coverage".into());
    }
    let explicit=std::env::var("SIDEWAYS_PROCEDURAL_GRASS_CELL").ok();
    attach_procedural_patch_with_mode(scene,stage,chunks,cameras,mesh_names,mode,explicit.as_deref()).map(Some)
}

/// Same production attachment with explicit configuration for CPU acceptance;
/// no environment mutation, GPU, visual process or unrelated scene build.
pub(crate) fn attach_procedural_patch_with_mode(scene:&mut render::Scene,stage:&sideways::stage::Stage,
    chunks:&[Chunk],cameras:&[Vec3],mesh_names:&mut Vec<String>,mode:ProceduralMode,explicit:Option<&str>)
    ->Result<ProceduralPatchReport,String> {
    if scene.far_grass.is_some(){return Err("procedural grass slice cannot overlap the statistical far-grass field".into());}
    let started=std::time::Instant::now();
    let ground=crate::live_ground_cover::Ground::new(chunks);
    let (excluded,tufts)=patch_bounds(stage,&ground,cameras,explicit)?;
    let assets=procedural_assets(stage.vegetation)?;
    let bake_ms=started.elapsed().as_secs_f64()*1000.0;
    attach_procedural(scene,&assets,&tufts,excluded,stage.vegetation,
        stage.noise_seed as u32^(stage.detail_seed as u32).rotate_left(13),mode,mesh_names,bake_ms)
}

fn attach_procedural(scene:&mut render::Scene,assets:&[ProceduralAsset],tufts:&[crate::live_ground_cover::Tuft],
    excluded:crate::live_ground_cover::CellRect,vegetation:Vegetation,generation:u32,mode:ProceduralMode,
    mesh_names:&mut Vec<String>,bake_ms:f64)->Result<ProceduralPatchReport,String> {
    use render::{cluster::{ClusterMesh,SimplificationPolicy},foliage_representation::{Binding,Group,GroupKey,OpticalEvidence,Policy},
        foliage_volume_data::{self,ALL_MATERIALS,PackLimits,VolumeBinding},part_wind::PartWind,scene::PART_WIND};
    if assets.len()!=3 || tufts.is_empty() || tufts.len()>48 || tufts.len()*5>render::part_wind::MAX_PARTS {
        return Err("procedural grass slice exceeds its 48-plant/240-node bound".into());
    }
    if !scene.foliage_representations.is_empty() || scene.voxel_forest.is_some() || !scene.part_wind.is_empty() {
        return Err("procedural grass requires an unused experimental foliage pool".into());
    }
    let mut meshes=Vec::new();let mut volumes=Vec::new();let mut source_triangles=0usize;
    for (variant,asset) in assets.iter().enumerate() {
        let base=scene.materials.len()as u32;
        for &material in &asset.source.materials{scene.add_material(material);}
        let mut branch_meshes=Vec::new();
        for branch in &asset.branches {
            let source=asset.source.flatten_branch(branch.branch)?;
            let parts:Vec<_>=source.iter().map(|p|Part {vertices:&p.vertices,indices:&p.indices,material:base+p.material}).collect();
            let mesh=ClusterMesh::build_with_policy(&parts,|_|SimplificationPolicy::TexturedGeometry);
            source_triangles+=mesh.leaf_triangles();
            let mesh=scene.add_mesh(&mesh);mesh_names.push(format!("procedural grass variant {variant} branch {}",branch.branch));
            let volume=volumes.len()as u32;volumes.push(branch.asset.clone());branch_meshes.push((branch.branch,mesh,volume));
        }
        meshes.push(branch_meshes);
    }
    let mut bindings=Vec::new();let mut provenance=Vec::new();
    for tuft in tufts {
        let id=[tuft.id.0,tuft.id.1];
        if !excluded.contains(tuft.id){return Err("procedural grass plant outside its exact replacement cells".into());}
        let variant=variant(id,vegetation.seed);let asset=&assets[variant];
        let scale=tuft.scale*vegetation.grass_scale();let root=glam::Mat4::from(tuft.transform(vegetation.grass_scale()));
        let rest=asset.source.branch_transforms()?;let pose_base=scene.part_wind.len()as u32;
        for (branch_index,branch) in asset.source.branches.iter().enumerate() {
            let max_angle=if branch.parent.is_none(){0.0}else{0.22*(1.0-branch.stiffness).clamp(0.15,1.0)};
            let attached=meshes[variant].iter().find(|(b,_,_)|*b==branch_index as u32);
            let (owner,volume)=if let Some(&(_,mesh,asset_index))=attached {
                let owner=scene.instances.len()as u32;let transform=root*rest[branch_index];
                scene.add_instance(mesh,Affine3A::from_mat4(transform));scene.instances.last_mut().unwrap().flags|=PART_WIND;
                let volume=bindings.len()as u32;
                bindings.push(VolumeBinding {owner,asset:asset_index,crown_material:ALL_MATERIALS});
                let volume_asset=&volumes[asset_index as usize];
                let margin=branch_wind_margin(volume_asset,scale,max_angle);
                let group=Group::from_voxels(GroupKey {instance:identity(id),branch:branch.id,generation},
                    volume_asset,transform,margin,OpticalEvidence::Unvalidated)?;
                scene.foliage_representations.push(Binding {owner,volume,group});
                (Some(owner),Some(volume))
            }else{(None,None)};
            let phase=(mix(identity(id)^branch.id)>>40)as f32/16_777_216.0*std::f32::consts::TAU;
            scene.part_wind.push(PartWind {parent:branch.parent.map(|p|pose_base+p),owner,volume,
                rest:if branch.parent.is_none(){root*branch.rest}else{branch.rest},max_angle,
                harmonic:7+(branch.id%13)as u32,phase});
            provenance.push(serde_json::json!({"cell":id,"plant_id":identity(id).to_string(),"variant":variant,
                "branch_id":branch.id.to_string(),"owner":owner,"volume":volume,"pose":pose_base+branch_index as u32}));
        }
    }
    scene.voxel_forest=Some(foliage_volume_data::pack(&volumes,&bindings,&scene.instances,PackLimits {
        max_assets:16,levels_per_asset:4,first_level_bounds:false,max_dense_indices:8_388_608,max_sparse_cells:1_048_576,..Default::default()
    })?);
    scene.voxel_lod_pixels=1.0;scene.voxel_active_tiles=true;
    scene.foliage_representation_policy=Policy {max_error_pixels:1.0,allow_unvalidated_optics:mode==ProceduralMode::Automatic,..Default::default()};
    let bytes=scene.voxel_forest.as_ref().unwrap().byte_len();
    let summary=format!("procedural grass slice: {} world-fixed plants, {} shared branch assets / {} source triangles, {} pose nodes / {} volume owners, {bytes} packed bytes, {:.1} ms CPU preparation, cells {:?}..{:?}; mode {:?}, per-view <=1px selection, unvalidated optical parity, native-only, no photo grass inside patch",
        tufts.len(),volumes.len(),source_triangles,scene.part_wind.len(),bindings.len(),bake_ms,excluded.min,excluded.max,mode);
    let metadata=serde_json::json!({"kind":"procedural_source_voxel_grass_slice","mode":format!("{mode:?}"),
        "cells_min":excluded.min,"cells_max":excluded.max,"plants":tufts.len(),"assets":volumes.len(),
        "source_triangles_shared":source_triangles,"pose_nodes":scene.part_wind.len(),"volume_owners":bindings.len(),
        "packed_bytes":bytes,"cpu_prepare_ms":bake_ms,"optical_equivalence_validated":false,"native_only":true,
        "plants_rest":tufts.iter().map(|p|serde_json::json!({"cell":[p.id.0,p.id.1],"root":p.position.to_array(),
            "scale":p.scale*vegetation.grass_scale(),"yaw":p.yaw,"variant":variant([p.id.0,p.id.1],vegetation.seed)})).collect::<Vec<_>>(),
        "coverage_scope":"one fixed native-ground patch; no infinite-world replacement", "bindings":provenance});
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
    fn procedural_slice_shares_exact_source_volume_owners_and_contains_moving_support() {
        use render::{foliage_representation::{self,Representation,View,ViewPlanner},scene::THIN_FOLIAGE};
        use glam::Mat4;
        let vegetation=Vegetation::new(41,0.6,sideways::vegetation::Season::Autumn).unwrap();
        let assets=procedural_assets(vegetation).unwrap();
        assert_eq!(assets.len(),3);
        assert!(assets.iter().all(|asset|asset.branches.len()==4&&asset.source.branches.len()==5));
        assert!(assets.iter().flat_map(|asset|asset.branches.iter()).all(|b|
            b.asset.levels.len()==4&&b.asset.levels[0].voxels.iter().all(|v|v.leaf_fraction>0.99)));
        let excluded=crate::live_ground_cover::CellRect {min:[-2,3],max:[6,9]};
        let tufts:Vec<_>=(0..4).map(|i|crate::live_ground_cover::Tuft {id:(i-2,3),
            position:Vec3::new(i as f32*0.45-0.9,-0.025,-70.0),scale:0.8+i as f32*0.13,yaw:i as f32*0.71}).collect();
        let mut scene=render::Scene::default();let mut names=Vec::new();
        let report=attach_procedural(&mut scene,&assets,&tufts,excluded,vegetation,19,
            ProceduralMode::Automatic,&mut names,0.0).unwrap();
        assert_eq!(report.metadata["plants"],4);assert_eq!(scene.part_wind.len(),20);
        assert_eq!(scene.foliage_representations.len(),16);
        assert_eq!(scene.voxel_forest.as_ref().unwrap().levels.len(),48,"twelve assets, four levels, not one bake per plant");
        foliage_representation::validate_bindings(&scene).unwrap();
        for (i,tuft) in tufts.iter().enumerate() {
            assert!(scene.part_wind[i*5].rest.abs_diff_eq(Mat4::from(tuft.transform(vegetation.grass_scale())),1e-6));
        }
        // The generated geometry uses true leaf normals/materials, not the
        // old photo grass UP-normal proxy or alpha-cutout branch.
        assert!(scene.materials.iter().any(|m|m.flags&THIN_FOLIAGE!=0));
        assert!(scene.materials.iter().all(|m|m.flags&(GRASS|ALPHA_TEST)==0));
        for time in [0.0,0.7,2.3,17.0,51.0] {
            let poses=render::part_wind::sample_poses(&scene.part_wind,&render::wind::WindState {
                time_seconds:time,strength:1.0,..Default::default()
            }).unwrap();
            for (pose,part) in poses.iter().zip(&scene.part_wind) {
                let Some(volume)=part.volume else{continue;};
                let binding=&scene.foliage_representations[volume as usize];
                let tree=scene.voxel_forest.as_ref().unwrap().instances[volume as usize];
                for mip in &scene.voxel_forest.as_ref().unwrap().levels[tree.meta[1]as usize..(tree.meta[1]+tree.meta[2])as usize] {
                    let h=mip.origin_cell[3];let lo=Vec3::from_slice(&mip.origin_cell[..3])-Vec3::splat(h*0.5);
                    let hi=lo+Vec3::new(mip.dims_offset[0]as f32+1.0,mip.dims_offset[1]as f32+1.0,mip.dims_offset[2]as f32+1.0)*h;
                    for bits in 0..8 {
                        let p=Vec3::new(if bits&1==0{lo.x}else{hi.x},if bits&2==0{lo.y}else{hi.y},if bits&4==0{lo.z}else{hi.z});
                        assert!(pose.transform_point3(p).distance(binding.group.center)<=binding.group.radius+1e-4,
                            "moving filtered support must remain inside per-view selection bound");
                    }
                }
            }
        }
        let view=View {view_projection:Mat4::perspective_rh(60f32.to_radians(),16.0/9.0,0.1,1000.0),width:1920,height:1080};
        let mut planner=ViewPlanner::default();
        assert!(planner.plan(&scene.foliage_representations,view,Default::default()).unwrap().iter()
            .all(|p|p.representation==Representation::Source),"without optical approval source remains authoritative");
        assert!(planner.plan(&scene.foliage_representations,view,scene.foliage_representation_policy).unwrap().iter()
            .all(|p|p.projected_error_pixels<=1.0));
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
