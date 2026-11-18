// Complete-cut incremental admission. Stable bucket ordering, no CPU tree list.
// Shared-recipe traversal. No CPU near-tree shortlist.
struct Node { sphere:vec4<f32>, error:vec4<f32>, links:vec4<u32> };
// links: first child (or NONE), exclusive subtree end, representation, reserved.
struct Tree { x:vec4<f32>, y:vec4<f32>, z:vec4<f32>, scale:vec4<f32>, info:vec4<u32> };
// info: root node, stable generation, reserved, reserved. scale.y: wind envelope.
struct Consumer { vp:mat4x4<f32>, pixels:vec4<f32> };
struct View { consumers:array<Consumer,32>, consumer_count:vec4<u32>, vp:mat4x4<f32>, previous_vp:mat4x4<f32>, pixels:vec4<f32>, counts:vec4<u32> };

@group(0) @binding(0) var<uniform> view:View;
@group(0) @binding(1) var<storage,read> nodes:array<Node>;
@group(0) @binding(2) var<storage,read> trees:array<Tree>;
struct AssetPage {resident:u32,requested:atomic<u32>,generation:u32,padding:u32};
@group(0) @binding(3) var<storage,read_write> pages:array<AssetPage>;
struct Proposal {old:vec4<u32>,replacement:vec4<u32>,prefix:vec4<u32>,control:vec4<u32>,output:vec4<u32>,publication:vec4<u32>};
@group(0) @binding(4) var<storage,read_write> proposals:array<Proposal>;
// task: root, node (NONE means emergency proxy), consumer mask, dormant index+2 (0=new).
// Both work queues in one binding: this round's tasks at queue_half.x, the next
// round's at queue_half.y, queue_half.z tasks each (foliage_refine.rs swaps the halves).
@group(0) @binding(5) var<storage,read_write> queue:array<vec4<u32>>;
@group(0) @binding(6) var<uniform> queue_half:vec4<u32>;
// 32 global priority totals, followed by 32 buckets per 256-task block.
@group(0) @binding(7) var<storage,read_write> blocks:array<vec4<u32>>;
@group(0) @binding(8) var<storage,read_write> records:array<vec4<u32>>;
// Selection counters (words 0..15) and, from word STATE_BASE, this pass's
// allocation state, in one binding (foliage_refine.rs STATE_OFFSET).
@group(0) @binding(9) var<storage,read_write> counters:array<atomic<u32>>;
const STATE_BASE:u32=64u;
@group(0) @binding(10) var<storage,read_write> fallback:array<atomic<u32>>;
@group(0) @binding(11) var<storage,read_write> direct:array<u32>;
struct Demand {count:atomic<u32>,bucket:u32,prefix:u32,failed:atomic<u32>};
@group(0) @binding(12) var<storage,read_write> demand:array<Demand>;
// used xyz, task count; snapshot xyz, next count; task and scan indirect args;
// fault at15, peak16, retained-for-budget17. Never read back to schedule work.
struct Parked { task:vec4<u32>, proposal:Proposal };
@group(0) @binding(14) var<storage,read_write> parked:array<Parked>;
@group(0) @binding(15) var<storage,read_write> record_consumers:array<u32>;
@group(0) @binding(16) var<storage,read> seed_visibility:array<u32>;
const NONE:u32=0xffffffffu;
// Error-driven cut (fast_*): multiplier on the pixel targets, 1 = exact target.
// The budgeted admission kernels never change it.
var<private> cut_scale:f32=1.0;
// Urgency ready() requests pages with (the page cache uploads the highest
// first): set by fast_round from the waiting cut's projected error.
var<private> request_urgency:u32=1u;
fn ready(rep:u32)->bool {
 if atomicLoad(&pages[rep].requested)<request_urgency {atomicMax(&pages[rep].requested,request_urgency);}
 return pages[rep].resident!=0u;
}
fn plus(a:vec4<u32>,b:vec4<u32>)->vec4<u32> {
 let sum=a+b;return select(sum,vec4<u32>(NONE),sum<a);
}
// z/w mask bits match the existing consumer diagnostics (camera256/shadow512).
fn mask(s:vec4<f32>)->u32 {
 var result=0u;
 for(var i=0u;i<view.consumer_count.x;i++) {
  let c=view.consumers[i];if visible_in(s,c.vp) {result|=select(512u,256u,c.pixels.z==0.0);}
 }
 return result;
}
fn limits()->vec4<u32> {return vec4<u32>(view.consumer_count.yzw,u32(view.pixels.w));}
fn used_now()->vec4<u32> {return vec4<u32>(atomicLoad(&counters[STATE_BASE+0]),atomicLoad(&counters[STATE_BASE+1]),atomicLoad(&counters[STATE_BASE+2]),atomicLoad(&counters[STATE_BASE+69]));}
fn used_snapshot()->vec4<u32> {return vec4<u32>(atomicLoad(&counters[STATE_BASE+4]),atomicLoad(&counters[STATE_BASE+5]),atomicLoad(&counters[STATE_BASE+6]),atomicLoad(&counters[STATE_BASE+70]));}
// Source clusters one view's cull visits for this node (cull.wgsl
// coarse_stop): its table (foliage_source_pages::source_work_table, stored
// after the nodes) at the coarsest bin no coarser than the view's cut, at
// the bound's nearest point. `pixels_per_m`: pixels per tree-local metre.
// Cut at the camera's 1 px cluster target (Renderer lod_pixels, default 1).
// error_in measures length(dx,dy), sqrt(2) times the pixels per metre along
// one axis. The table already allows for wind shear (the cull refines by
// 1 + |shear|, at most wind::MAX_TREE_SHEAR for trees). A node too close for
// the finest bin books every cluster.
const SOURCE_TABLE_T0:f32=0.00025;
fn source_work_at(node:u32,pixels_per_m:f32)->u32 {
 let full=nodes[node].links.w>>8u;
 if !(pixels_per_m>0.0) || pixels_per_m>=1e29 {return full;}
 let t=1.41421356/pixels_per_m;
 if t<SOURCE_TABLE_T0 {return full;}
 // Half-octave bins, two 16-bit counts per word (foliage_source_pages).
 let k=min(u32(floor(2.0*log2(t/SOURCE_TABLE_T0))),23u);
 let r=nodes[node+arrayLength(&nodes)/2u];
 var table=array<u32,12>(bitcast<u32>(r.sphere.x),bitcast<u32>(r.sphere.y),bitcast<u32>(r.sphere.z),bitcast<u32>(r.sphere.w),
  bitcast<u32>(r.error.x),bitcast<u32>(r.error.y),bitcast<u32>(r.error.z),bitcast<u32>(r.error.w),r.links.x,r.links.y,r.links.z,r.links.w);
 let count=(table[k/2u]>>(16u*(k&1u)))&0xffffu;
 if count==0xffffu {return full;}
 return min(count,full);
}
fn work(tree:u32,node:u32,consumers:u32)->vec4<u32> {
 var cost=vec4<u32>(0u);
 if node==NONE {cost.y=u32(trees[tree].scale.z);cost.z=u32(trees[tree].scale.w);}
 else {let n=nodes[node];cost=vec4<u32>(select(1u,0u,(n.links.w&2u)!=0u),u32(n.error.z),u32(n.error.w),0u);}
 if node!=NONE && (nodes[node].links.w>>8u)!=0u {
  var clusters=0u;let bound=sphere(trees[tree],nodes[node]);
  for(var i=0u;i<view.consumer_count.x;i++) {let c=view.consumers[i];
   if (consumers&select(512u,256u,c.pixels.z==0.0))!=0u && visible_in(bound,c.vp) {
    clusters+=source_work_at(node,error_in(bound,1.0,c.vp,c.pixels.xy)*trees[tree].scale.x);
   }
  }
  cost.w=clusters;
 }
 if (consumers&256u)==0u {cost.y=0u;}if (consumers&512u)==0u {cost.z=0u;}
 // Error cut: book splats for the part of the bound inside each class's
 // frustums (the raster drops cells outside them). The same fraction is
 // recomputed for the refund within one encode, so books stay balanced.
 if atomicLoad(&counters[STATE_BASE+80])!=0u && (cost.y!=0u || cost.z!=0u) {
  var bound=vec4<f32>(0.0);
  if node==NONE {bound=sphere(trees[tree],nodes[trees[tree].info.x]);}else{bound=sphere(trees[tree],nodes[node]);}
  var inside=vec2<f32>(0.0);
  for(var i=0u;i<view.consumer_count.x;i++) {let c=view.consumers[i];
   let k=select(1u,0u,c.pixels.z==0.0);inside[k]=max(inside[k],inside_fraction(bound,c.vp));
  }
  // The camera raster thins sub-pixel cells by disc area (volume_splat_append:
  // 1.487 side^2 alpha px, target 1, at most 64x): book that share too,
  // with the cell side at the bound's nearest point. Links bits 3-7 hold k,
  // cell <= error.x/2^k (foliage_runtime). error_in measures length(dx,dy),
  // sqrt(2) times one axis for square pixels. The sun maps do not thin.
  if node!=NONE {
   let n=nodes[node];let cell=n.error.x*exp2(-f32((n.links.w>>3u)&31u))*trees[tree].scale.x;
   var side=0.0;
   for(var i=0u;i<view.consumer_count.x;i++) {let c=view.consumers[i];
    if c.pixels.z==0.0 && visible_in(bound,c.vp) {side=max(side,error_in(bound,cell,c.vp,c.pixels.xy));}
   }
   inside.x*=clamp(0.743555*side*side,1.0/64.0,1.0);
  }
  cost.y=u32(ceil(f32(cost.y)*inside.x));cost.z=u32(ceil(f32(cost.z)*inside.y));
 }
 return cost;
}
// Share of a sphere's volume inside a frustum: product over the planes of
// the cap volume x^2(3-2x), x the inside share of the diameter, never below x.
fn inside_fraction(s:vec4<f32>,vp:mat4x4<f32>)->f32 {
 let r=transpose(vp);
 let planes=array<vec4<f32>,6>(r[3]+r[0],r[3]-r[0],r[3]+r[1],r[3]-r[1],r[2],r[3]-r[2]);
 var f=1.0;
 for(var i=0u;i<6u;i++){
  let d=dot(planes[i],vec4<f32>(s.xyz,1.0))/max(length(planes[i].xyz),1e-20);
  let x=clamp((d+s.w)/max(2.0*s.w,1e-20),0.0,1.0);f*=max(x,x*x*(3.0-2.0*x));
 }
 return f;
}
fn add_used(cost:vec4<u32>) {
 for(var k=0u;k<3u;k++) {
  let old=atomicAdd(&counters[STATE_BASE+k],cost[k]);if old>NONE-cost[k] {atomicOr(&counters[STATE_BASE+15],1u);}
 }
}
// Compact only live consumer tasks, in stable root/role order. Reserved world
// capacity and roots invisible to every consumer never enter refinement scans.
var<workgroup> seed_scan:array<vec4<u32>,256>;
@compute @workgroup_size(256) fn seed(@builtin(global_invocation_id) id:vec3<u32>,@builtin(local_invocation_index) lane:u32,@builtin(workgroup_id) group:vec3<u32>) {
 let tree=id.x;var value=vec4<u32>(0u);
 if tree<view.counts.x {
  let root=trees[tree];var consumers=0u;
  if root.info.w==0u {consumers=mask(sphere(root,nodes[root.info.x]));}
  if view.counts.w!=0u && seed_visibility[tree]==0u {consumers&=~256u;}
  demand[tree].bucket=consumers;demand[tree].prefix=0u;
  // Bits 16+: the rounds this tree's shadow reserved last frame (fast_round
  // writes them into the shadow half's failed word), its priority now.
  let shadow_rounds=min(atomicLoad(&demand[tree+view.counts.x].failed),255u);
  demand[tree+view.counts.x].bucket=(consumers&512u)|(shadow_rounds<<16u);demand[tree+view.counts.x].prefix=0u;
  // demand[tree].failed (camera half) carries last frame's camera record
  // count into this one: fast_round's hysteresis (CUT_HYSTERESIS).
  let previous=atomicLoad(&demand[tree].count);
  atomicStore(&demand[tree].count,0u);atomicStore(&demand[tree].failed,previous);
  atomicStore(&demand[tree+view.counts.x].count,0u);
  if consumers!=0u {let cost=work(tree,NONE,consumers);value=vec4<u32>(countOneBits(consumers),cost.yz,0u);}
 }
 seed_scan[lane]=value;workgroupBarrier();
 for(var step=1u;step<256u;step*=2u) {
  var prior=vec4<u32>(0u);if lane>=step {prior=seed_scan[lane-step];}
  workgroupBarrier();seed_scan[lane]=plus(seed_scan[lane],prior);workgroupBarrier();
 }
 if tree<view.counts.x {atomicStore(&demand[tree+view.counts.x].failed,seed_scan[lane].x-value.x);}
 if lane==255u {
  let sum=seed_scan[lane];blocks[32u+group.x*32u]=vec4<u32>(sum.x,0u,0u,0u);
  if sum.y==NONE || sum.z==NONE {atomicOr(&counters[STATE_BASE+15],1u);}
  add_used(vec4<u32>(0u,sum.yz,0u));
 }
}
var<workgroup> start_sums:array<u32,256>;
// Exclusive scan of the per-block seed counts: each lane owns a contiguous
// run of blocks, so one workgroup scans ~650 blocks in a few steps instead of
// one thread walking them serially.
@compute @workgroup_size(256) fn start(@builtin(local_invocation_index) li:u32) {
 let groups=(view.counts.x+255u)/256u;let per=(groups+255u)/256u;
 let first=min(li*per,groups);let last=min(first+per,groups);
 var sum=0u;
 for(var group=first;group<last;group++) {sum+=blocks[32u+group*32u].x;}
 start_sums[li]=sum;workgroupBarrier();
 for(var offset=1u;offset<256u;offset*=2u) {
  var add=0u;if li>=offset {add=start_sums[li-offset];}
  workgroupBarrier();start_sums[li]+=add;workgroupBarrier();
 }
 var count=start_sums[li]-sum;
 for(var group=first;group<last;group++) {let at=32u+group*32u;let n=blocks[at].x;blocks[at].y=count;count+=n;}
 if li==255u {
  atomicStore(&counters[STATE_BASE+3],count);atomicStore(&counters[STATE_BASE+16],count);
  if atomicLoad(&counters[STATE_BASE+1])>view.consumer_count.z || atomicLoad(&counters[STATE_BASE+2])>view.consumer_count.w {atomicOr(&counters[STATE_BASE+15],4u);}
 }
}
@compute @workgroup_size(64) fn seed_scatter(@builtin(global_invocation_id) id:vec3<u32>) {
 let tree=id.x;if tree>=view.counts.x {return;}
 let roles=demand[tree].bucket;var at=blocks[32u+(tree/256u)*32u].y+atomicLoad(&demand[tree+view.counts.x].failed);
 atomicStore(&demand[tree+view.counts.x].failed,0u);
 if (roles&256u)!=0u {queue[queue_half.x+at]=vec4<u32>(tree,NONE,256u,0u);at++;}
 if (roles&512u)!=0u {queue[queue_half.x+at]=vec4<u32>(tree,NONE,512u,0u);}
}
@compute @workgroup_size(1) fn prepare() {
 let count=atomicLoad(&counters[STATE_BASE+3]);
 atomicStore(&counters[STATE_BASE+70],atomicLoad(&counters[STATE_BASE+69]));
 for(var k=0u;k<3u;k++) {atomicStore(&counters[STATE_BASE+4u+k],atomicLoad(&counters[STATE_BASE+k]));}
 atomicStore(&counters[STATE_BASE+7],0u);atomicStore(&counters[STATE_BASE+18],0u);
 atomicStore(&counters[STATE_BASE+8],(count+63u)/64u);atomicStore(&counters[STATE_BASE+9],1u);atomicStore(&counters[STATE_BASE+10],1u);
 atomicStore(&counters[STATE_BASE+12],(count+255u)/256u);atomicStore(&counters[STATE_BASE+13],1u);atomicStore(&counters[STATE_BASE+14],1u);
 // A dormant queue is revisited only after the active frontier drains and
 // capacity or new pending work changed since its last wake attempt.
 let used=used_now();
 let previous=vec4<u32>(atomicLoad(&counters[STATE_BASE+56]),atomicLoad(&counters[STATE_BASE+57]),atomicLoad(&counters[STATE_BASE+58]),atomicLoad(&counters[STATE_BASE+71]));
 let parked_count=atomicLoad(&counters[STATE_BASE+29]);
 let wake=count==0u && (atomicLoad(&counters[STATE_BASE+15])&3u)==0u && (parked_count!=atomicLoad(&counters[STATE_BASE+59]) || any(used!=previous) || atomicLoad(&counters[STATE_BASE+30])!=0u);
 atomicStore(&counters[STATE_BASE+55],select(0u,1u,wake));
 atomicStore(&counters[STATE_BASE+64],select(0u,(parked_count+255u)/256u,wake));atomicStore(&counters[STATE_BASE+65],1u);atomicStore(&counters[STATE_BASE+66],1u);
 if wake {atomicStore(&counters[STATE_BASE+71],used.w);for(var k=0u;k<3u;k++) {atomicStore(&counters[STATE_BASE+56u+k],used[k]);}atomicStore(&counters[STATE_BASE+59],parked_count);atomicAdd(&counters[STATE_BASE+62],1u);}
 // Rounds that do work form a prefix: an idle round without a wake changes
 // nothing, so every later round is idle too. The CPU sizes the next frames'
 // round count from this (Refiner::encode).
 if count!=0u || wake {atomicAdd(&counters[STATE_BASE+68],1u);}
}
fn projected(t:Tree,n:Node,roles:u32)->vec2<f32> {
 let s=sphere(t,n);var all_scale=0.0;var camera_scale=0.0;
 for(var i=0u;i<view.consumer_count.x;i++) {
  let c=view.consumers[i];if (roles&select(512u,256u,c.pixels.z==0.0))!=0u && visible_in(s,c.vp) {
   let p=error_in(s,1.0,c.vp,c.pixels.xy);all_scale=max(all_scale,p);
   if c.pixels.z==0.0 {camera_scale=max(camera_scale,p);}
  }
 }
 return vec2<f32>(all_scale,camera_scale)*t.scale.x;
}
// A unary chain represents the same complete support at several LODs.
// Intermediate occupancy need not be monotonic: inspect through expensive or
// nonresident intermediate levels, but publish only a resident affordable end.
fn candidate(tree:u32,start:u32,request_pages:bool,roles:u32)->u32 {
 let t=trees[tree];var at=start;var best=start;
 let budget=work(tree,start,mask(sphere(t,nodes[start]))&roles);
 loop {
  let n=nodes[at];if n.links.x==NONE {break;}
  let scale=projected(t,n,roles);
  if at==best && (n.links.w&1u)==0u && scale.x<1e29 && scale.x*n.error.x<=view.pixels.z*cut_scale && (n.error.y==0.0 || scale.x*n.error.y<=cut_scale) {break;}
  let child=n.links.x;let c=nodes[child];if c.links.y!=n.links.y {break;}
  let cost=work(tree,child,mask(sphere(t,c))&roles);
  if all(cost<=budget) {
   var resident=pages[c.links.z].resident!=0u;
   if request_pages {resident=ready(c.links.z);}
   if resident {best=child;}
  }
  at=child;
 }
 return best;
}
// Complete endpoint admission skips temporary intermediate-cost barriers.
// Prefer the earliest pixel-acceptable cut, not an unconditional source cut.
// If its splat cost cannot fit, exact source remains a complete alternative.
// w: 1 = all source, 2 = pixel-acceptable mixed source/voxel frontier.
fn endpoint_acceptable(tree:u32,node:u32)->bool {
 let n=nodes[node];let scale=projected(trees[tree],n,256u);
 return (n.links.w&1u)==0u && scale.x<1e29 && scale.x*n.error.x<=view.pixels.z && scale.x*n.error.y<=1.0;
}
struct Endpoint {cost:vec4<u32>,mode:u32};
fn endpoint_frontier(tree:u32,node:u32,old:vec4<u32>,used:vec4<u32>,request_pages:bool,source_only:bool)->Endpoint {
 let t=trees[tree];let end=nodes[node].links.y;var at=nodes[node].links.x;
 var cost=vec4<u32>(0u);var resident=true;
 let available=limits()-min(used,limits());
 loop {
  if at==NONE || at>=end {break;}
  let n=nodes[at];let roles=mask(sphere(t,n))&256u;
  if roles==0u {at=n.links.y;continue;}
  let endpoint=select(endpoint_acceptable(tree,at),(n.links.w&4u)!=0u && n.links.x==NONE,source_only);
  if !endpoint {
   if n.links.x==NONE {return Endpoint(vec4<u32>(0u),0u);}
   at=n.links.x;continue;
  }
  cost=plus(cost,work(tree,at,roles));
  if any((cost-min(cost,old))>available) {return Endpoint(vec4<u32>(0u),0u);}
  if request_pages {resident=ready(n.links.z)&&resident;}
  else {resident=(pages[n.links.z].resident!=0u)&&resident;}
  at=n.links.y;
 }
 if !resident {return Endpoint(vec4<u32>(0u),0u);}
 return Endpoint(cost,select(2u,1u,source_only));
}
fn complete_endpoint(tree:u32,node:u32,old:vec4<u32>,used:vec4<u32>,request_pages:bool)->Endpoint {
 let mixed=endpoint_frontier(tree,node,old,used,request_pages,false);
 if mixed.mode!=0u {return mixed;}
 return endpoint_frontier(tree,node,old,used,request_pages,true);
}
@compute @workgroup_size(64) fn propose(@builtin(global_invocation_id) id:vec3<u32>) {
 let at=id.x;if at>=atomicLoad(&counters[STATE_BASE+3]) {return;}
 let task=queue[queue_half.x+at];var p:Proposal;
 // Cache lives only within this encode: camera, pages and nodes are immutable.
 // Available budget changes, so admission is always rechecked.
 if task.w>=2u {
  let cached=parked[task.w-2u].proposal;p.old=cached.old;p.replacement=cached.replacement;p.publication=cached.publication;
  p.control=vec4<u32>(cached.control.x,1u,cached.output.y,0u);p.output.y=cached.output.y;
  let used=used_snapshot();
  // A parked proposal lost collective admission. Reusing only its original
  // intermediate cut serializes cost-release waves even when all complete
  // complete endpoints fit together. Reconsider that endpoint on retry.
  if task.y!=NONE && task.z==256u && (p.replacement.y>p.old.y || p.replacement.w>p.old.w) {
   let source=complete_endpoint(task.x,task.y,p.old,used,true);
   if source.mode!=0u {
    p.replacement=source.cost;p.publication.x=source.mode;p.control.z=0u;p.output.y=0u;
    let n=nodes[task.y];let scale=projected(trees[task.x],n,task.z);
    let delta=source.cost-min(source.cost,p.old);let price=max(1.0,f32(delta.x)*256.0+f32(delta.y));
    let parent_error=select(n.error.x,n.sphere.w,(n.links.w&1u)!=0u);
    let benefit=max(0.001,scale.y*parent_error);
    p.control.x=u32(clamp(floor(log2(max(benefit/price,1e-20)))+12.0,0.0,15.0))+16u;
   }
  }
  let capacity=limits();let delta=p.replacement-min(p.replacement,p.old);
  if any(delta>capacity-min(used,capacity)) {p.control.y=2u;atomicAdd(&counters[STATE_BASE+17],1u);}
  else if (task.z&256u)!=0u {atomicOr(&counters[STATE_BASE+18],1u);}
  atomicAdd(&counters[STATE_BASE+28],1u);proposals[at]=p;return;
 }
 if task.z==0u {proposals[at]=p;return;}
 let t=trees[task.x];let node_id=select(task.y,t.info.x,task.y==NONE);let n=nodes[node_id];
 p.old=work(task.x,task.y,task.z);atomicAdd(&counters[4],1u);
 let scale=projected(t,n,task.z);
 var refine=task.y==NONE || (n.links.x!=NONE && ((n.links.w&1u)!=0u || scale.x>=1e29 || scale.x*n.error.x>view.pixels.z || scale.x*n.error.y>1.0));
 let wanted_refine=refine;
 var child=node_id;var end=node_id+1u;
 if task.y!=NONE {child=n.links.x;end=n.links.y;}
 var children=0u;var child_error=0.0;
 if refine {
  loop {
   if child==NONE || child>=end {break;}
   let selected=candidate(task.x,child,true,task.z);let c=nodes[selected];let consumers=mask(sphere(t,c))&task.z;
   if consumers!=0u {
    let resident=ready(c.links.z);refine=refine&&resident;
    p.replacement=plus(p.replacement,work(task.x,selected,consumers));
    if c.links.x!=NONE {children++;}
    child_error=max(child_error,c.error.x);
   }
   child=nodes[child].links.y;
  }
 }
 // Relieve a temporary camera-splat barrier without publishing intermediate
 // voxel levels. A missing intermediate page is harmless if every source leaf
 // is resident; missing source pages still retain the current complete parent.

 if wanted_refine && task.y!=NONE && task.z==256u {
  let used=used_snapshot();
  let delta=p.replacement-min(p.replacement,p.old);
  if delta.y>view.consumer_count.z-min(used.y,view.consumer_count.z) || delta.w>limits().w-min(used.w,limits().w) {
   let source=complete_endpoint(task.x,task.y,p.old,used,true);
   if source.mode!=0u {p.replacement=source.cost;p.publication.x=source.mode;children=0u;child_error=0.0;refine=true;}
  }
 }
 // All visible siblings must fit together. Missing pages keep the current parent.
 if refine {
  let delta=p.replacement-min(p.replacement,p.old);
  let used=used_snapshot();
  let capacity=limits();
  // An individually impossible proposal must not reserve prefix space and
  // starve cheaper candidates. Retain its parent without entering the scan.
  let individually_fits=all(delta<=capacity-min(used,capacity));
  if !individually_fits {atomicAdd(&counters[STATE_BASE+17],1u);}
  if individually_fits && (task.z&256u)!=0u {atomicOr(&counters[STATE_BASE+18],1u);}
  let price=max(1.0,f32(delta.x)*256.0+f32(delta.y)+f32(delta.z));
  // Mandatory root fallbacks carry zero authored error. Their projected size
  // still determines benefit; otherwise all roots tie regardless of distance.
  let parent_error=select(n.error.x,n.sphere.w,task.y==NONE || (n.links.w&1u)!=0u);
  let priority_scale=select(scale.x,scale.y,(task.z&256u)!=0u);
  let benefit=max(0.001,priority_scale*max(parent_error-child_error,parent_error*0.01));
  let bucket=u32(clamp(floor(log2(max(benefit/price,1e-20)))+12.0,0.0,15.0));
  p.control=vec4<u32>(bucket+select(0u,16u,(task.z&256u)!=0u),select(2u,1u,individually_fits),children,0u);
  p.output.y=children; // Deferred queue count is one; preserve replacement arity.
 }
 proposals[at]=p;
}
// Read each proposal once with coalesced lanes; the stable per-bucket scan
// then operates on 4 KiB of shared data instead of rereading global proposals.
var<workgroup> priority_items:array<vec4<u32>,256>;
var<workgroup> priority_source:array<u32,256>;
@compute @workgroup_size(256) fn priority_scan(@builtin(workgroup_id) block:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
 let start=block.x*256u;let end=min(start+256u,atomicLoad(&counters[STATE_BASE+3]));
 var item=vec4<u32>(0u);var source=0u;
 if start+lane<end {
  let p=proposals[start+lane];
  source=p.replacement.w-min(p.replacement.w,p.old.w);
  item=vec4<u32>((p.replacement-min(p.replacement,p.old)).xyz,select(0u,p.control.x+1u,p.control.y==1u));
 }
 priority_items[lane]=item;priority_source[lane]=source;workgroupBarrier();
 if lane>=32u {return;}
 var sum=vec4<u32>(0u);
 for(var offset=0u;offset<end-start;offset++) {
  let value=priority_items[offset];if value.w==lane+1u {
   proposals[start+offset].prefix=sum;sum=plus(sum,vec4<u32>(value.xyz,priority_source[offset]));
  }
 }
 blocks[32u+block.x*32u+lane]=sum;
}
@compute @workgroup_size(32) fn priority_totals(@builtin(local_invocation_index) bucket:u32) {
 var sum=vec4<u32>(0u);
 for(var b=0u;b<(atomicLoad(&counters[STATE_BASE+3])+255u)/256u;b++) {
  let at=32u+b*32u+bucket;let count=blocks[at];blocks[at]=sum;sum=plus(sum,count);
 }
 blocks[bucket]=sum;
}
@compute @workgroup_size(64) fn decide(@builtin(global_invocation_id) id:vec3<u32>) {
 let at=id.x;if at>=atomicLoad(&counters[STATE_BASE+3]) {return;}
 let p=proposals[at];if p.control.y==0u {return;}
 if p.control.y==2u {proposals[at].control.w=2u;return;}
 // Only branch slots are shared across consumer budgets. A shadow-only
 // raster upgrade with no additional slot demand cannot starve the camera,
 // and admission to a resident ancestor must not wait behind camera work.
 if (queue[queue_half.x+at].z&256u)==0u && atomicLoad(&counters[STATE_BASE+18])!=0u && p.replacement.x>p.old.x {
  proposals[at].control.w=2u;return;
 }
 var before=plus(p.prefix,blocks[32u+(at/256u)*32u+p.control.x]);
 for(var b=p.control.x+1u;b<32u;b++) {before=plus(before,blocks[b]);}
 let delta=p.replacement-min(p.replacement,p.old);
 let used=used_snapshot();
 let available=limits();
 // Zero/negative-cost replacements remain allowed when a baseline exceeds a cap.
 let cost=plus(before,delta);let fits=all(select(cost<=available-min(used,available),vec4<bool>(true),delta==vec4<u32>(0u)));
 if fits && (atomicLoad(&counters[STATE_BASE+15])&3u)==0u {proposals[at].control.w=1u;}
 else {proposals[at].control.w=2u;atomicAdd(&counters[STATE_BASE+17],1u);}
}
@compute @workgroup_size(1) fn output_scan(@builtin(workgroup_id) block:vec3<u32>) {
 var sum=0u;var asleep=0u;let end=min((block.x+1u)*256u,atomicLoad(&counters[STATE_BASE+3]));
 for(var at=block.x*256u;at<end;at++) {
  proposals[at].output.x=sum;proposals[at].output.z=asleep;
  if proposals[at].control.w==1u {sum+=proposals[at].control.z;}
  if proposals[at].control.w==2u && queue[queue_half.x+at].w<2u {asleep++;}
 }
 blocks[32u+block.x*32u]=vec4<u32>(sum,0u,asleep,0u);
}
@compute @workgroup_size(1) fn output_totals() {
 var sum=0u;var asleep=0u;
 for(var b=0u;b<(atomicLoad(&counters[STATE_BASE+3])+255u)/256u;b++) {
  let at=32u+b*32u;let count=blocks[at];blocks[at].y=sum;blocks[at].w=asleep;
  sum+=count.x;asleep+=count.z;
 }
 let base=atomicLoad(&counters[STATE_BASE+29]);atomicStore(&counters[STATE_BASE+31],base);
 atomicStore(&counters[STATE_BASE+29],min(base+asleep,arrayLength(&parked)));
 if asleep>arrayLength(&parked)-base {atomicAdd(&counters[STATE_BASE+63],asleep-(arrayLength(&parked)-base));}
 atomicStore(&counters[STATE_BASE+7],sum);atomicMax(&counters[STATE_BASE+16],sum);
 if sum>queue_half.z {atomicOr(&counters[STATE_BASE+15],2u);}
}
fn publish(task:vec4<u32>) {
 if task.z==0u {return;}
 if task.y==NONE {atomicOr(&fallback[task.x],task.z);return;}
 let n=nodes[task.y];let resident=ready(n.links.z);
 if !resident {atomicOr(&fallback[task.x],task.z);return;}
 if (n.links.w&2u)!=0u {
  let publication=task.x+select(0u,view.counts.x,task.z==512u);
  demand[publication].prefix=n.links.z+1u;
  // Compatibility diagnostic: camera root, or shadow root outside camera demand.
  if task.z==256u || (demand[task.x].bucket&256u)==0u {direct[task.x]=n.links.z+1u;}
  atomicAdd(&counters[7],1u);return;
 }
 let at=atomicAdd(&counters[0],1u);atomicAdd(&demand[task.x+select(0u,view.counts.x,task.z==512u)].count,1u);
 if at<view.counts.y {records[at]=vec4<u32>(task.x,task.y,n.links.z,trees[task.x].info.y);record_consumers[at]=task.z;}
 else {atomicOr(&fallback[task.x],1u);atomicAdd(&counters[3],1u);}
}
fn state_park_index(at:u32,p:Proposal)->u32 {
 if queue[queue_half.x+at].w>=2u {return queue[queue_half.x+at].w-2u;}
 return atomicLoad(&counters[STATE_BASE+31])+blocks[32u+(at/256u)*32u].w+p.output.z;
}
@compute @workgroup_size(64) fn apply(@builtin(global_invocation_id) id:vec3<u32>) {
 let at=id.x;if at>=atomicLoad(&counters[STATE_BASE+3]) {return;}
 let p=proposals[at];let task=queue[queue_half.x+at];
 if p.control.w==2u && (atomicLoad(&counters[STATE_BASE+15])&2u)==0u {
  var destination=state_park_index(at,p);
  if destination<arrayLength(&parked) {
   parked[destination].task=vec4<u32>(task.xyz,1u);parked[destination].proposal=p;
   atomicAdd(&counters[STATE_BASE+61],1u);
  }else {publish(task);}
  return;
 }
 if p.control.w==0u || (atomicLoad(&counters[STATE_BASE+15])&2u)!=0u {publish(task);return;}
 // No writes to children until every candidate's output range is reserved.
 for(var k=0u;k<3u;k++) {
  if p.replacement[k]>=p.old[k] {atomicAdd(&counters[STATE_BASE+k],p.replacement[k]-p.old[k]);}
  else {atomicSub(&counters[STATE_BASE+k],p.old[k]-p.replacement[k]);}
 }
 if p.replacement.w>=p.old.w {atomicAdd(&counters[STATE_BASE+69],p.replacement.w-p.old.w);}
 else {atomicSub(&counters[STATE_BASE+69],p.old.w-p.replacement.w);}
 // Admission reserved the entire endpoint frontier; replay exactly the same
 // traversal. Pages and views are immutable throughout this encode.
 if p.publication.x!=0u {
  let t=trees[task.x];let end=nodes[task.y].links.y;var child=nodes[task.y].links.x;
  loop {
   if child==NONE || child>=end {break;}
   let n=nodes[child];let roles=mask(sphere(t,n))&256u;
   if roles==0u {child=n.links.y;continue;}
   let endpoint=select(endpoint_acceptable(task.x,child),(n.links.w&4u)!=0u && n.links.x==NONE,p.publication.x==1u);
   if !endpoint {child=n.links.x;continue;}
   publish(vec4<u32>(task.x,child,roles,0u));child=n.links.y;
  }
  return;
 }
 var offset=p.output.x+blocks[32u+(at/256u)*32u].y;
 let t=trees[task.x];var child=t.info.x;var end=child+1u;
 if task.y!=NONE {child=nodes[task.y].links.x;end=nodes[task.y].links.y;}
 loop {
  if child==NONE || child>=end {break;}
  let selected=candidate(task.x,child,true,task.z);let c=nodes[selected];let consumers=mask(sphere(t,c))&task.z;
  if consumers!=0u {
   let replacement=vec4<u32>(task.x,selected,consumers,0u);
   if c.links.x==NONE {publish(replacement);}else{queue[queue_half.y+offset]=replacement;offset++;}
  }
  child=nodes[child].links.y;
 }
}
@compute @workgroup_size(1) fn advance() {
 var count=atomicLoad(&counters[STATE_BASE+7]);if (atomicLoad(&counters[STATE_BASE+15])&2u)!=0u {count=0u;atomicAdd(&counters[11],1u);}
 atomicStore(&counters[STATE_BASE+3],count);atomicStore(&counters[15],atomicLoad(&counters[STATE_BASE+16]));
}
// A depth-bounded final drain retains the current cut rather than dropping work.
@compute @workgroup_size(64) fn drain(@builtin(global_invocation_id) id:vec3<u32>) {
 if id.x<atomicLoad(&counters[STATE_BASE+3]) {publish(queue[queue_half.x+id.x]);}
 if id.x<atomicLoad(&counters[STATE_BASE+29]) && parked[id.x].task.w==1u {publish(parked[id.x].task);}
}
// Stable wake compaction is skipped entirely while active traversal continues.
fn wakeable(at:u32)->bool {
 if parked[at].task.w!=1u {return false;}
 let p=parked[at].proposal;let delta=p.replacement-min(p.replacement,p.old);
 let used=used_now();
 if all(delta<=limits()-min(used,limits())) {return true;}
 // A cheaper complete endpoint may wake even while the cached intermediate
 // splat reservation cannot. Publication remains in propose/decide/apply.
 let task=parked[at].task;
 if task.y!=NONE && task.z==256u && (p.replacement.y>p.old.y || p.replacement.w>p.old.w) {
  return complete_endpoint(task.x,task.y,p.old,used,true).mode!=0u;
 }
 return false;
}
var<workgroup> wake_scan:array<vec2<u32>,256>;
@compute @workgroup_size(256) fn wake_count(@builtin(workgroup_id) block:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
 let at=block.x*256u+lane;let valid=at<atomicLoad(&counters[STATE_BASE+29]);var selected=vec2<u32>(0u);
 if valid && wakeable(at) {selected=select(vec2<u32>(0u,1u),vec2<u32>(1u,0u),(parked[at].task.z&256u)!=0u);}
 wake_scan[lane]=selected;workgroupBarrier();
 for(var step=1u;step<256u;step*=2u) {
  var prior=vec2<u32>(0u);if lane>=step {prior=wake_scan[lane-step];}
  workgroupBarrier();wake_scan[lane]+=prior;workgroupBarrier();
 }
 if valid {
  let prefix=wake_scan[lane]-selected;
  parked[at].proposal.output.w=select(prefix.y,prefix.x,selected.x!=0u);
  parked[at].proposal.prefix.w=selected.x+2u*selected.y;
 }
 if lane==255u {blocks[32u+block.x*32u]=vec4<u32>(wake_scan[lane],0u,0u);}
}
@compute @workgroup_size(1) fn wake_prefix() {
 if atomicLoad(&counters[STATE_BASE+55])==0u {return;}
 var sum=vec2<u32>(0u);
 for(var b=0u;b<(atomicLoad(&counters[STATE_BASE+29])+255u)/256u;b++) {
  let at=32u+b*32u;let count=blocks[at].xy;blocks[at].z=sum.x;blocks[at].w=sum.y;sum+=count;
 }
 // Match camera-priority admission without repeatedly waking shadow work that
 // decide would immediately park again. Remember it even if camera work is free.
 let camera=sum.x!=0u;let count=select(sum.y,sum.x,camera);
 atomicStore(&counters[STATE_BASE+30],select(0u,1u,camera && sum.y!=0u));
 atomicStore(&counters[STATE_BASE+67],select(2u,1u,camera));
 atomicStore(&counters[STATE_BASE+3],count);atomicAdd(&counters[STATE_BASE+60],count);
 atomicStore(&counters[STATE_BASE+8],(count+63u)/64u);atomicStore(&counters[STATE_BASE+12],(count+255u)/256u);
}
@compute @workgroup_size(256) fn wake_scatter(@builtin(workgroup_id) block:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
 let at=block.x*256u+lane;if at>=atomicLoad(&counters[STATE_BASE+29]) {return;}
 let role=atomicLoad(&counters[STATE_BASE+67]);
 if parked[at].proposal.prefix.w==role {
  let base=select(blocks[32u+block.x*32u].w,blocks[32u+block.x*32u].z,role==1u);
  let offset=base+parked[at].proposal.output.w;
  queue[queue_half.x+offset]=vec4<u32>(parked[at].task.xyz,at+2u);parked[at].task.w=0u;
 }
}
fn sphere(t:Tree,n:Node)->vec4<f32> {
 let p=vec4<f32>(n.sphere.xyz,1.0);
 return vec4<f32>(dot(t.x,p),dot(t.y,p),dot(t.z,p),n.sphere.w*t.scale.x+t.scale.y);
}
fn visible_in(s:vec4<f32>,vp:mat4x4<f32>)->bool {
 let r=transpose(vp);
 let planes=array<vec4<f32>,6>(r[3]+r[0],r[3]-r[0],r[3]+r[1],r[3]-r[1],r[2],r[3]-r[2]);
 for(var i=0u;i<6u;i++){if dot(planes[i],vec4<f32>(s.xyz,1.0)) < -s.w*length(planes[i].xyz){return false;}}
 return true;
}
fn error_in(s:vec4<f32>,error:f32,vp:mat4x4<f32>,pixels:vec2<f32>)->f32 {
 let r=transpose(vp);let p=vec4<f32>(s.xyz,1.0);
 let wlen=length(r[3].xyz);let wmin=dot(r[3],p)-s.w*wlen;
 if wmin<=0.00001{return 1e30;}
 let lx=length(r[0].xyz);let ly=length(r[1].xyz);
 let dx=(lx/wmin+(abs(dot(r[0],p))+s.w*lx)*wlen/(wmin*wmin))*pixels.x*0.5;
 let dy=(ly/wmin+(abs(dot(r[1],p))+s.w*ly)*wlen/(wmin*wmin))*pixels.y*0.5;
 return error*length(vec2<f32>(dx,dy));
}

