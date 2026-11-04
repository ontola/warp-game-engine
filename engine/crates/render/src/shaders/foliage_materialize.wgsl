struct Source {rows:array<vec4<f32>,3>,sphere:vec4<f32>,first:u32,count:u32,scale:f32,flags:u32};
struct Volume {inverse:array<vec4<f32>,3>,low:vec4<f32>,high:vec4<f32>,identity:vec4<u32>};
struct Branch {rest:array<vec4<f32>,3>,source:Source,volume:Volume,low:vec4<f32>,high:vec4<f32>,animation:vec4<f32>,info:vec4<u32>};
struct Root {rows:array<vec4<f32>,3>,scale:vec4<f32>,info:vec4<u32>};
struct Node {sphere:vec4<f32>,error:vec4<f32>,links:vec4<u32>};
struct Slot {key:vec4<u32>,history:vec4<u32>};
struct State {wind:vec4<f32>,counts:vec4<u32>};
@group(0) @binding(0) var<uniform> state:State;
@group(0) @binding(1) var<storage,read> roots:array<Root>;
@group(0) @binding(2) var<storage,read> nodes:array<Node>;
@group(0) @binding(3) var<storage,read> branches:array<Branch>;
@group(0) @binding(4) var<storage,read> representations:array<vec4<u32>>;
@group(0) @binding(5) var<storage,read> slots:array<Slot>;
@group(0) @binding(6) var<storage,read> fallback:array<u32>;
@group(0) @binding(7) var<storage,read> proxies:array<Volume>;
@group(0) @binding(8) var<storage,read_write> sources:array<Source>;
@group(0) @binding(9) var<storage,read_write> volumes:array<Volume>;
@group(0) @binding(10) var<storage,read_write> choices:array<u32>;
@group(0) @binding(11) var<storage,read> direct_roots:array<u32>;
@group(0) @binding(12) var<storage,read_write> root_history:array<vec4<u32>>;
fn multiply(a:array<vec4<f32>,3>,b:array<vec4<f32>,3>)->array<vec4<f32>,3> {
 var out:array<vec4<f32>,3>;
 for(var i=0u;i<3u;i++){out[i]=a[i].x*b[0]+a[i].y*b[1]+a[i].z*b[2]+vec4<f32>(0.,0.,0.,a[i].w);}return out;
}
fn point(m:array<vec4<f32>,3>,p:vec3<f32>)->vec3<f32>{let v=vec4<f32>(p,1.);return vec3<f32>(dot(m[0],v),dot(m[1],v),dot(m[2],v));}
fn inverse(m:array<vec4<f32>,3>)->array<vec4<f32>,3> {
 let a=cross(m[1].xyz,m[2].xyz);let b=cross(m[2].xyz,m[0].xyz);let c=cross(m[0].xyz,m[1].xyz);
 let d=1./dot(m[0].xyz,a);let t=vec3<f32>(m[0].w,m[1].w,m[2].w);
 let x=vec3<f32>(a.x,b.x,c.x)*d;let y=vec3<f32>(a.y,b.y,c.y)*d;let z=vec3<f32>(a.z,b.z,c.z)*d;
 return array<vec4<f32>,3>(vec4<f32>(x,-dot(x,t)),vec4<f32>(y,-dot(y,t)),vec4<f32>(z,-dot(z,t)));
}
fn rotate(m:array<vec4<f32>,3>,axis:vec3<f32>,angle:f32)->array<vec4<f32>,3> {
 let c=cos(angle);let s=sin(angle);let k=1.-c;let x=axis.x;let y=axis.y;let z=axis.z;
 let r=array<vec3<f32>,3>(vec3<f32>(c+x*x*k,x*y*k-z*s,x*z*k+y*s),vec3<f32>(y*x*k+z*s,c+y*y*k,y*z*k-x*s),vec3<f32>(z*x*k-y*s,z*y*k+x*s,c+z*z*k));
 var out:array<vec4<f32>,3>;for(var i=0u;i<3u;i++){out[i]=vec4<f32>(r[i].x*m[0].xyz+r[i].y*m[1].xyz+r[i].z*m[2].xyz,m[i].w);}return out;
}
fn pose(root:Root,branch:u32)->array<vec4<f32>,3> {
 // Cook validation bounds ancestry at 64; branch data is shared, not copied
 // for every tree. Evaluate only selected branch chains, including ancestors.
 var stack:array<u32,64>;var n=0u;var b=branch;
 loop {stack[n]=b;n++;b=branches[b].info.x;if b==0xffffffffu || n==64u {break;}}
 var m=root.rows;let axis=normalize(vec3<f32>(-state.wind.y,0.,state.wind.x));
 loop {if n==0u {break;}n--;let part=branches[stack[n]];m=multiply(m,part.rest);
  m=rotate(m,axis,part.animation.x*state.wind.z*sin(state.wind.w*part.animation.y+part.animation.z));}
 return m;
}
fn bounds(v:Volume,m:array<vec4<f32>,3>,lo:vec3<f32>,hi:vec3<f32>)->Volume {
 var result=v;var low=vec3<f32>(1e30);var high=-low;
 for(var i=0u;i<8u;i++){let p=point(m,vec3<f32>(select(lo.x,hi.x,(i&1u)!=0u),select(lo.y,hi.y,(i&2u)!=0u),select(lo.z,hi.z,(i&4u)!=0u)));low=min(low,p);high=max(high,p);}
 result.low=vec4<f32>(low,0.);result.high=vec4<f32>(high,0.);return result;
}
@compute @workgroup_size(64)
fn materialize(@builtin(global_invocation_id) id:vec3<u32>) {
 let i=id.x;
 if i<state.counts.x {
  let slot=slots[i];let owner=state.counts.y+i;
  if slot.history.z==0u {sources[owner].sphere.w=-1.;sources[owner].first=0u;choices[i]=0u;choices[i+16384u]=0u;return;}
  let root=roots[slot.key.x];let rep=representations[slot.key.z];let branch=branches[rep.x];let m=pose(root,rep.x);
  var source=branch.source;source.rows=m;let bound=nodes[slot.key.y].sphere;
  source.sphere=vec4<f32>(point(root.rows,bound.xyz),bound.w*root.scale.x);
  source.scale=length(root.rows[0].xyz);source.flags=(source.flags&~128u)|slot.history.w;
  // A voxel choice emits no source members, including shadow passes.
  if rep.y==1u {source.first=0u;}
  sources[owner]=source;
  var volume=bounds(branch.volume,m,branch.low.xyz,branch.high.xyz);volume.inverse=inverse(m);volume.identity.x=owner;
  volumes[i]=volume;choices[i]=select(0u,1u|(rep.z<<2u)|512u,rep.y==1u);
  // Shadow consumers use the existing adaptive voxel-caster policy, even
  // when camera detail requires source needles. The trunk stays source:
  // branch zero's aggregate covers the WHOLE tree, not just its own members.
  choices[i+16384u]=select(0u,765u,rep.y==1u || branch.info.y!=0u);
 }else {
  let tree=i-state.counts.x;if tree>=state.counts.z {return;}
  let root=roots[tree];let original=proxies[tree];
  if root.info.w!=0u {
   var hidden=original;hidden.low.w=1.;volumes[i]=hidden;
   sources[original.identity.x].sphere.w=-1.;return;
  }
  let m=pose(root,root.info.z);
  let delta=multiply(m,original.inverse);
  var volume=bounds(original,delta,original.low.xyz,original.high.xyz);volume.inverse=inverse(m);
  let direct=direct_roots[tree];var history=0u;
  if direct!=0u {
   let rep=representations[direct-1u];let branch=branches[rep.x];
   volume=bounds(branch.volume,m,branch.low.xyz,branch.high.xyz);volume.inverse=inverse(m);
   volume.identity=vec4<u32>(original.identity.x,branch.volume.identity.y+rep.z,1u,branch.volume.identity.w);
   history=direct;
  }else {
   volume.low.w=select(1.,0.,fallback[tree]!=0u);
   if fallback[tree]!=0u {history=0xffffffffu;}
  }
  volumes[i]=volume;
  let owner=original.identity.x;var source=sources[owner];source.rows=m;
  let previous=root_history[tree];var epoch=previous.z;
  if previous.w==0u || previous.x!=history || previous.y!=root.info.y {epoch=epoch^128u;}
  source.flags=(source.flags&~128u)|epoch;
  root_history[tree]=vec4<u32>(history,root.info.y,epoch,1u);
  // Preserve the authored crown sphere for canopy density. Selection bounds
  // include conservative support/wind margins and would darken the forest.
  // Voxel visibility uses the independently animated volume bounds above.
  sources[owner]=source;
 }
}
