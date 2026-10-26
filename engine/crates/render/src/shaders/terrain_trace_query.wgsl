struct TerrainQueries {count_budget:vec4<u32>}
@group(0) @binding(0) var<uniform> query:TerrainQueries;
@group(0) @binding(1) var trace_heights:texture_2d<f32>;
@group(0) @binding(3) var<storage,read> trace_rays:array<TerrainRay>;
@group(0) @binding(4) var<storage,read_write> trace_results:array<TerrainTraceResult>;
@compute @workgroup_size(32)
fn query_terrain(@builtin(global_invocation_id) id:vec3<u32>){
    if id.x>=query.count_budget.x{return;}
    if query.count_budget.z==1u {trace_results[id.x]=terrain_trace_upward(trace_heights,trace_rays[id.x].origin_min.xyz,trace_rays[id.x].direction_max.xyz,trace_rays[id.x].origin_min.w,query.count_budget.y);}
    else {trace_results[id.x]=terrain_trace(trace_heights,trace_rays[id.x],query.count_budget.y);}
}