// Explicit diagnostic dispatch only: never part of normal frame scheduling.
// Positive finite float bits preserve ordering under atomicMax.
// Diagnose the next complete replacement against final remaining work. Read
// residency only: explicit diagnostics must not generate new asset requests.
fn audit_camera_limit(tree:u32,node:u32,camera_scale:f32) {
 let n=nodes[node];if camera_scale==0.0 {return;}
 if (n.links.w&1u)==0u && camera_scale*n.error.x<=view.pixels.z && camera_scale*n.error.y<=1.0 {return;}
 atomicAdd(&counters[STATE_BASE+48],1u);
 if n.links.x==NONE {atomicAdd(&counters[STATE_BASE+53],1u);return;}
 let t=trees[tree];var replacement=vec4<u32>(0u);var missing=false;var child=n.links.x;
 loop {
  if child==NONE || child>=n.links.y {break;}
  let selected=candidate(tree,child,false,256u);let c=nodes[selected];let consumers=mask(sphere(t,c))&256u;
  if consumers!=0u {replacement=plus(replacement,work(tree,selected,consumers));missing=missing||pages[c.links.z].resident==0u;}
  child=nodes[child].links.y;
 }
 let old=work(tree,node,mask(sphere(t,n))&256u);let delta=replacement-min(replacement,old);
 let used=used_now();
 let capacity=limits();var blocked=delta>capacity-min(used,capacity);
 if blocked.y || blocked.w {
  let source=complete_endpoint(tree,node,old,used,false);
  if source.mode!=0u {blocked=(source.cost-min(source.cost,old))>capacity-min(used,capacity);missing=false;}
 }
 if blocked.w {atomicAdd(&counters[STATE_BASE+72],1u);}
 if blocked.x {atomicAdd(&counters[STATE_BASE+49],1u);}if blocked.y || blocked.w {atomicAdd(&counters[STATE_BASE+50],1u);}if blocked.z {atomicAdd(&counters[STATE_BASE+51],1u);}
 if missing {atomicAdd(&counters[STATE_BASE+52],1u);}if !any(blocked) && !missing {atomicAdd(&counters[STATE_BASE+54],1u);}
}
fn audit_node(tree:u32,node:u32,roles:u32) {
 let n=nodes[node];let scale=projected(trees[tree],n,roles);
 audit_camera_limit(tree,node,scale.y);
 let spatial=min(scale*n.error.x,vec2<f32>(1e30));
 let ratio=min(max(spatial/max(view.pixels.z,1e-8),scale*n.error.y),vec2<f32>(1e30));
 atomicMax(&counters[STATE_BASE+19],bitcast<u32>(spatial.x));atomicMax(&counters[STATE_BASE+20],bitcast<u32>(spatial.y));
 atomicMax(&counters[STATE_BASE+21],bitcast<u32>(ratio.x));atomicMax(&counters[STATE_BASE+22],bitcast<u32>(ratio.y));
 if ratio.x>1.0 {atomicAdd(&counters[STATE_BASE+23],1u);}if ratio.y>1.0 {atomicAdd(&counters[STATE_BASE+24],1u);}
 if (n.links.w&1u)!=0u {atomicAdd(&counters[STATE_BASE+27],1u);}
}
@compute @workgroup_size(64) fn audit(@builtin(global_invocation_id) id:vec3<u32>) {
 let at=id.x;
 if at<min(atomicLoad(&counters[0]),view.counts.y) {
  let r=records[at];let n=nodes[r.y];let failure=atomicLoad(&fallback[r.x]);var roles=record_consumers[at]&~failure;
  if (failure&7u)!=0u {roles=0u;}if roles!=0u {audit_node(r.x,r.y,roles);
  let base=select(36u,32u,(n.links.w&4u)!=0u);
  atomicAdd(&counters[STATE_BASE+base+(roles>>8u)],1u);}
 }
 if at>=view.counts.x {return;}
 let failure=atomicLoad(&fallback[at]);if failure!=0u {atomicAdd(&counters[STATE_BASE+25],1u);}
 let first=trees[at].info.x;let end=nodes[first].links.y;
 for(var role=0u;role<2u;role++) {
  let rep=demand[at+role*view.counts.x].prefix;if rep==0u {continue;}
  let roles=256u<<role;if (failure&(roles|7u))!=0u {continue;}var found=false;
  for(var n=first;n<end;n++) {
   if nodes[n].links.z+1u==rep && (nodes[n].links.w&2u)!=0u {audit_node(at,n,roles);atomicAdd(&counters[STATE_BASE+40u+(roles>>8u)],1u);found=true;break;}
  }
  if !found {atomicAdd(&counters[STATE_BASE+26],1u);}
 }
}

