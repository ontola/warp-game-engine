struct Params { scale: f32, max_pixels: f32, pad: vec2<f32> };
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var velocity: texture_2d<f32>;
@group(0) @binding(2) var depth: texture_2d<f32>;
@group(0) @binding(3) var bias: texture_2d<f32>;
@group(0) @binding(4) var output: texture_storage_2d<rgba16float,write>;
@group(0) @binding(5) var linear_sampler: sampler;
@group(0) @binding(6) var<uniform> params: Params;

fn motion_pixel(uv: vec2<f32>) -> vec2<i32> {
    let size=vec2<i32>(textureDimensions(velocity));
    return clamp(vec2<i32>(uv*vec2<f32>(size)),vec2<i32>(0),size-1);
}
@compute @workgroup_size(8,8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size=vec2<f32>(textureDimensions(source));
    if any(id.xy>=vec2<u32>(size)) {return;}
    let uv=(vec2<f32>(id.xy)+.5)/size;
    let p=motion_pixel(uv);
    let original=textureLoad(source,vec2<i32>(id.xy),0);
    let motion_size=vec2<f32>(textureDimensions(velocity));
    var v=textureLoad(velocity,p,0).xy/motion_size*params.scale;
    var length_px=length(v*size);
    // Invalid/recycled instances and camera cuts never create giant streaks.
    if length_px<.75 || textureLoad(bias,p,0).r>.5 {
        textureStore(output,vec2<i32>(id.xy),original);return;
    }
    v*=min(1.,params.max_pixels/max(length_px,.001));
    length_px=min(length_px,params.max_pixels);
    let center_depth=textureLoad(depth,p,0).r;
    var sum=original.rgb;var weight=1.;
    for(var i=0u;i<12u;i++) {
        let t=(f32(i)+.5)/12.-.5;
        let at=clamp(uv+v*t,.5/size,vec2<f32>(1.)-.5/size);
        let q=motion_pixel(at);
        let other=textureLoad(velocity,q,0).xy/motion_size*params.scale;
        let moving=length(other*size);
        let d=textureLoad(depth,q,0).r;
        // Do not drag the nearly stationary player's craft over the road,
        // or gather a foreground silhouette into a distant background streak.
        let foreground=select(1.,0.,d>center_depth*1.18+.00001);
        let w=smoothstep(0.,.6,moving/max(length_px,.001))*foreground
            *(1.-step(.5,textureLoad(bias,q,0).r));
        sum+=textureSampleLevel(source,linear_sampler,at,0.).rgb*w;weight+=w;
    }
    textureStore(output,vec2<i32>(id.xy),vec4<f32>(sum/weight,original.a));
}
