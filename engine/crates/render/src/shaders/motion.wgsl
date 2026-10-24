struct MotionUniform {
    inverse_current: mat4x4<f32>, current: mat4x4<f32>, previous: mat4x4<f32>,
    camera: vec4<f32>, size: vec4<f32>, flags: vec4<u32>,
    grass: vec4<f32>,
};
struct MotionInstance {
    r0: vec4<f32>, r1: vec4<f32>, r2: vec4<f32>, sphere: vec4<f32>,
    first_cluster: u32, cluster_count: u32, scale: f32, flags: u32,
};
@group(0) @binding(0) var<uniform> motion: MotionUniform;
@group(0) @binding(1) var<storage,read> current_instances: array<MotionInstance>;
@group(0) @binding(2) var<storage,read> previous_instances: array<MotionInstance>;
@group(0) @binding(3) var<storage,read> current_wind: array<vec4<f32>>;
@group(0) @binding(4) var<storage,read> previous_wind: array<vec4<f32>>;
@group(0) @binding(5) var<storage,read> visible: array<vec2<u32>>;
@group(0) @binding(6) var visibility: texture_2d<u32>;
@group(0) @binding(7) var opaque_depth: texture_depth_2d;
@group(0) @binding(8) var grass_metadata: texture_2d<u32>;
@group(0) @binding(9) var glass_metadata: texture_2d<u32>;
@group(0) @binding(10) var far_color: texture_2d<f32>;
@group(0) @binding(11) var far_distance: texture_2d<f32>;
struct MotionOut { @location(0) vector: vec2<f32>, @location(1) depth: f32, @location(2) bias: f32 };
@vertex fn motion_vs(@builtin(vertex_index) index:u32)->@builtin(position) vec4<f32> {
    return vec4<f32>(array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0))[index],0.0,1.0);
}
fn finite4(v:vec4<f32>)->bool { return all(abs(v)<vec4<f32>(1e30)); }
fn transform(inst:MotionInstance,p:vec3<f32>)->vec3<f32> {
    let v=vec4<f32>(p,1.0);return vec3<f32>(dot(inst.r0,v),dot(inst.r1,v),dot(inst.r2,v));
}
fn rest_root(inst:MotionInstance)->vec3<f32> {return vec3<f32>(inst.r0.w,inst.r1.w,inst.r2.w);}
fn deform(p:vec3<f32>,root_y:f32,shear:vec2<f32>)->vec3<f32> {return p+vec3<f32>(shear.x,0.0,shear.y)*(p.y-root_y);}

