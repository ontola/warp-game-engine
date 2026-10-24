struct Pose {rotation:vec4<f32>,translation:vec4<f32>};
struct Actor {r0:vec4<f32>,r1:vec4<f32>,r2:vec4<f32>,animation:vec4<f32>,clip:vec4<u32>};
struct Part {identity:vec4<u32>,sphere:vec4<f32>};
struct Instance {r0:vec4<f32>,r1:vec4<f32>,r2:vec4<f32>,sphere:vec4<f32>,first_cluster:u32,cluster_count:u32,scale:f32,flags:u32};
struct State {time:vec4<f32>,counts:vec4<u32>};
@group(0) @binding(0) var<uniform> state:State;
@group(0) @binding(1) var<storage,read> poses:array<Pose>;
@group(0) @binding(2) var<storage,read> actors:array<Actor>;
@group(0) @binding(3) var<storage,read> parts:array<Part>;
@group(0) @binding(4) var<storage,read_write> instances:array<Instance>;
fn rotate(q:vec4<f32>,v:vec3<f32>)->vec3<f32> {return v+2.0*cross(q.xyz,cross(q.xyz,v)+q.w*v);}
fn root_vector(a:Actor,v:vec3<f32>)->vec3<f32> {return vec3<f32>(dot(a.r0.xyz,v),dot(a.r1.xyz,v),dot(a.r2.xyz,v));}
@compute @workgroup_size(64)
fn animate_parts(@builtin(global_invocation_id) id:vec3<u32>) {
    if id.x>=state.counts.x {return;}
    let p=parts[id.x];let a=actors[p.identity.x];
    let position=fract(state.time.x*a.animation.y/a.animation.z+a.animation.x)*f32(a.clip.y);
    let frame=u32(floor(position));let t=fract(position);
    let before=poses[a.clip.x+frame*a.clip.z+p.identity.y];
    let after=poses[a.clip.x+((frame+1u)%a.clip.y)*a.clip.z+p.identity.y];
    let next=select(after.rotation,-after.rotation,dot(before.rotation,after.rotation)<0.0);
    let q=normalize(mix(before.rotation,next,t));let translation=mix(before.translation.xyz,after.translation.xyz,t);
    let x=root_vector(a,rotate(q,vec3<f32>(1.0,0.0,0.0)));
    let y=root_vector(a,rotate(q,vec3<f32>(0.0,1.0,0.0)));
    let z=root_vector(a,rotate(q,vec3<f32>(0.0,0.0,1.0)));
    let origin=root_vector(a,translation)+vec3<f32>(a.r0.w,a.r1.w,a.r2.w);
    var instance=instances[p.identity.z];
    instance.r0=vec4<f32>(x.x,y.x,z.x,origin.x);instance.r1=vec4<f32>(x.y,y.y,z.y,origin.y);instance.r2=vec4<f32>(x.z,y.z,z.z,origin.z);
    // Exact current-pose center and conservative radius at every interpolation
    // fraction; the same live matrix feeds cluster culling and shadow passes.
    let center=origin+x*p.sphere.x+y*p.sphere.y+z*p.sphere.z;
    instance.sphere=vec4<f32>(center,p.sphere.w*a.animation.w);instance.scale=a.animation.w;
    instances[p.identity.z]=instance;
}
