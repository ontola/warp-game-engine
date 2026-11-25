//! Every `KERN_*` environment switch the renderer reads, declared once.
//!
//! A switch reads the environment on every call and caches nothing: tests and
//! examples change a variable with `std::env::set_var` between two renderers
//! and expect the new value to be seen. Call sites that cache (a `OnceLock`)
//! do so themselves. What each switch does is in
//! `engine/docs/renderer-switches.md`; the tests below check that every switch
//! is listed there (or in `building-and-running.md`) and that no other file
//! reads a `KERN_*` variable directly.

use std::{env::VarError, ffi::OsString, str::FromStr};

/// One environment variable, named once.
pub struct Switch { pub name: &'static str }

impl Switch {
    pub const fn new(name: &'static str) -> Self { Self { name } }
    /// The raw result of reading the variable.
    pub fn var(&self) -> Result<String, VarError> { std::env::var(self.name) }
    /// The value, if set to valid UTF-8.
    pub fn get(&self) -> Option<String> { std::env::var(self.name).ok() }
    /// The value as the OS stores it, if set.
    pub fn os(&self) -> Option<OsString> { std::env::var_os(self.name) }
    /// Set to anything, even the empty string.
    pub fn set(&self) -> bool { std::env::var_os(self.name).is_some() }
    /// Set to exactly `value`.
    pub fn is(&self, value: &str) -> bool { std::env::var(self.name).as_deref() == Ok(value) }
    /// Set to exactly `1`.
    pub fn on(&self) -> bool { self.is("1") }
    /// Set to exactly `0`.
    pub fn off(&self) -> bool { self.is("0") }
    /// The value parsed as `T`, if set and valid.
    pub fn parse<T: FromStr>(&self) -> Option<T> { self.get().and_then(|v| v.parse().ok()) }
}

pub const ADAPTER: Switch = Switch::new("KERN_ADAPTER");
pub const ASYNC_SHADOW_VALIDATION: Switch = Switch::new("KERN_ASYNC_SHADOW_VALIDATION");
pub const BOUNDED_DRAWS: Switch = Switch::new("KERN_BOUNDED_DRAWS");
pub const BRANCH_BRICKS: Switch = Switch::new("KERN_BRANCH_BRICKS");
pub const BRANCH_CELL_M: Switch = Switch::new("KERN_BRANCH_CELL_M");
pub const BRICK_EMPTY_JUMP: Switch = Switch::new("KERN_BRICK_EMPTY_JUMP");
pub const BRICK_EMPTY_SKIP: Switch = Switch::new("KERN_BRICK_EMPTY_SKIP");
pub const BRICK_SMOOTH_MIPS: Switch = Switch::new("KERN_BRICK_SMOOTH_MIPS");
pub const CAMERA_CUT_FLOOR: Switch = Switch::new("KERN_CAMERA_CUT_FLOOR");
pub const CAMERA_SUN_OCCLUSION: Switch = Switch::new("KERN_CAMERA_SUN_OCCLUSION");
pub const CANOPY_CLUMPING: Switch = Switch::new("KERN_CANOPY_CLUMPING");
pub const CANOPY_COMPACT_SUPPORT: Switch = Switch::new("KERN_CANOPY_COMPACT_SUPPORT");
pub const CLUSTERED_LIGHTS: Switch = Switch::new("KERN_CLUSTERED_LIGHTS");
pub const CLUSTERED_SHADOW_FACES: Switch = Switch::new("KERN_CLUSTERED_SHADOW_FACES");
pub const CLUSTERED_SHADOW_LAYERS: Switch = Switch::new("KERN_CLUSTERED_SHADOW_LAYERS");
pub const COMPACT_SHADE: Switch = Switch::new("KERN_COMPACT_SHADE");
pub const CLUSTER_BUCKETS: Switch = Switch::new("KERN_CLUSTER_BUCKETS");
pub const CLUSTER_TRAVERSAL: Switch = Switch::new("KERN_CLUSTER_TRAVERSAL");
pub const COARSE_FOOTPRINT_M: Switch = Switch::new("KERN_COARSE_FOOTPRINT_M");
pub const COOK_THREADS: Switch = Switch::new("KERN_COOK_THREADS");
pub const CPU_SPANS: Switch = Switch::new("KERN_CPU_SPANS");
pub const CULL_PROFILE: Switch = Switch::new("KERN_CULL_PROFILE");
pub const CUTOUT_LINEAR_RGB: Switch = Switch::new("KERN_CUTOUT_LINEAR_RGB");
pub const CUT_HYSTERESIS: Switch = Switch::new("KERN_CUT_HYSTERESIS");
pub const DIAGNOSTIC_HIDE: Switch = Switch::new("KERN_DIAGNOSTIC_HIDE");
pub const DIRECT_DRAWS: Switch = Switch::new("KERN_DIRECT_DRAWS");
pub const DLSS_RUNTIME_DIR: Switch = Switch::new("KERN_DLSS_RUNTIME_DIR");
pub const EARLY_GRASS_CULL: Switch = Switch::new("KERN_EARLY_GRASS_CULL");
pub const FAR_CROWN: Switch = Switch::new("KERN_FAR_CROWN");
pub const FAR_CROWN_DEBUG: Switch = Switch::new("KERN_FAR_CROWN_DEBUG");
pub const FAR_SUN_LOD: Switch = Switch::new("KERN_FAR_SUN_LOD");
pub const FAR_SUN_REACH: Switch = Switch::new("KERN_FAR_SUN_REACH");
pub const FAR_SUN_SHADOW: Switch = Switch::new("KERN_FAR_SUN_SHADOW");
pub const FAR_TREE_SHADOWS: Switch = Switch::new("KERN_FAR_TREE_SHADOWS");
pub const FAR_TREE_SHADOW_FROM: Switch = Switch::new("KERN_FAR_TREE_SHADOW_FROM");
pub const FAR_TREE_SHADOW_MIN_M: Switch = Switch::new("KERN_FAR_TREE_SHADOW_MIN_M");
pub const FOLIAGE_CUT: Switch = Switch::new("KERN_FOLIAGE_CUT");
pub const FOLIAGE_OCCLUDERS: Switch = Switch::new("KERN_FOLIAGE_OCCLUDERS");
pub const FOLIAGE_SUN_LOD: Switch = Switch::new("KERN_FOLIAGE_SUN_LOD");
pub const FOLIAGE_SUN_PAGE_METRES: Switch = Switch::new("KERN_FOLIAGE_SUN_PAGE_METRES");
pub const FOREST_FLOOR_DARK: Switch = Switch::new("KERN_FOREST_FLOOR_DARK");
pub const FROXEL_FOG: Switch = Switch::new("KERN_FROXEL_FOG");
pub const GPU_PARTICLES: Switch = Switch::new("KERN_GPU_PARTICLES");
pub const GRASS_DEPTH_PREPASS: Switch = Switch::new("KERN_GRASS_DEPTH_PREPASS");
pub const GRASS_LEAF_BRDF: Switch = Switch::new("KERN_GRASS_LEAF_BRDF");
pub const GRASS_SUN_OCCLUSION: Switch = Switch::new("KERN_GRASS_SUN_OCCLUSION");
pub const HEADLIGHT_INSTANCE_GROUPS: Switch = Switch::new("KERN_HEADLIGHT_INSTANCE_GROUPS");
pub const INSTANCE_BROADPHASE: Switch = Switch::new("KERN_INSTANCE_BROADPHASE");
pub const INSTANCE_GROUPS: Switch = Switch::new("KERN_INSTANCE_GROUPS");
pub const LEGACY_LIGHT_LOOKAHEAD: Switch = Switch::new("KERN_LEGACY_LIGHT_LOOKAHEAD");
pub const MEADOW_AGGREGATE: Switch = Switch::new("KERN_MEADOW_AGGREGATE");
pub const MEADOW_AGGREGATE_HALF_RES: Switch = Switch::new("KERN_MEADOW_AGGREGATE_HALF_RES");
pub const MEADOW_AGGREGATE_SAMPLE_PRUNE: Switch = Switch::new("KERN_MEADOW_AGGREGATE_SAMPLE_PRUNE");
pub const MEADOW_CONTACT_AO: Switch = Switch::new("KERN_MEADOW_CONTACT_AO");
pub const MEADOW_EDGE_SHADOW: Switch = Switch::new("KERN_MEADOW_EDGE_SHADOW");
pub const MEADOW_GROUP_CACHE: Switch = Switch::new("KERN_MEADOW_GROUP_CACHE");
pub const MEADOW_GROUP_REPLACE: Switch = Switch::new("KERN_MEADOW_GROUP_REPLACE");
pub const MEADOW_INDEXED: Switch = Switch::new("KERN_MEADOW_INDEXED");
pub const MEADOW_OCCLUSION: Switch = Switch::new("KERN_MEADOW_OCCLUSION");
pub const MEADOW_SKIP_HIDDEN_SHADE: Switch = Switch::new("KERN_MEADOW_SKIP_HIDDEN_SHADE");
pub const MEADOW_SPATIAL_FILTER: Switch = Switch::new("KERN_MEADOW_SPATIAL_FILTER");
pub const MEADOW_SUBSTRATE: Switch = Switch::new("KERN_MEADOW_SUBSTRATE");
pub const MEADOW_VISIBILITY: Switch = Switch::new("KERN_MEADOW_VISIBILITY");
pub const MEASURED_ERRORS: Switch = Switch::new("KERN_MEASURED_ERRORS");
pub const NEAR_SHADOW_LOD: Switch = Switch::new("KERN_NEAR_SHADOW_LOD");
pub const NEAR_SUN_SOURCE: Switch = Switch::new("KERN_NEAR_SUN_SOURCE");
pub const NEEDLE_BRUSH_OWN: Switch = Switch::new("KERN_NEEDLE_BRUSH_OWN");
pub const OCCLUSION: Switch = Switch::new("KERN_OCCLUSION");
pub const OCCLUSION_REJECT: Switch = Switch::new("KERN_OCCLUSION_REJECT");
pub const OWNER_RANGE_INDEX: Switch = Switch::new("KERN_OWNER_RANGE_INDEX");
pub const PAGED_FOLIAGE_SUN: Switch = Switch::new("KERN_PAGED_FOLIAGE_SUN");
pub const PIPELINE_CACHE: Switch = Switch::new("KERN_PIPELINE_CACHE");
pub const PIPELINE_CACHE_DIR: Switch = Switch::new("KERN_PIPELINE_CACHE_DIR");
pub const PORTABLE: Switch = Switch::new("KERN_PORTABLE");
pub const PROBE_AGE: Switch = Switch::new("KERN_PROBE_AGE");
pub const PROBE_GRADIENTS: Switch = Switch::new("KERN_PROBE_GRADIENTS");
pub const RASTER_FLOOR: Switch = Switch::new("KERN_RASTER_FLOOR");
pub const RASTER_LARGE_FIRST: Switch = Switch::new("KERN_RASTER_LARGE_FIRST");
pub const RASTER_SPLIT: Switch = Switch::new("KERN_RASTER_SPLIT");
pub const RECEIVER_SHADOW_DEMAND: Switch = Switch::new("KERN_RECEIVER_SHADOW_DEMAND");
pub const REFINE_ROUNDS: Switch = Switch::new("KERN_REFINE_ROUNDS");
pub const REFINE_WAVE_PUBLISH: Switch = Switch::new("KERN_REFINE_WAVE_PUBLISH");
pub const REFLECTION_PROBES: Switch = Switch::new("KERN_REFLECTION_PROBES");
pub const REFRACTION: Switch = Switch::new("KERN_REFRACTION");
pub const REUSE_SUN_TEXTURE: Switch = Switch::new("KERN_REUSE_SUN_TEXTURE");
pub const ROCK_VARIATION: Switch = Switch::new("KERN_ROCK_VARIATION");
pub const RSM: Switch = Switch::new("KERN_RSM");
pub const SHADER_DIR: Switch = Switch::new("KERN_SHADER_DIR");
pub const SHADE_CLASSES: Switch = Switch::new("KERN_SHADE_CLASSES");
pub const SHADE_FEATURES: Switch = Switch::new("KERN_SHADE_FEATURES");
pub const SHADE_UNLIT: Switch = Switch::new("KERN_SHADE_UNLIT");
pub const SHADE_PROBE: Switch = Switch::new("KERN_SHADE_PROBE");
pub const PHONE_GI: Switch = Switch::new("KERN_PHONE_GI");
pub const LIGHT_TILES: Switch = Switch::new("KERN_LIGHT_TILES");
pub const FOOTPRINT_LOD: Switch = Switch::new("KERN_FOOTPRINT_LOD");
pub const ALBEDO_LOD_BIAS: Switch = Switch::new("KERN_ALBEDO_LOD_BIAS");
pub const SHADOW_DISTANCE_LOD: Switch = Switch::new("KERN_SHADOW_DISTANCE_LOD");
pub const SHADOW_REACH: Switch = Switch::new("KERN_SHADOW_REACH");
pub const SIMPLE_CULL: Switch = Switch::new("KERN_SIMPLE_CULL");
pub const SIMPLE_LIGHTING: Switch = Switch::new("KERN_SIMPLE_LIGHTING");
pub const SKY_TILES: Switch = Switch::new("KERN_SKY_TILES");
pub const SKY_VIS_CACHE: Switch = Switch::new("KERN_SKY_VIS_CACHE");
pub const SOURCE_CLUSTERS: Switch = Switch::new("KERN_SOURCE_CLUSTERS");
pub const SOURCE_CROWN_BOUNDS: Switch = Switch::new("KERN_SOURCE_CROWN_BOUNDS");
pub const SOURCE_PER_TREE: Switch = Switch::new("KERN_SOURCE_PER_TREE");
pub const SPLAT_BRICK_HZB: Switch = Switch::new("KERN_SPLAT_BRICK_HZB");
pub const SPLIT_CULL_TIMING: Switch = Switch::new("KERN_SPLIT_CULL_TIMING");
pub const SPLIT_SELECTION_TIMING: Switch = Switch::new("KERN_SPLIT_SELECTION_TIMING");
pub const SSSR: Switch = Switch::new("KERN_SSSR");
pub const STARTUP_PROFILE: Switch = Switch::new("KERN_STARTUP_PROFILE");
pub const STATIC_SUN_CACHE: Switch = Switch::new("KERN_STATIC_SUN_CACHE");
pub const SUN_CACHE_GUARD_METRES: Switch = Switch::new("KERN_SUN_CACHE_GUARD_METRES");
pub const SUN_CLIPMAP: Switch = Switch::new("KERN_SUN_CLIPMAP");
pub const SUN_DAMAGE_REPORT: Switch = Switch::new("KERN_SUN_DAMAGE_REPORT");
pub const SUN_CLOSE: Switch = Switch::new("KERN_SUN_CLOSE");
pub const SUN_DITHER_TEXEL: Switch = Switch::new("KERN_SUN_DITHER_TEXEL");
pub const SUN_INSTANCE_GROUPS: Switch = Switch::new("KERN_SUN_INSTANCE_GROUPS");
pub const SUN_LEVELS: Switch = Switch::new("KERN_SUN_LEVELS");
pub const SUN_RECEIVER_PLANE: Switch = Switch::new("KERN_SUN_RECEIVER_PLANE");
pub const SUN_SCENE_DEPTH: Switch = Switch::new("KERN_SUN_SCENE_DEPTH");
pub const SUN_STAGGER: Switch = Switch::new("KERN_SUN_STAGGER");
pub const SUN_SURFACE_SOURCE: Switch = Switch::new("KERN_SUN_SURFACE_SOURCE");
pub const SUN_TERRAIN_ERROR: Switch = Switch::new("KERN_SUN_TERRAIN_ERROR");
pub const SUN_TERRAIN_LOD_DAMAGE: Switch = Switch::new("KERN_SUN_TERRAIN_LOD_DAMAGE");
pub const SUN_VOXEL_M: Switch = Switch::new("KERN_SUN_VOXEL_M");
pub const TERRAIN_COARSE: Switch = Switch::new("KERN_TERRAIN_COARSE");
pub const TERRAIN_NODE_BUDGET: Switch = Switch::new("KERN_TERRAIN_NODE_BUDGET");
pub const TEXTURE_CACHE: Switch = Switch::new("KERN_TEXTURE_CACHE");
pub const TEXTURE_CACHE_DIR: Switch = Switch::new("KERN_TEXTURE_CACHE_DIR");
pub const TEXTURE_COMPRESSION: Switch = Switch::new("KERN_TEXTURE_COMPRESSION");
pub const TEXTURE_STREAMING: Switch = Switch::new("KERN_TEXTURE_STREAMING");
pub const THIN_PIECES: Switch = Switch::new("KERN_THIN_PIECES");
pub const TILED_POST: Switch = Switch::new("KERN_TILED_POST");
pub const TRACED_REFLECTIONS: Switch = Switch::new("KERN_TRACED_REFLECTIONS");
pub const TREE_OCCLUSION: Switch = Switch::new("KERN_TREE_OCCLUSION");
pub const TREE_OCCLUSION_REJECT: Switch = Switch::new("KERN_TREE_OCCLUSION_REJECT");
pub const VEGETATION_BUDGETS: Switch = Switch::new("KERN_VEGETATION_BUDGETS");
pub const VEGETATION_QUALITY: Switch = Switch::new("KERN_VEGETATION_QUALITY");
pub const VIRTUAL_MEMBERS: Switch = Switch::new("KERN_VIRTUAL_MEMBERS");
pub const VIRTUAL_SERIAL_CULL: Switch = Switch::new("KERN_VIRTUAL_SERIAL_CULL");
pub const VOXEL_ANTIPODAL_NORMALS: Switch = Switch::new("KERN_VOXEL_ANTIPODAL_NORMALS");
pub const VOXEL_BRICK_VISIBILITY: Switch = Switch::new("KERN_VOXEL_BRICK_VISIBILITY");
pub const VOXEL_EMPTY_CEILING: Switch = Switch::new("KERN_VOXEL_EMPTY_CEILING");
pub const VOXEL_GAP_MIPS: Switch = Switch::new("KERN_VOXEL_GAP_MIPS");
pub const VOXEL_GLOBAL_INTERVALS: Switch = Switch::new("KERN_VOXEL_GLOBAL_INTERVALS");
pub const VOXEL_OWN_IN_CLIPMAP: Switch = Switch::new("KERN_VOXEL_OWN_IN_CLIPMAP");
pub const VOXEL_LEAF_PARITY: Switch = Switch::new("KERN_VOXEL_LEAF_PARITY");
pub const VOXEL_DISC_DEPTH: Switch = Switch::new("KERN_VOXEL_DISC_DEPTH");
pub const VOXEL_PASS: Switch = Switch::new("KERN_VOXEL_PASS");
pub const VOXEL_RASTER: Switch = Switch::new("KERN_VOXEL_RASTER");
pub const VOXEL_SELF_SHADOW_COARSER: Switch = Switch::new("KERN_VOXEL_SELF_SHADOW_COARSER");
pub const VOXEL_STAMPS: Switch = Switch::new("KERN_VOXEL_STAMPS");
pub const VOXEL_SURFACE: Switch = Switch::new("KERN_VOXEL_SURFACE");
pub const WATER: Switch = Switch::new("KERN_WATER");
pub const WEBGPU_LIMITS: Switch = Switch::new("KERN_WEBGPU_LIMITS");
pub const WIND_FREEZE: Switch = Switch::new("KERN_WIND_FREEZE");
pub const WORLD_TERRAIN_VISIBILITY: Switch = Switch::new("KERN_WORLD_TERRAIN_VISIBILITY");
pub const ZERO_SUN_FAST_PATH: Switch = Switch::new("KERN_ZERO_SUN_FAST_PATH");

/// Every switch above.
pub const ALL: &[&Switch] = &[
    &ADAPTER,
    &ASYNC_SHADOW_VALIDATION,
    &BOUNDED_DRAWS,
    &BRANCH_CELL_M,
    &BRICK_EMPTY_JUMP,
    &BRICK_EMPTY_SKIP,
    &BRICK_SMOOTH_MIPS,
    &CAMERA_CUT_FLOOR,
    &CAMERA_SUN_OCCLUSION,
    &CANOPY_CLUMPING,
    &CANOPY_COMPACT_SUPPORT,
    &CLUSTERED_LIGHTS,
    &CLUSTERED_SHADOW_FACES,
    &CLUSTERED_SHADOW_LAYERS,
    &CLUSTER_BUCKETS,
    &COMPACT_SHADE,
    &CLUSTER_TRAVERSAL,
    &COARSE_FOOTPRINT_M,
    &COOK_THREADS,
    &CPU_SPANS,
    &CULL_PROFILE,
    &CUTOUT_LINEAR_RGB,
    &CUT_HYSTERESIS,
    &DIAGNOSTIC_HIDE,
    &DIRECT_DRAWS,
    &DLSS_RUNTIME_DIR,
    &EARLY_GRASS_CULL,
    &FAR_CROWN,
    &FAR_CROWN_DEBUG,
    &FAR_SUN_LOD,
    &FAR_SUN_REACH,
    &FAR_SUN_SHADOW,
    &FAR_TREE_SHADOWS,
    &FAR_TREE_SHADOW_FROM,
    &FAR_TREE_SHADOW_MIN_M,
    &FOLIAGE_CUT,
    &FOLIAGE_OCCLUDERS,
    &FOLIAGE_SUN_LOD,
    &FOLIAGE_SUN_PAGE_METRES,
    &FOREST_FLOOR_DARK,
    &FROXEL_FOG,
    &GPU_PARTICLES,
    &GRASS_DEPTH_PREPASS,
    &GRASS_LEAF_BRDF,
    &GRASS_SUN_OCCLUSION,
    &HEADLIGHT_INSTANCE_GROUPS,
    &INSTANCE_BROADPHASE,
    &INSTANCE_GROUPS,
    &LEGACY_LIGHT_LOOKAHEAD,
    &MEADOW_AGGREGATE,
    &MEADOW_AGGREGATE_HALF_RES,
    &MEADOW_AGGREGATE_SAMPLE_PRUNE,
    &MEADOW_CONTACT_AO,
    &MEADOW_EDGE_SHADOW,
    &MEADOW_GROUP_CACHE,
    &MEADOW_GROUP_REPLACE,
    &MEADOW_INDEXED,
    &MEADOW_OCCLUSION,
    &MEADOW_SKIP_HIDDEN_SHADE,
    &MEADOW_SPATIAL_FILTER,
    &MEADOW_SUBSTRATE,
    &MEADOW_VISIBILITY,
    &MEASURED_ERRORS,
    &NEAR_SHADOW_LOD,
    &NEAR_SUN_SOURCE,
    &NEEDLE_BRUSH_OWN,
    &OCCLUSION,
    &OCCLUSION_REJECT,
    &OWNER_RANGE_INDEX,
    &PAGED_FOLIAGE_SUN,
    &PIPELINE_CACHE,
    &PIPELINE_CACHE_DIR,
    &PORTABLE,
    &PROBE_GRADIENTS,
    &RASTER_FLOOR,
    &RASTER_LARGE_FIRST,
    &RASTER_SPLIT,
    &RECEIVER_SHADOW_DEMAND,
    &REFLECTION_PROBES,
    &REFRACTION,
    &REUSE_SUN_TEXTURE,
    &ROCK_VARIATION,
    &RSM,
    &SHADER_DIR,
    &SHADE_CLASSES,
    &SHADE_FEATURES,
    &SHADE_UNLIT,
    &SHADE_PROBE,
    &PHONE_GI,
    &LIGHT_TILES,
    &FOOTPRINT_LOD,
    &ALBEDO_LOD_BIAS,
    &SHADOW_DISTANCE_LOD,
    &SHADOW_REACH,
    &SIMPLE_CULL,
    &SIMPLE_LIGHTING,
    &SKY_TILES,
    &SKY_VIS_CACHE,
    &SOURCE_CLUSTERS,
    &SOURCE_CROWN_BOUNDS,
    &SOURCE_PER_TREE,
    &SPLAT_BRICK_HZB,
    &SPLIT_CULL_TIMING,
    &SPLIT_SELECTION_TIMING,
    &SSSR,
    &STARTUP_PROFILE,
    &STATIC_SUN_CACHE,
    &SUN_CACHE_GUARD_METRES,
    &SUN_CLIPMAP,
    &SUN_DAMAGE_REPORT,
    &SUN_CLOSE,
    &SUN_DITHER_TEXEL,
    &SUN_INSTANCE_GROUPS,
    &SUN_LEVELS,
    &SUN_RECEIVER_PLANE,
    &SUN_SCENE_DEPTH,
    &SUN_STAGGER,
    &SUN_SURFACE_SOURCE,
    &SUN_TERRAIN_ERROR,
    &SUN_TERRAIN_LOD_DAMAGE,
    &SUN_VOXEL_M,
    &TERRAIN_COARSE,
    &TERRAIN_NODE_BUDGET,
    &TEXTURE_CACHE,
    &TEXTURE_CACHE_DIR,
    &TEXTURE_COMPRESSION,
    &TEXTURE_STREAMING,
    &THIN_PIECES,
    &TILED_POST,
    &TRACED_REFLECTIONS,
    &TREE_OCCLUSION,
    &TREE_OCCLUSION_REJECT,
    &VEGETATION_BUDGETS,
    &VEGETATION_QUALITY,
    &VIRTUAL_MEMBERS,
    &VIRTUAL_SERIAL_CULL,
    &VOXEL_ANTIPODAL_NORMALS,
    &VOXEL_BRICK_VISIBILITY,
    &VOXEL_EMPTY_CEILING,
    &VOXEL_GAP_MIPS,
    &VOXEL_GLOBAL_INTERVALS,
    &VOXEL_OWN_IN_CLIPMAP,
    &VOXEL_LEAF_PARITY,
    &VOXEL_DISC_DEPTH,
    &VOXEL_PASS,
    &VOXEL_RASTER,
    &VOXEL_SELF_SHADOW_COARSER,
    &VOXEL_STAMPS,
    &VOXEL_SURFACE,
    &WATER,
    &WEBGPU_LIMITS,
    &WIND_FREEZE,
    &WORLD_TERRAIN_VISIBILITY,
    &ZERO_SUN_FAST_PATH,
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::HashSet, fs, path::Path};

