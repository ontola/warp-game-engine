//! Shader sources for iteration without a rebuild.
//!
//! The shading chain (wind_common, shade, car_material, grass_lighting,
//! headlights and transparency) is
//! compiled into the binary, and can instead be read from disk: set
//! `KERN_SHADER_DIR` to a directory holding those `.wgsl` files (normally
//! `crates/render/src/shaders`). A missing or unreadable file falls back to
//! the built-in copy, so a partial directory is fine. `Renderer::reload_shaders`
//! takes one immutable snapshot and prepares every active material consumer.
//! The entire generation commits only after validation succeeds. Layout or
//! wind deformation changes require a renderer rebuild.

use std::path::PathBuf;

/// The built-in copies of every file that can be read from disk.
pub(crate) const SHARED_NAMES: [&str; 22] = ["cloud_weather.wgsl", "game_surface.wgsl", "terrain_field.wgsl", "forest_field.wgsl", "terrain_surface.wgsl", "wind_common.wgsl", "terrain_trace.wgsl", "meadow_field.wgsl", "meadow_canopy.wgsl", "foliage_volume.wgsl", "surface_fog.wgsl", "shade.wgsl",
    "world_visibility.wgsl", "foliage_sun_pages.wgsl", "car_material.wgsl", "grass_lighting.wgsl", "incident_lights.wgsl", "headlights.wgsl", "clustered_lights.wgsl", "rsm.wgsl", "car_lamps.wgsl", "procedural_surface.wgsl"];
const BUILT_IN: [(&str, &str); 26] = [
    ("surface_fog.wgsl", include_str!("shaders/surface_fog.wgsl")),
    ("cloud_weather.wgsl", include_str!("shaders/cloud_weather.wgsl")),
    ("terrain_field.wgsl", crate::terrain_field::WGSL),
    ("forest_field.wgsl", crate::forest_field::WGSL),
    ("game_surface.wgsl", include_str!("shaders/game_surface.wgsl")),
    ("terrain_surface.wgsl", include_str!("shaders/terrain_surface.wgsl")),
    ("procedural_surface.wgsl", include_str!("shaders/procedural_surface.wgsl")),
    ("foliage_volume.wgsl", include_str!("shaders/foliage_volume.wgsl")),
    ("meadow_field.wgsl", include_str!("shaders/meadow_field.wgsl")),
    ("meadow_canopy.wgsl", include_str!("shaders/meadow_canopy.wgsl")),
    ("foliage_sun_pages.wgsl", include_str!("shaders/foliage_sun_pages.wgsl")),
    ("terrain_trace.wgsl", include_str!("shaders/terrain_trace.wgsl")),
    ("world_visibility.wgsl", include_str!("shaders/world_visibility.wgsl")),
    ("wind_common.wgsl", include_str!("shaders/wind_common.wgsl")),
    ("shade.wgsl", include_str!("shaders/shade.wgsl")),
    ("car_material.wgsl", include_str!("shaders/car_material.wgsl")),
    ("car_lamps.wgsl", include_str!("shaders/car_lamps.wgsl")),
    ("grass_lighting.wgsl", include_str!("shaders/grass_lighting.wgsl")),
    ("incident_lights.wgsl", include_str!("shaders/incident_lights.wgsl")),
    ("headlights.wgsl", include_str!("shaders/headlights.wgsl")),
    ("clustered_lights.wgsl", include_str!("shaders/clustered_lights.wgsl")),
    ("rsm.wgsl", include_str!("shaders/rsm.wgsl")),
    ("transparency.wgsl", include_str!("shaders/transparency.wgsl")),
    ("grass_transition.wgsl", include_str!("shaders/grass_transition.wgsl")),
    ("far_grass.wgsl", include_str!("shaders/far_grass.wgsl")),
    ("voxel_forest.wgsl", include_str!("shaders/voxel_forest.wgsl")),
];

