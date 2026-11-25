// One invocation per bounded assembly, parent-before-child. Each pose is
// evaluated once and shared by source raster/shadows and voxel inverse rays.
struct PartUniform {field:vec4<f32>,counts:vec4<u32>,schedule:vec4<u32>,lod:vec4<f32>};
struct PartSource {
    rows:array<vec4<f32>,3>,sphere:vec4<f32>,
    first_cluster:u32,cluster_count:u32,scale:f32,flags:u32,
};
struct PartVolume {
    inverse:array<vec4<f32>,3>,bounds_min:vec4<f32>,bounds_max:vec4<f32>,identity:vec4<u32>,
};
struct PartDescriptor {
    rest:array<vec4<f32>,3>,source:PartSource,volume:PartVolume,
    local_min:vec4<f32>,local_max:vec4<f32>,local_sphere:vec4<f32>,
    animation:vec4<f32>,identity:vec4<u32>,
};
struct PartPose {current:array<vec4<f32>,3>,previous:array<vec4<f32>,3>};
struct PartAttachmentDescriptor {
    rest:array<vec4<f32>,3>,source:PartSource,local_sphere:vec4<f32>,identity:vec4<u32>,
};
struct PartAssembly {range:vec4<u32>,sphere:vec4<f32>,motion:vec4<f32>};
struct AssemblyState {state:vec4<u32>,animation:vec4<f32>};
struct AnimationView {rows:array<vec4<f32>,4>,viewport:vec4<f32>};
@group(0) @binding(0) var<uniform> part_state:PartUniform;
@group(0) @binding(1) var<storage,read> part_descriptors:array<PartDescriptor>;
@group(0) @binding(2) var<storage,read> part_assemblies:array<PartAssembly>;
@group(0) @binding(3) var<storage,read_write> part_poses:array<PartPose>;
@group(0) @binding(4) var<storage,read_write> part_sources:array<PartSource>;
@group(0) @binding(5) var<storage,read_write> part_volumes:array<PartVolume>;
@group(0) @binding(6) var<storage,read> part_attachments:array<PartAttachmentDescriptor>;
@group(0) @binding(7) var<storage,read_write> assembly_states:array<AssemblyState>;
@group(0) @binding(8) var<storage,read> animation_views:array<AnimationView>;
// The append counter is separate from INDIRECT arguments: binding a writable
// storage buffer while reading it for indirect dispatch is an invalid usage.
@group(0) @binding(9) var<storage,read_write> attachment_counter:atomic<u32>;
@group(0) @binding(10) var<storage,read_write> active_attachment_pages:array<u32>;
@group(0) @binding(11) var<storage,read> attachment_pages:array<vec4<u32>>;
@group(0) @binding(12) var<storage,read_write> attachment_dispatch:array<u32>;

fn assembly_index(invocation:vec3<u32>)->u32 {
    return invocation.x+invocation.y*part_state.schedule.z*32u;
}

// Conservative perspective derivative over the whole all-wind envelope.
// Orthographic shadow views naturally have a zero denominator derivative.
// A sphere crossing the near plane requests full animation.
fn projected_motion(view:AnimationView,sphere:vec4<f32>,motion:f32)->f32 {
    let p=vec4<f32>(sphere.xyz,1.0);
    let clip=vec4<f32>(dot(view.rows[0],p),dot(view.rows[1],p),dot(view.rows[2],p),dot(view.rows[3],p));
    if !all(abs(clip)<vec4<f32>(1e30)) {return 1e20;}
    let planes=array<vec4<f32>,6>(view.rows[3]+view.rows[0],view.rows[3]-view.rows[0],
        view.rows[3]+view.rows[1],view.rows[3]-view.rows[1],view.rows[2],view.rows[3]-view.rows[2]);
    for(var i=0u;i<6u;i++) {
        if dot(planes[i],p)+length(planes[i].xyz)*sphere.w<0.0 {return 0.0;}
    }
    let w_gradient=length(view.rows[3].xyz);
    let minimum_w=clip.w-w_gradient*sphere.w;
    if minimum_w<=1e-5 {return 1e20;}
    let numerator_gradient=vec2<f32>(length(view.rows[0].xyz),length(view.rows[1].xyz));
    let numerator_bound=abs(clip.xy)+numerator_gradient*sphere.w;
    let derivative=view.viewport.xy*(numerator_gradient/minimum_w+numerator_bound*w_gradient/(minimum_w*minimum_w));
    let pixels=length(derivative)*motion;
    if !(pixels>=0.0 && pixels<1e20) {return 1e20;}
    return pixels;
}

