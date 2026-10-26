// Shared world-space heightfield visibility. No camera state or implicit sky.
struct TerrainWorld { origin_cell:vec4<f32>, dims:vec4<u32> }
struct TerrainHierarchy { world:TerrainWorld, levels:array<vec4<u32>,16>, info:vec4<u32>, bounds:array<vec2<f32>> }
@group(0) @binding(24) var<storage,read> terrain_hierarchy:TerrainHierarchy;
struct TerrainRay { origin_min:vec4<f32>, direction_max:vec4<f32> }
struct TerrainTraceResult { info:vec4<u32>, position_t:vec4<f32>, normal:vec4<f32> }
// info: status(clear=0/hit=1/unknown=2), reason, visited nodes, triangle tests.
// reason: none=0/outside=1/unknown-height=2/budget=3/invalid=4/stack=5.
fn terrain_result(status:u32,reason:u32,t:f32,p:vec3<f32>,d:vec3<f32>,n:vec3<f32>,nodes:u32,tests:u32)->TerrainTraceResult {
    return TerrainTraceResult(vec4<u32>(status,reason,nodes,tests),vec4<f32>(p+d*t,t),vec4<f32>(n,0.0));
}
fn terrain_known(h:f32)->bool {return h > -1e8 && abs(h)<1e30;}
fn terrain_empty(h:vec3<f32>)->bool {return terrain_hierarchy.info.z==1u && all(abs(h)<vec3<f32>(1e30)) && any(h<=vec3<f32>(-1e8));}
fn terrain_clip(p:vec3<f32>,d:vec3<f32>,lo:vec2<f32>,hi:vec2<f32>,range:vec2<f32>)->vec2<f32>{
    var a=range.x;var b=range.y;
    for(var axis=0u;axis<2u;axis++){
        let v=select(p.x,p.z,axis==1u);let r=select(d.x,d.z,axis==1u);
        if abs(r)<1e-20 {if v<lo[axis] || v>hi[axis] {return vec2<f32>(1.0,-1.0);}}
        else {let q=(lo[axis]-v)/r;let s=(hi[axis]-v)/r;a=max(a,min(q,s));b=min(b,max(q,s));}
    }
    return vec2<f32>(a,b);
}
fn terrain_leaf(heights:texture_2d<f32>,world:TerrainWorld,p:vec3<f32>,d:vec3<f32>,cell:vec2<u32>,range:vec2<f32>,nodes:u32,old_tests:u32)->TerrainTraceResult {
    let q=(p.xz-world.origin_cell.xy)/world.origin_cell.z-vec2<f32>(cell);
    let rate=d.xz/world.origin_cell.z;let sum=rate.x+rate.y;
    var split=range.y;
    if abs(sum)>1e-20 {let t=(1.0-q.x-q.y)/sum;if t>range.x&&t<range.y{split=t;}}
    let ij=vec2<i32>(cell);
    let h=vec4<f32>(textureLoad(heights,ij,0).r,textureLoad(heights,ij+vec2<i32>(1,0),0).r,
        textureLoad(heights,ij+vec2<i32>(0,1),0).r,textureLoad(heights,ij+vec2<i32>(1,1),0).r);
    var tests=old_tests;
    for(var part=0u;part<2u;part++){
        let a=select(range.x,split,part==1u);let b=select(split,range.y,part==1u);
        if b<a || (a==b&&range.x!=range.y){continue;}
        tests++;
        let upper=q.x+q.y+sum*(a+b)*0.5>1.0;
        var coefficients=vec3<f32>(h.x,h.y-h.x,h.z-h.x);
        var values=h.xyz;var valid=terrain_known(h.x)&&terrain_known(h.y)&&terrain_known(h.z);
        if upper{values=h.yzw;coefficients=vec3<f32>(h.y+h.z-h.w,h.w-h.z,h.w-h.y);valid=terrain_known(h.y)&&terrain_known(h.z)&&terrain_known(h.w);}
        if terrain_empty(values){continue;}
        if !valid{return terrain_result(2u,2u,a,p,d,vec3<f32>(0.0),nodes,tests);}
        let intercept=p.y-coefficients.x-dot(coefficients.yz,q);
        let slope=d.y-dot(coefficients.yz,rate);let da=intercept+slope*a;let db=intercept+slope*b;
        if da<=0.0 || db<=0.0 {
            var t=a;if da>0.0{t=clamp(-intercept/slope,a,b);}
            let n=normalize(vec3<f32>(-coefficients.y/world.origin_cell.z,1.0,-coefficients.z/world.origin_cell.z));
            return terrain_result(1u,0u,t,p,d,n,nodes,tests);
        }
    }
    return terrain_result(0u,0u,range.y,p,d,vec3<f32>(0.0),nodes,tests);
}
fn terrain_trace(heights:texture_2d<f32>,ray:TerrainRay,max_nodes:u32)->TerrainTraceResult {
    let world=terrain_hierarchy.world;
    let p=ray.origin_min.xyz;let length2=dot(ray.direction_max.xyz,ray.direction_max.xyz);
    var d=vec3<f32>(0.0,1.0,0.0);
    if !all(abs(p)<vec3<f32>(1e30)) || !all(abs(ray.direction_max)<vec4<f32>(1e30))
        || !(length2>1e-20) || !(length2<=3.4028234663852886e38)
        || !(ray.origin_min.w>=0.0) || !(ray.direction_max.w>=ray.origin_min.w){
        return terrain_result(2u,4u,0.0,p,d,vec3<f32>(0.0),0u,0u);
    }
    d=ray.direction_max.xyz*inverseSqrt(length2);
    let low=world.origin_cell.xy;let high=low+vec2<f32>(world.dims.xy-vec2<u32>(1u))*world.origin_cell.z;
    let start=p+d*ray.origin_min.w;
    if terrain_hierarchy.info.z==0u && (any(start.xz<low)||any(start.xz>high)){return terrain_result(2u,1u,ray.origin_min.w,p,d,vec3<f32>(0.0),0u,0u);}
    let root=terrain_clip(p,d,low,high,vec2<f32>(ray.origin_min.w,ray.direction_max.w));
    if root.y<root.x{if terrain_hierarchy.info.z==1u{return terrain_result(0u,0u,ray.direction_max.w,p,d,vec3<f32>(0.0),0u,0u);}
        return terrain_result(2u,1u,ray.origin_min.w,p,d,vec3<f32>(0.0),0u,0u);}
    // Only IDs live on the stack. Intervals are recomputed, bounding private
    // storage at 1 KiB/ray independently of world distance or resolution.
    var stack:array<vec4<u32>,64>;var count=1u;
    stack[0]=vec4<u32>(0u,0u,terrain_hierarchy.info.x-1u,0u);
    var visited=0u;var tests=0u;var certified=root.x;
    loop {
        if count==0u{break;}
        count--;let node=stack[count];let span=f32(1u<<node.z)*world.origin_cell.z;
        let lo=low+vec2<f32>(node.xy)*span;let hi=min(lo+vec2<f32>(span),high);
        let interval=terrain_clip(p,d,lo,hi,root);
        if visited>=min(max_nodes,4096u){return terrain_result(2u,3u,interval.x,p,d,vec3<f32>(0.0),visited,tests);}
        visited++;
        let level_info=terrain_hierarchy.levels[node.z];
        let bounds=terrain_hierarchy.bounds[level_info.x+node.y*level_info.y+node.x];
        let low_y=min(p.y+d.y*interval.x,p.y+d.y*interval.y);
        let epsilon=1e-5+8.0*1.1920928955078125e-7*max(abs(low_y),abs(bounds.y));
        if low_y>bounds.y+epsilon{certified=interval.y;continue;}
        if node.z==0u {
            let r=terrain_leaf(heights,world,p,d,node.xy,interval,visited,tests);tests=r.info.w;
            if r.info.x!=0u{return r;}certified=interval.y;continue;
        }
        let level=node.z-1u;let size=terrain_hierarchy.levels[level].yz;let child_span=span*0.5;
        var children:array<vec4<u32>,4>;var starts:array<f32,4>;var child_count=0u;
        for(var k=0u;k<4u;k++){
            let ij=node.xy*2u+vec2<u32>(k&1u,k>>1u);if any(ij>=size){continue;}
            let a=low+vec2<f32>(ij)*child_span;let b=min(a+vec2<f32>(child_span),high);
            if any(a>=b){continue;}
            let t=terrain_clip(p,d,a,b,interval);
            if t.y>t.x || (interval.x==interval.y&&t.y==t.x){
                // Descending entry order: popping is nearest first.
                var at=child_count;
                loop{if at==0u{break;}if starts[at-1u]>=t.x{break;}starts[at]=starts[at-1u];children[at]=children[at-1u];at--;}
                starts[at]=t.x;children[at]=vec4<u32>(ij,level,0u);child_count++;
            }
        }
        if count+child_count>64u{return terrain_result(2u,5u,interval.x,p,d,vec3<f32>(0.0),visited,tests);}
        for(var k=0u;k<child_count;k++){stack[count]=children[k];count++;}
    }
    if terrain_hierarchy.info.z==0u && root.y<ray.direction_max.w{return terrain_result(2u,1u,certified,p,d,vec3<f32>(0.0),visited,tests);}
    return terrain_result(0u,0u,ray.direction_max.w,p,d,vec3<f32>(0.0),visited,tests);
}

// AboveTerrainBounds=3 is terrain-only certification, never implicit mesh/sky radiance.
fn terrain_trace_upward(heights:texture_2d<f32>,origin:vec3<f32>,direction:vec3<f32>,t_min:f32,max_nodes:u32)->TerrainTraceResult {
    let level=terrain_hierarchy.levels[terrain_hierarchy.info.x-1u];
    let maximum=terrain_hierarchy.bounds[level.x].y;
    if maximum>=1e29{return terrain_result(2u,2u,t_min,origin,vec3<f32>(0.0),vec3<f32>(0.0),0u,0u);}
    let length2=dot(direction,direction);
    if !all(abs(direction)<vec3<f32>(1e30))||!(length2>1e-20)||!(length2<=3.4028234663852886e38)||!(direction.y>0.0){return terrain_result(2u,4u,t_min,origin,vec3<f32>(0.0),vec3<f32>(0.0),0u,0u);}
    let d=normalize(direction);let end=max(t_min,(maximum+0.01-origin.y)/d.y);
    var result=terrain_trace(heights,TerrainRay(vec4<f32>(origin,t_min),vec4<f32>(d,end)),max_nodes);
    if result.info.x==0u{result.info.x=3u;}return result;
}
