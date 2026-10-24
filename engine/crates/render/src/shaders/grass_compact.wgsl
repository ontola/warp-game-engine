// Appended to common shade/wind/grass declarations and foliage_compact.
struct GrassRangeUniform { indices: vec4<u32>, shape: vec4<f32> };
@group(2) @binding(0) var<uniform> grass_range: GrassRangeUniform;

fn grass_transition_candidate(index: u32) -> bool {
    if index >= compact_params.count { return false; }
    let owner = compact_params.first + index;
    let inst = instances[owner];
    if inst.cluster_count == 0u || (inst.pad & 8u) == 0u { return false; }
    let root = vec3<f32>(inst.r0.w, inst.r1.w, inst.r2.w);
    let opacity = grass_card_opacity(root, view.cam.xyz, view.grass.xy, view.extra.w);
    if opacity <= 0.0 || opacity >= GRASS_OPAQUE_THRESHOLD { return false; }
    // Same conservative sphere and current wind expansion as cluster culling.
    // Never test only the root: a leaning card can enter the edge of the view.
    let radius = inst.sphere.w + wind_coefficients[owner].z;
    for (var p = 0u; p < 6u; p++) {
        if dot(view.planes[p].xyz, inst.sphere.xyz) + view.planes[p].w < -radius { return false; }
    }
    return true;
}

@compute @workgroup_size(256)
fn grass_compact(@builtin(global_invocation_id) id: vec3<u32>,
    @builtin(local_invocation_index) lane: u32, @builtin(workgroup_id) block: vec3<u32>) {
    let keep = grass_transition_candidate(id.x);
    let rank = compact_rank(lane, select(0u,1u,keep));
    if id.x < compact_params.count { compact_ranks[id.x] = select(0u,rank,keep); }
    if lane == 255u { compact_blocks[block.x] = rank; }
}
