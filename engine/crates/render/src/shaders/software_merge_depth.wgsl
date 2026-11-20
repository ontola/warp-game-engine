// Fold the depth-only software raster into a shadow map: complemented
// depth bits per texel, 0 = untouched. Pipeline depth test is Less.
@group(0) @binding(0) var<storage, read> depth32: array<u32>;
@group(0) @binding(1) var<uniform> merge_size: vec4<u32>;

@vertex
fn merge_vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn merge_depth_fs(@builtin(position) pos: vec4<f32>) -> @builtin(frag_depth) f32 {
    let p = vec2<u32>(pos.xy);
    let v = depth32[p.y * merge_size.x + p.x];
    if v == 0u { discard; }
    return bitcast<f32>(~v);
}
