// Exact caster history, not a hash: collisions must not retain obsolete depth.
// 64x64 tiles on the 4096-square shared sun map. Both producers use world bounds.
struct Source {rows:array<vec4<f32>,3>,sphere:vec4<f32>,tail:vec4<u32>};
struct Volume {inverse:array<vec4<f32>,3>,low:vec4<f32>,high:vec4<f32>,identity:vec4<u32>};
struct State {vp:mat4x4<f32>,counts:vec4<u32>,control:vec4<u32>};
struct Snapshot {words:array<vec4<u32>,8>};
@group(0) @binding(0) var<uniform> state:State;
@group(0) @binding(1) var<storage,read> sources:array<Source>;
@group(0) @binding(2) var<storage,read> volumes:array<Volume>;
@group(0) @binding(3) var<storage,read> choices:array<u32>;
@group(0) @binding(4) var<storage,read_write> history:array<Snapshot>;
@group(0) @binding(5) var<storage,read_write> damage:array<atomic<u32>,128>;

@compute @workgroup_size(64) fn reset(@builtin(global_invocation_id) id:vec3<u32>) {
 if id.x<128u {
  var bits=0u;
  let shift=bitcast<vec2<i32>>(state.control.zw);let tile_size=i32(state.control.y/64u);
  for(var k=0u;k<32u;k++) {
   let tile=id.x*32u+k;let low=vec2<i32>(i32(tile%64u),i32(tile/64u))*tile_size-shift;
   let high=low+vec2<i32>(tile_size-1);
   if state.control.x!=0u || any(low<vec2<i32>(0)) || any(high>=vec2<i32>(i32(state.control.y))) {bits|=1u<<k;}
  }
  atomicStore(&damage[id.x],bits);
 }
}
fn all_dirty() {for(var i=0u;i<128u;i++){atomicStore(&damage[i],0xffffffffu);}}
fn mark(low:vec3<f32>,high:vec3<f32>) {
 var lo=vec2<f32>(1e30);var hi=vec2<f32>(-1e30);var zlo=1e30;var zhi=-1e30;
 for(var k=0u;k<8u;k++) {
  let p=vec3<f32>(select(low.x,high.x,(k&1u)!=0u),select(low.y,high.y,(k&2u)!=0u),select(low.z,high.z,(k&4u)!=0u));
  let q=state.vp*vec4<f32>(p,1.);
  // Unsupported/singular projection must invalidate, never retain stale pixels.
  if !(q.w>0.000001) {all_dirty();return;}
  let uv=vec2<f32>(q.x/q.w*.5+.5,.5-q.y/q.w*.5);
  lo=min(lo,uv);hi=max(hi,uv);zlo=min(zlo,q.z/q.w);zhi=max(zhi,q.z/q.w);
 }
 // Two texels cover raster footprint/rounding at tile boundaries. Receiver
 // filtering reads neighboring texels; those stay valid independently.
 let margin=vec2<f32>(2./f32(state.control.y));lo-=margin;hi+=margin;
 if any(hi<vec2<f32>(0.)) || any(lo>vec2<f32>(1.)) || zhi<0. || zlo>1. {return;}
 let a=vec2<u32>(clamp(floor(lo*64.),vec2<f32>(0.),vec2<f32>(63.)));
 let b=vec2<u32>(clamp(floor(hi*64.),vec2<f32>(0.),vec2<f32>(63.)));
 for(var y=a.y;y<=b.y;y++){for(var x=a.x;x<=b.x;x++) {
  let tile=y*64u+x;atomicOr(&damage[tile>>5u],1u<<(tile&31u));
 }}
}
fn mark_snapshot(s:Snapshot,voxel:bool) {
 if voxel {
  let lo=bitcast<vec4<f32>>(s.words[3]);let hi=bitcast<vec4<f32>>(s.words[4]);
  if lo.w==0. && s.words[5].z!=0u {mark(lo.xyz,hi.xyz);}
 }else {
  let bound=bitcast<vec4<f32>>(s.words[3]);
  // A virtual owner has its encoded member count in first_cluster. No source
  // payload (first_cluster=0) is also the GPU forest voxel replacement marker.
  if bound.w>=0. && (s.words[4].x!=0u || s.words[4].y!=0u) {mark(bound.xyz-vec3<f32>(bound.w),bound.xyz+vec3<f32>(bound.w));}
 }
}
@compute @workgroup_size(64) fn track(@builtin(global_invocation_id) id:vec3<u32>) {
 let i=id.x;if i>=state.counts.x+state.counts.y {return;}
 let voxel=i>=state.counts.x;var now:Snapshot;
 if voxel {
  let index=i-state.counts.x;let v=volumes[index];
  for(var k=0u;k<3u;k++){now.words[k]=bitcast<vec4<u32>>(v.inverse[k]);}
  now.words[3]=bitcast<vec4<u32>>(v.low);now.words[4]=bitcast<vec4<u32>>(v.high);now.words[5]=v.identity;
  if index<state.counts.z {now.words[6].x=choices[index+16384u];}
 }else {
  let s=sources[i];for(var k=0u;k<3u;k++){now.words[k]=bitcast<vec4<u32>>(s.rows[k]);}
  now.words[3]=bitcast<vec4<u32>>(s.sphere);now.words[4]=s.tail;
  if i>=state.counts.w && i-state.counts.w<state.counts.z {now.words[6].x=choices[i-state.counts.w+16384u];}
 }
 let old=history[i];var changed=state.control.x!=0u;
 for(var k=0u;k<8u;k++){changed=changed || any(old.words[k]!=now.words[k]);}
 if changed {
  if state.control.x==0u {mark_snapshot(old,voxel);mark_snapshot(now,voxel);}
  history[i]=now;
 }
}
