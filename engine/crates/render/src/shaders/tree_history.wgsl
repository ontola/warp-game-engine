// Only pixels surviving the actual alpha/depth test seed the next frame.
// Owner IDs may be reused: that merely schedules their NEW geometry first,
// never trusts stale depth or omits a newly visible owner.
@group(0) @binding(0) var visibility:texture_2d<u32>;
@group(0) @binding(1) var<storage,read> records:array<vec2<u32>>;
@group(0) @binding(2) var<storage,read_write> history:array<atomic<u32>>;
// Per slot the smallest history word any lane wants there (EMPTY when none),
// and the bits gathered for it. A min, not a compare-exchange: Safari 27
// turns atomicCompareExchangeWeak on workgroup memory into Metal its own
// compiler rejects, which left the whole browser page black.
const EMPTY:u32=0xffffffffu;
var<workgroup> keys_local:array<atomic<u32>,16>;
var<workgroup> masks:array<atomic<u32>,16>;
@compute @workgroup_size(8,8)
fn gather(@builtin(global_invocation_id) id:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
 // Persistent policy words are outside the cleared 32 KiB history payload.
 if all(id==vec3<u32>(0u)) {atomicAdd(&history[8192],1u);}
 if lane<16u {atomicStore(&keys_local[lane],EMPTY);atomicStore(&masks[lane],0u);}
 workgroupBarrier();
 var words=vec2<u32>(EMPTY);var bits=vec2<u32>(0u);
 if all(id.xy<textureDimensions(visibility)) {
  let pixel=textureLoad(visibility,vec2<i32>(id.xy),0).x;
  if pixel!=0xffffffffu && (pixel>>7u)<arrayLength(&records) {
   let record=records[pixel>>7u];let keys=tree_history_keys(record.x,record.y);
   for(var k=0u;k<2u;k++) {
    let index=keys[k];words[k]=index>>5u;bits[k]=1u<<(index&31u);
    atomicMin(&keys_local[words[k]&15u],words[k]);
   }
  }
 }
 workgroupBarrier();
 // Bloom collisions only schedule extra current geometry in phase one.
 // They can never reject a record or omit phase-two recovery.
 for(var k=0u;k<2u;k++) {
  let word=words[k];
  if word!=EMPTY {
   let slot=word&15u;
   if atomicLoad(&keys_local[slot])==word {atomicOr(&masks[slot],bits[k]);}
   else {atomicOr(&history[word],bits[k]);}
  }
 }
 workgroupBarrier();
 if lane<16u {
  let mask=atomicLoad(&masks[lane]);
  if mask!=0u {atomicOr(&history[atomicLoad(&keys_local[lane])],mask);}
 }
}
