// Shared immutable cluster-predicate BVH. Header: enabled, cluster count,
// first node word, first member word. Two words per original first-cluster
// slot identify its root and exact mesh count; UINT_MAX means flat fallback.
// Nodes are20words: cull/parent/own spheres, max parent/min own errors,
// two reserved words, escape/first member/member count/descendant count.
@group(1) @binding(12) var<storage,read> cluster_tree:array<u32>;
// Traversal diagnostics occupy counter[32..40], sharing the cull binding.

var<workgroup> cluster_cursor:u32;
var<workgroup> cluster_end:u32;
var<workgroup> cluster_action:u32;
var<workgroup> cluster_linear_bound:f32;
var<workgroup> cluster_error_scale:f32;

fn cluster_tree_sphere(word:u32)->vec4<f32> {
    return vec4<f32>(bitcast<f32>(cluster_tree[word]),bitcast<f32>(cluster_tree[word+1u]),
        bitcast<f32>(cluster_tree[word+2u]),bitcast<f32>(cluster_tree[word+3u]));
}

// sqrt(max absolute row sum(A^T A)) bounds the affine operator norm, including
// shear/nonuniform parent + rotating child. max(scale,...) also encloses the
// original selector's individual sphere radii when its scale is conservative.
fn cluster_transform_bound(inst:Instance)->f32 {
    let a=vec3<f32>(inst.r0.x,inst.r1.x,inst.r2.x);
    let b=vec3<f32>(inst.r0.y,inst.r1.y,inst.r2.y);
    let c=vec3<f32>(inst.r0.z,inst.r1.z,inst.r2.z);
    let ab=abs(dot(a,b));let ac=abs(dot(a,c));let bc=abs(dot(b,c));
    let gram=max(dot(a,a)+ab+ac,max(dot(b,b)+ab+bc,dot(c,c)+ac+bc));
    return max(inst.scale,sqrt(max(gram,0.0)))*(1.0+1e-5);
}

fn cluster_world_bound(inst:Instance,local:vec4<f32>,wind:f32)->vec4<f32> {
    let center=to_world(inst,local.xyz);
    let radius=local.w*cluster_linear_bound+wind;
    let roundoff=max(max(abs(center.x),abs(center.y)),abs(center.z))*2e-6+radius*1e-5+1e-4;
    return vec4<f32>(center,radius+roundoff);
}

// 0 retain;1 visibility reject;2 parent already fine;3 own error too large.
// These are bounds over ALL descendants. Final decisions still use the exact
// existing per-cluster predicate, including material and current-frame Hi-Z.
fn cluster_node_reject(node:u32,inst:Instance,threshold:f32,wind:f32)->u32 {
    let cull=cluster_world_bound(inst,cluster_tree_sphere(node),wind);
    if !all(abs(cull)<vec4<f32>(1e19)){return 0u;}
    if !in_frustum(cull.xyz,cull.w) || hidden_by_camera_fog(cull.xyz,cull.w){return 1u;}
    let parent_error=bitcast<f32>(cluster_tree[node+12u]);
    if parent_error<=1e30 {
        let parent=cluster_world_bound(inst,cluster_tree_sphere(node+4u),wind);
        let nearest=max(distance(parent.xyz,view.cam.xyz)-parent.w,view.params.y);
        let upper=parent_error*cluster_error_scale/nearest*view.cam.w;
        // Use a margin, never equality, to leave borderline f32 cuts to the
        // exact predicate. Root sentinel errors must never become finite.
        if upper>=0.0 && upper+max(1e-4,abs(upper)*2e-5)<threshold {return 2u;}
    }
    let own_error=bitcast<f32>(cluster_tree[node+13u]);
    if own_error>0.0 {
        let own=cluster_world_bound(inst,cluster_tree_sphere(node+8u),wind);
        let farthest=max(distance(own.xyz,view.cam.xyz)+own.w,view.params.y);
        let lower=own_error*cluster_error_scale/farthest*view.cam.w;
        if lower<1e30 && lower-max(1e-4,abs(lower)*2e-5)>threshold {return 3u;}
    }
    return 0u;
}

fn cluster_traverse(id:u32,inst:Instance,li:u32,threshold:f32,wind:f32)->bool {
    if cluster_tree[0]==0u {return false;}
    if inst.first_cluster>=cluster_tree[1] {
        if li==0u {atomicAdd(&counter[38],1u);}return false;
    }
    let table=4u+inst.first_cluster*2u;
    let root=cluster_tree[table];
    if root==0xffffffffu {if li==0u {atomicAdd(&counter[37],1u);}return false;}
    // Mesh replacement may change an owner's range. Unknown/sub-ranges do
    // not borrow a broader mesh's hierarchy, and never silently lose clusters.
    if cluster_tree[table+1u]!=inst.cluster_count || root<cluster_tree[2]
        || root+20u>cluster_tree[3] || !(inst.scale>0.0 && inst.scale<1e12)
        || !(wind>=0.0 && wind<1e12) || !(threshold>=0.0 && threshold<1e20)
        || !all(abs(inst.r0)<vec4<f32>(1e18)) || !all(abs(inst.r1)<vec4<f32>(1e18))
        || !all(abs(inst.r2)<vec4<f32>(1e18)) || !all(abs(cull_wind[id].xy)<vec2<f32>(1e12)) {
        if li==0u {atomicAdd(&counter[38],1u);}return false;
    }
    if li==0u {
        cluster_cursor=root;cluster_end=cluster_tree[root+16u];
        cluster_linear_bound=cluster_transform_bound(inst);
        cluster_error_scale=inst.scale*(1.0+length(cull_wind[id].xy));
        atomicAdd(&counter[32],1u);atomicAdd(&counter[39],inst.cluster_count);
    }
    workgroupBarrier();
    loop {
        // Uniform loads: WGSL's uniformity rules (enforced by browsers) do not
        // accept a barrier under a branch on a plain workgroup read.
        let node=workgroupUniformLoad(&cluster_cursor);
        if node>=workgroupUniformLoad(&cluster_end) {break;}
        let count=cluster_tree[node+18u];
        if li==0u {
            atomicAdd(&counter[33],1u);
            cluster_action=cluster_node_reject(node,inst,threshold,wind);
            if cluster_action!=0u {
                atomicAdd(&counter[34],1u);
                atomicAdd(&counter[36],cluster_tree[node+19u]);
            }else if count!=0u {atomicAdd(&counter[35],count);}
        }
        workgroupBarrier();
        if cluster_action==0u && li<count {
            let ci=cluster_tree[cluster_tree[node+17u]+li];
            cull_cluster(id,inst,ci,threshold,wind);
        }
        // Every lane follows the same node loop. No lane's local rejection
        // or empty work can skip a barrier or change the shared traversal.
        workgroupBarrier();
        if li==0u {
            cluster_cursor=select(node+20u,cluster_tree[node+16u],cluster_action!=0u || count!=0u);
        }
        workgroupBarrier();
    }
    return true;
}