// ---------------------------------------------------------------------------
// Error-driven cut, Nanite style. Every task decides alone: refine while the
// projected error exceeds the target and every replacement page is resident.
// No priority scan, no collective admission, no dormant queue. Budgets are
// safety caps reserved with atomics; when they bind, fast_adapt loosens the
// error target for the next frame (state[79], kept across encodes) so the
// caps stay slack and the cut does not depend on dispatch order.
// Camera and shadow consumers adapt separately (state[79] camera, [78] shadow):
// a full shadow reservation must not coarsen what the camera sees.
fn fast_scale_word(word:u32)->f32 {let s=bitcast<f32>(atomicLoad(&counters[STATE_BASE+word]));return select(1.0,s,s>=1.0 && s<1e6);}
// The camera never goes below CAMERA_CUT_FLOOR (foliage_refine.rs, from
// KERN_CAMERA_CUT_FLOOR): with few trees the caps never bind, the scale sat
// at 1 and exact source won out to 300 m, so where voxels start depended on
// how many trees there were. The floor keeps it near where a forest puts it.
fn fast_scale(roles:u32)->f32 {
 if (roles&256u)!=0u {return max(fast_scale_word(79u),CAMERA_CUT_FLOOR);}
 return fast_scale_word(78u);
}
// The camera's step from voxels to source leaves has its own scale
// (state[85], kept across encodes), never finer than the voxel cut: when
// source clusters bind, only that step coarsens and the voxel levels stay
// at the voxel cut (one shared scale gave the whole forest ~13 px voxels
// to keep about 25 trees in source, Joep 7 November).
fn fast_source_scale()->f32 {return max(fast_scale_word(85u),fast_scale(256u));}
// Shadow consumers may hold at most a quarter of the branch slots (state[81]
// counts theirs): grass shadows otherwise took 78% of slots from the camera.
fn fast_shadow_slots()->u32 {return limits().x/4u;}
// Booked camera splats per splat actually stored last frame (state[83]):
// bookings bound every direction, alpha and pixel size conservatively, so
// the camera may book this much past its splat buffer. 1 without feedback.
fn fast_splat_ratio()->f32 {let r=bitcast<f32>(atomicLoad(&counters[STATE_BASE+83]));return select(1.0,r,r>=1.0 && r<=8.0);}
fn fast_limits()->vec4<u32> {
 var cap=limits();cap.y=u32(min(f32(cap.y)*fast_splat_ratio(),4e9));return cap;
}
fn fast_reserve(delta:vec4<u32>,shadow:bool)->bool {
 let cap=fast_limits();var ok=true;var taken=vec4<u32>(0u);
 for(var k=0u;k<4u;k++) {
  if delta[k]==0u {continue;}
  let word=select(k,69u,k==3u);
  let before=atomicAdd(&counters[STATE_BASE+word],delta[k]);taken[k]=delta[k];
  if before>cap[k] || delta[k]>cap[k]-before {ok=false;break;}
 }
 var own=false;
 if ok && shadow && delta.x!=0u {
  let before=atomicAdd(&counters[STATE_BASE+81],delta.x);own=true;
  if before>fast_shadow_slots() || delta.x>fast_shadow_slots()-before {ok=false;}
 }
 if !ok {
  for(var k=0u;k<4u;k++) {if taken[k]!=0u {atomicSub(&counters[STATE_BASE+select(k,69u,k==3u)],taken[k]);}}
  if own {atomicSub(&counters[STATE_BASE+81],delta.x);}
 }
 return ok;
}
fn fast_refund(amount:vec4<u32>,shadow:bool) {
 for(var k=0u;k<4u;k++) {if amount[k]!=0u {atomicSub(&counters[STATE_BASE+select(k,69u,k==3u)],amount[k]);}}
 if shadow && amount.x!=0u {atomicSub(&counters[STATE_BASE+81],min(amount.x,atomicLoad(&counters[STATE_BASE+81])));}
}
fn fast_set_args(count:u32) {
 atomicStore(&counters[STATE_BASE+3],count);atomicMax(&counters[STATE_BASE+16],count);
 atomicStore(&counters[STATE_BASE+8],(count+63u)/64u);atomicStore(&counters[STATE_BASE+9],1u);atomicStore(&counters[STATE_BASE+10],1u);
}
@compute @workgroup_size(1) fn fast_mode() {atomicStore(&counters[STATE_BASE+80],1u);}
@compute @workgroup_size(1) fn fast_begin() {
 let count=atomicLoad(&counters[STATE_BASE+3]);fast_set_args(count);atomicStore(&counters[STATE_BASE+7],0u);
 atomicStore(&counters[STATE_BASE+89],(max(count,atomicLoad(&counters[STATE_BASE+29]))+63u)/64u);atomicStore(&counters[STATE_BASE+90],1u);atomicStore(&counters[STATE_BASE+91],1u);
}
// Each round runs twice (fast_round, then fast_round_late). First the camera and the
// shadow tasks of trees whose shadow reserved this deep last frame, then the
// other shadow tasks. The shadow splat cap is full in a forest (~37k refusals
// per frame): racing for it re-dealt ~3.5k trees' shadow mips every frame and
// dirtied the cached sun maps (2-12 ms spikes, Mancave 10 November). Now the
// trees holding the budget keep it; newcomers get what is left.
fn fast_shadow_retained(task:vec4<u32>)->bool {
 return (demand[task.x+view.counts.x].bucket>>16u)>atomicLoad(&counters[STATE_BASE+87]);
}
// The phase is the entry point, not a state word: a one-thread dispatch
// that flipped it cost two extra serial dispatches per round.
@compute @workgroup_size(64) fn fast_round(@builtin(global_invocation_id) id:vec3<u32>,@builtin(local_invocation_index) lane:u32) {fast_round_wave(id.x,lane,false);}
@compute @workgroup_size(64) fn fast_round_late(@builtin(global_invocation_id) id:vec3<u32>,@builtin(local_invocation_index) lane:u32) {fast_round_wave(id.x,lane,true);}
// Most tasks of a round keep their node (every far tree in round 0) and
// publish it. One global atomic per published record (counters[0], [7] for
// direct roots, [4] tested nodes) serialized ~160k adds on three words;
// a workgroup now reserves its records with one add each (WAVE_PUBLISH,
// KERN_REFINE_WAVE_PUBLISH=0 publishes per task as before).
var<workgroup> wave_counts:array<atomic<u32>,3>;
var<workgroup> wave_base:u32;
fn fast_round_wave(at:u32,lane:u32,phase_late:bool) {
 if !WAVE_PUBLISH {let task=fast_round_phase(at,phase_late);publish(task);return;}
 if lane<3u {atomicStore(&wave_counts[lane],0u);}
 workgroupBarrier();
 let task=fast_round_phase(at,phase_late);
 let slot=publish_reserve(task);
 workgroupBarrier();
 if lane==0u {
  wave_base=atomicAdd(&counters[0],atomicLoad(&wave_counts[0]));
  let direct_roots=atomicLoad(&wave_counts[1]);if direct_roots!=0u {atomicAdd(&counters[7],direct_roots);}
  let tested=atomicLoad(&wave_counts[2]);if tested!=0u {atomicAdd(&counters[4],tested);}
 }
 workgroupBarrier();
 if slot!=NONE {publish_write(task,wave_base+slot);}
}
// publish() in two halves: everything but the record slot, which the
// workgroup reserves at once; then the record write.
fn publish_reserve(task:vec4<u32>)->u32 {
 if task.z==0u {return NONE;}
 if task.y==NONE {atomicOr(&fallback[task.x],task.z);return NONE;}
 let n=nodes[task.y];let resident=ready(n.links.z);
 if !resident {atomicOr(&fallback[task.x],task.z);return NONE;}
 if (n.links.w&2u)!=0u {
  let publication=task.x+select(0u,view.counts.x,task.z==512u);
  demand[publication].prefix=n.links.z+1u;
  if task.z==256u || (demand[task.x].bucket&256u)==0u {direct[task.x]=n.links.z+1u;}
  atomicAdd(&wave_counts[1],1u);return NONE;
 }
 atomicAdd(&demand[task.x+select(0u,view.counts.x,task.z==512u)].count,1u);
 return atomicAdd(&wave_counts[0],1u);
}
fn publish_write(task:vec4<u32>,at:u32) {
 if at<view.counts.y {records[at]=vec4<u32>(task.x,task.y,nodes[task.y].links.z,trees[task.x].info.y);record_consumers[at]=task.z;}
 else {atomicOr(&fallback[task.x],1u);atomicAdd(&counters[3],1u);}
}
// Returns the task to publish (z 0: none). Children that are leaves still
// publish directly (few: near trees only).
fn fast_round_phase(at:u32,phase_late:bool)->vec4<u32> {
 if at>=atomicLoad(&counters[STATE_BASE+3]) {return vec4<u32>(0u);}
 let task=queue[queue_half.x+at];if task.z==0u {return vec4<u32>(0u);}
 let late=task.z==512u && !fast_shadow_retained(task);
 if late!=phase_late {return vec4<u32>(0u);}
 cut_scale=fast_scale(task.z);
 // Hysteresis: a tree the camera saw in detail last frame (more than one
 // record) keeps it down to CUT_HYSTERESIS times less error, so a small
 // turn or a cut-scale step no longer flips trees between voxels, levels
 // and mesh (Joep's playtest, 9 November: ~1k flips on a 20 degree turn).
 let detailed=(task.z&256u)!=0u && atomicLoad(&demand[task.x].failed)>1u;
 if detailed {cut_scale=cut_scale/CUT_HYSTERESIS;}
 let t=trees[task.x];let node_id=select(task.y,t.info.x,task.y==NONE);let n=nodes[node_id];
 if WAVE_PUBLISH {atomicAdd(&wave_counts[2],1u);} else {atomicAdd(&counters[4],1u);}
 let scale=projected(t,n,task.z);
 let wanted=task.y==NONE || (n.links.x!=NONE && ((n.links.w&1u)!=0u || scale.x>=1e29 || scale.x*n.error.x>view.pixels.z*cut_scale || scale.x*n.error.y>cut_scale));
 if !wanted {return task;}
 var first=node_id;var end=node_id+1u;
 if task.y!=NONE {first=n.links.x;end=n.links.y;}
 var replacement=vec4<u32>(0u);var resident=true;var branching=0u;var child=first;var source_leaves=false;
 // Pages for a coarse cut close to the camera come first: urgency is the
 // pixel error waiting on them (x16), x4 for camera tasks.
 request_urgency=u32(clamp(min(scale.x,1e8)*n.error.x*16.0*select(1.0,4.0,(task.z&256u)!=0u),1.0,1e9));
 loop {
  if child==NONE || child>=end {break;}
  let selected=candidate(task.x,child,true,task.z);let c=nodes[selected];let consumers=mask(sphere(t,c))&task.z;
  if consumers!=0u {
   resident=ready(c.links.z)&&resident;
   replacement=plus(replacement,work(task.x,selected,consumers));
   if c.links.x!=NONE {branching++;}
   if (c.links.w&4u)!=0u {source_leaves=true;}
  }
  child=nodes[child].links.y;
 }
 // Missing pages keep the complete parent; ready() has requested them.
 if !resident {atomicAdd(&counters[STATE_BASE+73],1u);return task;}
 // Into source leaves only past the coarser source cut. The step is judged
 // at the whole tree's nearest point (its root bound encloses every branch),
 // so a tree turns to needles as one: per branch, the far side of a near
 // crown and the branches that lost the budget race stayed 4-16 px voxel
 // balls among needles (Joep, 13 November). KERN_SOURCE_PER_TREE=0: per branch.
 var step=scale.x;
 if SOURCE_PER_TREE {step=max(step,projected(t,nodes[t.info.x],task.z).x);}
 if (task.z&256u)!=0u && source_leaves && task.y!=NONE && (n.links.w&1u)==0u && step<1e29 {
  let s=fast_source_scale()/select(1.0,CUT_HYSTERESIS,detailed);
  if step*n.error.x<=view.pixels.z*s && step*n.error.y<=s {return task;}
 }
 // Shadows stay voxel: source leaves in shadow maps cost hundreds of
 // millions of triangles (hill view) that no slot or splat cap measures.
 if (task.z&256u)==0u && source_leaves {return task;}
 let old=work(task.x,task.y,task.z);
 let shadow=(task.z&256u)==0u;
 if !fast_reserve(replacement-min(replacement,old),shadow) {atomicAdd(&counters[STATE_BASE+17],1u);atomicAdd(&counters[STATE_BASE+select(select(75u,76u,(task.z&256u)!=0u),86u,!shadow && source_leaves)],1u);return task;}
 if shadow {atomicMax(&demand[task.x+view.counts.x].failed,atomicLoad(&counters[STATE_BASE+87])+1u);}
 let offset=atomicAdd(&counters[STATE_BASE+7],branching);let capacity=queue_half.z;
 if offset>capacity || branching>capacity-offset {
  // Queue overflow: keep the parent and blank the claimed in-range entries.
  for(var i=offset;i<min(offset+branching,capacity);i++) {queue[queue_half.y+i]=vec4<u32>(0u,NONE,0u,0u);}
  fast_refund(replacement-min(replacement,old),shadow);atomicOr(&counters[STATE_BASE+15],8u);return task;
 }
 fast_refund(old-min(old,replacement),shadow);
 var out=offset;child=first;
 loop {
  if child==NONE || child>=end {break;}
  let selected=candidate(task.x,child,true,task.z);let c=nodes[selected];let consumers=mask(sphere(t,c))&task.z;
  if consumers!=0u {
   let replacement_task=vec4<u32>(task.x,selected,consumers,0u);
   if c.links.x==NONE {publish(replacement_task);}else{queue[queue_half.y+out]=replacement_task;out++;}
  }
  child=nodes[child].links.y;
 }
 return vec4<u32>(0u);
}
@compute @workgroup_size(1) fn fast_advance() {
 atomicAdd(&counters[STATE_BASE+87],1u);
 let count=min(atomicLoad(&counters[STATE_BASE+7]),queue_half.z);atomicStore(&counters[STATE_BASE+7],0u);
 if count!=0u {atomicAdd(&counters[STATE_BASE+68],1u);}
 fast_set_args(count);
 // The drain after the last round covers the live frontier (and any parked
 // tasks), not the whole work capacity: state[89..92] are its indirect args.
 atomicStore(&counters[STATE_BASE+89],(max(count,atomicLoad(&counters[STATE_BASE+29]))+63u)/64u);atomicStore(&counters[STATE_BASE+90],1u);atomicStore(&counters[STATE_BASE+91],1u);
}
// After the drain: compare the reserved totals with their caps. Coarsen fast
// when a cap refused work or is nearly full, refine back slowly when slack.
@compute @workgroup_size(1) fn fast_adapt() {
 // Last frame's bookings over the splats its raster stored (state[82], copied
 // from the first camera view), smoothed, with 15% margin for the next frame.
 let stored=atomicLoad(&counters[STATE_BASE+82]);let booked=atomicLoad(&counters[STATE_BASE+84]);
 var ratio=fast_splat_ratio();
 if stored>4096u && booked>0u {ratio=clamp(mix(ratio,0.85*f32(booked)/f32(stored),0.25),1.0,8.0);}
 atomicStore(&counters[STATE_BASE+83],bitcast<u32>(ratio));atomicStore(&counters[STATE_BASE+84],atomicLoad(&counters[STATE_BASE+1]));
 let used=used_now();let cap=max(fast_limits(),vec4<u32>(1u));
 let f=vec4<f32>(used)/vec4<f32>(cap);
 // Slots and source work are shared; splats are per class (y camera, z shadow).
 let shadow_slots=f32(atomicLoad(&counters[STATE_BASE+81]))/f32(max(fast_shadow_slots(),1u));
 // Source clusters drive the camera's source scale alone (state[85]).
 let fills=vec2<f32>(max(f.x,f.y),max(max(shadow_slots,f.z),f.w));
 for(var c=0u;c<2u;c++) {
  let word=79u-c;var s=fast_scale_word(word);
  let blocked=atomicLoad(&counters[STATE_BASE+76u-c])!=0u;
  if blocked || fills[c]>0.92 {s=s*1.06;}
  else if fills[c]<0.85 {s=max(select(1.0,CAMERA_CUT_FLOOR,c==0u),s/1.02);}
  atomicStore(&counters[STATE_BASE+word],bitcast<u32>(min(s,1e5)));
 }
 {
  var s=fast_scale_word(85u);let floor=fast_scale(256u);
  if atomicLoad(&counters[STATE_BASE+86])!=0u || f.w>0.92 {s=max(s,floor)*1.06;}
  else if f.w<0.85 {s=s/1.02;}
  atomicStore(&counters[STATE_BASE+85],bitcast<u32>(clamp(s,floor,1e5)));
 }
 atomicStore(&counters[STATE_BASE+74],bitcast<u32>(fills.x));atomicStore(&counters[STATE_BASE+77],bitcast<u32>(fills.y));
}
