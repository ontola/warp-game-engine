// Only pixels surviving the actual alpha/depth test seed the next frame.
// Owner IDs may be reused: that merely schedules their NEW geometry first,
// never trusts stale depth or omits a newly visible owner.
@group(0) @binding(0) var visibility:texture_2d<u32>;
@group(0) @binding(1) var<storage,read> records:array<vec2<u32>>;
@group(0) @binding(2) var<storage,read_write> history:array<atomic<u32>>;
var<workgroup> keys_local:array<atomic<u32>,16>;
var<workgroup> masks:array<atomic<u32>,16>;
@compute @workgroup_size(8,8)
fn gather(@builtin(global_invocation_id) id:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
 // Persistent policy words are outside the cleared 32 KiB history payload.
 if all(id==vec3<u32>(0u)) {atomicAdd(&history[8192],1u);}
 if lane<16u {atomicStore(&keys_local[lane],0u);atomicStore(&masks[lane],0u);}
 workgroupBarrier();
 if all(id.xy<textureDimensions(visibility)) {
  let pixel=textureLoad(visibility,vec2<i32>(id.xy),0).x;
  if pixel!=0xffffffffu && (pixel>>7u)<arrayLength(&records) {
   let record=records[pixel>>7u];let keys=tree_history_keys(record.x,record.y);
   for(var k=0u;k<2u;k++) {
    let index=keys[k];let word=index>>5u;let bit=1u<<(index&31u);
    // Bloom collisions only schedule extra current geometry in phase one.
    // They can never reject a record or omit phase-two recovery.
    let slot=word&15u;let claim=atomicCompareExchangeWeak(&keys_local[slot],0u,word+1u);
    if claim.exchanged || claim.old_value==word+1u {atomicOr(&masks[slot],bit);}
    else {atomicOr(&history[word],bit);}
   }
  }
 }
 workgroupBarrier();
 if lane<16u {
  let mask=atomicLoad(&masks[lane]);
  if mask!=0u {atomicOr(&history[atomicLoad(&keys_local[lane])-1u],mask);}
 }
}
