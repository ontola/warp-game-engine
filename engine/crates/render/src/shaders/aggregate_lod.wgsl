struct Group {sphere:vec4<f32>,info:vec4<u32>,scale:vec4<f32>};
struct Level {errors:vec4<f32>,triangle:vec4<f32>,info:vec4<u32>};
struct Uniform {vp:mat4x4<f32>,output:vec4<f32>,policy:vec4<f32>,info:vec4<u32>};
@group(0) @binding(0) var<uniform> state:Uniform;
@group(0) @binding(1) var<storage,read> groups:array<Group>;
@group(0) @binding(2) var<storage,read> levels:array<Level>;
@group(0) @binding(3) var<storage,read_write> decisions:array<u32>;
// One packed u32 per group (cull.wgsl selection_unpack).
fn selection_pack(d:vec4<u32>)->u32 {
    return min(d.x,3u)|(min(d.y,63u)<<2u)|(min(d.w,3u)<<8u)|(pack2x16float(vec2<f32>(bitcast<f32>(d.z),0.0))<<16u);
}

fn choose(g:Group)->vec4<u32> {
    // w=1 marks an explicit source cut, including error zero. Legacy
    // per-branch decisions have w=0 and keep their ordinary triangle CLOD.
    var decision=vec4<u32>(0u,0u,0u,1u);
    if state.info.y==0u {return decision;}
    let rows=transpose(state.vp);let p=vec4<f32>(g.sphere.xyz,1.0);let radius=g.sphere.w;
    let planes=array<vec4<f32>,6>(rows[3]+rows[0],rows[3]-rows[0],rows[3]+rows[1],rows[3]-rows[1],rows[2],rows[3]-rows[2]);
    for(var i=0u;i<6u;i++) {
        if dot(planes[i],p)+radius*length(planes[i].xyz)<0.0 {
            return vec4<u32>(2u,0u,0u,1u);
        }
    }
    let wlen=length(rows[3].xyz);let wmin=dot(rows[3],p)-radius*wlen;
    if wmin<=0.000001 || dot(rows[2],p)-radius*length(rows[2].xyz)<=0.0
        || dot(rows[3]-rows[2],p)-radius*length((rows[3]-rows[2]).xyz)<=0.0 {
        return decision;
    }
    let lx=length(rows[0].xyz);let ly=length(rows[1].xyz);
    let dx=(lx/wmin+(abs(dot(rows[0],p))+radius*lx)*wlen/(wmin*wmin))*state.output.x*0.5;
    let dy=(ly/wmin+(abs(dot(rows[1],p))+radius*ly)*wlen/(wmin*wmin))*state.output.y*0.5;
    let ppm=length(vec2<f32>(dx,dy))*g.scale.x;
    for(var i=0u;i<g.info.z;i++) {
        let c=levels[g.info.y+i];
        if c.errors.x*ppm>state.output.z {continue;}
        let voxel_ok=c.info.y==0u && c.errors.y<=state.policy.x && c.errors.z<=state.policy.y;
        let triangle_ok=c.triangle.x<=state.policy.x && c.triangle.y<=state.policy.y;
        // Same sampled optical metric, same source. Prefer voxels when the
        // triangle candidate destroys more aggregate coverage or material.
        if voxel_ok && (!triangle_ok || c.errors.y+c.errors.z<c.triangle.x+c.triangle.y) {
            decision=vec4<u32>(1u,c.info.x,0u,1u);
        }else if triangle_ok {
            // A negative cut tells cull.wgsl to use its own screen-error LOD.
            decision=vec4<u32>(0u,0u,bitcast<u32>(select(-1.0,c.triangle.z*g.scale.x,state.policy.z>0.5)),1u);
        }
    }
    return decision;
}

@compute @workgroup_size(64)
fn select_aggregate(@builtin(global_invocation_id) id:vec3<u32>) {
    if id.x>=state.info.x {return;}
    let g=groups[id.x];
    // Parent choices are recomputed from immutable metadata, so there is no
    // cross-workgroup read/write dependency or frame-late suppression.
    if g.info.w!=0xffffffffu {
        let parent=choose(groups[g.info.w]);
        if parent.x!=0u { decisions[g.info.x]=selection_pack(vec4<u32>(2u,0u,0u,1u));return; }
        // A whole-subtree triangle candidate was measured across ALL members.
        // Propagate that exact cut instead of independently changing children.
        if parent.z!=0u { decisions[g.info.x]=selection_pack(parent);return; }
    }
    decisions[g.info.x]=selection_pack(choose(g));
}