/// One source generation, shared by all pipeline builders. Reading each
/// consumer independently could compile different edits into the same frame.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Snapshot {
    directory: Option<PathBuf>,
    files: Vec<String>,
}
pub(crate) struct LightingSources {
    pub shade: String,
    pub transparency: String,
    pub grass: String,
    pub grass_selection: String,
    pub far_grass: String,
    pub voxel: String,
}
impl Snapshot {
    pub(crate) fn load() -> Result<Self, String> { Self::read(dir()) }
    fn read(directory: Option<PathBuf>) -> Result<Self, String> {
        let files = BUILT_IN.iter().map(|(name, built_in)| {
            let Some(path) = directory.as_ref().map(|d| d.join(name)) else { return Ok((*built_in).to_owned()); };
            match std::fs::read_to_string(&path) {
                Ok(text) => Ok(text),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok((*built_in).to_owned()),
                Err(error) => Err(format!("cannot read shader {}: {error}", path.display())),
            }
        }).collect::<Result<Vec<_>, String>>()?;
        let mut files = files;
        if let Some(game) = GAME_SURFACE.lock().unwrap().clone() {
            files[BUILT_IN.iter().position(|(n, _)| *n == "game_surface.wgsl").unwrap()] = game;
        }
        Ok(Self { directory, files })
    }
    pub(crate) fn source(&self, name: &str) -> &str {
        &self.files[BUILT_IN.iter().position(|(n, _)| *n == name).expect("registered shader")]
    }
    pub(crate) fn shade(&self) -> String {
        format!("{}\n{}", include_str!("shaders/virtual_member.wgsl"), SHARED_NAMES.map(|name| self.source(name)).join("\n"))
    }
    pub(crate) fn lighting(&self) -> LightingSources {
        let shade = self.shade();
        let grass_common = include_str!("shaders/grass_common.wgsl");
        LightingSources {
            transparency: format!("{shade}\n{}", self.source("transparency.wgsl")),
            grass: format!("{shade}\n{grass_common}\n{}", self.source("grass_transition.wgsl")),
            grass_selection: format!("{shade}\n{grass_common}\n{}\n{}", crate::foliage_compact::selection_source(), include_str!("shaders/grass_compact.wgsl")),
            far_grass: format!("{shade}\n{}", self.source("far_grass.wgsl")),
            voxel: format!("{shade}\n{}\n{}", self.source("voxel_forest.wgsl"),include_str!("shaders/shadow_damage_query.wgsl")),
            shade,
        }
    }
    /// Abort rather than committing a mixed save while compilers were busy.
    pub(crate) fn ensure_current(&self) -> Result<(), String> {
        if self.directory != dir() || *self != Self::read(self.directory.clone())? {
            return Err("shader files changed during compilation; retry reload".into());
        }
        Ok(())
    }
}

static GAME_SURFACE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Replace game_surface.wgsl (material flag SURFACE) with a game's own
/// `fn game_surface(s: SurfaceIn) -> SurfaceOut`. Call it before creating
/// the renderer; it wins over `KERN_SHADER_DIR`.
pub fn set_game_surface(source: &str) {
    *GAME_SURFACE.lock().unwrap() = Some(source.to_owned());
}

/// The override directory, if `KERN_SHADER_DIR` names one.
pub fn dir() -> Option<PathBuf> {
    std::env::var_os("KERN_SHADER_DIR").map(PathBuf::from).filter(|d| d.is_dir())
}

/// The built-in source of `name` (one of the files above).
pub(crate) fn built_in(name: &str) -> &'static str {
    BUILT_IN.iter().find(|(n, _)| *n == name).map(|(_, s)| *s).unwrap_or_else(|| panic!("unknown shader {name}"))
}

/// `name` from `KERN_SHADER_DIR` when it is there, else the built-in copy.
pub(crate) fn source(name: &str) -> String {
    if name == "game_surface.wgsl" {
        if let Some(game) = GAME_SURFACE.lock().unwrap().clone() {
            return game;
        }
    }
    if let Some(path) = dir().map(|d| d.join(name)) {
        if let Ok(text) = std::fs::read_to_string(&path) {
            return text;
        }
    }
    built_in(name).to_owned()
}

/// Run `build` under a validation error scope: the compiler's message
/// instead of a panic when new shader code does not validate.
pub fn checked<T>(gpu: &crate::Gpu, build: impl FnOnce() -> T) -> Result<T, String> {
    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let value = build();
    // A browser answers the scope later, on its own event loop, so waiting
    // here would never return: its errors go to the device's handler.
    #[cfg(target_arch = "wasm32")]
    { drop(scope.pop()); return Ok(value); }
    #[cfg(not(target_arch = "wasm32"))]
    match pollster::block_on(scope.pop()) {
        Some(e) => Err(e.to_string()),
        None => Ok(value),
    }
}

