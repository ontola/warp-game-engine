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
struct Proposal {old:vec4<u32>,replacement:vec4<u32>,prefix:vec4<u32>,control:vec4<u32>,output:vec4<u32>};
@group(0) @binding(4) var<storage,read_write> proposals:array<Proposal>;
// task: root, node (NONE means emergency proxy), consumer mask, dormant index+2 (0=new).
@group(0) @binding(5) var<storage,read_write> current:array<vec4<u32>>;
@group(0) @binding(6) var<storage,read_write> next:array<vec4<u32>>;
// 32 global priority totals, followed by 32 buckets per 256-task block.
@group(0) @binding(7) var<storage,read_write> blocks:array<vec4<u32>>;
@group(0) @binding(8) var<storage,read_write> records:array<vec4<u32>>;
@group(0) @binding(9) var<storage,read_write> counters:array<atomic<u32>>;
@group(0) @binding(10) var<storage,read_write> fallback:array<atomic<u32>>;
@group(0) @binding(11) var<storage,read_write> direct:array<u32>;
struct Demand {count:atomic<u32>,bucket:u32,prefix:u32,failed:atomic<u32>};
@group(0) @binding(12) var<storage,read_write> demand:array<Demand>;
// used xyz, task count; snapshot xyz, next count; task and scan indirect args;
// fault at15, peak16, retained-for-budget17. Never read back to schedule work.
@group(0) @binding(13) var<storage,read_write> state:array<atomic<u32>>;
struct Parked { task:vec4<u32>, proposal:Proposal };
@group(0) @binding(14) var<storage,read_write> parked:array<Parked>;
@group(0) @binding(15) var<storage,read_write> record_consumers:array<u32>;
const NONE:u32=0xffffffffu;
fn ready(rep:u32)->bool {
 if atomicLoad(&pages[rep].requested)==0u {atomicOr(&pages[rep].requested,1u);}
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
fn work(tree:u32,node:u32,consumers:u32)->vec4<u32> {
 var cost=vec4<u32>(0u);
 if node==NONE {cost.y=u32(trees[tree].scale.z);cost.z=u32(trees[tree].scale.w);}
 else {let n=nodes[node];cost=vec4<u32>(select(1u,0u,(n.links.w&2u)!=0u),u32(n.error.z),u32(n.error.w),0u);}
 if (consumers&256u)==0u {cost.y=0u;}if (consumers&512u)==0u {cost.z=0u;}
 return cost;
}
fn add_used(cost:vec4<u32>) {
 for(var k=0u;k<3u;k++) {
  let old=atomicAdd(&state[k],cost[k]);if old>NONE-cost[k] {atomicOr(&state[15],1u);}
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
  demand[tree].bucket=consumers;demand[tree].prefix=0u;
  demand[tree+view.counts.x].bucket=consumers&512u;demand[tree+view.counts.x].prefix=0u;
  atomicStore(&demand[tree].count,0u);atomicStore(&demand[tree].failed,0u);
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
  if sum.y==NONE || sum.z==NONE {atomicOr(&state[15],1u);}
  add_used(vec4<u32>(0u,sum.yz,0u));
 }
}
@compute @workgroup_size(1) fn start() {
 var count=0u;
 for(var group=0u;group<(view.counts.x+255u)/256u;group++) {
  let at=32u+group*32u;let n=blocks[at].x;blocks[at].y=count;count+=n;
 }
 atomicStore(&state[3],count);atomicStore(&state[16],count);
 if atomicLoad(&state[1])>view.consumer_count.z || atomicLoad(&state[2])>view.consumer_count.w {atomicOr(&state[15],4u);}
}
@compute @workgroup_size(64) fn seed_scatter(@builtin(global_invocation_id) id:vec3<u32>) {
 let tree=id.x;if tree>=view.counts.x {return;}
 let roles=demand[tree].bucket;var at=blocks[32u+(tree/256u)*32u].y+atomicLoad(&demand[tree+view.counts.x].failed);
 if (roles&256u)!=0u {current[at]=vec4<u32>(tree,NONE,256u,0u);at++;}
 if (roles&512u)!=0u {current[at]=vec4<u32>(tree,NONE,512u,0u);}
}
@compute @workgroup_size(1) fn prepare() {
 let count=atomicLoad(&state[3]);
 for(var k=0u;k<3u;k++) {atomicStore(&state[4u+k],atomicLoad(&state[k]));}
 atomicStore(&state[7],0u);atomicStore(&state[18],0u);
 atomicStore(&state[8],(count+63u)/64u);atomicStore(&state[9],1u);atomicStore(&state[10],1u);
 atomicStore(&state[12],(count+255u)/256u);atomicStore(&state[13],1u);atomicStore(&state[14],1u);
 // A dormant queue is revisited only after the active frontier drains and
 // capacity or new pending work changed since its last wake attempt.
 let used=vec3<u32>(atomicLoad(&state[0]),atomicLoad(&state[1]),atomicLoad(&state[2]));
 let previous=vec3<u32>(atomicLoad(&state[56]),atomicLoad(&state[57]),atomicLoad(&state[58]));
 let parked_count=atomicLoad(&state[29]);
 let wake=count==0u && (atomicLoad(&state[15])&3u)==0u && (parked_count!=atomicLoad(&state[59]) || any(used!=previous) || atomicLoad(&state[30])!=0u);
 atomicStore(&state[55],select(0u,1u,wake));
 atomicStore(&state[64],select(0u,(parked_count+255u)/256u,wake));atomicStore(&state[65],1u);atomicStore(&state[66],1u);
 if wake {for(var k=0u;k<3u;k++) {atomicStore(&state[56u+k],used[k]);}atomicStore(&state[59],parked_count);atomicAdd(&state[62],1u);}

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
  if at==best && (n.links.w&1u)==0u && scale.x<1e29 && scale.x*n.error.x<=view.pixels.z && (n.error.y==0.0 || scale.x*n.error.y<=1.0) {break;}
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
@compute @workgroup_size(64) fn propose(@builtin(global_invocation_id) id:vec3<u32>) {
 let at=id.x;if at>=atomicLoad(&state[3]) {return;}
 let task=current[at];var p:Proposal;
 // Cache lives only within this encode: camera, pages and nodes are immutable.
 // Available budget changes, so admission is always rechecked.
 if task.w>=2u {
  let cached=parked[task.w-2u].proposal;p.old=cached.old;p.replacement=cached.replacement;
  p.control=vec4<u32>(cached.control.x,1u,cached.output.y,0u);p.output.y=cached.output.y;
  let used=vec3<u32>(atomicLoad(&state[4]),atomicLoad(&state[5]),atomicLoad(&state[6]));
  let capacity=view.consumer_count.yzw;let delta=p.replacement-min(p.replacement,p.old);
  if any(delta.xyz>capacity-min(used,capacity)) {p.control.y=2u;atomicAdd(&state[17],1u);}
  else if (task.z&256u)!=0u {atomicOr(&state[18],1u);}
  atomicAdd(&state[28],1u);proposals[at]=p;return;
 }
 if task.z==0u {proposals[at]=p;return;}
 let t=trees[task.x];let node_id=select(task.y,t.info.x,task.y==NONE);let n=nodes[node_id];
 p.old=work(task.x,task.y,task.z);atomicAdd(&counters[4],1u);
 let scale=projected(t,n,task.z);
 var refine=task.y==NONE || (n.links.x!=NONE && ((n.links.w&1u)!=0u || scale.x>=1e29 || scale.x*n.error.x>view.pixels.z || scale.x*n.error.y>1.0));
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
 // All visible siblings must fit together. Missing pages keep the current parent.
 if refine {
  let delta=p.replacement-min(p.replacement,p.old);
  let used=vec3<u32>(atomicLoad(&state[4]),atomicLoad(&state[5]),atomicLoad(&state[6]));
  let capacity=view.consumer_count.yzw;
  // An individually impossible proposal must not reserve prefix space and
  // starve cheaper candidates. Retain its parent without entering the scan.
  let individually_fits=all(delta.xyz<=capacity-min(used,capacity));
  if !individually_fits {atomicAdd(&state[17],1u);}
  if individually_fits && (task.z&256u)!=0u {atomicOr(&state[18],1u);}
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
@compute @workgroup_size(256) fn priority_scan(@builtin(workgroup_id) block:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
 let start=block.x*256u;let end=min(start+256u,atomicLoad(&state[3]));
 var item=vec4<u32>(0u);
 if start+lane<end {
  let p=proposals[start+lane];
  item=vec4<u32>((p.replacement-min(p.replacement,p.old)).xyz,select(0u,p.control.x+1u,p.control.y==1u));
 }
 priority_items[lane]=item;workgroupBarrier();
 if lane>=32u {return;}
 var sum=vec4<u32>(0u);
 for(var offset=0u;offset<end-start;offset++) {
  let value=priority_items[offset];if value.w==lane+1u {
   proposals[start+offset].prefix=sum;sum=plus(sum,vec4<u32>(value.xyz,0u));
  }
 }
 blocks[32u+block.x*32u+lane]=sum;
}
@compute @workgroup_size(32) fn priority_totals(@builtin(local_invocation_index) bucket:u32) {
 var sum=vec4<u32>(0u);
 for(var b=0u;b<(atomicLoad(&state[3])+255u)/256u;b++) {
  let at=32u+b*32u+bucket;let count=blocks[at];blocks[at]=sum;sum=plus(sum,count);
 }
 blocks[bucket]=sum;
}
@compute @workgroup_size(64) fn decide(@builtin(global_invocation_id) id:vec3<u32>) {
 let at=id.x;if at>=atomicLoad(&state[3]) {return;}
 let p=proposals[at];if p.control.y==0u {return;}
 if p.control.y==2u {proposals[at].control.w=2u;return;}
 // Only branch slots are shared across consumer budgets. A shadow-only
 // raster upgrade with no additional slot demand cannot starve the camera,
 // and admission to a resident ancestor must not wait behind camera work.
 if (current[at].z&256u)==0u && atomicLoad(&state[18])!=0u && p.replacement.x>p.old.x {
  proposals[at].control.w=2u;return;
 }
 var before=plus(p.prefix,blocks[32u+(at/256u)*32u+p.control.x]);
 for(var b=p.control.x+1u;b<32u;b++) {before=plus(before,blocks[b]);}
 let delta=p.replacement-min(p.replacement,p.old);
 let used=vec4<u32>(atomicLoad(&state[4]),atomicLoad(&state[5]),atomicLoad(&state[6]),0u);
 let available=vec4<u32>(view.consumer_count.y,view.consumer_count.z,view.consumer_count.w,0u);
 // Zero/negative-cost replacements remain allowed when a baseline exceeds a cap.
 let cost=plus(before,delta);let fits=all(select(cost.xyz<=available.xyz-min(used.xyz,available.xyz),vec3<bool>(true),delta.xyz==vec3<u32>(0u)));
 if fits && (atomicLoad(&state[15])&3u)==0u {proposals[at].control.w=1u;}
 else {proposals[at].control.w=2u;atomicAdd(&state[17],1u);}
}
@compute @workgroup_size(1) fn output_scan(@builtin(workgroup_id) block:vec3<u32>) {
 var sum=0u;var asleep=0u;let end=min((block.x+1u)*256u,atomicLoad(&state[3]));
 for(var at=block.x*256u;at<end;at++) {
  proposals[at].output.x=sum;proposals[at].output.z=asleep;
  if proposals[at].control.w==1u {sum+=proposals[at].control.z;}
  if proposals[at].control.w==2u && current[at].w<2u {asleep++;}
 }
 blocks[32u+block.x*32u]=vec4<u32>(sum,0u,asleep,0u);
}
@compute @workgroup_size(1) fn output_totals() {
 var sum=0u;var asleep=0u;
 for(var b=0u;b<(atomicLoad(&state[3])+255u)/256u;b++) {
  let at=32u+b*32u;let count=blocks[at];blocks[at].y=sum;blocks[at].w=asleep;
  sum+=count.x;asleep+=count.z;
 }
 let base=atomicLoad(&state[29]);atomicStore(&state[31],base);
 atomicStore(&state[29],min(base+asleep,arrayLength(&parked)));
 if asleep>arrayLength(&parked)-base {atomicAdd(&state[63],asleep-(arrayLength(&parked)-base));}
 atomicStore(&state[7],sum);atomicMax(&state[16],sum);
 if sum>arrayLength(&next) {atomicOr(&state[15],2u);}
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
 if current[at].w>=2u {return current[at].w-2u;}
 return atomicLoad(&state[31])+blocks[32u+(at/256u)*32u].w+p.output.z;
}
@compute @workgroup_size(64) fn apply(@builtin(global_invocation_id) id:vec3<u32>) {
 let at=id.x;if at>=atomicLoad(&state[3]) {return;}
 let p=proposals[at];let task=current[at];
 if p.control.w==2u && (atomicLoad(&state[15])&2u)==0u {
  var destination=state_park_index(at,p);
  if destination<arrayLength(&parked) {
   parked[destination].task=vec4<u32>(task.xyz,1u);parked[destination].proposal=p;
   atomicAdd(&state[61],1u);
  }else {publish(task);}
  return;
 }
 if p.control.w==0u || (atomicLoad(&state[15])&2u)!=0u {publish(task);return;}
 // No writes to children until every candidate's output range is reserved.
 for(var k=0u;k<3u;k++) {
  if p.replacement[k]>=p.old[k] {atomicAdd(&state[k],p.replacement[k]-p.old[k]);}
  else {atomicSub(&state[k],p.old[k]-p.replacement[k]);}
 }
 var offset=p.output.x+blocks[32u+(at/256u)*32u].y;
 let t=trees[task.x];var child=t.info.x;var end=child+1u;
 if task.y!=NONE {child=nodes[task.y].links.x;end=nodes[task.y].links.y;}
 loop {
  if child==NONE || child>=end {break;}
  let selected=candidate(task.x,child,true,task.z);let c=nodes[selected];let consumers=mask(sphere(t,c))&task.z;
  if consumers!=0u {
   let replacement=vec4<u32>(task.x,selected,consumers,0u);
   if c.links.x==NONE {publish(replacement);}else{next[offset]=replacement;offset++;}
  }
  child=nodes[child].links.y;
 }
}
@compute @workgroup_size(1) fn advance() {
 var count=atomicLoad(&state[7]);if (atomicLoad(&state[15])&2u)!=0u {count=0u;atomicAdd(&counters[11],1u);}
 atomicStore(&state[3],count);atomicStore(&counters[15],atomicLoad(&state[16]));
}
// A depth-bounded final drain retains the current cut rather than dropping work.
@compute @workgroup_size(64) fn drain(@builtin(global_invocation_id) id:vec3<u32>) {
 if id.x<atomicLoad(&state[3]) {publish(current[id.x]);}
 if id.x<atomicLoad(&state[29]) && parked[id.x].task.w==1u {publish(parked[id.x].task);}
}
// Stable wake compaction is skipped entirely while active traversal continues.
fn wakeable(at:u32)->bool {
 if parked[at].task.w!=1u {return false;}
 let p=parked[at].proposal;let delta=p.replacement-min(p.replacement,p.old);
 let used=vec3<u32>(atomicLoad(&state[0]),atomicLoad(&state[1]),atomicLoad(&state[2]));
 return all(delta.xyz<=view.consumer_count.yzw-min(used,view.consumer_count.yzw));
}
var<workgroup> wake_scan:array<vec2<u32>,256>;
@compute @workgroup_size(256) fn wake_count(@builtin(workgroup_id) block:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
 let at=block.x*256u+lane;let valid=at<atomicLoad(&state[29]);var selected=vec2<u32>(0u);
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
 if atomicLoad(&state[55])==0u {return;}
 var sum=vec2<u32>(0u);
 for(var b=0u;b<(atomicLoad(&state[29])+255u)/256u;b++) {
  let at=32u+b*32u;let count=blocks[at].xy;blocks[at].z=sum.x;blocks[at].w=sum.y;sum+=count;
 }
 // Match camera-priority admission without repeatedly waking shadow work that
 // decide would immediately park again. Remember it even if camera work is free.
 let camera=sum.x!=0u;let count=select(sum.y,sum.x,camera);
 atomicStore(&state[30],select(0u,1u,camera && sum.y!=0u));
 atomicStore(&state[67],select(2u,1u,camera));
 atomicStore(&state[3],count);atomicAdd(&state[60],count);
 atomicStore(&state[8],(count+63u)/64u);atomicStore(&state[12],(count+255u)/256u);
}
@compute @workgroup_size(256) fn wake_scatter(@builtin(workgroup_id) block:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
 let at=block.x*256u+lane;if at>=atomicLoad(&state[29]) {return;}
 let role=atomicLoad(&state[67]);
 if parked[at].proposal.prefix.w==role {
  let base=select(blocks[32u+block.x*32u].w,blocks[32u+block.x*32u].z,role==1u);
  let offset=base+parked[at].proposal.output.w;
  current[offset]=vec4<u32>(parked[at].task.xyz,at+2u);parked[at].task.w=0u;
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
 atomicAdd(&state[48],1u);
 if n.links.x==NONE {atomicAdd(&state[53],1u);return;}
 let t=trees[tree];var replacement=vec4<u32>(0u);var missing=false;var child=n.links.x;
 loop {
  if child==NONE || child>=n.links.y {break;}
  let selected=candidate(tree,child,false,256u);let c=nodes[selected];let consumers=mask(sphere(t,c))&256u;
  if consumers!=0u {replacement=plus(replacement,work(tree,selected,consumers));missing=missing||pages[c.links.z].resident==0u;}
  child=nodes[child].links.y;
 }
 let old=work(tree,node,mask(sphere(t,n))&256u);let delta=replacement-min(replacement,old);
 let used=vec3<u32>(atomicLoad(&state[0]),atomicLoad(&state[1]),atomicLoad(&state[2]));
 let capacity=view.consumer_count.yzw;let blocked=delta.xyz>capacity-min(used,capacity);
 if blocked.x {atomicAdd(&state[49],1u);}if blocked.y {atomicAdd(&state[50],1u);}if blocked.z {atomicAdd(&state[51],1u);}
 if missing {atomicAdd(&state[52],1u);}if !any(blocked) && !missing {atomicAdd(&state[54],1u);}
}
fn audit_node(tree:u32,node:u32,roles:u32) {
 let n=nodes[node];let scale=projected(trees[tree],n,roles);
 audit_camera_limit(tree,node,scale.y);
 let spatial=min(scale*n.error.x,vec2<f32>(1e30));
 let ratio=min(max(spatial/max(view.pixels.z,1e-8),scale*n.error.y),vec2<f32>(1e30));
 atomicMax(&state[19],bitcast<u32>(spatial.x));atomicMax(&state[20],bitcast<u32>(spatial.y));
 atomicMax(&state[21],bitcast<u32>(ratio.x));atomicMax(&state[22],bitcast<u32>(ratio.y));
 if ratio.x>1.0 {atomicAdd(&state[23],1u);}if ratio.y>1.0 {atomicAdd(&state[24],1u);}
 if (n.links.w&1u)!=0u {atomicAdd(&state[27],1u);}
}
@compute @workgroup_size(64) fn audit(@builtin(global_invocation_id) id:vec3<u32>) {
 let at=id.x;
 if at<min(atomicLoad(&counters[0]),view.counts.y) {
  let r=records[at];let n=nodes[r.y];let failure=atomicLoad(&fallback[r.x]);var roles=record_consumers[at]&~failure;
  if (failure&7u)!=0u {roles=0u;}if roles!=0u {audit_node(r.x,r.y,roles);
  let base=select(36u,32u,(n.links.w&4u)!=0u);
  atomicAdd(&state[base+(roles>>8u)],1u);}
 }
 if at>=view.counts.x {return;}
 let failure=atomicLoad(&fallback[at]);if failure!=0u {atomicAdd(&state[25],1u);}
 let first=trees[at].info.x;let end=nodes[first].links.y;
 for(var role=0u;role<2u;role++) {
  let rep=demand[at+role*view.counts.x].prefix;if rep==0u {continue;}
  let roles=256u<<role;if (failure&(roles|7u))!=0u {continue;}var found=false;
  for(var n=first;n<end;n++) {
   if nodes[n].links.z+1u==rep && (nodes[n].links.w&2u)!=0u {audit_node(at,n,roles);atomicAdd(&state[40u+(roles>>8u)],1u);found=true;break;}
  }
  if !found {atomicAdd(&state[26],1u);}
 }
}
