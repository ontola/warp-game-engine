// Crowd animation, after Growing Guns' crowd.gdshader: per spectator a body
// instance (torso + head) and an arms instance, posed every frame from the
// clock and the mood. Cheer waves raise the arms and hop the figure,
// excitement double-times the wave, panic crouches and trembles, blasts
// scatter nearby spectators along their row and they walk back after.
struct Seat {
    // position, yaw
    pos_yaw: vec4<f32>,
    // scale, phase, tempo, hop amplitude
    params: vec4<f32>,
    // body first cluster, count; arms first cluster, count
    meshes: vec4<u32>,
    // shoulder width relative to the height, unused x3
    shape: vec4<f32>,
};
struct Instance { r0: vec4<f32>, r1: vec4<f32>, r2: vec4<f32>, sphere: vec4<f32>, first_cluster: u32, cluster_count: u32, scale: f32, flags: u32 };
struct Uniform {
    // time, excitement, panic, optional local shoulder pivot height
    mood: vec4<f32>,
    // ring centre xz, seats, first instance (as f32 bits: see counts)
    ring: vec4<f32>,
    counts: vec4<u32>,
};
@group(0) @binding(0) var<uniform> u: Uniform;
@group(0) @binding(1) var<storage, read> seats: array<Seat>;
@group(0) @binding(2) var<storage, read> alive: array<u32>;
@group(0) @binding(3) var<storage, read_write> instances: array<Instance>;

const FLEE_OUT_END: f32 = 1.2;
const FLEE_RETURN_DUR: f32 = 4.5;


// One persistent movement and death record per spectator, shared by all cameras.
struct Reaction {
    start: f32, hold_end: f32, initial: f32, goal: f32,
    dead: u32, bullet_lo: u32, bullet_hi: u32, pad: u32,
}
@group(0) @binding(4) var<storage,read_write> reactions:array<Reaction>;
fn displacement(r:Reaction,time:f32)->f32 {
    let outward=smoothstep(0.,FLEE_OUT_END,time-r.start);
    let home=smoothstep(0.,FLEE_RETURN_DUR,time-r.hold_end);
    return mix(r.initial,r.goal,outward)*(1.-home);
}
fn row_direction(s:Seat)->vec2<f32> {
    let v=s.pos_yaw.xz-u.ring.xy;
    return vec2<f32>(-v.y,v.x)/max(length(v),0.001);
}
struct BodyPose { origin:vec3<f32>, up:f32, crouch:f32, shear:f32 }
fn body_pose(s:Seat,r:Reaction,time:f32,mood:vec2<f32>)->BodyPose {
    let t=time*(1.4+s.params.z*2.2)+s.params.y*6.2832;
    let up=max(max(sin(t),0.),max(sin(t*1.93+1.7),0.)*mood.x);
    let hop=(s.params.w+mood.x*0.22)*(1.+mood.x*0.8)*(1.-mood.y);
    let disp=row_direction(s)*displacement(r,time);
    let stride=min(abs(displacement(r,time+0.016)-displacement(r,time))*24.,1.);
    // Rigged spectators shift weight in place while watching. Only a cheer
    // lifts them off the step. The hit test uses this same root position.
    let cheering=select(1.,smoothstep(0.28,0.85,mood.x),u.counts.z!=0u);
    let lift=(up*hop*cheering*0.4+abs(sin(time*10.5+s.params.y*41.))*0.17*stride)*s.params.x;
    return BodyPose(s.pos_yaw.xyz+vec3<f32>(disp.x,lift,disp.y),up,1.-mood.y*0.22,
        sin(t*0.6)*0.035+mood.y*sin(time*21.+s.params.y*47.)*0.045);
}
