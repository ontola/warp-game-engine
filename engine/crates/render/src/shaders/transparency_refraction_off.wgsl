// Refraction disabled: no group 3, refracting glass blends as before.
const REFRACTION_ENABLED: bool = false;
struct RefractedBackground { bent: vec3<f32> };
fn refraction_phase_keeps(phase: u32, pixel: vec2<f32>, z: f32) -> bool { return true; }
fn refraction_background(pixel: vec2<f32>, glass_depth: f32, p: vec3<f32>, v: vec3<f32>, n: vec3<f32>,
    ior: f32, thickness: f32, rough: f32) -> RefractedBackground {
    return RefractedBackground(vec3<f32>(0.0));
}