    #[test]
    fn names_are_unique_and_prefixed() {
        let mut seen = HashSet::new();
        for s in ALL {
            assert!(s.name.starts_with("KERN_"), "{} lacks the KERN_ prefix", s.name);
            assert!(seen.insert(s.name), "{} is declared twice", s.name);
        }
    }

    /// `name` appears as a whole word, so `KERN_FAR_CROWN` is not satisfied by `KERN_FAR_CROWN_DEBUG`.
    fn mentioned(docs: &str, name: &str) -> bool {
        docs.match_indices(name).any(|(at, _)| {
            !docs[at + name.len()..].starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
        })
    }

    #[test]
    fn every_switch_is_documented() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
        let docs = fs::read_to_string(root.join("renderer-switches.md")).unwrap()
            + &fs::read_to_string(root.join("building-and-running.md")).unwrap();
        let missing: Vec<_> = ALL.iter().map(|s| s.name).filter(|n| !mentioned(&docs, n)).collect();
        assert!(missing.is_empty(), "not in renderer-switches.md or building-and-running.md: {missing:?}");
    }

    #[test]
    fn no_raw_reads_outside_this_module() {
        fn walk(dir: &Path, found: &mut Vec<String>) {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() { walk(&path, found); continue; }
                if path.extension().map_or(true, |e| e != "rs") || path.file_name().unwrap() == "switches.rs" { continue; }
                for (i, line) in fs::read_to_string(&path).unwrap().lines().enumerate() {
                    let raw = line.contains("var(\"KERN_") || line.contains("var_os(\"KERN_");
                    if raw && !line.contains("set_var") && !line.contains("remove_var") {
                        found.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
                    }
                }
            }
        }
        let mut found = Vec::new();
        walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut found);
        assert!(found.is_empty(), "read KERN_* through crate::switches instead:\n{}", found.join("\n"));
    }
}
