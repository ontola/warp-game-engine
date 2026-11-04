// Rasterized foliage voxels: one disc per listed cell, facing the view
// (or the sun), covering the cell's opacity. Writes the splat id;
// depth testing leaves the nearest kept cell. Records and tree poses come
// from voxel_splats in voxel_forest.wgsl, which documents the layout.
@group(0) @binding(0) var<storage,read> splats: array<vec4<u32>>;
const HEADER: u32 = 12u;
fn tree(index: u32, row: u32) -> vec4<f32> { return bitcast<vec4<f32>>(splats[HEADER + index * 4u + row]); }
struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) splat: u32,
    @location(1) @interpolate(flat) alpha: f32,
    @location(2) @interpolate(flat) key: u32,
    // Position on the disc, -1..1 across it.
    @location(3) disc: vec2<f32>,
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
    // A cube's mean projected area is 1.5 faces: a disc of radius 0.69 cells
    // covers what three faces did (a square of side 1.22 read as blocks at a
    // distance). Its axes are the view's right and up (rows x and y of the
    // view-projection, also for the sun's orthographic map).
    // Opacity becomes size: a faint cell is a smaller solid disc (radius
    // with sqrt(alpha), the same coverage), so crowns end in ragged edges
    // rather than speckled squares; below alpha 0.25 the rest is dithered.
    let scale = length(vec3<f32>(r0.x, r1.x, r2.x));
    let alpha = f32(record.w & 0xffffu) / 65535.0;
    let solid = max(alpha, 0.25);
    let half = 0.688 * grid.w * scale * sqrt(solid);
    let corners = array<vec2<f32>, 6>(vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0));
    let corner = corners[vertex % 6u];
    let view_proj = mat4x4<f32>(bitcast<vec4<f32>>(splats[0]), bitcast<vec4<f32>>(splats[1]), bitcast<vec4<f32>>(splats[2]), bitcast<vec4<f32>>(splats[3]));
    let right = normalize(vec3<f32>(view_proj[0].x, view_proj[1].x, view_proj[2].x));
    let up = normalize(vec3<f32>(view_proj[0].y, view_proj[1].y, view_proj[2].y));
    // Local cell coordinate is stable when physical asset pages relocate.
    // record.y is a physical cell address and must never seed appearance.
    // Packed appearance seed comes from stable root/branch identity when available.
    out.key = hash(record.w >> 19u) ^ record.z;
    // A stable offset of up to a quarter cell: discs do not line up in rows.
    let jitter = vec2<f32>(f32(hash(out.key) & 0xffffu), f32(hash(out.key ^ 0x9e3779b9u) & 0xffffu)) / 65535.0 - 0.5;
    let shift = (right * jitter.x + up * jitter.y) * 0.5 * grid.w * scale;
    let p = centre + shift + (right * corner.x + up * corner.y) * half;
    out.position = view_proj * vec4<f32>(p, 1.0);
    out.splat = instance + 1u;
    out.alpha = alpha / solid;
    out.disc = corner;
    return out;
}
fn kept(in: Out) -> bool {
    if dot(in.disc, in.disc) > 1.0 { return false; }
    let p = vec2<u32>(in.position.xy);
    return f32(hash(p.x ^ hash(p.y ^ hash(in.key))) >> 8u) / 16777216.0 < in.alpha;
}
@fragment fn fs(in: Out) -> @location(0) u32 {
    if !kept(in) { discard; }
    return in.splat;
}
// Into the scene's own visibility buffer (bit 31 marks a voxel; shade.wgsl)
// and the splat id target the surface pass reads.
struct MainOut { @location(0) visibility: u32, @location(1) splat: u32 };
@fragment fn fs_main(in: Out) -> MainOut {
    if !kept(in) { discard; }
    return MainOut(0x80000000u | in.splat, in.splat);
}
// Sun map: depth only.
@fragment fn fs_depth(in: Out) {
    if !kept(in) { discard; }
}

override SHADOW_TILE_PIXELS:u32=64u;
// Shared sun tile cache only; ordinary/near/headlamp entry points stay independent.
@group(1) @binding(0) var<uniform> shadow_damage_mask:array<vec4<u32>,32>;
@fragment fn fs_cached_depth(in:Out) {
 let xy=vec2<u32>(in.position.xy)/SHADOW_TILE_PIXELS;let tile=xy.y*64u+xy.x;
 let word=shadow_damage_mask[tile>>7u][(tile>>5u)&3u];
 if (word&(1u<<(tile&31u)))==0u {discard;}
 if !kept(in) {discard;}
}