@compute @workgroup_size(32)
fn schedule_assemblies(@builtin(global_invocation_id) invocation:vec3<u32>) {
    let index=assembly_index(invocation);
    if index>=part_state.counts.x {return;}
    let assembly=part_assemblies[index];let old=assembly_states[index];
    var factor=1.0;
    if part_state.schedule.y!=0u && part_state.schedule.x!=0u {
        var pixels=0.0;
        for(var view=0u;view<part_state.schedule.x;view++) {
            pixels=max(pixels,projected_motion(animation_views[view],assembly.sphere,assembly.motion.x*part_state.field.z));
        }
        factor=smoothstep(part_state.lod.x,part_state.lod.y,pixels);
    }
    let first=part_state.counts.z!=0u;let awake=factor>0.0;let was_awake=old.animation.x>0.0;
    // The host moved this assembly (PartWindRenderer::move_assembly): write
    // every pose once, with previous=current so the jump has no motion.
    let moved=old.animation.y>0.0;
    let wake=(awake && !was_awake && !first) || moved;
    let idle=select(min(old.state.y+1u,2u),0u,awake);
    // Mode2 writes current poses/members. Mode1 settles previous=current once
    // after entering rest, including volume consumers. Mode0 touches neither.
    var mode=0u;
    if first || awake || was_awake || moved {mode=2u;} else if old.state.y<2u {mode=1u;}
    let epoch=old.state.z ^ select(0u,128u,wake);
    assembly_states[index]=AssemblyState(vec4<u32>(mode,idle,epoch,u32(first || wake)),vec4<f32>(factor,0.0,0.0,0.0));
    if mode==2u && assembly.range.w!=0u {
        let offset=atomicAdd(&attachment_counter,assembly.range.w);
        for(var page=0u;page<assembly.range.w;page++) {active_attachment_pages[offset+page]=assembly.range.z+page;}
    }
}

@compute @workgroup_size(1)
fn finish_dispatch() {
    let count=atomicLoad(&attachment_counter);let width=part_state.schedule.z;
    attachment_dispatch[0]=count;
    attachment_dispatch[1]=min(count,width);
    attachment_dispatch[2]=count/width+u32(count%width!=0u);
    attachment_dispatch[3]=1u;
}

fn part_multiply(a:array<vec4<f32>,3>,b:array<vec4<f32>,3>)->array<vec4<f32>,3> {
    var result:array<vec4<f32>,3>;
    for(var row=0u;row<3u;row++) {result[row]=a[row].x*b[0]+a[row].y*b[1]+a[row].z*b[2]+vec4<f32>(0.0,0.0,0.0,a[row].w);}
    return result;
}
fn part_point(m:array<vec4<f32>,3>,p:vec3<f32>)->vec3<f32> {
    let v=vec4<f32>(p,1.0);return vec3<f32>(dot(m[0],v),dot(m[1],v),dot(m[2],v));
}
fn part_rotate(base:array<vec4<f32>,3>,axis:vec3<f32>,angle:f32)->array<vec4<f32>,3> {
    let c=cos(angle);let s=sin(angle);let k=1.0-c;let x=axis.x;let y=axis.y;let z=axis.z;
    let r=array<vec3<f32>,3>(vec3<f32>(c+x*x*k,x*y*k-z*s,x*z*k+y*s),
        vec3<f32>(y*x*k+z*s,c+y*y*k,y*z*k-x*s),vec3<f32>(z*x*k-y*s,z*y*k+x*s,c+z*z*k));
    var result:array<vec4<f32>,3>;
    for(var row=0u;row<3u;row++) {
        let linear=r[row].x*base[0].xyz+r[row].y*base[1].xyz+r[row].z*base[2].xyz;
        // Rotate about this branch's parent-animated pivot, never world zero.
        result[row]=vec4<f32>(linear,base[row].w);
    }
    return result;
}
fn part_inverse(m:array<vec4<f32>,3>)->array<vec4<f32>,3> {
    let a=cross(m[1].xyz,m[2].xyz);let b=cross(m[2].xyz,m[0].xyz);let c=cross(m[0].xyz,m[1].xyz);
    let reciprocal=1.0/dot(m[0].xyz,a);let t=vec3<f32>(m[0].w,m[1].w,m[2].w);
    let r=array<vec3<f32>,3>(vec3<f32>(a.x,b.x,c.x)*reciprocal,vec3<f32>(a.y,b.y,c.y)*reciprocal,vec3<f32>(a.z,b.z,c.z)*reciprocal);
    return array<vec4<f32>,3>(vec4<f32>(r[0],-dot(r[0],t)),vec4<f32>(r[1],-dot(r[1],t)),vec4<f32>(r[2],-dot(r[2],t)));
}

