//! Shader sources for iteration without a rebuild.
//!
//! The shading chain (wind_common, shade, car_material, grass_lighting,
//! headlights and transparency) is
//! compiled into the binary, and can instead be read from disk: set
//! `KERN_SHADER_DIR` to a directory holding those `.wgsl` files (normally
//! `crates/render/src/shaders`). A missing or unreadable file falls back to
//! the built-in copy, so a partial directory is fine. `Renderer::reload_shaders`
//! re-reads them and rebuilds the affected pipelines, keeping the old ones
//! when the new code fails validation.

use std::path::PathBuf;

/// The built-in copies of every file that can be read from disk.
const BUILT_IN: [(&str, &str); 6] = [
    ("wind_common.wgsl", include_str!("shaders/wind_common.wgsl")),
    ("shade.wgsl", include_str!("shaders/shade.wgsl")),
    ("car_material.wgsl", include_str!("shaders/car_material.wgsl")),
    ("grass_lighting.wgsl", include_str!("shaders/grass_lighting.wgsl")),
    ("headlights.wgsl", include_str!("shaders/headlights.wgsl")),
    ("transparency.wgsl", include_str!("shaders/transparency.wgsl")),
];

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
    if let Some(path) = dir().map(|d| d.join(name)) {
        if let Ok(text) = std::fs::read_to_string(&path) {
            return text;
        }
    }
    built_in(name).to_owned()
}

/// Run `build` under a validation error scope: the compiler's message
/// instead of a panic when new shader code does not validate.
pub(crate) fn checked<T>(gpu: &crate::Gpu, build: impl FnOnce() -> T) -> Result<T, String> {
    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let value = build();
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
