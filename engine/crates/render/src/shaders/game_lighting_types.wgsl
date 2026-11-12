struct LightingSample {
    kind: i32,         // Luau surface kind, or -1 for ordinary materials
    flags: u32,        // Warp material flags
    position: vec3<f32>,
    normal: vec3<f32>,
    albedo: vec3<f32>,
    radiance: vec3<f32>,
    sun: vec3<f32>,    // direction toward the sun
    sun_lux: f32,
    shadow: f32,       // 0 occluded, 1 visible
    time: f32,         // lighting animation clock, seconds
}
struct StyledLight { radiance: vec3<f32>, tag: u32 }
