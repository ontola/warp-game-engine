// Current-frame seed depth only. Historical depth must never reject a root here.
struct Node { sphere:vec4<f32>, error:vec4<f32>, links:vec4<u32> };
struct Tree { x:vec4<f32>, y:vec4<f32>, z:vec4<f32>, scale:vec4<f32>, info:vec4<u32> };
struct Camera { vp:mat4x4<f32>, count:vec4<u32> };
@group(0) @binding(0) var<uniform> camera:Camera;
@group(0) @binding(1) var<storage,read> nodes:array<Node>;
@group(0) @binding(2) var<storage,read> trees:array<Tree>;
@group(0) @binding(3) var depth:texture_2d<f32>;
@group(0) @binding(4) var<storage,read_write> visible:array<atomic<u32>>;
fn visible_in(s:vec4<f32>,vp:mat4x4<f32>)->bool {
 let r=transpose(vp);
 let planes=array<vec4<f32>,6>(r[3]+r[0],r[3]-r[0],r[3]+r[1],r[3]-r[1],r[2],r[3]-r[2]);
 for(var i=0u;i<6u;i++){if dot(planes[i],vec4<f32>(s.xyz,1.0)) < -s.w*length(planes[i].xyz){return false;}}
 return true;
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
 let size=textureDimensions(depth,0);
 let a=vec2<u32>(max(floor(lo*vec2<f32>(size))-1.0,vec2<f32>(0.0)));
 let b=min(vec2<u32>(ceil(hi*vec2<f32>(size))+1.0),size-1u);
 var mip=0u;
 loop {if mip+1u>=textureNumLevels(depth) || all((b>>vec2<u32>(mip))-(a>>vec2<u32>(mip))<=vec2<u32>(1u)){break;} mip++;}
 let msize=textureDimensions(depth,i32(mip));
 let first=min(a>>vec2<u32>(mip),msize-1u);let last=min(b>>vec2<u32>(mip),msize-1u);
 var minimum=1.0;
 for(var y=first.y;y<=last.y;y++){for(var x=first.x;x<=last.x;x++){
  minimum=min(minimum,textureLoad(depth,vec2<i32>(i32(x),i32(y)),i32(mip)).x);
 }}
 return nearest+0.00002<minimum;
}

@compute @workgroup_size(64) fn classify(@builtin(global_invocation_id) id:vec3<u32>) {
 let i=id.x;if i>=camera.count.x {return;}
 let t=trees[i];if t.info.w!=0u {return;}
 let n=nodes[t.info.x];let p=vec4<f32>(n.sphere.xyz,1.0);
 let s=vec4<f32>(dot(t.x,p),dot(t.y,p),dot(t.z,p),n.sphere.w*t.scale.x+t.scale.y);
 // Only cameras containing the root contribute. An unrelated camera looking
 // away must not undo another camera's proven current-depth rejection.
 if !visible_in(s,camera.vp) {return;}
 // Uncertain screen-edge/near-plane support remains visible.
 if !occluded(s,camera.vp) {atomicOr(&visible[i],1u);}
}