/// The newest modification time of the overridable files on disk: cheap to
/// poll every frame, and `None` without `KERN_SHADER_DIR`.
pub fn stamp() -> Option<std::time::SystemTime> {
    let d = dir()?;
    BUILT_IN.iter().filter_map(|(n, _)| std::fs::metadata(d.join(n)).and_then(|m| m.modified()).ok()).max()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_volume_optics_have_no_resource_bindings_and_preserve_cell_layout() {
        // A different provider can compile the optical module without any
        // scene/forest globals. Only the existing pure BRDF maths are needed.
        let source = format!("const PI:f32=3.14159265359;\n\
            fn env_brdf(f0:vec3<f32>,rough:f32,ndv:f32)->vec3<f32>{{return f0;}}\n\
            fn ggx(n:vec3<f32>,v:vec3<f32>,l:vec3<f32>,rough:f32,f0:vec3<f32>)->vec3<f32>{{return vec3<f32>(0.0);}}\n{}",
            built_in("foliage_volume.wgsl"));
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module).unwrap_or_else(|error| panic!("{}", error.emit_to_string(&source)));
        assert!(module.global_variables.is_empty());
        for (name, bytes) in [("VolumeLevel", 32), ("VolumeCell", 176), ("VolumeSample", 160)] {
            let ty = module.types.iter().find(|(_, ty)| ty.name.as_deref() == Some(name)).unwrap().1;
            let naga::TypeInner::Struct { span, .. } = ty.inner else { panic!("{name} is not a struct") };
            assert_eq!(span, bytes, "shared extraction changed {name}");
        }
        for name in ["volume_direction", "volume_sigma", "volume_accumulate_cell", "volume_normalize_sample",
            "volume_blend_mips", "volume_transport_segment", "volume_thin_leaf_direct"] {
            assert!(module.functions.iter().any(|(_, function)| function.name.as_deref() == Some(name)));
        }
        let forest = built_in("voxel_forest.wgsl");
        assert!(forest.contains("result=volume_accumulate_cell(result,cell,sigma)"));
        assert!(forest.contains("return volume_blend_mips(a,b,blend)"));
        assert!(forest.contains("volume_transport_segment(transmittance,total_sigma,ds)"));
        assert!(forest.contains("volume_leaf_headlights(p,n,v,base,rough,sample.leaf_fraction)"));
        assert!(forest.contains("if all(lamp.irradiance==vec3<f32>(0.0)) {continue;}"));
    }

    #[test]
    fn volume_thin_leaf_diffuse_budget_matches_source_front_back_and_zero_light() {
        // Read the actual shader coefficient, then independently integrate its
        // two hemispheres. A rear lamp must transmit leaf pigment, not vanish
        // as it did in the opaque headlight call. Bark stays one-sided.
        let composed = Snapshot::read(None).unwrap().shade();
        let module = naga::front::wgsl::parse_str(&composed)
            .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&composed)));
        let constant = module.constants.iter().find(|(_, value)|
            value.name.as_deref() == Some("VOLUME_LEAF_TRANSMISSION")).unwrap().1;
        let naga::Expression::Literal(naga::Literal::F32(transmission)) = module.global_expressions[constant.init]
            else { panic!("transmission must remain an explicit scalar") };
        assert_eq!(transmission, 0.25);
        let t = f64::from(transmission);
        let pigment = [0.13, 0.31, 0.07];
        for leaf in [0.0, 0.3, 1.0] {
            let mut integrated = 0.0;
            // Uniform cosine-coordinate quadrature: dOmega=2*pi*dMu.
            for i in 0..2048 {
                let mu = -1.0 + (i as f64 + 0.5) / 1024.0;
                let response = (1.0-t*leaf)*mu.max(0.0)+(t*leaf)*(-mu).max(0.0);
                integrated += response * 2.0 / 1024.0;
            }
            assert!((integrated - 1.0).abs() < 1e-12, "diffuse split created energy");
            for base in pigment {
                let front = base*(1.0-t*leaf)/std::f64::consts::PI;
                let back = base*t*leaf/std::f64::consts::PI;
                assert!((front+back-base/std::f64::consts::PI).abs()<1e-15);
                assert_eq!(back == 0.0, leaf == 0.0);
                assert_eq!(back * 0.0, 0.0, "blocked/out-of-range light must contribute zero");
            }
        }
        let source = include_str!("shaders/meadow_render.wgsl");
        assert!(source.contains("0.75*max(ndl,0.0)+0.25*max(-ndl,0.0)"));
    }

    #[test]
    fn volume_transport_conserves_energy_under_segment_partition_and_vacuum() {
        let source = built_in("foliage_volume.wgsl");
        assert!(source.contains("let attenuation=exp(-sigma*distance)"));
        assert!(source.contains("transmittance*(1.0-attenuation)"));
        // Independent analytic solution for a constant medium: splitting the
        // same physical path cannot alter its total radiance or transmittance.
        for sigma in [0.0_f64, 0.001, 0.4, 8.0] {
            for distance in [0.0_f64, 0.03, 2.0, 50.0] {
                let expected = (-sigma*distance).exp();
                let mut transmittance = 1.0;
                let mut opacity = 0.0;
                for fraction in [0.07, 0.2, 0.31, 0.42] {
                    let attenuation = (-sigma*distance*fraction).exp();
                    opacity += transmittance*(1.0-attenuation);
                    transmittance *= attenuation;
                }
                assert!((transmittance-expected).abs()<1e-14);
                assert!((opacity+transmittance-1.0).abs()<1e-14);
                if sigma == 0.0 || distance == 0.0 { assert_eq!((opacity,transmittance),(0.0,1.0)); }
            }
        }
    }

    fn consumers(s: &LightingSources) -> [&str; 6] {
        [&s.shade, &s.transparency, &s.grass, &s.grass_selection, &s.far_grass, &s.voxel]
    }
    #[test]
    fn lighting_snapshot_propagates_every_shared_dependency_to_every_consumer() {
        let original = Snapshot::read(None).unwrap();
        let baseline = original.lighting();
        for name in ["shade.wgsl", "car_material.wgsl", "car_lamps.wgsl", "grass_lighting.wgsl", "incident_lights.wgsl", "headlights.wgsl", "foliage_volume.wgsl"] {
            let mut changed = original.clone();
            let index = BUILT_IN.iter().position(|(n, _)| *n == name).unwrap();
            changed.files[index].push_str("\n// reload generation marker\n");
            let next = changed.lighting();
            for (old, new) in consumers(&baseline).into_iter().zip(consumers(&next)) {
                assert_ne!(old, new, "{name} was omitted from a material consumer");
                assert!(new.starts_with(&next.shade));
                assert_eq!(new.matches("// reload generation marker").count(), 1);
            }
        }
    }
    #[test]
    fn lighting_snapshot_entrypoint_edits_have_explicit_dependencies() {
        let original = Snapshot::read(None).unwrap();
        let baseline = original.lighting();
        for (name, consumer) in [("transparency.wgsl", 1), ("grass_transition.wgsl", 2), ("far_grass.wgsl", 4), ("voxel_forest.wgsl", 5)] {
            let mut changed = original.clone();
            let index = BUILT_IN.iter().position(|(n, _)| *n == name).unwrap();
            changed.files[index].push_str("\n// entrypoint edit\n");
            let next = changed.lighting();
            for (i, (old, new)) in consumers(&baseline).into_iter().zip(consumers(&next)).enumerate() {
                assert_eq!(old != new, i == consumer, "{name} dependency {i}");
            }
        }
    }
    #[test]
    fn lighting_snapshot_is_immutable_across_disk_edits_and_partial_directories() {
        let unique = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("kern-shader-snapshot-{}-{unique}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        let file = path.join("shade.wgsl");
        std::fs::write(&file, "first generation").unwrap();
        let a = Snapshot::read(Some(path.clone())).unwrap();
        assert_eq!(a.source("shade.wgsl"), "first generation");
        assert_eq!(a.source("far_grass.wgsl"), built_in("far_grass.wgsl"));
        std::fs::write(&file, "second generation").unwrap();
        let b = Snapshot::read(Some(path.clone())).unwrap();
        assert!(a != b);
        assert_eq!(a.source("shade.wgsl"), "first generation");
        // Cleanup only the two exact paths created by this test.
        std::fs::remove_file(file).unwrap();
        std::fs::remove_dir(path).unwrap();
    }
}

// Optional game-owned visibility mask, shared by colour and depth raster passes.
static GAME_MASK: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
/// Supply `fn game_mask(world:vec3<f32>, instance:u32, material:u32)->bool`.
/// True discards the fragment, including its shadow. May read `aux` and materials.
/// Set before renderer construction; this hook is immutable for that renderer.
pub fn set_game_mask(source: &str) { *GAME_MASK.lock().unwrap() = Some(source.to_owned()); }
/// Whether a game supplied [`set_game_mask`]: it may discard any material.
pub(crate) fn has_game_mask() -> bool { GAME_MASK.lock().unwrap().is_some() }
pub(crate) fn game_mask() -> String {
    GAME_MASK.lock().unwrap().clone().unwrap_or_else(||
        "fn game_mask(world:vec3<f32>, instance:u32, material:u32)->bool {return false;}".into())
}
