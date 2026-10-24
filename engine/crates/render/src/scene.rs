//! What gets drawn: cluster meshes, instances of them, materials, the
//! textures they sample and the sky. Built on the CPU, uploaded once.

use crate::cluster::{ClusterMesh, GpuCluster, Vertex};
use bytemuck::{Pod, Zeroable};
use glam::{Affine3A, Vec3};
use std::collections::HashMap;

/// Material flags.
pub const ALPHA_TEST: u32 = 1;
pub const TERRAIN: u32 = 2;
pub const ROAD: u32 = 4;
pub const TWO_SIDED: u32 = 8;
/// Albedo projected from world space along the dominant normal axis.
pub const TRIPLANAR: u32 = 16;
/// A glossy clear coat over the base (car paint).
pub const CLEARCOAT: u32 = 32;
/// Photographed low grass cards: upward lighting normal and root occlusion.
pub const GRASS: u32 = 64;
/// Glass/blended surfaces must not become opaque ambient-light occluders.
/// This flag does not change the material's rasterization or albedo.
pub const NO_AMBIENT_OCCLUSION: u32 = 128;
/// Drawn by the sorted forward pass, never the opaque or shadow raster pass.
pub const BLEND: u32 = 256;
/// True leaf/needle geometry: thin-leaf lighting without alpha-tested cards.
pub const THIN_FOLIAGE: u32 = 512;

/// Material flags from this bit up hold 1 + an index into
/// `Scene::car_materials`: a procedural car surface (car_material.wgsl).
pub const CAR_SHIFT: u32 = 16;

/// One procedural car surface: paint, or another part of a car, with its
/// wear and grime. The meaning of each field is in car_material.wgsl; the
/// CLI fills these from engine/data/car_materials.toml.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuCarMaterial {
    /// rgb paint or part colour (linear), w: kind.
    pub colour: [f32; 4],
    /// rgb pearl shift / candy flake colour, w: paint finish.
    pub second: [f32; 4],
    /// Base roughness, metalness, clear coat, coat roughness.
    pub surface: [f32; 4],
    /// Flake amount, flake size (m), flake spread, orange peel.
    pub flake: [f32; 4],
    /// Chips, scratches, polish swirls, sun fade.
    pub wear: [f32; 4],
    /// Dust, mud, rust, seed.
    pub grime: [f32; 4],
    /// rgb dust colour, w: mud wetness.
    pub dirt: [f32; 4],
    /// Object-space bounds: min y, max y, min z, max z.
    pub body: [f32; 4],
    /// Front axle z, rear axle z, wheel radius, half track (negative: a wheel).
    pub axles: [f32; 4],
}