@fragment fn motion_fs(@builtin(position) position:vec4<f32>)->MotionOut {
    let pixel=vec2<i32>(position.xy);
    let ndc=vec2<f32>(position.x*motion.size.z*2.0-1.0,1.0-position.y*motion.size.w*2.0);
    var out:MotionOut;out.vector=vec2<f32>(0.0);out.depth=textureLoad(opaque_depth,pixel,0);out.bias=0.0;
    var owner=0xffffffffu;
    if out.depth>0.0 {
        let slot=textureLoad(visibility,pixel,0).r>>7u;
        if slot>=arrayLength(&visible) {out.bias=1.0;return out;}
        owner=visible[slot].x;
    }
    var grass=vec2<u32>(0u);var glass=vec2<u32>(0u);
    if motion.flags.y!=0u {grass=textureLoad(grass_metadata,pixel,0).rg;}
    if motion.flags.z!=0u {glass=textureLoad(glass_metadata,pixel,0).rg;}
    let grass_alpha=f32(grass.x&255u)/255.0;
    let glass_alpha=f32(glass.x&255u)/255.0;
    let opaque=out.depth;
    // Glass is composited after near/far grass. A faint front glass layer must
    // not hide a stronger grass owner's motion merely because it is closer.
    out.bias=max(4.0*grass_alpha*(1.0-grass_alpha),4.0*glass_alpha*(1.0-glass_alpha));
    if grass_alpha*(1.0-glass_alpha)>=0.5 {
        let d=bitcast<f32>(grass.y);
        if d>=opaque && d<=1.0 {owner=(grass.x>>8u)-1u;out.depth=d;}
    }
    if glass_alpha>=0.5 {
        let d=bitcast<f32>(glass.y);
        if d>=opaque && d<=1.0 {owner=(glass.x>>8u)-1u;out.depth=d;}
    }
    // Statistical distant grass has no tracked individual surface. Use its
    // optical centroid only when dominant and bias history by its coverage;
    // never pretend that an opaque ground vector describes swaying needles.
    var far_point=vec4<f32>(0.0);
    if motion.flags.w!=0u {
        let q=clamp(pixel/2,vec2<i32>(0),vec2<i32>(textureDimensions(far_color))-1);
        let far=textureLoad(far_color,q,0);let horizontal_distance=textureLoad(far_distance,q,0).r;
        let near_weight=select(grass_distance_opacity(horizontal_distance,motion.grass.xy),grass_alpha,grass_alpha>0.0);
        let contribution=far.a*(1.0-near_weight)*(1.0-glass_alpha);
        if contribution>0.0 && horizontal_distance>0.0 {
            out.bias=max(out.bias,contribution);
            if contribution>=0.5 {
                // The colour/distance were traced at the half-resolution ray
                // centre and store horizontal distance, not full ray length.
                let centre=min(vec2<f32>(q*2)+1.0,motion.size.xy-0.5);
                let half_ndc=centre*motion.size.zw*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0);
                let direction4=motion.inverse_current*vec4<f32>(half_ndc,0.0,1.0);
                let direction=normalize(direction4.xyz);
                let point=motion.camera.xyz+direction*(horizontal_distance/max(length(direction.xz),1e-8));
                let clip=motion.current*vec4<f32>(point,1.0);
                if clip.w>0.0 && clip.z/clip.w>=out.depth {
                    far_point=vec4<f32>(point,1.0);owner=0xffffffffu;out.depth=clip.z/clip.w;
                }
            }
        }
    }
    // Reset rejects correspondence, not the current frame's selected depth.
    if motion.flags.x!=0u {out.bias=1.0;return out;}
    var current=motion.inverse_current*vec4<f32>(ndc,out.depth,1.0);
    var previous=current;
    if far_point.w!=0.0 {current=far_point;previous=far_point;}
    else if out.depth<=0.0 {current.w=0.0;previous.w=0.0;}
    else {
        if !finite4(current) || abs(current.w)<1e-10 {out.bias=1.0;return out;}
        current=vec4<f32>(current.xyz/current.w,1.0);
        if owner>=arrayLength(&current_instances) || owner>=arrayLength(&previous_instances)
            || owner>=arrayLength(&current_wind) || owner>=arrayLength(&previous_wind) {out.bias=1.0;return out;}
        let now=current_instances[owner];let old=previous_instances[owner];
        if old.cluster_count==0u || now.first_cluster!=old.first_cluster || now.cluster_count!=old.cluster_count
            || now.flags!=old.flags || ((now.flags&8u)!=0u && any(rest_root(now)!=rest_root(old))) {out.bias=1.0;return out;}
        let rest=deform(current.xyz,now.r1.w,-current_wind[owner].xy)-rest_root(now);
        let co0=cross(now.r1.xyz,now.r2.xyz);let co1=cross(now.r2.xyz,now.r0.xyz);let co2=cross(now.r0.xyz,now.r1.xyz);
        let determinant=dot(now.r0.xyz,co0);
        if !(abs(determinant)>1e-12 && abs(determinant)<1e20) {out.bias=1.0;return out;}
        let local=(co0*rest.x+co1*rest.y+co2*rest.z)/determinant;
        previous=vec4<f32>(deform(transform(old,local),old.r1.w,previous_wind[owner].xy),1.0);
    }
    let a=motion.current*current;let b=motion.previous*previous;
    if !finite4(a) || !finite4(b) || a.w<=1e-7 || b.w<=1e-7 {out.bias=1.0;return out;}
    let prev_ndc=b.xy/b.w;
    let vector=(prev_ndc-a.xy/a.w)*vec2<f32>(0.5,-0.5)*motion.size.xy;
    if !all(abs(vector)<vec2<f32>(32752.0)) {out.bias=1.0;return out;}
    out.vector=vector;
    if any(abs(prev_ndc)>vec2<f32>(1.0)) {out.bias=1.0;}
    return out;
}
