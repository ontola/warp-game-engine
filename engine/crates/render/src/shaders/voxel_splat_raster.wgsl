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
// One splat's disc: world centre (jittered), half size, the view's right
// and up, its appearance key and coverage alpha (alpha/64 in 16 bits).
struct Disc { centre: vec3<f32>, half: f32, right: vec3<f32>, alpha: f32, up: vec3<f32>, key: u32, solid: f32 };
fn view_proj() -> mat4x4<f32> {
    return mat4x4<f32>(bitcast<vec4<f32>>(splats[0]), bitcast<vec4<f32>>(splats[1]), bitcast<vec4<f32>>(splats[2]), bitcast<vec4<f32>>(splats[3]));
}
fn disc(record: vec4<u32>) -> Disc {
    var d: Disc;
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
    // alpha/64 in 16 bits (VOLUME_SPLAT_ALPHA_RANGE): above 1 it is a
    // sub-pixel splat standing for several, drawn larger and solid.
    let alpha = f32(record.w & 0xffffu) / 65535.0 * 64.0;
    d.solid = max(alpha, 0.25);
    // 1.15x: kept() thins the rim of discs several pixels wide into ragged
    // leaf edges (and every disc evenly when it is tiny), losing ~24 % of
    // the area; the larger disc keeps the cell's coverage.
    d.half = 1.15 * 0.688 * grid.w * scale * sqrt(d.solid);
    let vp = view_proj();
    d.right = normalize(vec3<f32>(vp[0].x, vp[1].x, vp[2].x));
    d.up = normalize(vec3<f32>(vp[0].y, vp[1].y, vp[2].y));
    // Local cell coordinate is stable when physical asset pages relocate.
    // record.y is a physical cell address and must never seed appearance.
    // Packed appearance seed comes from stable root/branch identity when available.
    d.key = hash(record.w >> 19u) ^ record.z;
    // A stable offset of up to a quarter cell: discs do not line up in rows.
    let jitter = vec2<f32>(f32(hash(d.key) & 0xffffu), f32(hash(d.key ^ 0x9e3779b9u) & 0xffffu)) / 65535.0 - 0.5;
    d.centre = centre + (d.right * jitter.x + d.up * jitter.y) * 0.5 * grid.w * scale;
    d.alpha = min(alpha, 1.0) / d.solid;
    return d;
}
// Camera targets carry their render height in header row 11.y
// (voxel_splats; sun maps their map size). The disc's radius in pixels.
fn radius_px(d: Disc, clip_w: f32) -> f32 {
    let vp = view_proj();
    let row_y = vec3<f32>(vp[0].y, vp[1].y, vp[2].y);
    return d.half * length(row_y) / max(clip_w, 1e-6) * 0.5 * f32(splats[11].y);
}
// Discs below this radius (pixels) go to the compute raster
// (voxel_splat_compute_*) instead of quads: a pixel-sized quad runs its
// discard fragment shader over a 2x2 quad with late depth, at the ~10x
// overdraw of a crown (KERN_SPLAT_COMPUTE=1).
override COMPUTE_SMALL: bool = false;
override SMALL_PX: f32 = 1.5;
@vertex fn vs(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> Out {
    var out: Out;
    let first = HEADER + splats[11].x * 4u;
    // Brick culling's second draw continues after the first (11.z); its
    // queue starts at 11.w (voxel_forest.wgsl VOXEL_BRICK_CULL).
    let splat = instance + splats[11].z;
    let limit = select(arrayLength(&splats), splats[11].w, splats[11].w != 0u);
    if first + splat >= limit { out.position = vec4<f32>(0.0, 0.0, -1.0, 1.0); return out; }
    let record = splats[first + splat];
    let d = disc(record);
    let vp = view_proj();
    if COMPUTE_SMALL {
        let c = vp * vec4<f32>(d.centre, 1.0);
        if c.w > 1e-4 && radius_px(d, c.w) < SMALL_PX { out.position = vec4<f32>(0.0, 0.0, -1.0, 1.0); return out; }
    }
    let corners = array<vec2<f32>, 6>(vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0));
    let corner = corners[vertex % 6u];
    let p = d.centre + (d.right * corner.x + d.up * corner.y) * d.half;
    out.position = vp * vec4<f32>(p, 1.0);
    out.key = d.key;
    out.splat = splat + 1u;
    out.alpha = d.alpha;
    out.disc = corner;
    return out;
}
fn kept(in: Out) -> bool {
    let r2 = dot(in.disc, in.disc);
    if r2 > 1.0 { return false; }
    // Disc radius in pixels: wide discs read as balls (Joep, 6 November),
    // so their rim breaks up stochastically; small ones drop evenly.
    let radius_px = 1.0 / max(fwidth(in.disc.x), 1e-4);
    let wide = smoothstep(1.5, 4.0, radius_px);
    let keep = mix(0.76, 1.0 - 0.75 * smoothstep(0.3, 1.0, r2), wide);
    let p = vec2<u32>(in.position.xy);
    return f32(hash(p.x ^ hash(p.y ^ hash(in.key))) >> 8u) / 16777216.0 < in.alpha * keep;
}
// Sun maps: the coverage pattern is anchored to the splat (an 8x8 grid over
// its disc), not to the map's texels. The sun clipmap scrolls with the
// camera and redraws tiles, and a texel-anchored pattern then moved across
// the ground under every partly covered crown edge (Joep, flying, 12
// November). KERN_SUN_DITHER_TEXEL=1 restores the texel pattern.
override SUN_DITHER_ANCHORED: bool = true;
fn kept_sun(in: Out) -> bool {
    if !SUN_DITHER_ANCHORED { return kept(in); }
    let r2 = dot(in.disc, in.disc);
    if r2 > 1.0 { return false; }
    let radius_px = 1.0 / max(fwidth(in.disc.x), 1e-4);
    let wide = smoothstep(1.5, 4.0, radius_px);
    let keep = mix(0.76, 1.0 - 0.75 * smoothstep(0.3, 1.0, r2), wide);
    let q = vec2<u32>(clamp((in.disc + 1.0) * 4.0, vec2<f32>(0.0), vec2<f32>(7.0)));
    return f32(hash(q.x ^ hash(q.y ^ hash(in.key))) >> 8u) / 16777216.0 < in.alpha * keep;
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
    if !kept_sun(in) { discard; }
}