/// glTF alphaMode=BLEND data, separate from the fixed GPU material layout.
#[derive(Clone, Copy, Debug)]
pub struct BlendMaterial {
    pub alpha: f32,
    pub double_sided: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Material {
    /// Albedo tint (linear), multiplied with the texture.
    pub tint: [f32; 3],
    /// Texture array layer, or -1 for none.
    pub layer: i32,
    pub roughness: f32,
    pub flags: u32,
    /// World-space metres per texture repeat (triplanar/planar materials).
    pub scale: f32,
    /// Metalness 0..1.
    pub metal: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuInstance {
    /// Rows of the 3x4 object-to-world transform.
    pub rows: [[f32; 4]; 3],
    /// World-space bounding sphere.
    pub sphere: [f32; 4],
    pub first_cluster: u32,
    pub cluster_count: u32,
    /// Largest axis scale: errors and radii grow with it.
    pub scale: f32,
    /// Instance flags (FOLIAGE: fills the canopy volume for GI).
    pub flags: u32,
}

pub const FOLIAGE: u32 = 1;
/// An instance whose opaque geometry supplies near-field ambient occluders.
pub const SOLID: u32 = 2;
/// Small ground cover receives shadows but does not fill the sun shadow map.
pub const NO_SHADOW: u32 = 4;
/// Fixed-pool grass with per-view opaque/transparent transition selection.
pub const LIVE_GRASS: u32 = 8;
/// Ground-cover wind response without enabling a live distance transition.
pub const WIND_GRASS: u32 = 16;
/// A rigid vegetation part driven by the shared hierarchical GPU pose pass.
pub const PART_WIND: u32 = 32;

pub const MAX_SOLID_SEGMENTS: usize = 6;

/// Static object-space proxy. Shaders fetch the current transform from the
/// existing instance buffer, so moving/rotating cars need no proxy re-upload.
/// WGSL layout: center:vec4f, half_extent:vec4f, meta:vec4u (48 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuSolidOccluder {
    /// Object-space box center; w is reserved (zero).
    pub center: [f32; 4],
    /// Object-space half-extents; w is their bounding-sphere radius.
    pub half_extent: [f32; 4],
    /// x: owner instance index, y/z/w reserved (zero).
    pub meta: [u32; 4],
}

fn clip_axis(polygon: &[Vec3], axis: usize, plane: f32, keep_greater: bool) -> Vec<Vec3> {
    let mut out=Vec::with_capacity(polygon.len()+1);
    let Some(mut previous)=polygon.last().copied() else{return out;};
    let inside=|point:Vec3|if keep_greater{point[axis]>=plane}else{point[axis]<=plane};
    let mut was_inside=inside(previous);
    for &point in polygon {
        let is_inside=inside(point);
        if is_inside!=was_inside {
            let t=(plane-previous[axis])/(point[axis]-previous[axis]);
            let mut intersection=previous.lerp(point,t.clamp(0.0,1.0));
            intersection[axis]=plane;
            out.push(intersection);
        }
        if is_inside{out.push(point);}
        previous=point;was_inside=is_inside;
    }
    out
}

/// Fit a small number of local boxes from actual opaque triangle coverage.
/// Slicing along the body's longest horizontal axis prevents one tall cabin
/// box from extending over both the bonnet and boot. It is a bounded proxy,
/// not a claim to reconstruct every wheel arch or cavity from the mesh.
fn solid_segments(triangles: &[[Vec3;3]]) -> Vec<(Vec3,Vec3)> {
    let mut lo=Vec3::splat(f32::INFINITY);let mut hi=Vec3::splat(f32::NEG_INFINITY);
    for triangle in triangles {for &point in triangle {lo=lo.min(point);hi=hi.max(point);}}
    if !lo.is_finite() || !hi.is_finite(){return Vec::new();}
    let extent=hi-lo;let axis=if extent.z>=extent.x{2}else{0};
    if extent[axis]<=0.001{return Vec::new();}
    let step=extent[axis]/MAX_SOLID_SEGMENTS as f32;
    let mut boxes=Vec::with_capacity(MAX_SOLID_SEGMENTS);
    for segment in 0..MAX_SOLID_SEGMENTS {
        let lower=lo[axis]+step*segment as f32;
        let upper=if segment+1==MAX_SOLID_SEGMENTS{hi[axis]}else{lo[axis]+step*(segment+1) as f32};
        let mut part_lo=Vec3::splat(f32::INFINITY);let mut part_hi=Vec3::splat(f32::NEG_INFINITY);
        for triangle in triangles {
            let tri_lo=triangle[0][axis].min(triangle[1][axis]).min(triangle[2][axis]);
            let tri_hi=triangle[0][axis].max(triangle[1][axis]).max(triangle[2][axis]);
            if tri_hi<lower || tri_lo>upper{continue;}
            let clipped=clip_axis(&clip_axis(triangle,axis,lower,true),axis,upper,false);
            for point in clipped{part_lo=part_lo.min(point);part_hi=part_hi.max(point);}
        }
        if !part_lo.is_finite() || !part_hi.is_finite(){continue;}
        let half=(part_hi-part_lo)*0.5;
        // Do not inflate isolated open sheets into a fictitious solid volume.
        if half.min_element()<=0.001{continue;}
        let inset=half*0.08;
        let inset=inset.min(Vec3::splat(0.04));
        for dimension in 0..3 {
            if dimension!=axis || segment==0{part_lo[dimension]+=inset[dimension];}
            if dimension!=axis || segment+1==MAX_SOLID_SEGMENTS{part_hi[dimension]-=inset[dimension];}
        }
        // Adjacent longitudinal slices meet exactly: no artificial slit every
        // segment. Only the exterior shell is inset to reduce self-occlusion.
        boxes.push(((part_lo+part_hi)*0.5,(part_hi-part_lo)*0.5));
    }
    boxes
}

/// Terrain heights on a regular grid, for GI occlusion (and later grass).
#[derive(Default, Clone)]
pub struct Heightfield {
    pub origin: [f32; 2],
    pub cell: f32,
    pub w: u32,
    pub h: u32,
    pub heights: Vec<f32>,
}

pub struct MeshRef {
    pub first_cluster: u32,
    pub cluster_count: u32,
    pub sphere: [f32; 4],
}

/// Textures to put in the albedo array (all resized to one size).
pub struct TextureSource {
    pub name: String,
    pub rgba: image::RgbaImage,
    /// Tangent-space normal map (OpenGL convention, like Godot), if any.
    pub normal: Option<image::RgbaImage>,
}

#[derive(Default)]
pub struct Scene {
    pub vertices: Vec<Vertex>,
    pub cluster_verts: Vec<u32>,
    pub tris: Vec<u32>,
    pub clusters: Vec<GpuCluster>,
    pub meshes: Vec<MeshRef>,
    pub instances: Vec<GpuInstance>,
    pub materials: Vec<Material>,
    /// Procedural car surfaces, referenced through `CAR_SHIFT` flag bits.
    /// `Renderer::set_car_materials` replaces them without a rebuild.
    pub car_materials: Vec<GpuCarMaterial>,
    pub blended_materials: HashMap<u32, BlendMaterial>,
    pub grass_ranges: Vec<crate::grass::GrassRange>,
    /// Shared CPU residency / per-view GPU fade policy for source grass.
    pub grass_detail: crate::vegetation_detail::DistanceProfile,
    /// Optional world-fixed terrain-following grass density field.
    pub far_grass: Option<crate::far_grass::FarGrassField>,
    /// Optional bounded validation forest; absent in normal live scenes.
    pub voxel_forest: Option<crate::foliage_volume_data::GpuForest>,
    /// Explicit projected voxel LOD experiment; zero keeps finest-level mode.
    pub voxel_lod_pixels: f32,
    /// Experimental GPU worklist: integrate only tiles with volume candidates.
    /// Defaults to the full-screen reference until image/timing validation.
    pub voxel_active_tiles: bool,
    /// Optional bounded shared branch poses for both source and voxel parts.
    pub part_wind: Vec<crate::part_wind::PartWind>,
    /// Shared baked rigid-part clips. Optional, with bounded actor/part pools.
    pub baked_animations: Option<crate::baked_animation::BakedAnimations>,
    pub textures: Vec<TextureSource>,
    pub heightfield: Heightfield,
    /// Game data for materials (Sideways: road surface per 2 m row).
    pub aux: Vec<u32>,
    /// Equirectangular HDR sky, linear RGB.
    pub sky: Option<(u32, u32, Vec<[f32; 4]>)>,
    /// Absolute, sun-disc-free radiance with its solar state supplied by the
    /// atmosphere bake. Bypass HDRI brightest-pixel rotation and lux rescaling.
    pub sky_physical: bool,
}

impl Scene {
    /// At most six opaque local boxes per SOLID instance. Build once after
    /// scene assembly; each descriptor's owner points at the live instance
    /// transform. Alpha-tested and explicitly non-occluding materials are
    /// excluded before fitting, so transparent geometry cannot enlarge boxes.
    ///
    /// Shaders should test a nearby owner bound before its boxes, bias the
    /// sample from the real surface, and ignore a containing proxy belonging
    /// to the shaded owner. These are conservative interior approximations,
    /// not replacements for the sun shadow map or a full mesh ray tracer.
    pub fn solid_occluders(&self) -> Vec<GpuSolidOccluder> {
        let mut meshes:HashMap<(u32,u32),Vec<(Vec3,Vec3)>>=HashMap::new();
        let mut result=Vec::new();
        for (owner,instance) in self.instances.iter().enumerate() {
            if instance.flags & SOLID==0{continue;}
            let boxes=meshes.entry((instance.first_cluster,instance.cluster_count)).or_insert_with(||{
                let mut triangles=Vec::new();
                let start=instance.first_cluster as usize;
                let end=start.saturating_add(instance.cluster_count as usize).min(self.clusters.len());
                for cluster in self.clusters.get(start..end).unwrap_or_default() {
                    if cluster.own_error!=0.0{continue;}
                    let Some(material)=self.materials.get(cluster.material as usize) else{continue;};
                    if material.flags & (ALPHA_TEST|NO_AMBIENT_OCCLUSION)!=0{continue;}
                    for triangle in cluster.first_tri..cluster.first_tri.saturating_add(cluster.tri_count) {
                        let Some(&packed)=self.tris.get(triangle as usize) else{continue;};
                        let mut points=[Vec3::ZERO;3];let mut valid=true;
                        for (corner,point) in points.iter_mut().enumerate() {
                            let local=(packed>>(corner*8))&255;
                            let index=self.cluster_verts.get(cluster.first_vert as usize+local as usize).and_then(|i|self.vertices.get(*i as usize));
                            match index {Some(vertex)=>{*point=Vec3::from(vertex.pos);valid&=point.is_finite();},None=>valid=false}
                        }
                        if valid{triangles.push(points);}
                    }
                }
                solid_segments(&triangles)
            });
            for (index,&(center,half)) in boxes.iter().enumerate() {
                result.push(GpuSolidOccluder{center:[center.x,center.y,center.z,0.0],half_extent:[half.x,half.y,half.z,half.length()],meta:[owner as u32,(boxes.len()-index) as u32,0,0]});
            }
        }
        result
    }

