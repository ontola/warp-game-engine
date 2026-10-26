// Stable bounded owner compaction. Selection appends this at group 3 and
// invokes compact_rank for every lane, including out-of-range/inactive lanes.
// The standalone finalization module binds these resources at group 0.
struct CompactParams { first: u32, count: u32, vertices: u32, blocks: u32 };
@group(0) @binding(0) var<uniform> compact_params: CompactParams;
@group(0) @binding(1) var<storage, read_write> compact_ranks: array<u32>;
@group(0) @binding(2) var<storage, read_write> compact_blocks: array<u32>;
@group(0) @binding(3) var<storage, read_write> compact_owners: array<u32>;
@group(0) @binding(4) var<storage, read_write> compact_args: array<u32, 4>;
var<workgroup> compact_scan: array<u32, 256>;
var<workgroup> compact_carry: u32;

fn compact_rank(lane: u32, value: u32) -> u32 {
    compact_scan[lane] = value;
    workgroupBarrier();
    for (var offset = 1u; offset < 256u; offset *= 2u) {
        var add = 0u;
        if lane >= offset { add = compact_scan[lane-offset]; }
        workgroupBarrier();
        compact_scan[lane] += add;
        workgroupBarrier();
    }
    return compact_scan[lane];
}

@compute @workgroup_size(256)
fn compact_offsets(@builtin(local_invocation_index) lane: u32) {
    if lane == 0u { compact_carry = 0u; }
    workgroupBarrier();
    // Scan block totals in bounded 256-element batches. The default split
    // grass pool needs two batches, not a serial loop over 115,000 owners.
    for (var start = 0u; start < compact_params.blocks; start += 256u) {
        let index = start + lane;
        var value = 0u;
        if index < compact_params.blocks { value = compact_blocks[index]; }
        let inclusive = compact_rank(lane, value);
        let carry = compact_carry;
        if index < compact_params.blocks { compact_blocks[index] = carry + inclusive - value; }
        workgroupBarrier();
        if lane == 255u { compact_carry = carry + inclusive; }
        workgroupBarrier();
    }
    if lane == 0u {
        compact_args[0] = compact_params.vertices;
        compact_args[1] = compact_carry;
        compact_args[2] = 0u;
        compact_args[3] = 0u;
    }
}

@compute @workgroup_size(256)
fn compact_scatter(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= compact_params.count { return; }
    let rank = compact_ranks[id.x];
    if rank == 0u { return; }
    let output_index = compact_blocks[id.x / 256u] + rank - 1u;
    // Every input contributes at most once; output capacity equals input count.
    // Keeping owner order also preserves equal-depth fragment tie-breaking.
    compact_owners[output_index] = compact_params.first + id.x;
}