// Same sampled pressure and exact damped response as part_wind.rs.
const PRESSURE_DT: f32 = 1.0 / 6.0;
fn unload(age: f32) -> f32 {
    let t = clamp(age / 0.42, 0.0, 1.0);
    let arch = 4.0 * t * (1.0 - t);
    return arch * arch;
}
fn part_pressure(root: vec2<f32>, direction: vec2<f32>, phase: f32, strength: f32) -> vec4<f32> {
    let gust = plant_gust(root, direction, phase, strength, true);
    let crosswind = vec2<f32>(-direction.y, direction.x);
    let along = phase - dot(root, direction) * (6.28318530718 / (120.0 * 12.0));
    let across = dot(root, crosswind);
    let pressure = length(gust.load);
    let slow = 0.12 * sin(67.0 * along - 0.019 * across);
    let fast = 0.12 * sin(169.0 * along + 0.026 * across);
    let local = 0.19 * pressure;
    let eddy = 217.0 * along - 0.015 * across;
    var load = (0.78 + 0.10 * gust.storm_mix) * gust.load + pressure * (slow + fast) * crosswind;
    load -= 0.42 * (1.0 - gust.storm_mix) * unload(gust.breeze_age) * direction;
    return vec4<f32>(load, local * sin(eddy), local * cos(eddy));
}
fn complex_mul(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(a.x * b.x - a.y * b.y, a.x * b.y + a.y * b.x);
}
fn spring_kernel(angle: f32, harmonic: u32, phase: f32) -> vec4<f32> {
    let compliance = angle / (angle + 0.26);
    let identity = 0.65 * phase / 6.28318530718 + 0.35 * f32(harmonic % 17u) / 16.0;
    let omega = 6.28318530718 * (0.30 + 3.10 * compliance) * (0.86 + 0.28 * identity);
    let a = min(max(3.75 - 4.8 * compliance, 1.20), 0.66 * omega);
    let b = sqrt(omega * omega - a * a);
    let advance = exp(-a * PRESSURE_DT) * vec2<f32>(cos(b * PRESSURE_DT), sin(b * PRESSURE_DT));
    let integral = complex_mul(advance - vec2<f32>(1.0, 0.0), vec2<f32>(-a, -b)) / (omega * omega * PRESSURE_DT);
    return vec4<f32>(complex_mul(vec2<f32>(1.0, -a / b), integral), advance);
}

