// GPU-owned transient branch residency. Dispatch boundaries publish the hash,
// retained slots, free list, then allocations. No spin waits or CPU shortlist.
// offsets: x first vec4 of the record consumer masks in `selected` (foliage_select.rs
// Region::selected), y first word of the free list in `slot_lists`.
struct Config { counts:vec4<u32>, limits:vec4<u32>, offsets:vec4<u32> };
struct Slot { key:vec4<u32>, history:vec4<u32> };
@group(0) @binding(0) var<uniform> config:Config;
// The selector's records (vec4 each, from 0) and their consumer masks, one binding.
@group(0) @binding(1) var<storage,read> selected:array<vec4<u32>>;
fn request_at(i:u32)->vec4<u32> {return selected[i];}
fn request_consumers(i:u32)->u32 {return selected[config.offsets.x+i/4u][i%4u];}
@group(0) @binding(2) var<storage,read> selection_counts:array<u32>;
@group(0) @binding(3) var<storage,read_write> fallback:array<atomic<u32>>;
@group(0) @binding(4) var<storage,read_write> slots:array<Slot>;
@group(0) @binding(5) var<storage,read_write> table:array<atomic<u32>>;
// Each request's slot (from 0) and the free list (from config.offsets.y), one binding.
@group(0) @binding(6) var<storage,read_write> slot_lists:array<u32>;
// free count, allocation attempts, retained, newly assigned, capacity failures,
// hash failures, occupied after allocation, reserved.
@group(0) @binding(8) var<storage,read_write> stats:array<atomic<u32>>;
@group(0) @binding(9) var<storage,read> asset_pages:array<vec4<u32>>;
@group(0) @binding(11) var<storage,read_write> consumer_union:array<atomic<u32>>;
fn available(root:u32)->u32 {
 let failure=atomicLoad(&fallback[root]);return select(768u&~failure,0u,(failure&7u)!=0u);
}
fn consumers(i:u32)->u32 {let roles=request_consumers(i)&768u;return select(roles,768u,roles==0u)&available(request_at(i).x);}
const NONE:u32=0xffffffffu;
fn count()->u32 {return min(selection_counts[0],config.counts.x);}
fn same(a:vec4<u32>,b:vec4<u32>)->bool {
    // Representation is payload. Node ID and generation define identity.
    return a.x==b.x && a.y==b.y && a.w==b.w;
}
fn hash(k:vec4<u32>)->u32 {
    var h=(k.x*0x9e3779b9u) ^ (k.y*0x85ebca6bu) ^ (k.w*0xc2b2ae35u);
    h=(h^(h>>16u))*0x7feb352du;return (h^(h>>15u))&(config.counts.z-1u);
}
fn lookup(key:vec4<u32>)->u32 {
    let start=hash(key);
    for(var probe=0u;probe<config.limits.x;probe++) {
        let entry=atomicLoad(&table[(start+probe)&(config.counts.z-1u)]);
        if entry==0u {return NONE;}
        if same(request_at(entry-1u),key) {return entry-1u;}
    }
    return NONE;
}
@compute @workgroup_size(64)
fn index_requests(@builtin(global_invocation_id) id:vec3<u32>) {
    let i=id.x;if i>=count(){return;}
    slot_lists[i]=NONE;
    let key=request_at(i);
    if key.x>=config.counts.w {return;}
    if consumers(i)==0u {return;}
    let start=hash(key);
    for(var probe=0u;probe<config.limits.x;probe++) {
        let cell=(start+probe)&(config.counts.z-1u);
        // Atomic max/CAS-weak requires retrying spurious failure. atomicMin
        // cannot claim empty zero; atomicCompareExchangeWeak is retried only
        // while the cell remains empty, never waiting for another invocation.
        var entry=atomicLoad(&table[cell]);
        loop {
            if entry!=0u {break;}
            let claim=atomicCompareExchangeWeak(&table[cell],0u,i+1u);
            if claim.exchanged {atomicOr(&consumer_union[i],consumers(i));return;}
            entry=claim.old_value;
        }
        if same(request_at(entry-1u),key) {atomicOr(&consumer_union[entry-1u],consumers(i));return;}
    }
    atomicOr(&fallback[key.x],1u);atomicAdd(&stats[5],1u);
}
@compute @workgroup_size(64)
fn retain_slots(@builtin(global_invocation_id) id:vec3<u32>) {
    let i=id.x;if i>=config.counts.y {return;}
    var slot=slots[i];var request=NONE;
    if slot.history.x!=0u && slot.key.x<config.counts.w && slot.history.x==asset_pages[slot.key.z].z {
        if available(slot.key.x)!=0u {request=lookup(slot.key);}
    }
    if request!=NONE {
        slot_lists[request]=i;slot.key=request_at(request);
        // history.y is valid only if this slot was published last frame.
        slot.history.y=slot.history.z;
        let added=atomicLoad(&consumer_union[request])&~slot.history.w&768u;
        if added!=0u && slot.history.z!=0u {slot.history.y=0u;slot.history.w=slot.history.w^128u;}
        // Packed virtual-member motion also carries this epoch in flags.
        // Waking a retained but unpublished record must reject that history.
        if slot.history.z==0u {slot.history.w=slot.history.w^128u;}
        slot.history.z=0u;
        slots[i]=slot;atomicAdd(&stats[2],1u);return;
    }
    slot.history.x=0u;slot.history.y=0u;slot.history.z=0u;slots[i]=slot;
    let offset=atomicAdd(&stats[0],1u);slot_lists[config.offsets.y+offset]=i;
}
@compute @workgroup_size(64)
fn allocate_slots(@builtin(global_invocation_id) id:vec3<u32>) {
    let i=id.x;if i>=count(){return;}
    let key=request_at(i);if key.x>=config.counts.w {return;}
    if consumers(i)==0u || lookup(key)!=i {return;}
    if slot_lists[i]!=NONE {return;}
    let offset=atomicAdd(&stats[1],1u);
    if offset>=atomicLoad(&stats[0]) {
        atomicOr(&fallback[key.x],1u);atomicAdd(&stats[4],1u);return;
    }
    let index=slot_lists[config.offsets.y+offset];let old_epoch=slots[index].history.w&128u;
    slots[index]=Slot(key,vec4<u32>(asset_pages[key.z].z,0u,0u,old_epoch^128u));
    slot_lists[i]=index;atomicAdd(&stats[3],1u);
}
// Only this final pass publishes slot validity. A failed request elsewhere in
// the same tree invalidates ALL its partial branches, including retained ones.
@compute @workgroup_size(64)
fn publish_slots(@builtin(global_invocation_id) id:vec3<u32>) {
    let i=id.x;if i>=config.counts.y {return;}
    var slot=slots[i];
    let published=slot.history.x!=0u && slot.key.x<config.counts.w;
    if published {
        let request=lookup(slot.key);
        var roles=0u;if request!=NONE {roles=atomicLoad(&consumer_union[request])&available(slot.key.x);}
        slot.history.z=u32(roles!=0u);
        slot.history.w=(slot.history.w&128u)|roles;
        if slot.history.z==0u {slot.history.y=0u;}
        atomicAdd(&stats[6],slot.history.z);
    }
    slots[i]=slot;
}
@compute @workgroup_size(64)
fn resolve_requests(@builtin(global_invocation_id) id:vec3<u32>) {
    let i=id.x;if i>=count(){return;}
    let key=request_at(i);
    if key.x>=config.counts.w {slot_lists[i]=NONE;return;}
    if consumers(i)==0u {slot_lists[i]=NONE;return;}
    // Duplicate requests intentionally retain NONE: only the canonical request
    // emits a draw. Reading another mapping here would race its publication.
}
