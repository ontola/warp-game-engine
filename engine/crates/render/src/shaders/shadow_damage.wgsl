// Exact caster history, not a hash: collisions must not retain obsolete depth.
// One history serves every cached sun map (the shared map and each view's
// clipmap levels): a changed caster marks its old and new bounds as 64x64
// tiles in every active map. Both producers use world bounds.
struct Source {rows:array<vec4<f32>,3>,sphere:vec4<f32>,tail:vec4<u32>};
struct Volume {inverse:array<vec4<f32>,3>,low:vec4<f32>,high:vec4<f32>,identity:vec4<u32>};
// control: force, size, shift x, shift y (i32); live.x: tracked this frame,
// live.y: start a fresh mask after consumption or a projection invalidation.
struct Map {vp:mat4x4<f32>,control:vec4<u32>,live:vec4<u32>};
const MAX_MAPS:u32=33u;
// control.x: map count, control.y: any map forced.
struct State {counts:vec4<u32>,control:vec4<u32>,maps:array<Map,33>};
struct Snapshot {words:array<vec4<u32>,8>};
@group(0) @binding(0) var<uniform> state:State;
@group(0) @binding(1) var<storage,read> sources:array<Source>;
@group(0) @binding(2) var<storage,read> volumes:array<Volume>;
@group(0) @binding(3) var<storage,read> choices:array<u32>;
@group(0) @binding(4) var<storage,read_write> history:array<Snapshot>;
// 128 words (one 512-byte mask) per map slot.
@group(0) @binding(5) var<storage,read_write> damage:array<atomic<u32>>;

