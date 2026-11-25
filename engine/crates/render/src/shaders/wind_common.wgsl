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

// A terminal stand keeps its ordinary owner/cluster visibility address. Its
// triangle selects the original planting's gust for every raster/shading pass.
fn triangle_wind(owner: u32, root_y: f32, packed: u32) -> vec3<f32> {
    let w = wind_coefficients[owner];
    if w.w > 0.0 {
        let first = u32(w.w);
        let last = u32(wind_coefficients[first - 1u].w) - 1u;
        let member = wind_coefficients[first + min(packed >> 24u, last)];
        return vec3<f32>(member.xy, member.w);
    }
    return vec3<f32>(w.xy, root_y);
}
fn triangle_wind_position(w: vec3<f32>, p: vec3<f32>) -> vec3<f32> {
    return p + vec3<f32>(w.x, 0.0, w.y) * (p.y - w.z);
}
fn triangle_wind_normal(w: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(n.x,n.y-dot(w.xy,n.xz),n.z);
}