@compute @workgroup_size(32)
fn update_parts(@builtin(global_invocation_id) invocation:vec3<u32>) {
    let assembly_id=assembly_index(invocation);
    if assembly_id>=part_state.counts.x {return;}
    let assembly=part_assemblies[assembly_id].range;let schedule=assembly_states[assembly_id];
    if schedule.state.x==0u {return;}
    if schedule.state.x==1u {
        for(var index=assembly.x;index<assembly.x+assembly.y;index++) {part_poses[index].previous=part_poses[index].current;}
        return;
    }
    let root = part_descriptors[assembly.x].rest;
    let direction = part_state.field.xy;
    let crosswind = vec2<f32>(-direction.y, direction.x);
    let animate_joints = part_state.field.z > 0.0 && schedule.animation.x > 0.0;
    var trace: array<vec4<f32>, 25>;
    if animate_joints {
        for (var i = 0u; i < 25u; i++) {
            let phase = part_state.field.w - f32(i) * (PRESSURE_DT * 6.28318530718 / 120.0);
            trace[i] = part_pressure(vec2<f32>(root[0].w, root[2].w), direction, phase, part_state.field.z);
        }
    }
    let flutter_dir = direction * 0.4 + crosswind * 0.9165;
    for(var index=assembly.x;index<assembly.x+assembly.y;index++) {
        let part=part_descriptors[index];var base=part.rest;
        if part.identity.x!=0xffffffffu {base=part_multiply(part_poses[part.identity.x].current,part.rest);}
        var current = base;
        if animate_joints && part.animation.x > 0.0 {
            var response = trace[0];
            let kernel = spring_kernel(part.animation.x, u32(part.animation.y), part.animation.z);
            var coefficient = kernel.xy;
            for (var i = 0u; i < 24u; i++) {
                response -= (trace[i] - trace[i + 1u]) * coefficient.x;
                coefficient = complex_mul(coefficient, kernel.zw);
            }
            let force = response.xy + flutter_dir * dot(response.zw, vec2<f32>(cos(part.animation.z), sin(part.animation.z)));
            let limb = normalize(vec3<f32>(base[0].y, base[1].y, base[2].y));
            let torque = cross(limb, vec3<f32>(force.x, 0.0, force.y));
            let magnitude = length(torque);
            if magnitude > 1e-6 {
                let angle = part.animation.x * part_state.field.z * min(magnitude, 1.0) * schedule.animation.x;
                current = part_rotate(base, torque / magnitude, angle);
            }
        }
        var previous=part_poses[index].current;
        if schedule.state.w!=0u {previous=current;}
        part_poses[index]=PartPose(current,previous);
        if part.identity.y!=0xffffffffu {
            var source=part.source;source.rows=current;source.flags=source.flags ^ schedule.state.z;
            // Child binds are rigid and roots have orthogonal scale. Left /
            // right rotations preserve singular values and the rest radius.
            source.sphere=vec4<f32>(part_point(current,part.local_sphere.xyz),part.local_sphere.w);
            part_sources[part.identity.y]=source;
        }
        if part.identity.z!=0xffffffffu {
            var volume=part.volume;volume.inverse=part_inverse(current);
            var lo=vec3<f32>(1e30);var hi=vec3<f32>(-1e30);
            for(var corner=0u;corner<8u;corner++) {
                let p=vec3<f32>(select(part.local_min.x,part.local_max.x,(corner&1u)!=0u),
                    select(part.local_min.y,part.local_max.y,(corner&2u)!=0u),select(part.local_min.z,part.local_max.z,(corner&4u)!=0u));
                let world=part_point(current,p);lo=min(lo,world);hi=max(hi,world);
            }
            volume.bounds_min=vec4<f32>(lo,0.0);volume.bounds_max=vec4<f32>(hi,0.0);
            part_volumes[part.identity.z]=volume;
        }
    }
}

// Shared primitive meshes get the already-evaluated branch pose. No wind
// trigonometry, hierarchy traversal or independent animation phase per leaf.
// CPU validation guarantees unique owners, disjoint from branch proxy owners.
@compute @workgroup_size(64)
fn update_attachments(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    let page_index=group.x+group.y*part_state.schedule.z;
    if page_index>=atomicLoad(&attachment_counter) {return;}
    let page=attachment_pages[active_attachment_pages[page_index]];
    let schedule=assembly_states[page.z];
    for(var at=lane;at<page.y;at+=64u) {
        let attachment=part_attachments[page.x+at];
        let current=part_multiply(part_poses[attachment.identity.x].current,attachment.rest);
        var source=attachment.source;source.rows=current;source.flags=source.flags ^ schedule.state.z;
        source.sphere=vec4<f32>(part_point(current,attachment.local_sphere.xyz),attachment.local_sphere.w);
        part_sources[attachment.identity.y]=source;
    }
}
