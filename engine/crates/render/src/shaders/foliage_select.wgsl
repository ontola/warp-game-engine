// Shared-recipe traversal. No CPU near-tree shortlist.
struct Node { sphere:vec4<f32>, error:vec4<f32>, links:vec4<u32> };
// links: first child (or NONE), exclusive subtree end, representation, reserved.
struct Tree { x:vec4<f32>, y:vec4<f32>, z:vec4<f32>, scale:vec4<f32>, info:vec4<u32> };
// info: root node, stable generation, reserved, reserved. scale.y: wind envelope.
struct Consumer { vp:mat4x4<f32>, pixels:vec4<f32> };
struct View { consumers:array<Consumer,32>, consumer_count:vec4<u32>, vp:mat4x4<f32>, previous_vp:mat4x4<f32>, pixels:vec4<f32>, counts:vec4<u32>, catalog:vec4<u32> };
@group(0) @binding(0) var<uniform> view:View;
@group(0) @binding(1) var<storage,read> catalog:array<vec4<u32>>;
// Hierarchy nodes (three vec4 each) and trees (five each) in one read-only
// binding (foliage_select.rs Region::catalog): nodes from 0, trees from
// view.catalog.x. Read by field so a loop touching one field loads one vec4.
fn catalog_trees()->u32 {return view.catalog.x;}
fn node_sphere(i:u32)->vec4<f32> {return bitcast<vec4<f32>>(catalog[i*3u]);}
fn node_error(i:u32)->vec4<f32> {return bitcast<vec4<f32>>(catalog[i*3u+1u]);}
fn node_links(i:u32)->vec4<u32> {return catalog[i*3u+2u];}
fn node_at(i:u32)->Node {return Node(node_sphere(i),node_error(i),node_links(i));}
fn tree_x(i:u32)->vec4<f32> {return bitcast<vec4<f32>>(catalog[catalog_trees()+i*5u]);}
fn tree_y(i:u32)->vec4<f32> {return bitcast<vec4<f32>>(catalog[catalog_trees()+i*5u+1u]);}
fn tree_z(i:u32)->vec4<f32> {return bitcast<vec4<f32>>(catalog[catalog_trees()+i*5u+2u]);}
fn tree_scale(i:u32)->vec4<f32> {return bitcast<vec4<f32>>(catalog[catalog_trees()+i*5u+3u]);}
fn tree_info(i:u32)->vec4<u32> {return catalog[catalog_trees()+i*5u+4u];}
fn tree_at(i:u32)->Tree {return Tree(tree_x(i),tree_y(i),tree_z(i),tree_scale(i),tree_info(i));}
@group(0) @binding(3) var<storage,read_write> records:array<vec4<u32>>;
@group(0) @binding(4) var<storage,read_write> deferred:array<vec2<u32>>;
@group(0) @binding(5) var<storage,read_write> counters:array<atomic<u32>>;
@group(0) @binding(6) var<storage,read_write> fallback:array<atomic<u32>>;
@group(0) @binding(7) var hzb:texture_2d<f32>;
@group(0) @binding(8) var<storage,read_write> direct_roots:array<u32>;
// Per-root demand and stable prefix within a projected-size priority bucket.
// bucket low 5 bits: priority; bit 8: camera demand; bit 9: shadow demand.
struct Demand { count:atomic<u32>, bucket:u32, prefix:u32, failed:atomic<u32> };
@group(0) @binding(9) var<storage,read_write> demand:array<Demand>;
@group(0) @binding(10) var<storage,read_write> bucket_cost:array<u32>;
struct WorkQueue { count:atomic<u32>, padding:array<u32,3>, tasks:array<vec2<u32>> };
@group(0) @binding(11) var<storage,read_write> input_work:WorkQueue;
@group(0) @binding(12) var<storage,read_write> output_work:WorkQueue;
@group(0) @binding(13) var<storage,read> child_list:array<u32>;
// Logical representation pages. Physical allocation/upload is a separate owner.
struct AssetPage {resident:u32,requested:atomic<u32>,generation:u32,padding:u32};
@group(0) @binding(14) var<storage,read_write> asset_pages:array<AssetPage>;
@group(0) @binding(15) var<storage,read_write> asset_requests:array<atomic<u32>>;
fn asset_ready(representation:u32)->bool {
 // Thousands of roots share each page. Once requested, avoid contending
 // read-modify-writes; duplicate first writers remain harmless atomics.
 // Compaction resets these bits only after all selection dispatches finish.
 if atomicLoad(&asset_pages[representation].requested)==0u {
  atomicOr(&asset_pages[representation].requested,1u);
 }
 return asset_pages[representation].resident!=0u;
}
@compute @workgroup_size(64) fn compact_asset_requests(@builtin(global_invocation_id) id:vec3<u32>) {
 let page=id.x;if page>=arrayLength(&asset_pages) {return;}
 // requested holds the urgency (foliage_refine.wgsl ready), 1 the lowest.
 let urgency=atomicExchange(&asset_pages[page].requested,0u);if urgency==0u {return;}
 let i=atomicAdd(&asset_requests[0],1u);let at=4u+i*4u;
 // Exactly one invocation per page: capacity equals logical page count.
 atomicStore(&asset_requests[at],page);atomicStore(&asset_requests[at+1u],asset_pages[page].resident);
 atomicStore(&asset_requests[at+2u],asset_pages[page].generation);atomicStore(&asset_requests[at+3u],urgency);
}
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
// Refinement is a complete replacement: do not enter a child cut until all
// visible immediate children have a resident representation. Each child can
// then stop at its own loaded ancestor if deeper detail is missing. This gate
// runs identically in count and publication; no partial sibling cut is emitted.
// Requests remain shared logical asset requests, never a CPU tree shortlist.
fn children_ready(tree:u32,node:u32)->bool {
 let t=tree_at(tree);let start=child_list[node*2u];let count=child_list[node*2u+1u];
 var ready=true;
 for(var i=0u;i<count;i++) {
  let child=node_at(child_list[start+i]);
  if visible(sphere(t,child)) {
   // Do not short circuit: request every missing sibling in the same frame.
   let resident=asset_ready(child.links.z);ready=ready && resident;
  }
 }
 return ready;
}
fn emit(tree:u32,node:u32) {
 if !asset_ready(node_links(node).z) {atomicStore(&fallback[tree],4u);return;}
 if (node_links(node).w&2u)!=0u {direct_roots[tree]=node_links(node).z+1u;atomicAdd(&counters[7],1u);return;}
 let at=atomicAdd(&counters[0],1u);
 if at<view.counts.y {records[at]=vec4<u32>(tree,node,node_links(node).z,tree_info(tree).y);}
 else {atomicStore(&fallback[tree],1u);atomicAdd(&counters[3],1u);}
}
fn walk(tree:u32,start:u32,phase:u32) {
 let t=tree_at(tree);let end=node_links(start).y;var n=start;
 loop {
  if n>=end {break;}
  let node=node_at(n);let s=sphere(t,node);atomicAdd(&counters[4],1u);
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
  if node.links.x==NONE || ((node.links.w&1u)==0u && error_pixels(s,node.error.x*t.scale.x)<=view.pixels.z && (node.error.y==0.0 || error_pixels(s,node.error.y*t.scale.x)<=1.0)) || !children_ready(tree,n) {
   emit(tree,n);n=node.links.y;
  }else{n=node.links.x;}
 }
}
@compute @workgroup_size(64) fn first(@builtin(global_invocation_id) id:vec3<u32>) {
 if id.x<view.counts.x && tree_info(id.x).w==0u {
  walk(id.x,tree_info(id.x).x,1u);
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
