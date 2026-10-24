// One invocation per bounded assembly, parent-before-child. Each pose is
// evaluated once and shared by source raster/shadows and voxel inverse rays.
struct PartUniform {field:vec4<f32>,counts:vec4<u32>};
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
@group(0) @binding(0) var<uniform> part_state:PartUniform;
@group(0) @binding(1) var<storage,read> part_descriptors:array<PartDescriptor>;
@group(0) @binding(2) var<storage,read> part_assemblies:array<vec4<u32>>;
@group(0) @binding(3) var<storage,read_write> part_poses:array<PartPose>;
@group(0) @binding(4) var<storage,read_write> part_sources:array<PartSource>;
@group(0) @binding(5) var<storage,read_write> part_volumes:array<PartVolume>;

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

@compute @workgroup_size(32)
fn update_parts(@builtin(global_invocation_id) invocation:vec3<u32>) {
    if invocation.x>=part_state.counts.x {return;}
    let assembly=part_assemblies[invocation.x];
    var axis=vec3<f32>(1.0,0.0,0.0);
    if part_state.field.z>0.0 {axis=normalize(vec3<f32>(-part_state.field.y,0.0,part_state.field.x));}
    for(var index=assembly.x;index<assembly.x+assembly.y;index++) {
        let part=part_descriptors[index];var base=part.rest;
        if part.identity.x!=0xffffffffu {base=part_multiply(part_poses[part.identity.x].current,part.rest);}
        let angle=part.animation.x*part_state.field.z*sin(part_state.field.w*part.animation.y+part.animation.z);
        let current=part_rotate(base,axis,angle);
        var previous=part_poses[index].current;
        if part_state.counts.z!=0u {previous=current;}
        part_poses[index]=PartPose(current,previous);
        if part.identity.y!=0xffffffffu {
            var source=part.source;source.rows=current;
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
