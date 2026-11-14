// Body of volume_splat_append_packet on subgroup-capable devices.
// All lanes participate, including rejected cells and padding in the last packet.
let keep = u32(record.w != 0u);
let count = subgroupAdd(keep);
let rank = subgroupExclusiveAdd(keep);
let first_lane = subgroupBroadcastFirst(lane);
var base = 0u;
if count > 0u && lane == first_lane {
    base = atomicAdd(&volume_stats[VOLUME_DRAW_WORD + 1u], count);
}
base = subgroupBroadcastFirst(base);
if keep != 0u {
    volume_splat_store(record, base + rank);
}