    /// Add a mesh; returns its id for instancing.
    pub fn add_mesh(&mut self, m: &ClusterMesh) -> usize {
        let vbase = self.vertices.len() as u32;
        let cvbase = self.cluster_verts.len() as u32;
        let tbase = self.tris.len() as u32;
        let first_cluster = self.clusters.len() as u32;
        self.vertices.extend_from_slice(&m.vertices);
        self.cluster_verts.extend(m.cluster_verts.iter().map(|v| v + vbase));
        self.tris.extend_from_slice(&m.tris);
        for c in &m.clusters {
            let mut c = *c;
            c.first_vert += cvbase;
            c.first_tri += tbase;
            self.clusters.push(c);
        }
        self.meshes.push(MeshRef { first_cluster, cluster_count: m.clusters.len() as u32, sphere: m.sphere });
        self.meshes.len() - 1
    }

    pub fn add_instance(&mut self, mesh: usize, xf: Affine3A) {
        self.instances.push(Self::instance(&self.meshes[mesh], xf, 0));
    }

    pub fn update_instance(&mut self, index: usize, mesh: usize, xf: Affine3A) {
        let flags = self.instances[index].flags;
        self.instances[index] = Self::instance(&self.meshes[mesh], xf, flags);
    }

    fn instance(m: &MeshRef, xf: Affine3A, flags: u32) -> GpuInstance {
        let c = xf.transform_point3(Vec3::from_slice(&m.sphere[..3]));
        let cols = xf.matrix3;
        let scale = cols.x_axis.length().max(cols.y_axis.length()).max(cols.z_axis.length());
        let mat = glam::Mat4::from(xf);
        let r = mat.transpose();
        GpuInstance {
            rows: [r.x_axis.to_array(), r.y_axis.to_array(), r.z_axis.to_array()],
            sphere: [c.x, c.y, c.z, m.sphere[3] * scale],
            first_cluster: m.first_cluster,
            cluster_count: m.cluster_count,
            scale,
            flags,
        }
    }

