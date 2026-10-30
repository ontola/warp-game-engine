// Rasterized foliage voxels: three camera-facing faces of each listed cell,
// kept per pixel with the cell's opacity. Writes the splat id; depth testing
// leaves the nearest kept cell. Records and tree poses come from
// voxel_splats in voxel_forest.wgsl, which documents the layout.
@group(0) @binding(0) var<storage,read> splats: array<vec4<u32>>;
const HEADER: u32 = 12u;
fn tree(index: u32, row: u32) -> vec4<f32> { return bitcast<vec4<f32>>(splats[HEADER + index * 4u + row]); }
struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) splat: u32,
    @location(1) @interpolate(flat) alpha: f32,
    @location(2) @interpolate(flat) key: u32,
};
fn hash(x: u32) -> u32 {
    var h = x * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    return (h >> 22u) ^ h;
}
@vertex fn vs(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> Out {
    var out: Out;
    let first = HEADER + splats[11].x * 4u;
    if first + instance >= arrayLength(&splats) { out.position = vec4<f32>(0.0, 0.0, -1.0, 1.0); return out; }
    let record = splats[first + instance];
    let r0 = tree(record.x, 0u);
    let r1 = tree(record.x, 1u);
    let r2 = tree(record.x, 2u);
    let grid = tree(record.x, 3u);
    let coord = vec3<f32>(f32(record.z & 1023u), f32((record.z >> 10u) & 1023u), f32(record.z >> 20u));
    let local = vec4<f32>(grid.xyz + (coord + 0.5) * grid.w, 1.0);
    let centre = vec3<f32>(dot(r0, local), dot(r1, local), dot(r2, local));
    let half = 0.5 * grid.w;
    var axes = array<vec3<f32>, 3>(vec3<f32>(r0.x, r1.x, r2.x) * half, vec3<f32>(r0.y, r1.y, r2.y) * half, vec3<f32>(r0.z, r1.z, r2.z) * half);
    let face = vertex / 6u;
    var corners = array<vec2<f32>, 6>(vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0));
    let corner = corners[vertex % 6u];
    let side = select(-1.0, 1.0, ((record.w >> 16u) & (1u << face)) != 0u);
    let p = centre + axes[face] * side + axes[(face + 1u) % 3u] * corner.x + axes[(face + 2u) % 3u] * corner.y;
    let view_proj = mat4x4<f32>(bitcast<vec4<f32>>(splats[0]), bitcast<vec4<f32>>(splats[1]), bitcast<vec4<f32>>(splats[2]), bitcast<vec4<f32>>(splats[3]));
    out.position = view_proj * vec4<f32>(p, 1.0);
    out.splat = instance + 1u;
    out.alpha = f32(record.w & 0xffffu) / 65535.0;
    // Keyed on the tree and cell, not the per-frame list slot, so a still
    // image does not shimmer.
    out.key = hash(record.x) ^ record.y;
    return out;
}
fn kept(in: Out) -> bool {
    let p = vec2<u32>(in.position.xy);
    return f32(hash(p.x ^ hash(p.y ^ hash(in.key))) >> 8u) / 16777216.0 < in.alpha;
}
@fragment fn fs(in: Out) -> @location(0) u32 {
    if !kept(in) { discard; }
    return in.splat;
}
// Sun map: depth only.
@fragment fn fs_depth(in: Out) {
    if !kept(in) { discard; }
}
