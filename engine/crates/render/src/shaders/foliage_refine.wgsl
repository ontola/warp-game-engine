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
// task: root, node (NONE means emergency proxy), consumer mask, padding.
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
@compute @workgroup_size(64) fn seed(@builtin(global_invocation_id) id:vec3<u32>) {
 let tree=id.x;if tree>=view.counts.x {return;}
 let root=trees[tree];var consumers=0u;
 if root.info.w==0u {consumers=mask(sphere(root,nodes[root.info.x]));}
 current[tree]=vec4<u32>(tree,NONE,consumers,0u);
 demand[tree].bucket=consumers;demand[tree].prefix=0u;
 atomicStore(&demand[tree].count,0u);atomicStore(&demand[tree].failed,0u);
 if consumers!=0u {add_used(work(tree,NONE,consumers));}
}
@compute @workgroup_size(1) fn start() {
 atomicStore(&state[3],view.counts.x);atomicStore(&state[16],view.counts.x);
 if atomicLoad(&state[1])>view.consumer_count.z || atomicLoad(&state[2])>view.consumer_count.w {atomicOr(&state[15],4u);}
}
@compute @workgroup_size(1) fn prepare() {
 let count=atomicLoad(&state[3]);
 for(var k=0u;k<3u;k++) {atomicStore(&state[4u+k],atomicLoad(&state[k]));}
 atomicStore(&state[7],0u);atomicStore(&state[18],0u);
 atomicStore(&state[8],(count+63u)/64u);atomicStore(&state[9],1u);atomicStore(&state[10],1u);
 atomicStore(&state[12],(count+255u)/256u);atomicStore(&state[13],1u);atomicStore(&state[14],1u);
}
fn projected(t:Tree,n:Node)->vec2<f32> {
 let s=sphere(t,n);var all_scale=0.0;var camera_scale=0.0;
 for(var i=0u;i<view.consumer_count.x;i++) {
  let c=view.consumers[i];if visible_in(s,c.vp) {
   let p=error_in(s,1.0,c.vp,c.pixels.xy);all_scale=max(all_scale,p);
   if c.pixels.z==0.0 {camera_scale=max(camera_scale,p);}
  }
 }
 return vec2<f32>(all_scale,camera_scale)*t.scale.x;
}
// Free unary transitions do not need global reservations or frontier storage.
// Stop before a missing page, any increased work, or an admitted error target.
fn candidate(tree:u32,start:u32)->u32 {
 let t=trees[tree];var at=start;
 loop {
  let n=nodes[at];if n.links.x==NONE {break;}
  let child=n.links.x;let c=nodes[child];if c.links.y!=n.links.y {break;}
  let scale=projected(t,n);
  if (n.links.w&1u)==0u && scale.x<1e29 && scale.x*n.error.x<=view.pixels.z && (n.error.y==0.0 || scale.x*n.error.y<=1.0) {break;}
  let consumers=mask(sphere(t,n));let next_mask=mask(sphere(t,c));
  if any(work(tree,child,next_mask)>work(tree,at,consumers)) {break;}
  if !ready(c.links.z) {break;}
  at=child;
 }
 return at;
}
@compute @workgroup_size(64) fn propose(@builtin(global_invocation_id) id:vec3<u32>) {
 let at=id.x;if at>=atomicLoad(&state[3]) {return;}
 let task=current[at];var p:Proposal;
 if task.z==0u {proposals[at]=p;return;}
 let t=trees[task.x];let node_id=select(task.y,t.info.x,task.y==NONE);let n=nodes[node_id];
 p.old=work(task.x,task.y,task.z);atomicAdd(&counters[4],1u);
 let scale=projected(t,n);
 var refine=task.y==NONE || (n.links.x!=NONE && ((n.links.w&1u)!=0u || scale.x>=1e29 || scale.x*n.error.x>view.pixels.z || scale.x*n.error.y>1.0));
 var child=node_id;var end=node_id+1u;
 if task.y!=NONE {child=n.links.x;end=n.links.y;}
 var children=0u;var child_error=0.0;
 if refine {
  loop {
   if child==NONE || child>=end {break;}
   let selected=candidate(task.x,child);let c=nodes[selected];let consumers=mask(sphere(t,c));
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
  if (task.z&256u)!=0u {atomicOr(&state[18],1u);}
  let delta=p.replacement-min(p.replacement,p.old);
  let price=max(1.0,f32(delta.x)*256.0+f32(delta.y)+f32(delta.z));
  // Mandatory root fallbacks carry zero authored error. Their projected size
  // still determines benefit; otherwise all roots tie regardless of distance.
  let parent_error=select(n.error.x,n.sphere.w,task.y==NONE || (n.links.w&1u)!=0u);
  let priority_scale=select(scale.x,scale.y,(task.z&256u)!=0u);
  let benefit=max(0.001,priority_scale*max(parent_error-child_error,parent_error*0.01));
  let bucket=u32(clamp(floor(log2(max(benefit/price,1e-20)))+12.0,0.0,15.0));
  p.control=vec4<u32>(bucket+select(0u,16u,(task.z&256u)!=0u),1u,children,0u);
 }
 proposals[at]=p;
}
@compute @workgroup_size(32) fn priority_scan(@builtin(workgroup_id) block:vec3<u32>,@builtin(local_invocation_index) bucket:u32) {
 var sum=vec4<u32>(0u);let end=min((block.x+1u)*256u,atomicLoad(&state[3]));
 for(var at=block.x*256u;at<end;at++) {
  let p=proposals[at];if p.control.y!=0u && p.control.x==bucket {
   proposals[at].prefix=sum;sum=plus(sum,p.replacement-min(p.replacement,p.old));
  }
 }
 blocks[32u+block.x*32u+bucket]=sum;
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
 // Keep shadow-only work pending while any camera refinement remains. A coarse
 // shadow upgrade cannot consume slots needed by a later camera hierarchy level.
 if (current[at].z&256u)==0u && atomicLoad(&state[18])!=0u {
  proposals[at].control.w=2u;proposals[at].control.z=1u;return;
 }
 var before=plus(p.prefix,blocks[32u+(at/256u)*32u+p.control.x]);
 for(var b=p.control.x+1u;b<32u;b++) {before=plus(before,blocks[b]);}
 let delta=p.replacement-min(p.replacement,p.old);
 let used=vec4<u32>(atomicLoad(&state[4]),atomicLoad(&state[5]),atomicLoad(&state[6]),0u);
 let available=vec4<u32>(view.consumer_count.y,view.consumer_count.z,view.consumer_count.w,0u);
 // Zero/negative-cost replacements remain allowed when a baseline exceeds a cap.
 let cost=plus(before,delta);let fits=all(select(cost.xyz<=available.xyz-min(used.xyz,available.xyz),vec3<bool>(true),delta.xyz==vec3<u32>(0u)));
 if fits && (atomicLoad(&state[15])&3u)==0u {proposals[at].control.w=1u;}
 else {atomicAdd(&state[17],1u);}
}
@compute @workgroup_size(1) fn output_scan(@builtin(workgroup_id) block:vec3<u32>) {
 var sum=0u;let end=min((block.x+1u)*256u,atomicLoad(&state[3]));
 for(var at=block.x*256u;at<end;at++) {
  proposals[at].output.x=sum;
  if proposals[at].control.w!=0u {sum+=proposals[at].control.z;}
 }
 blocks[32u+block.x*32u]=vec4<u32>(sum,0u,0u,0u);
}
@compute @workgroup_size(1) fn output_totals() {
 var sum=0u;
 for(var b=0u;b<(atomicLoad(&state[3])+255u)/256u;b++) {
  let at=32u+b*32u;let count=blocks[at].x;blocks[at].y=sum;sum+=count;
 }
 atomicStore(&state[7],sum);atomicMax(&state[16],sum);
 if sum>arrayLength(&next) {atomicOr(&state[15],2u);}
}
fn publish(task:vec4<u32>) {
 if task.z==0u {return;}
 if task.y==NONE {atomicStore(&fallback[task.x],2u);return;}
 let n=nodes[task.y];let resident=ready(n.links.z);
 if !resident {atomicStore(&fallback[task.x],4u);return;}
 if (n.links.w&2u)!=0u {direct[task.x]=n.links.z+1u;atomicAdd(&counters[7],1u);return;}
 let at=atomicAdd(&counters[0],1u);atomicAdd(&demand[task.x].count,1u);
 if at<view.counts.y {records[at]=vec4<u32>(task.x,task.y,n.links.z,trees[task.x].info.y);}
 else {atomicStore(&fallback[task.x],1u);atomicAdd(&counters[3],1u);}
}
@compute @workgroup_size(64) fn apply(@builtin(global_invocation_id) id:vec3<u32>) {
 let at=id.x;if at>=atomicLoad(&state[3]) {return;}
 let p=proposals[at];let task=current[at];
 if p.control.w==2u && (atomicLoad(&state[15])&2u)==0u {
  next[p.output.x+blocks[32u+(at/256u)*32u].y]=task;return;
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
  let selected=candidate(task.x,child);let c=nodes[selected];let consumers=mask(sphere(t,c));
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
