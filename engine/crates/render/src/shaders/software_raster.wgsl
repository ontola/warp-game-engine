// Software rasterizer for clusters whose triangles are about a pixel or
// smaller (Nanite's split): hardware rasterizers shade 2x2 quads and pay a
// fixed setup cost per triangle, so pixel-sized geometry wastes most of
// their work. One workgroup per cluster, one thread per triangle. Every
// covered pixel centre does one 64-bit atomicMax of (reversed-Z depth bits,
// visibility id); `software_merge` then folds the winners into the ordinary
// visibility and depth targets with the hardware depth test, so shading,
// motion and picking see one visibility buffer. Requires
// SHADER_INT64 + SHADER_INT64_ATOMIC_MIN_MAX; the cull pass only routes
// clusters here when the renderer enabled this path.

struct View {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    cam: vec4<f32>,
    planes: array<vec4<f32>, 6>,
    params: vec4<f32>,
    size: vec4<f32>,
    extra: vec4<f32>,
    grass: vec4<f32>,
    fog_cull: vec4<f32>,
};

struct Cluster {
    cull: vec4<f32>,
    parent: vec4<f32>,
    own: vec4<f32>,
    parent_error: f32,
    own_error: f32,
    first_tri: u32,
    tri_count: u32,
    first_vert: u32,
    material: u32,
    depth: u32,
    pad: u32,
};

struct Instance {
    r0: vec4<f32>,
    r1: vec4<f32>,
    r2: vec4<f32>,
    sphere: vec4<f32>,
    first_cluster: u32,
    cluster_count: u32,
    scale: f32,
    pad: u32,
};

@group(0) @binding(0) var<storage, read> vertices: array<f32>;
@group(0) @binding(1) var<storage, read> cluster_verts: array<u32>;
@group(0) @binding(2) var<storage, read> tris: array<u32>;
@group(0) @binding(3) var<storage, read> clusters: array<Cluster>;
@group(0) @binding(4) var<storage, read> instances: array<Instance>;

@group(1) @binding(0) var<uniform> view: View;
@group(1) @binding(1) var<storage, read> visible: array<vec2<u32>>;
@group(1) @binding(2) var<storage, read> draw_map: array<u32>;
// 0..19 hardware draws and bucket starts; 20..22 this dispatch; 23 first
// software draw_map entry; 24 software cluster count.
@group(1) @binding(3) var<storage, read> draw_args: array<u32, 25>;
@group(1) @binding(4) var<storage, read_write> vis64: array<atomic<u64>>;

// Larger bounding boxes than this are not this path's job; the cull
// predicate keeps whole clusters far below it. Never loop unbounded.
const MAX_BOX_PIXELS: u32 = 4096u;

fn edge(a: vec2<f32>, b: vec2<f32>, p: vec2<f32>) -> f32 {
    return (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
}

@compute @workgroup_size(128)
fn software_raster(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let k = wg.x + wg.y * 65535u;
    if k >= draw_args[24] { return; }
    let slot = draw_map[draw_args[23] + k];
    let vis = visible[slot];
    let cl = clusters[vis.y];
    if li >= cl.tri_count { return; }
    let inst = instances[vis.x];
    let packed = tris[cl.first_tri + li];
    let size = view.size.xy;
    var s: array<vec3<f32>, 3>;
    for (var c = 0u; c < 3u; c++) {
        let local = (packed >> (8u * c)) & 0xffu;
        let v = cluster_verts[cl.first_vert + local] * 8u;
        let p = vec4<f32>(vertices[v], vertices[v + 1u], vertices[v + 2u], 1.0);
        let rest = vec3<f32>(dot(inst.r0, p), dot(inst.r1, p), dot(inst.r2, p));
        let clip = view.view_proj * vec4<f32>(wind_position(vis.x, inst.r1.w, rest), 1.0);
        // The cull predicate keeps software clusters well in front of the
        // near plane; anything else is dropped rather than clipped.
        if clip.w <= 1e-6 { return; }
        let ndc = clip.xyz / clip.w;
        s[c] = vec3<f32>((ndc.x * 0.5 + 0.5) * size.x, (0.5 - ndc.y * 0.5) * size.y, ndc.z);
    }
    let a = s[0].xy; let b = s[1].xy; let c = s[2].xy;
    let area = edge(a, b, c);
    if abs(area) < 1e-12 { return; }
    // Two-sided, like the hardware pipeline (cull_mode None).
    let inv = 1.0 / area;
    let lo = max(floor(min(min(a, b), c) - 0.5), vec2<f32>(0.0));
    let hi = min(ceil(max(max(a, b), c) - 0.5), size - 1.0);
    if any(hi < lo) { return; }
    let box = vec2<u32>(hi - lo) + 1u;
    if box.x * box.y > MAX_BOX_PIXELS { return; }
    let id = (slot << 7u) | li;
    for (var y = u32(lo.y); y <= u32(hi.y); y++) {
        for (var x = u32(lo.x); x <= u32(hi.x); x++) {
            let p = vec2<f32>(f32(x) + 0.5, f32(y) + 0.5);
            let w0 = edge(b, c, p) * inv;
            let w1 = edge(c, a, p) * inv;
            let w2 = edge(a, b, p) * inv;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 { continue; }
            let z = w0 * s[0].z + w1 * s[1].z + w2 * s[2].z;
            // Reversed Z: nearer is larger; positive floats order like u32.
            if z <= 0.0 || z > 1.0 { continue; }
            let packed64 = (u64(bitcast<u32>(z)) << 32u) | u64(id);
            atomicMax(&vis64[y * u32(size.x) + x], packed64);
        }
    }
}