    /// Mark the last instance as a source of opaque near-field proxies.
    pub fn mark_solid(&mut self) {
        if let Some(i) = self.instances.last_mut() {
            i.flags |= SOLID;
        }
    }

    /// Mark the last instance as foliage (a tree crown for the GI volume).
    pub fn mark_foliage(&mut self) {
        if let Some(i) = self.instances.last_mut() {
            i.flags |= FOLIAGE;
        }
    }

    pub fn mark_no_shadow(&mut self) {
        if let Some(i) = self.instances.last_mut() {
            i.flags |= NO_SHADOW;
        }
    }

    pub fn add_material(&mut self, m: Material) -> u32 {
        self.materials.push(m);
        (self.materials.len() - 1) as u32
    }

    /// Add a procedural car surface; returns the material flag bits that
    /// select it (or them into `Material::flags`).
    pub fn add_car_material(&mut self, c: GpuCarMaterial) -> u32 {
        self.car_materials.push(c);
        (self.car_materials.len() as u32) << CAR_SHIFT
    }

    /// Preserve continuous alpha, including opaque texels of a blended badge.
    /// The same material is excluded from both depth/shadow and solid proxies.
    pub fn set_material_blend(&mut self, material: u32, alpha: f32, double_sided: bool) {
        self.materials[material as usize].flags |= BLEND | NO_AMBIENT_OCCLUSION;
        self.blended_materials.insert(material, BlendMaterial { alpha: alpha.clamp(0.0, 1.0), double_sided });
    }