@compute @workgroup_size(64) fn reset(@builtin(global_invocation_id) id:vec3<u32>) {
 let m=id.x/128u;let word=id.x%128u;
 if m>=state.control.x || state.maps[m].live.y==0u {return;}
 let map=state.maps[m];
 var bits=0u;
 let shift=bitcast<vec2<i32>>(map.control.zw);let tile_size=i32(map.control.y/64u);
 for(var k=0u;k<32u;k++) {
  let tile=word*32u+k;let low=vec2<i32>(i32(tile%64u),i32(tile/64u))*tile_size-shift;
  let high=low+vec2<i32>(tile_size-1);
  if map.control.x!=0u || any(low<vec2<i32>(0)) || any(high>=vec2<i32>(i32(map.control.y))) {bits|=1u<<k;}
 }
 atomicStore(&damage[id.x],bits);
}
fn all_dirty(m:u32) {for(var i=0u;i<128u;i++){atomicStore(&damage[m*128u+i],0xffffffffu);}}
fn mark(m:u32,low:vec3<f32>,high:vec3<f32>) {
 let vp=state.maps[m].vp;let size=f32(state.maps[m].control.y);
 var lo=vec2<f32>(1e30);var hi=vec2<f32>(-1e30);var zlo=1e30;var zhi=-1e30;
 for(var k=0u;k<8u;k++) {
  let p=vec3<f32>(select(low.x,high.x,(k&1u)!=0u),select(low.y,high.y,(k&2u)!=0u),select(low.z,high.z,(k&4u)!=0u));
  let q=vp*vec4<f32>(p,1.);
  // Unsupported/singular projection must invalidate, never retain stale pixels.
  if !(q.w>0.000001) {all_dirty(m);return;}
  let uv=vec2<f32>(q.x/q.w*.5+.5,.5-q.y/q.w*.5);
  lo=min(lo,uv);hi=max(hi,uv);zlo=min(zlo,q.z/q.w);zhi=max(zhi,q.z/q.w);
 }
 // Two texels cover raster footprint/rounding at tile boundaries. Receiver
 // filtering reads neighboring texels; those stay valid independently.
 let margin=vec2<f32>(2./size);lo-=margin;hi+=margin;
 if any(hi<vec2<f32>(0.)) || any(lo>vec2<f32>(1.)) || zhi<0. || zlo>1. {return;}
 let a=vec2<u32>(clamp(floor(lo*64.),vec2<f32>(0.),vec2<f32>(63.)));
 let b=vec2<u32>(clamp(floor(hi*64.),vec2<f32>(0.),vec2<f32>(63.)));
 for(var y=a.y;y<=b.y;y++){for(var x=a.x;x<=b.x;x++) {
  let tile=y*64u+x;atomicOr(&damage[m*128u+(tile>>5u)],1u<<(tile&31u));
 }}
}
fn mark_bounds(low:vec3<f32>,high:vec3<f32>) {
 for(var m=0u;m<min(state.control.x,MAX_MAPS);m++) {
  if state.maps[m].live.x!=0u && state.maps[m].control.x==0u {mark(m,low,high);}
 }
}
fn mark_snapshot(s:Snapshot,voxel:bool) {
 if voxel {
  let lo=bitcast<vec4<f32>>(s.words[3]);let hi=bitcast<vec4<f32>>(s.words[4]);
  if (lo.w==0. || lo.w==2.) && s.words[5].z!=0u {mark_bounds(lo.xyz,hi.xyz);}
 }else {
  let bound=bitcast<vec4<f32>>(s.words[3]);
  // A virtual owner has its encoded member count in first_cluster. No source
  // payload (first_cluster=0) is also the GPU forest voxel replacement marker.
  if bound.w>=0. && (s.words[4].x!=0u || s.words[4].y!=0u) {mark_bounds(bound.xyz-vec3<f32>(bound.w),bound.xyz+vec3<f32>(bound.w));}
 }
}
// Procedural terrain patches are re-tessellated around the camera: their
// sphere and cluster change with camera LOD while the surface stays the same
// (within the LOD's own error). Counting that as a moved caster redrew most
// sun clipmap tiles every frame in flight (~300 patches per frame, Mancave 12
// November). Their history still follows, so a later real change compares
// against the current patch. KERN_SUN_TERRAIN_LOD_DAMAGE=1: old behaviour.
override TERRAIN_LOD_DAMAGE:bool=false;
fn terrain_patch(s:Snapshot)->bool {let flags=s.words[4].w;return (flags&128u)!=0u && ((flags>>24u)&7u)==1u;}
@compute @workgroup_size(64) fn track(@builtin(global_invocation_id) id:vec3<u32>) {
 let i=id.x;if i>=state.counts.x+state.counts.y {return;}
 let voxel=i>=state.counts.x;var now:Snapshot;
 if voxel {
  let index=i-state.counts.x;let v=volumes[index];
  for(var k=0u;k<3u;k++){now.words[k]=bitcast<vec4<u32>>(v.inverse[k]);}
  now.words[3]=bitcast<vec4<u32>>(v.low);now.words[4]=bitcast<vec4<u32>>(v.high);now.words[5]=v.identity;
  if index<state.counts.z {now.words[6].x=choices[index+state.counts.z];}
 }else {
  let s=sources[i];for(var k=0u;k<3u;k++){now.words[k]=bitcast<vec4<u32>>(s.rows[k]);}
  now.words[3]=bitcast<vec4<u32>>(s.sphere);now.words[4]=s.tail;
  if i>=state.counts.w && i-state.counts.w<state.counts.z {now.words[6].x=choices[i-state.counts.w+state.counts.z];}
 }
 let old=history[i];var changed=false;var first=8u;
 for(var k=0u;k<8u;k++){let differs=any(old.words[k]!=now.words[k]);if differs && first==8u {first=k;}changed=changed || differs;}
 let retessellated=!TERRAIN_LOD_DAMAGE && !voxel && terrain_patch(now) && terrain_patch(old);
 if changed && !retessellated {
  mark_snapshot(old,voxel);mark_snapshot(now,voxel);
  // Diagnostics (KERN_SUN_DAMAGE_REPORT): changed casters by kind and by
  // the first differing snapshot row, after the masks.
  atomicAdd(&damage[MAX_MAPS*128u+select(0u,8u,voxel)+first],1u);
 }
 // A forced map is wholly dirty; it still resynchronizes the history.
 if changed || state.control.y!=0u {history[i]=now;}
}
