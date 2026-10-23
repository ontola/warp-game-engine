// Convert atomic canopy splats into a hardware-filtered 3D density field.
// The grid is world-aligned; trilinear filtering removes moving voxel edges.
struct Gi {
    vol_origin: vec4<f32>,
    vol_dims: vec4<u32>,
    hf: vec4<f32>,
    hf_dims: vec4<u32>,
    bounce: vec4<f32>,
    params: vec4<f32>,
};
@group(0) @binding(0) var<uniform> gi: Gi;
@group(0) @binding(1) var<storage, read> density: array<u32>;
@group(0) @binding(2) var filtered: texture_storage_3d<rgba8unorm, write>;
@compute @workgroup_size(8, 4, 8)
fn main(@builtin(global_invocation_id) q: vec3<u32>) {
    if any(q >= gi.vol_dims.xyz) { return; }
    let i = (q.z * gi.vol_dims.y + q.y) * gi.vol_dims.x + q.x;
    textureStore(filtered, vec3<i32>(q), vec4<f32>(f32(density[i]) / 255.0, 0.0, 0.0, 1.0));
}
