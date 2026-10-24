// Cluster culling and LOD selection, one workgroup per instance.
// A cluster is drawn when its own error projects to at most the threshold
// and its group's simplified replacement would project above it.
override CLUSTER_BUCKETS: bool = true;
override EARLY_GRASS_CULL: bool = true;

struct View {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    cam: vec4<f32>,          // position; w: pixels per unit error at distance 1 (or per metre, ortho)
    planes: array<vec4<f32>, 6>,
    params: vec4<f32>,       // threshold px, znear, ortho, max visible
    size: vec4<f32>,
    extra: vec4<f32>,        // x: alpha test threshold
    grass: vec4<f32>,        // source-grass fade start/end, shared with residency
    fog_cull: vec4<f32>,     // full-fog distance, camera-colour enable, reserved
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
    flags: u32,
};

struct CullMaterial {
    tint: vec3<f32>,
    layer: i32,
    roughness: f32,
    flags: u32,
    scale: f32,
    metal: f32,
};

@group(0) @binding(3) var<storage, read> clusters: array<Cluster>;
@group(0) @binding(4) var<storage, read> instances: array<Instance>;
@group(0) @binding(5) var<storage, read> cull_materials: array<CullMaterial>;
@group(0) @binding(12) var<storage, read> cull_wind: array<vec4<f32>>;

@group(1) @binding(0) var<uniform> view: View;
@group(1) @binding(1) var<storage, read_write> visible: array<vec2<u32>>;
// 0: candidate count, 1: retained triangles, 2: submitted vertices,
// 3: early grass rejects; 4..7: bucket counts, 8..11: scatter cursors.
@group(1) @binding(2) var<storage, read_write> counter: array<atomic<u32>, 12>;
// Four draw records followed by four bucket starts.
@group(1) @binding(3) var<storage, read_write> args: array<u32, 20>;
@group(1) @binding(4) var<storage, read_write> draw_map: array<u32>;

fn cluster_bucket(triangles: u32) -> u32 {
    if triangles <= 8u { return 0u; }
    if triangles <= 32u { return 1u; }
    if triangles <= 64u { return 2u; }
    return 3u;
}
fn bucket_vertices(bucket: u32) -> u32 {
    return array<u32, 4>(24u, 96u, 192u, 384u)[bucket];
}

fn in_frustum(c: vec3<f32>, r: f32) -> bool {
    for (var i = 0u; i < 6u; i++) {
        if dot(view.planes[i].xyz, c) + view.planes[i].w < -r {
            return false;
        }
    }
    return true;
}

fn hidden_by_camera_fog(c: vec3<f32>, r: f32) -> bool {
    // Only the colour view opts in. Shadow/GI/reflection caster lists stay
    // independent, including trees completely hidden from this camera.
    if view.fog_cull.y < 0.5 || view.params.z > 0.5 { return false; }
    return distance(c, view.cam.xyz) > view.fog_cull.x + max(r, 0.0);
}

fn to_world(inst: Instance, p: vec3<f32>) -> vec3<f32> {
    let q = vec4<f32>(p, 1.0);
    return vec3<f32>(dot(inst.r0, q), dot(inst.r1, q), dot(inst.r2, q));
}

// Screen-space size in pixels of `err` metres at the sphere's nearest point.
fn projected(err: f32, c: vec3<f32>, r: f32) -> f32 {
    if view.params.z > 0.5 {
        return err * view.cam.w;
    }
    let d = max(distance(c, view.cam.xyz) - r, view.params.y);
    return err / d * view.cam.w;
}

var<workgroup> keep: bool;

