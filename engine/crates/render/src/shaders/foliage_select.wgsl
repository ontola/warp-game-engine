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
@group(0) @binding(3) var<storage,read_write> records:array<vec4<u32>>;
@group(0) @binding(4) var<storage,read_write> deferred:array<vec2<u32>>;
@group(0) @binding(5) var<storage,read_write> counters:array<atomic<u32>>;
@group(0) @binding(6) var<storage,read_write> fallback:array<atomic<u32>>;
@group(0) @binding(7) var hzb:texture_2d<f32>;
@group(0) @binding(8) var<storage,read_write> direct_roots:array<u32>;
// Per-root demand and stable prefix within a projected-size priority bucket.
struct Demand { count:atomic<u32>, bucket:u32, prefix:u32, failed:atomic<u32> };
@group(0) @binding(9) var<storage,read_write> demand:array<Demand>;
@group(0) @binding(10) var<storage,read_write> bucket_cost:array<u32>;
struct WorkQueue { count:atomic<u32>, padding:array<u32,3>, tasks:array<vec2<u32>> };
@group(0) @binding(11) var<storage,read_write> input_work:WorkQueue;
@group(0) @binding(12) var<storage,read_write> output_work:WorkQueue;
@group(0) @binding(13) var<storage,read> child_list:array<u32>;
const NONE:u32=0xffffffffu;
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
fn visible(s:vec4<f32>)->bool {
 if view.consumer_count.x==0u {return visible_in(s,view.vp);}
 for(var i=0u;i<view.consumer_count.x;i++) {if visible_in(s,view.consumers[i].vp){return true;}}
 return false;
}
fn occluded(s:vec4<f32>,vp:mat4x4<f32>)->bool {
 var lo=vec2<f32>(1e30);var hi=vec2<f32>(-1e30);var nearest=0.0;
 for(var k=0u;k<8u;k++) {
  let sign=vec3<f32>(select(-1.0,1.0,(k&1u)!=0u),select(-1.0,1.0,(k&2u)!=0u),select(-1.0,1.0,(k&4u)!=0u));
  let q=vp*vec4<f32>(s.xyz+sign*s.w,1.0);
  if q.w<=0.00001 || q.z<=0.0 || q.z>=q.w-0.00001 {return false;}
  let uv=vec2<f32>(q.x/q.w*0.5+0.5,0.5-q.y/q.w*0.5);
  lo=min(lo,uv);hi=max(hi,uv);nearest=max(nearest,q.z/q.w);
 }
 // Offscreen bounds in the historical view cannot prove occlusion.
 if any(lo<vec2<f32>(0.0)) || any(hi>vec2<f32>(1.0)){return false;}
 let size=textureDimensions(hzb,0);
 let a=vec2<u32>(max(floor(lo*vec2<f32>(size))-1.0,vec2<f32>(0.0)));
 let b=min(vec2<u32>(ceil(hi*vec2<f32>(size))+1.0),size-1u);
 var mip=0u;
 loop {if mip+1u>=textureNumLevels(hzb) || all((b>>vec2<u32>(mip))-(a>>vec2<u32>(mip))<=vec2<u32>(1u)){break;} mip++;}
 let msize=textureDimensions(hzb,i32(mip));
 let first=min(a>>vec2<u32>(mip),msize-1u);let last=min(b>>vec2<u32>(mip),msize-1u);
 var minimum=1.0;
 for(var y=first.y;y<=last.y;y++){for(var x=first.x;x<=last.x;x++){
  minimum=min(minimum,textureLoad(hzb,vec2<i32>(i32(x),i32(y)),i32(mip)).x);
 }}
 return nearest+0.00002<minimum;
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
fn error_pixels(s:vec4<f32>,error:f32)->f32 {
 if view.consumer_count.x==0u {return error_in(s,error,view.vp,view.pixels.xy);}
 var result=0.0;
 for(var i=0u;i<view.consumer_count.x;i++) {
  let c=view.consumers[i];
  if visible_in(s,c.vp) {result=max(result,error_in(s,error,c.vp,c.pixels.xy));}
 }
 return result;
}
fn emit(tree:u32,node:u32) {
 if (nodes[node].links.w&2u)!=0u {direct_roots[tree]=nodes[node].links.z+1u;atomicAdd(&counters[7],1u);return;}
 let at=atomicAdd(&counters[0],1u);
 if at<view.counts.y {records[at]=vec4<u32>(tree,node,nodes[node].links.z,trees[tree].info.y);}
 else {atomicStore(&fallback[tree],1u);atomicAdd(&counters[3],1u);}
}
fn walk(tree:u32,start:u32,phase:u32) {
 let t=trees[tree];let end=nodes[start].links.y;var n=start;
 loop {
  if n>=end {break;}
  let node=nodes[n];let s=sphere(t,node);atomicAdd(&counters[4],1u);
  if !visible(s){atomicAdd(&counters[5],1u);n=node.links.y;continue;}
  let use_hzb=phase==2u || view.counts.w!=0u;
  var matrix=view.previous_vp;
  if phase==2u {matrix=view.vp;}
  if use_hzb && occluded(s,matrix) {
   if phase==2u {atomicAdd(&counters[6],1u);n=node.links.y;continue;}
   let at=atomicAdd(&counters[1],1u);
   if at<view.counts.z {deferred[at]=vec2<u32>(tree,n);n=node.links.y;continue;}
   // Queue overflow cannot hide geometry. Traverse without this historical
   // rejection; if output fills, the resident root is an explicit fallback.
   atomicAdd(&counters[2],1u);
  }
  if node.links.x==NONE || ((node.links.w&1u)==0u && error_pixels(s,node.error.x*t.scale.x)<=view.pixels.z) {
   emit(tree,n);n=node.links.y;
  }else{n=node.links.x;}
 }
}
@compute @workgroup_size(64) fn first(@builtin(global_invocation_id) id:vec3<u32>) {
 if id.x<view.counts.x {
  if view.consumer_count.y!=0u {
   let bucket=demand[id.x].bucket;let count=atomicLoad(&demand[id.x].count);
   var before=demand[id.x].prefix+bucket_cost[32u+(id.x/256u)*32u+bucket];
   for(var b=bucket+1u;b<32u;b++){before+=bucket_cost[b];}
   if atomicLoad(&demand[id.x].failed)!=0u || (count!=0u && before+count>view.consumer_count.y) {
    atomicStore(&fallback[id.x],2u);return;
   }
  }
  walk(id.x,trees[id.x].info.x,1u);
 }
}
@compute @workgroup_size(1) fn finish() {
 // [8..10] drives phase 2 without readback. [12..14] drives consumers.
 atomicStore(&counters[8],(min(atomicLoad(&counters[1]),view.counts.z)+63u)/64u);
 atomicStore(&counters[9],1u);atomicStore(&counters[10],1u);
 atomicStore(&counters[12],(min(atomicLoad(&counters[0]),view.counts.y)+63u)/64u);
 atomicStore(&counters[13],1u);atomicStore(&counters[14],1u);
}
@compute @workgroup_size(64) fn second(@builtin(global_invocation_id) id:vec3<u32>) {
 if id.x<min(atomicLoad(&counters[1]),view.counts.z) {
  let task=deferred[id.x];walk(task.x,task.y,2u);
 }
}

// Count the complete requested cut before writing any shared queue. Admission
// is deterministic (projected size bucket, then stable root ID), not atomic
// thread arrival order. Whole-root voxel cuts consume no branch slots.
@compute @workgroup_size(64) fn seed_demand(@builtin(global_invocation_id) id:vec3<u32>) {
 let tree=id.x;if tree>=view.counts.x{return;}
 let t=trees[tree];let root=nodes[t.info.x];
 let s=sphere(t,root);var priority=0.0;
 for(var i=0u;i<view.consumer_count.x;i++) {
  let c=view.consumers[i];
  // Perspective receivers get first claim; orthographic sun-only casters
  // retain their resident root when the shared branch budget is exhausted.
  if length(transpose(c.vp)[3].xyz)>0.00001 && visible_in(s,c.vp) {
   priority=max(priority,error_in(s,s.w,c.vp,c.pixels.xy));
  }
 }
 let bucket=u32(clamp(floor(log2(max(priority,1.0))),0.0,31.0));
 atomicStore(&demand[tree].count,0u);demand[tree].bucket=bucket;demand[tree].prefix=0u;
 atomicStore(&demand[tree].failed,0u);
 if visit(tree,t.info.x,false) {
  let slot=atomicAdd(&input_work.count,1u);
  input_work.tasks[slot]=vec2<u32>(tree,t.info.x);
 }
}
// GPU-produced breadth frontiers. Dispatch boundaries publish all task writes;
// no spin loops, CPU counts or device-wide synchronization inside a shader.
@compute @workgroup_size(1) fn prepare_wave() {
 let count=min(atomicLoad(&input_work.count),arrayLength(&input_work.tasks));
 atomicStore(&counters[8],min(count,32768u));
 atomicStore(&counters[9],(count+32767u)/32768u);atomicStore(&counters[10],1u);
 atomicStore(&output_work.count,0u);
}
// A frontier item is a subtree to expand, not a leaf. Lanes process its
// immediate child span together; only refined internal children re-enter the
// queue. This bounds storage by active branches rather than emitted needles.
fn visit(tree:u32,n:u32,write_records:bool)->bool {
 let node=nodes[n];let t=trees[tree];let s=sphere(t,node);
 if write_records {atomicAdd(&counters[4],1u);}
 if !visible(s){if write_records {atomicAdd(&counters[5],1u);}return false;}
 if node.links.x==NONE || ((node.links.w&1u)==0u && error_pixels(s,node.error.x*t.scale.x)<=view.pixels.z) {
  if write_records {emit(tree,n);}else if (node.links.w&2u)==0u {atomicAdd(&demand[tree].count,1u);}
  return false;
 }
 return true;
}
var<workgroup> expand:u32;
var<workgroup> task_shared:vec2<u32>;
fn wave(at:u32,lane:u32,write_records:bool) {
 if lane==0u {
  expand=0u;task_shared=vec2<u32>(0u);
  if at<min(atomicLoad(&input_work.count),arrayLength(&input_work.tasks)) {
   let task=input_work.tasks[at];task_shared=task;
   if atomicLoad(&fallback[task.x])==0u {expand=u32(visit(task.x,task.y,write_records));}
  }
 }
 let needs_expansion=workgroupUniformLoad(&expand);
 let task=workgroupUniformLoad(&task_shared);
 if needs_expansion==0u {return;}
 let start=child_list[task.y*2u];let count=child_list[task.y*2u+1u];
 for(var i=lane;i<count;i+=64u) {
  let child=child_list[start+i];
  if visit(task.x,child,write_records) {
   let slot=atomicAdd(&output_work.count,1u);
   atomicMax(&counters[15],slot+1u);
   if slot<arrayLength(&output_work.tasks) {output_work.tasks[slot]=vec2<u32>(task.x,child);}
   else {atomicStore(&fallback[task.x],3u);atomicStore(&demand[task.x].failed,1u);atomicAdd(&counters[11],1u);}
  }
 }
}
@compute @workgroup_size(64) fn count_wave(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32){wave(group.x+group.y*32768u,lane,false);}
@compute @workgroup_size(64) fn emit_wave(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32){wave(group.x+group.y*32768u,lane,true);}
@compute @workgroup_size(64) fn seed_emit(@builtin(global_invocation_id) id:vec3<u32>) {
 let tree=id.x;if tree>=view.counts.x{return;}
 let bucket=demand[tree].bucket;let count=atomicLoad(&demand[tree].count);
 var before=demand[tree].prefix+bucket_cost[32u+(tree/256u)*32u+bucket];
 for(var b=bucket+1u;b<32u;b++){before+=bucket_cost[b];}
 if atomicLoad(&fallback[tree])!=0u {return;}
 if atomicLoad(&demand[tree].failed)!=0u || (count!=0u && before+count>view.consumer_count.y) {
  atomicStore(&fallback[tree],2u);return;
 }
 if visit(tree,trees[tree].info.x,true) {
  let slot=atomicAdd(&input_work.count,1u);
  // At most one seed per root; construction guarantees queue >= root count.
  input_work.tasks[slot]=vec2<u32>(tree,trees[tree].info.x);
 }
}
@compute @workgroup_size(32) fn prefix_demand(@builtin(local_invocation_index) bucket:u32,@builtin(workgroup_id) block:vec3<u32>) {
 var sum=0u;let end=min((block.x+1u)*256u,view.counts.x);
 for(var tree=block.x*256u;tree<end;tree++) {
  if demand[tree].bucket==bucket {
   let count=atomicLoad(&demand[tree].count);
   if count>view.consumer_count.y {atomicStore(&demand[tree].failed,1u);}
   demand[tree].prefix=sum;
   if atomicLoad(&demand[tree].failed)==0u {sum+=count;}
  }
 }
 bucket_cost[32u+block.x*32u+bucket]=sum;
}
@compute @workgroup_size(32) fn prefix_buckets(@builtin(local_invocation_index) bucket:u32) {
 var sum=0u;
 for(var block=0u;block<(view.counts.x+255u)/256u;block++) {
  let index=32u+block*32u+bucket;let count=bucket_cost[index];
  bucket_cost[index]=sum;sum+=count;
 }
 bucket_cost[bucket]=sum;
}
