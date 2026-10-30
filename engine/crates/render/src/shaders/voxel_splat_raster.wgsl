// Rasterized foliage voxels: three camera-facing faces of each listed cell,
// kept per pixel with the cell's opacity. Writes the splat id; depth testing
// leaves the nearest kept cell. Records come from voxel_splats in
// voxel_forest.wgsl, which documents the layout.
@group(0) @binding(0) var<storage,read> splats: array<vec4<f32>>;
const HEADER: u32 = 12u;
struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) splat: u32,
    @location(1) @interpolate(flat) alpha: f32,
};
@vertex fn vs(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> Out {
    var out: Out;
    let base = HEADER + instance * 4u;
    if base + 4u > arrayLength(&splats) { out.position = vec4<f32>(0.0, 0.0, -1.0, 1.0); return out; }
    let centre = splats[base];
    let faces = bitcast<u32>(splats[base + 2u].w);
    var axes = array<vec3<f32>, 3>(splats[base + 1u].xyz, splats[base + 2u].xyz, splats[base + 3u].xyz);
    let face = vertex / 6u;
    var corners = array<vec2<f32>, 6>(vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0));
    let corner = corners[vertex % 6u];
    let side = select(-1.0, 1.0, (faces & (1u << face)) != 0u);
    let p = centre.xyz + axes[face] * side + axes[(face + 1u) % 3u] * corner.x + axes[(face + 2u) % 3u] * corner.y;
    let view_proj = mat4x4<f32>(splats[0], splats[1], splats[2], splats[3]);
    out.position = view_proj * vec4<f32>(p, 1.0);
    out.splat = instance + 1u;
    out.alpha = centre.w;
    return out;
}
fn hash(x: u32) -> u32 {
    var h = x * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    return (h >> 22u) ^ h;
}
// Keyed on the static cell, not the per-frame list slot, so a still image
// does not shimmer.
fn kept(in: Out) -> bool {
    let cell = bitcast<u32>(splats[HEADER + (in.splat - 1u) * 4u + 1u].w);
    let p = vec2<u32>(in.position.xy);
    return f32(hash(p.x ^ hash(p.y ^ hash(cell))) >> 8u) / 16777216.0 < in.alpha;
}
@fragment fn fs(in: Out) -> @location(0) u32 {
    if !kept(in) { discard; }
    return in.splat;
}
// Sun map: depth only.
@fragment fn fs_depth(in: Out) {
    if !kept(in) { discard; }
}
