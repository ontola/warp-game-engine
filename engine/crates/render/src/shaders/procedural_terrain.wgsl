// Builds terrain patches from terrain_field.wgsl into a reserved slice of
// the scene's vertex and cluster buffers. One workgroup per new patch: an
// 8x8-cell grid (81 vertices, one 128-triangle cluster) plus a one-cell
// border for normals. Edges next to a coarser patch sample that patch's
// detail and put odd vertices on its straight edge, so there are no cracks.

struct Params {
    first_vertex: u32,
    first_cluster: u32,
    count: u32,
    seed: u32,
    style: TerrainStyle,
};

struct Job {
    origin: vec2<f32>,
    spacing: f32,
    // Bits 0..3: west, east, north, south edge borders a coarser patch.
    // Bits 4..7: corners (x0z0, x1z0, x0z1, x1z1) touch a coarser patch.
    mask: u32,
    slot: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> jobs: array<Job>;
@group(0) @binding(2) var<storage, read_write> vertices: array<f32>;
@group(0) @binding(3) var<storage, read_write> clusters: array<u32>;

const N: i32 = 8;
const B: i32 = 11; // with border

var<workgroup> pos: array<vec3<f32>, 121>;
// (rock, scree, wet, up) and (water, moor, snow) per point.
var<workgroup> attr: array<vec4<f32>, 121>;
var<workgroup> attr2: array<vec3<f32>, 121>;

fn cell(i: i32, j: i32) -> i32 { return (j + 1) * B + (i + 1); }

// Cut (sampling spacing) for grid point (i, j) of this patch.
fn cut_at(i: i32, j: i32, s: f32, mask: u32) -> f32 {
    var coarse = false;
    if (i == 0 || i == N) && (j == 0 || j == N) {
        let bit = 4u + select(0u, 1u, i == N) + select(0u, 2u, j == N);
        coarse = (mask & (1u << bit)) != 0u;
    } else {
        coarse = (i == 0 && j > 0 && j < N && (mask & 1u) != 0u)
            || (i == N && j > 0 && j < N && (mask & 2u) != 0u)
            || (j == 0 && i > 0 && i < N && (mask & 4u) != 0u)
            || (j == N && i > 0 && i < N && (mask & 8u) != 0u);
    }
    return select(s, 2.0 * s, coarse);
}

@compute @workgroup_size(128)
fn build(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let job_index = wid.x + wid.y * 65535u;
    if job_index >= params.count { return; }
    let job = jobs[job_index];
    let s = job.spacing;
    if t < 121u {
        let i = i32(t % 11u) - 1;
        let j = i32(t / 11u) - 1;
        // Exact in f32: power-of-two spacing, origin a multiple of the patch.
        let x = job.origin.x + f32(i) * s;
        let z = job.origin.y + f32(j) * s;
        let e = terrain_eval_world(x, z, cut_at(i, j, s, job.mask), params.seed, params.style);
        pos[t] = e.pos;
        attr[t] = vec4<f32>(e.rock, e.scree, e.wet, e.up);
        // Water too narrow for this patch's cells to carve stays as a thin
        // film over its share of them (terrain_surface.wgsl WATER_FILM).
        attr2[t] = vec3<f32>(max(e.water, 0.04 * e.film), e.moor, e.snow);
    }
    workgroupBarrier();
    // Odd vertices on a coarser edge sit on that edge's straight segment.
    if t < 81u {
        let i = i32(t % 9u);
        let j = i32(t / 9u);
        let odd_w = i == 0 && (j & 1) == 1 && (job.mask & 1u) != 0u;
        let odd_e = i == N && (j & 1) == 1 && (job.mask & 2u) != 0u;
        let odd_n = j == 0 && (i & 1) == 1 && (job.mask & 4u) != 0u;
        let odd_s = j == N && (i & 1) == 1 && (job.mask & 8u) != 0u;
        if odd_w || odd_e {
            let a = cell(i, j - 1);
            let b = cell(i, j + 1);
            pos[cell(i, j)] = 0.5 * (pos[a] + pos[b]);
            attr[cell(i, j)] = 0.5 * (attr[a] + attr[b]);
            attr2[cell(i, j)] = 0.5 * (attr2[a] + attr2[b]);
        } else if odd_n || odd_s {
            let a = cell(i - 1, j);
            let b = cell(i + 1, j);
            pos[cell(i, j)] = 0.5 * (pos[a] + pos[b]);
            attr[cell(i, j)] = 0.5 * (attr[a] + attr[b]);
            attr2[cell(i, j)] = 0.5 * (attr2[a] + attr2[b]);
        }
    }
    workgroupBarrier();
    let slot = job.slot;
    if t < 81u {
        let i = i32(t % 9u);
        let j = i32(t / 9u);
        let c = cell(i, j);
        let n = normalize(cross(pos[cell(i, j + 1)] - pos[cell(i, j - 1)], pos[cell(i + 1, j)] - pos[cell(i - 1, j)]));
        let v = (params.first_vertex + slot * 81u + t) * 8u;
        let p = pos[c];
        vertices[v] = p.x;
        vertices[v + 1u] = p.y;
        vertices[v + 2u] = p.z;
        vertices[v + 3u] = n.x;
        vertices[v + 4u] = n.y;
        vertices[v + 5u] = n.z;
        // 24 bits per uv channel, exact integers in f32: rock, scree, wet
        // at 8 bits; up at 8, water at 6 (square root of depth / 2.55 m,
        // fine steps for thin films), moor and snow at 5. The surface
        // shader unpacks them per corner before blending.
        let a = clamp(attr[c], vec4<f32>(0.0), vec4<f32>(1.0));
        let w = vec3<f32>(clamp(attr2[c].x / 2.55, 0.0, 1.0), clamp(attr2[c].y, 0.0, 1.0), clamp(attr2[c].z, 0.0, 1.0));
        vertices[v + 6u] = round(a.x * 255.0) + round(a.y * 255.0) * 256.0 + round(a.z * 255.0) * 65536.0;
        vertices[v + 7u] = round(a.w * 255.0) + round(sqrt(w.x) * 63.0) * 256.0 + round(w.y * 31.0) * 16384.0 + round(w.z * 31.0) * 524288.0;
    }
    if t == 0u {
        var lo = pos[cell(0, 0)];
        var hi = lo;
        for (var k = 0; k < 81; k++) {
            let p = pos[cell(k % 9, k / 9)];
            lo = min(lo, p);
            hi = max(hi, p);
        }
        let centre = 0.5 * (lo + hi);
        let radius = length(hi - lo) * 0.5 * 1.0001 + 1e-3;
        let w = (params.first_cluster + slot) * 20u;
        for (var k = 0u; k < 3u; k++) {
            clusters[w + k * 4u] = bitcast<u32>(centre.x);
            clusters[w + k * 4u + 1u] = bitcast<u32>(centre.y);
            clusters[w + k * 4u + 2u] = bitcast<u32>(centre.z);
            clusters[w + k * 4u + 3u] = bitcast<u32>(radius);
        }
    }
}
