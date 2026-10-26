struct TerrainWorld {origin_cell:vec4<f32>,dims:vec4<u32>}
struct TerrainHierarchy {world:TerrainWorld,levels:array<vec4<u32>,16>,info:vec4<u32>,bounds:array<vec2<f32>>}
@group(0) @binding(0) var<uniform> build_level:vec4<u32>;
@group(0) @binding(1) var heights:texture_2d<f32>;
@group(0) @binding(2) var<storage,read_write> hierarchy:TerrainHierarchy;
fn triangle_bounds(h:vec3<f32>)->vec2<f32>{
    if hierarchy.info.z==1u && all(abs(h)<vec3<f32>(1e30)) && any(h<=vec3<f32>(-1e8)){return vec2<f32>(1e30,-1e30);}
    if !all(abs(h)<vec3<f32>(1e30)) || any(h<=vec3<f32>(-1e8)){return vec2<f32>(-1e30,1e30);}
    return vec2<f32>(min(h.x,min(h.y,h.z)),max(h.x,max(h.y,h.z)));
}
@compute @workgroup_size(8,8)
fn build(@builtin(global_invocation_id) id:vec3<u32>){
    let level=build_level.x;let level_info=hierarchy.levels[level];if any(id.xy>=level_info.yz){return;}
    var range=vec2<f32>(1e30,-1e30);
    if level==0u{
        if all(id.xy+vec2<u32>(1u)<hierarchy.world.dims.xy){
            let q=vec2<i32>(id.xy);
            let h=vec4<f32>(textureLoad(heights,q,0).r,textureLoad(heights,q+vec2<i32>(1,0),0).r,
                textureLoad(heights,q+vec2<i32>(0,1),0).r,textureLoad(heights,q+vec2<i32>(1,1),0).r);
            let a=triangle_bounds(h.xyz);let b=triangle_bounds(h.yzw);range=vec2<f32>(min(a.x,b.x),max(a.y,b.y));
        }
    }else{
        let prev=hierarchy.levels[level-1u];
        for(var k=0u;k<4u;k++){let q=id.xy*2u+vec2<u32>(k&1u,k>>1u);if all(q<prev.yz){let b=hierarchy.bounds[prev.x+q.y*prev.y+q.x];range=vec2<f32>(min(range.x,b.x),max(range.y,b.y));}}
    }
    hierarchy.bounds[level_info.x+id.y*level_info.y+id.x]=range;
}