    /// Load an image file into the texture array; returns its layer.
    pub fn add_texture(&mut self, path: &std::path::Path) -> Result<i32, String> {
        // The normal map sits beside the albedo: x_albedo.jpg, x_normal.jpg (or .png).
        let normal_path = path.file_name().and_then(|f| f.to_str()).filter(|f| f.contains("_albedo.")).and_then(|f| {
            let stem = f.split("_albedo.").next().unwrap_or(f);
            ["jpg", "png"].iter().map(|e| path.with_file_name(format!("{stem}_normal.{e}"))).find(|p| p.exists())
        });
        let (img, normal) = std::thread::scope(|s| {
            let n = s.spawn(|| normal_path.and_then(|p| image::open(p).ok()).map(|i| i.to_rgba8()));
            (image::open(path), n.join().unwrap())
        });
        let img = img.map_err(|e| format!("{}: {e}", path.display()))?.to_rgba8();
        self.textures.push(TextureSource { name: path.display().to_string(), rgba: img, normal });
        Ok((self.textures.len() - 1) as i32)
    }

    /// Add an already decoded image to the texture array; returns its layer.
    pub fn add_image(&mut self, name: &str, rgba: image::RgbaImage) -> i32 {
        self.textures.push(TextureSource { name: name.to_string(), rgba, normal: None });
        (self.textures.len() - 1) as i32
    }

    pub fn load_sky(&mut self, path: &std::path::Path) -> Result<(), String> {
        let img = image::open(path).map_err(|e| format!("{}: {e}", path.display()))?.to_rgba32f();
        let (w, h) = img.dimensions();
        let px = img.pixels().map(|p| [p[0], p[1], p[2], 1.0]).collect();
        self.sky = Some((w, h, px));
        Ok(())
    }

    pub fn stats(&self) -> String {
        let leaf: usize = self.clusters.iter().filter(|c| c.own_error == 0.0).map(|c| c.tri_count as usize).sum();
        format!(
            "{} meshes, {} instances, {} clusters ({} leaf triangles), {} vertices, {} textures",
            self.meshes.len(),
            self.instances.len(),
            self.clusters.len(),
            leaf,
            self.vertices.len(),
            self.textures.len()
        )
    }
}

#[cfg(test)]
mod solid_tests {
    use super::*;

    fn material(flags:u32)->Material {Material{tint:[1.0;3],layer:-1,roughness:0.5,flags,scale:1.0,metal:0.0}}