// Cached sun maps: 64x64 tiles of the map size in header row 11.y.
@group(1) @binding(0) var<uniform> shadow_damage_mask:array<vec4<u32>,32>;
@fragment fn fs_cached_depth(in:Out) {
 let xy=vec2<u32>(in.position.xy)/max(splats[11].y/64u,1u);let tile=xy.y*64u+xy.x;
 let word=shadow_damage_mask[tile>>7u][(tile>>5u)&3u];
 if (word&(1u<<(tile&31u)))==0u {discard;}
 if !kept_sun(in) {discard;}
}

// Compute raster for small discs (KERN_SPLAT_COMPUTE=1). Portable: no 64-bit
// atomics. Pass 1 keeps each pixel's nearest disc depth (reversed Z: the
// largest float, whose bits order like u32) by atomicMax, pass 2 writes the
// id of a disc at that depth (any one of equal depth), and a full-screen
// pass merges both into the raster targets with the depth test, as if the
// quads had drawn them. Coverage, dither and depth match the quads: a
// constant-depth disc at its centre, kept() for a disc under 1.5 pixels.
@group(1) @binding(0) var<storage, read> stats: array<u32>;
@group(1) @binding(1) var<storage, read_write> nearest: array<atomic<u32>>;
@group(1) @binding(2) var<storage, read_write> winner: array<u32>;
@group(1) @binding(3) var scene_depth: texture_depth_2d;
@group(1) @binding(4) var<storage, read_write> dispatch: array<u32>;
// Pass 1 or 2 of brick culling: its instance count word in volume_stats
// (voxel_forest.wgsl VOLUME_DRAW_WORD+1, VOLUME_CULL_WORD+1).
override COUNT_WORD: u32 = 23u;
@compute @workgroup_size(1) fn voxel_splat_compute_args() {
    let groups = (stats[COUNT_WORD] + 63u) / 64u;
    dispatch[0] = min(groups, 65535u); dispatch[1] = (groups + 65534u) / 65535u; dispatch[2] = 1u;
}
fn compute_splat(group: vec3<u32>, lane: u32, resolve: bool) {
    let instance = (group.x + group.y * 65535u) * 64u + lane;
    if instance >= stats[COUNT_WORD] { return; }
    let first = HEADER + splats[11].x * 4u;
    let splat = instance + splats[11].z;
    let limit = select(arrayLength(&splats), splats[11].w, splats[11].w != 0u);
    if first + splat >= limit { return; }
    let d = disc(splats[first + splat]);
    let vp = view_proj();
    let c = vp * vec4<f32>(d.centre, 1.0);
    if c.w <= 1e-4 { return; }
    let r = radius_px(d, c.w);
    if r >= SMALL_PX { return; }
    let z = c.z / c.w;
    if z <= 0.0 || z > 1.0 { return; }
    let size = textureDimensions(scene_depth);
    let centre = (c.xy / c.w * vec2<f32>(0.5, -0.5) + 0.5) * vec2<f32>(size);
    let lo = vec2<i32>(floor(centre - r));
    let hi = vec2<i32>(ceil(centre + r));
    let bits = bitcast<u32>(z);
    for (var y = max(lo.y, 0); y < min(hi.y, i32(size.y)); y++) {
        for (var x = max(lo.x, 0); x < min(hi.x, i32(size.x)); x++) {
            let q = (vec2<f32>(f32(x), f32(y)) + 0.5 - centre) / max(r, 1e-6);
            if dot(q, q) > 1.0 { continue; }
            // kept() for a disc under 1.5 px: every pixel at 0.76 x alpha.
            let p = vec2<u32>(u32(x), u32(y));
            if f32(hash(p.x ^ hash(p.y ^ hash(d.key))) >> 8u) / 16777216.0 >= d.alpha * 0.76 { continue; }
            if textureLoad(scene_depth, vec2<i32>(x, y), 0) >= z { continue; }
            let at = u32(y) * size.x + u32(x);
            if resolve {
                if atomicLoad(&nearest[at]) == bits { winner[at] = splat + 1u; }
            } else {
                atomicMax(&nearest[at], bits);
            }
        }
    }
}
@compute @workgroup_size(64) fn voxel_splat_compute_depth(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) lane: u32) { compute_splat(group, lane, false); }
@compute @workgroup_size(64) fn voxel_splat_compute_id(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) lane: u32) { compute_splat(group, lane, true); }
@group(0) @binding(1) var<storage, read> merge_depth: array<u32>;
@group(0) @binding(2) var<storage, read> merge_id: array<u32>;
struct MergeOut { @builtin(position) position: vec4<f32> };
@vertex fn vs_merge(@builtin(vertex_index) vertex: u32) -> MergeOut {
    let uv = vec2<f32>(f32((vertex << 1u) & 2u), f32(vertex & 2u));
    return MergeOut(vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0));
}
@group(0) @binding(3) var<uniform> merge_size: vec4<u32>;
fn merged(position: vec4<f32>) -> vec2<u32> {
    let at = u32(position.y) * merge_size.x + u32(position.x);
    return vec2<u32>(merge_depth[at], merge_id[at]);
}
struct MergeMain { @location(0) visibility: u32, @location(1) splat: u32, @builtin(frag_depth) depth: f32 };
@fragment fn fs_merge_main(in: MergeOut) -> MergeMain {
    let m = merged(in.position);
    if m.x == 0u || m.y == 0u { discard; }
    return MergeMain(0x80000000u | m.y, m.y, bitcast<f32>(m.x));
}
struct MergeId { @location(0) splat: u32, @builtin(frag_depth) depth: f32 };
@fragment fn fs_merge(in: MergeOut) -> MergeId {
    let m = merged(in.position);
    if m.x == 0u || m.y == 0u { discard; }
    return MergeId(m.y, bitcast<f32>(m.x));
}
