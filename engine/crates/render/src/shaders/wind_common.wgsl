// Affine whole-plant sway. Coefficients are generated once per instance, never
// independently per view/LOD/shadow. Root and physical height remain fixed.
@group(0) @binding(12) var<storage,read> wind_coefficients: array<vec4<f32>>;
fn wind_position(owner: u32, root_y: f32, p: vec3<f32>) -> vec3<f32> {
    let shear = wind_coefficients[owner].xy;
    return p + vec3<f32>(shear.x,0.0,shear.y)*(p.y-root_y);
}
fn wind_normal(owner: u32, n: vec3<f32>) -> vec3<f32> {
    // Inverse transpose of x'=x+sx*y, z'=z+sz*y; determinant is exactly one.
    let shear = wind_coefficients[owner].xy;
    return vec3<f32>(n.x,n.y-dot(shear,n.xz),n.z);
}