    fn boxes(parts:&[(Vec3,Vec3,u32)])->ClusterMesh {
        let mut mesh=ClusterMesh::default();
        let indices=[[0u32,2,1],[1,2,3],[4,5,6],[5,7,6],[0,1,4],[1,5,4],[2,6,3],[3,6,7],[0,4,2],[2,4,6],[1,3,5],[3,7,5]];
        for &(lo,hi,material) in parts {
            let first_vert=mesh.cluster_verts.len() as u32;let first_tri=mesh.tris.len() as u32;
            for corner in 0..8 {
                let p=Vec3::new(if corner&1==0{lo.x}else{hi.x},if corner&2==0{lo.y}else{hi.y},if corner&4==0{lo.z}else{hi.z});
                mesh.cluster_verts.push(mesh.vertices.len() as u32);
                mesh.vertices.push(Vertex{pos:p.to_array(),normal:Vec3::Y.to_array(),uv:[0.0;2]});
            }
            for [a,b,c] in indices{mesh.tris.push(a|(b<<8)|(c<<16));}
            let mut cluster=GpuCluster::zeroed();cluster.first_vert=first_vert;cluster.first_tri=first_tri;cluster.tri_count=12;cluster.material=material;
            mesh.clusters.push(cluster);
        }
        mesh.sphere=[0.0,0.0,0.0,10.0];mesh
    }

    fn scene(extra:Option<u32>)->(Scene,usize) {
        let mut scene=Scene::default();scene.add_material(material(0));
        let mut parts=vec![(Vec3::new(-1.0,0.0,-3.0),Vec3::new(1.0,1.0,3.0),0)];
        if let Some(flags)=extra {
            let glass=scene.add_material(material(flags));
            parts.push((Vec3::new(-4.0,3.0,-8.0),Vec3::new(4.0,6.0,8.0),glass));
        }
        let mesh=scene.add_mesh(&boxes(&parts));scene.add_instance(mesh,Affine3A::IDENTITY);scene.mark_solid();(scene,mesh)
    }

    #[test]
    fn local_opaque_segments_fit_geometry_without_transparent_expansion() {
        let (opaque,_)=scene(None);let expected=opaque.solid_occluders();assert_eq!(expected.len(),6);
        for flag in [ALPHA_TEST,NO_AMBIENT_OCCLUSION] {
            let (mixed,_)=scene(Some(flag));let actual=mixed.solid_occluders();
            assert_eq!(bytemuck::cast_slice::<_,u8>(&actual),bytemuck::cast_slice::<_,u8>(&expected));
        }
        for occluder in &expected {
            let center=Vec3::from_slice(&occluder.center);let half=Vec3::from_slice(&occluder.half_extent);
            assert!(half.min_element()>0.0 && half.is_finite());
            assert!((center-half).cmpge(Vec3::new(-1.0,0.0,-3.0)).all());
            assert!((center+half).cmple(Vec3::new(1.0,1.0,3.0)).all());
        }
        for pair in expected.windows(2) {
            assert!((pair[0].center[2]+pair[0].half_extent[2]-(pair[1].center[2]-pair[1].half_extent[2])).abs()<1e-5);
        }
    }

    #[test]
    fn static_descriptor_uses_live_owner_and_does_not_need_pose_reupload() {
        let (mut scene,mesh)=scene(None);let before=scene.solid_occluders();
        scene.update_instance(0,mesh,Affine3A::from_scale_rotation_translation(Vec3::new(1.1,0.9,1.0),glam::Quat::from_rotation_y(1.2),Vec3::new(140.0,4.0,-720.0)));
        let after=scene.solid_occluders();
        assert_eq!(std::mem::size_of::<GpuSolidOccluder>(),48);
        assert_eq!(bytemuck::cast_slice::<_,u8>(&before),bytemuck::cast_slice::<_,u8>(&after));
        for car in 1..4 {scene.add_instance(mesh,Affine3A::from_translation(Vec3::X*car as f32*5.0));scene.mark_solid();}
        let four=scene.solid_occluders();assert_eq!(four.len(),4*MAX_SOLID_SEGMENTS);
        for (owner,group) in four.chunks_exact(MAX_SOLID_SEGMENTS).enumerate(){assert!(group.iter().all(|b|b.meta[0]==owner as u32));}
    }

    #[test]
    fn non_solids_and_fully_transparent_bodies_emit_no_occluders() {
        let (mut scene,_)=scene(None);scene.instances[0].flags=FOLIAGE;assert!(scene.solid_occluders().is_empty());
        scene.instances[0].flags=SOLID;scene.materials[0].flags=NO_AMBIENT_OCCLUSION;assert!(scene.solid_occluders().is_empty());
        let p=Vec3::ZERO;assert!(solid_segments(&[[p,p,p]]).is_empty());
    }
}
