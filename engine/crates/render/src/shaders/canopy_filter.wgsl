// Convert integrated fixed-point crown mass into density in common units.
// Each level is a separate Y slab; shading must clamp filtering inside it.
struct Parameters {
    origins:array<vec4<f32>,3>, dims:vec4<u32>, mass:vec4<f32>, state:vec4<u32>,
};
@group(1) @binding(0) var<uniform> params:Parameters;
@group(1) @binding(1) var<storage,read_write> density:array<atomic<u32>>;
@group(1) @binding(2) var filtered:texture_storage_3d<rgba16float,write>;

fn decode_ordered(key:u32)->f32 {
    return bitcast<f32>(select(~key,key^0x80000000u,(key&0x80000000u)!=0u));
}

fn support_kernel(u:f32,lo:f32,hi:f32,width:f32)->f32 {
    let lower=select(1.0,smoothstep(lo,lo+width,u),lo>-1.0);
    let upper=select(1.0,1.0-smoothstep(hi-width,hi,u),hi<1.0);
    return max(1.0-abs(u),0.0)*lower*upper;
}

// The windowed tent is piecewise degree <=4. Three-point Gauss quadrature
// integrates each interval exactly; no per-pixel normalization or sampling.
fn support_integral(lo:f32,hi:f32,width:f32)->f32 {
    let breaks=array<f32,5>(lo,lo+width,clamp(0.0,lo+width,hi-width),hi-width,hi);
    let nodes=array<f32,3>(-0.7745966692,0.0,0.7745966692);
    let weights=array<f32,3>(0.5555555556,0.8888888889,0.5555555556);
    var result=0.0;
    for(var k=0u;k<4u;k++) {
        // The triangle's kink at zero may fall inside either taper interval.
        let split=clamp(0.0,breaks[k],breaks[k+1u]);
        for(var side=0u;side<2u;side++) {
            let a=select(breaks[k],split,side==1u);let b=select(split,breaks[k+1u],side==1u);
            let m=(a+b)*0.5;let h=(b-a)*0.5;
            for(var j=0u;j<3u;j++) {result+=h*weights[j]*support_kernel(m+h*nodes[j],lo,hi,width);}
        }
    }
    return result;
}
@compute @workgroup_size(8,4,8)
fn main(@builtin(global_invocation_id) gid:vec3<u32>) {
    if gid.x>=params.dims.x || gid.y>=params.dims.y*3u || gid.z>=params.dims.z {return;}
    let level=gid.y/params.dims.y;
    if (params.state.x&(1u<<level))==0u {return;}
    let q=vec3<u32>(gid.x,gid.y%params.dims.y,gid.z);
    let count=params.dims.x*params.dims.y*params.dims.z;
    let compact=(params.state.w&2u)!=0u;
    let index=level*count*select(1u,3u,compact)+(q.z*params.dims.y+q.y)*params.dims.x+q.x;
    let pitch=params.origins[level].w;
    let value=f32(atomicLoad(&density[index]))*params.mass.y/(pitch*pitch*pitch);
    // A pathological number of overlapping crowns can exceed half float;
    // saturation here is already optically opaque, never integer wrapping.
    if !compact {
        textureStore(filtered,vec3<i32>(gid),vec4<f32>(min(value,65504.0),0.0,0.0,1.0));return;
    }
    if value<=0.0 {textureStore(filtered,vec3<i32>(gid),vec4<f32>(0.0));return;}
    let centre=params.origins[level].y+(f32(q.y)+0.5)*pitch;
    let lower=-decode_ordered(atomicLoad(&density[index+count]));
    let upper=decode_ordered(atomicLoad(&density[index+2u*count]));
    // Inward quantization is representable in half float and never puts
    // extinction above the source crown. Its <=pitch/1024 shift is <2.4cm
    // at the usual coarse level. Normalization uses these very same bounds.
    let lo=ceil(clamp((lower-centre)/pitch,-1.0,1.0)*1024.0)/1024.0;
    let hi=floor(clamp((upper-centre)/pitch,-1.0,1.0)*1024.0)/1024.0;
    let width=min(0.25*(hi-lo),0.5);
    if width<=0.0 {textureStore(filtered,vec3<i32>(gid),vec4<f32>(0.0));return;}
    let integral=support_integral(lo,hi,width);
    textureStore(filtered,vec3<i32>(gid),vec4<f32>(min(value/max(integral,1e-12),65504.0),lo,hi,width));
}