@compute @workgroup_size(64)
fn cull(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let id = wg.x + wg.y * 65535u;
    let n = arrayLength(&instances);
    if id >= n {
        return;
    }
    let inst = instances[id];
    // Unoccupied fixed-pool slots have no geometry. This is uniform across
    // the workgroup, so skip the frustum test/barrier without divergence.
    if inst.cluster_count == 0u {
        return;
    }
    // The orthographic pass is the sun shadow map. Low ground cover only
    // receives shadows; exclude its tiny cards from this expensive pass.
    if (view.params.z > 0.5 || view.extra.y > 0.5) && (inst.flags & 4u) != 0u {
        return;
    }
    // Exactly the same root, camera and opacity decision as raster.vs.
    // Transition grass still renders in its own pass; no geometry is removed
    // from that pass and no threshold, alpha or source plant is changed.
    if EARLY_GRASS_CULL && (inst.flags & 8u) != 0u {
        let root = vec3<f32>(inst.r0.w, inst.r1.w, inst.r2.w);
        if grass_card_opacity(root, view.cam.xyz, view.grass.xy, view.extra.w) < GRASS_OPAQUE_THRESHOLD {
            if li == 0u { atomicAdd(&counter[3], 1u); }
            return;
        }
    }
    let wind_pad = cull_wind[id].z;
    if li == 0u {
        let radius = inst.sphere.w + wind_pad;
        keep = in_frustum(inst.sphere.xyz, radius) && !hidden_by_camera_fog(inst.sphere.xyz, radius);
    }
    workgroupBarrier();
    if !keep {
        return;
    }
    // Keep car body geometry consistent in the visible and sun passes. A
    // coarse shadow mesh can cut across its own finer visible panels.
    let thr = select(view.params.x, 0.0, (inst.flags & 2u) != 0u);
    let max_visible = u32(view.params.w);
    for (var c = li; c < inst.cluster_count; c += 64u) {
        let ci = inst.first_cluster + c;
        let cl = clusters[ci];
        // Continuous-alpha surfaces are drawn separately after opaque HDR.
        // Excluding them here also prevents opaque glass sun shadows.
        if (cull_materials[cl.material].flags & 256u) != 0u { continue; }
        let s = inst.scale;
        // Shear can stretch simplification error as well as move bounds.
        // ||I + shear|| <= 1 + |shear|; use the identical factor on both
        // sides of every parent/child cut so the DAG stays complete.
        let error_scale = s * (1.0 + length(cull_wind[id].xy));
        let pc = to_world(inst, cl.parent.xyz);
        let parent_px = select(projected(cl.parent_error * error_scale, pc, cl.parent.w * s + wind_pad), 1e30, cl.parent_error > 1e30);
        if parent_px <= thr {
            continue; // a coarser cluster covers this patch
        }
        // Every cluster, the DAG's roots included, must itself be fine
        // enough; where it is not, its finer clusters are drawn instead.
        let oc = to_world(inst, cl.own.xyz);
        if projected(cl.own_error * error_scale, oc, cl.own.w * s + wind_pad) > thr {
            continue;
        }
        let cc = to_world(inst, cl.cull.xyz);
        let radius = cl.cull.w * s + wind_pad;
        if !in_frustum(cc, radius) || hidden_by_camera_fog(cc, radius) {
            continue;
        }
        let slot = atomicAdd(&counter[0], 1u);
        if slot < max_visible {
            visible[slot] = vec2<u32>(id, ci);
            atomicAdd(&counter[1], cl.tri_count);
            if CLUSTER_BUCKETS { atomicAdd(&counter[4u + cluster_bucket(cl.tri_count)], 1u); }
        }
    }
}

// Keep visible[] and its IDs unchanged. Only draw order and padded vertex
// count change. first_vertex encodes the bucket, avoiding any dependency on
// the optional INDIRECT_FIRST_INSTANCE device feature.
@compute @workgroup_size(1)
fn finalize() {
    let n = min(atomicLoad(&counter[0]), u32(view.params.w));
    if !CLUSTER_BUCKETS {
        args[0] = 384u;
        args[1] = n;
        args[2] = 0u;
        args[3] = 0u;
        atomicStore(&counter[2], n * 384u);
        return;
    }
    var start = 0u;
    var submitted = 0u;
    for (var bucket = 0u; bucket < 4u; bucket++) {
        let count = atomicLoad(&counter[4u + bucket]);
        let vertices = bucket_vertices(bucket);
        args[bucket * 4u] = vertices;
        args[bucket * 4u + 1u] = count;
        args[bucket * 4u + 2u] = bucket * 384u;
        args[bucket * 4u + 3u] = 0u;
        args[16u + bucket] = start;
        start += count;
        submitted += count * vertices;
    }
    atomicStore(&counter[2], submitted);
}

@compute @workgroup_size(64)
fn scatter(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let n = min(atomicLoad(&counter[0]), u32(view.params.w));
    for (var slot = gid.x; slot < n; slot += groups.x * 64u) {
        let bucket = cluster_bucket(clusters[visible[slot].y].tri_count);
        let offset = atomicAdd(&counter[8u + bucket], 1u);
        draw_map[args[16u + bucket] + offset] = slot;
    }
}
