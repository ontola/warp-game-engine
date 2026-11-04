// Experimental shared-recipe traversal. No CPU near-tree shortlist.
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
@group(0) @binding(9) var<storage,read_write> demand:array<vec4<u32>>;
@group(0) @binding(10) var<storage,read_write> bucket_cost:array<u32>;
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
   let d=demand[id.x];var before=d.z+bucket_cost[32u+(id.x/256u)*32u+d.y];
   for(var b=d.y+1u;b<32u;b++){before+=bucket_cost[b];}
   if d.w!=0u || (d.x!=0u && before+d.x>view.consumer_count.y) {
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
@compute @workgroup_size(64) fn count_demand(@builtin(global_invocation_id) id:vec3<u32>) {
 let tree=id.x;if tree>=view.counts.x{return;}
 let t=trees[tree];let root=nodes[t.info.x];let end=root.links.y;var n=t.info.x;var count=0u;
 loop {
  if n>=end {break;}
  let node=nodes[n];let s=sphere(t,node);
  if !visible(s){n=node.links.y;continue;}
  if node.links.x==NONE || ((node.links.w&1u)==0u && error_pixels(s,node.error.x*t.scale.x)<=view.pixels.z) {
   if (node.links.w&2u)==0u {count++;}
   n=node.links.y;
  }else{n=node.links.x;}
 }
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
 let oversize=u32(count>view.consumer_count.y);
 demand[tree]=vec4<u32>(select(count,0u,oversize!=0u),bucket,0u,oversize);
}
@compute @workgroup_size(32) fn prefix_demand(@builtin(local_invocation_index) bucket:u32,@builtin(workgroup_id) block:vec3<u32>) {
 var sum=0u;let end=min((block.x+1u)*256u,view.counts.x);
 for(var tree=block.x*256u;tree<end;tree++) {
  let d=demand[tree].xy;if d.y==bucket {demand[tree].z=sum;sum+=d.x;}
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
