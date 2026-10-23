// Convert integrated fixed-point crown mass into density in common units.
// Each level is a separate Y slab; shading must clamp filtering inside it.
struct Parameters {
    origins:array<vec4<f32>,3>, dims:vec4<u32>, mass:vec4<f32>, state:vec4<u32>,
};
@group(1) @binding(0) var<uniform> params:Parameters;
@group(1) @binding(1) var<storage,read_write> density:array<atomic<u32>>;
@group(1) @binding(2) var filtered:texture_storage_3d<rgba16float,write>;
@compute @workgroup_size(8,4,8)
fn main(@builtin(global_invocation_id) gid:vec3<u32>) {
    if gid.x>=params.dims.x || gid.y>=params.dims.y*3u || gid.z>=params.dims.z {return;}
    let level=gid.y/params.dims.y;
    if (params.state.x&(1u<<level))==0u {return;}
    let q=vec3<u32>(gid.x,gid.y%params.dims.y,gid.z);
    let count=params.dims.x*params.dims.y*params.dims.z;
    let index=level*count+(q.z*params.dims.y+q.y)*params.dims.x+q.x;
    let pitch=params.origins[level].w;
    let value=f32(atomicLoad(&density[index]))*params.mass.y/(pitch*pitch*pitch);
    // A pathological number of overlapping crowns can exceed half float;
    // saturation here is already optically opaque, never integer wrapping.
    textureStore(filtered,vec3<i32>(gid),vec4<f32>(min(value,65504.0),0.0,0.0,1.0));
}
