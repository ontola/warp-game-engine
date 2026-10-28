// Exposed-linear highlight bloom, isolated to one reconstructed view.
// Never modify scene HDR/history. Storage is bounded well below half-float max.
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var output: texture_storage_2d<rgba16float,write>;
@group(0) @binding(2) var<uniform> params: vec4<f32>; // exposure, threshold, knee, cap (extract); (0, 0, 0, tap spacing) in wide rounds
@group(0) @binding(3) var linear_sampler: sampler;
// Screen-style glow sums every wide round: acc_out = acc_in + weight*round.
@group(0) @binding(4) var acc_in: texture_2d<f32>;
@group(0) @binding(5) var acc_out: texture_storage_2d<rgba16float,write>;

fn highlight(raw:vec3<f32>)->vec3<f32> {
    let finite=select(vec3<f32>(0.0),clamp(raw,vec3<f32>(0.0),vec3<f32>(65504.0)),raw==raw);
    let c=min(finite*params.x,vec3<f32>(params.w));
    // Peak-channel threshold also preserves saturated red lamp highlights.
    let brightness=max(c.x,max(c.y,c.z));
    // A negative knee asks for Godot's glow: smoothstep over -knee stops.
    if params.z<0.0 {return c*smoothstep(params.y,params.y-params.z,brightness);}
    let soft=clamp(brightness-params.y+params.z,0.0,2.0*params.z);
    let excess=max(brightness-params.y,soft*soft/(4.0*params.z));
    return c*(max(excess,0.0)/max(brightness,1e-6));
}

@compute @workgroup_size(8,8)
fn extract(@builtin(global_invocation_id) id:vec3<u32>) {
    if any(id.xy>=textureDimensions(output)) {return;}
    let size=textureDimensions(source);let base=id.xy*4u;
    var sum=vec3<f32>(0.0);var count=0.0;
    for(var y=0u;y<4u;y++) {for(var x=0u;x<4u;x++) {
        let p=base+vec2<u32>(x,y);if all(p<size) {sum+=highlight(textureLoad(source,vec2<i32>(p),0).rgb);count+=1.0;}
    }}
    textureStore(output,vec2<i32>(id.xy),vec4<f32>(sum/max(count,1.0),0.0));
}

fn filtered(p:vec2<u32>,axis:vec2<f32>)->vec4<f32> {
    // A wide round's uniform is (0, 0, 0, tap spacing); the first round
    // shares the extract uniform and samples neighbouring texels.
    let spacing=select(1.0,params.w,params.x==0.0 && params.y==0.0);
    let size=vec2<f32>(textureDimensions(source));let uv=(vec2<f32>(p)+0.5)/size;let d=axis*spacing/size;
    var c=textureSampleLevel(source,linear_sampler,uv,0.0)*0.227027027;
    c+=(textureSampleLevel(source,linear_sampler,uv+d*1.384615385,0.0)+textureSampleLevel(source,linear_sampler,uv-d*1.384615385,0.0))*0.316216216;
    c+=(textureSampleLevel(source,linear_sampler,uv+d*3.230769231,0.0)+textureSampleLevel(source,linear_sampler,uv-d*3.230769231,0.0))*0.070270270;
    return c;
}
@compute @workgroup_size(8,8)
fn horizontal(@builtin(global_invocation_id) id:vec3<u32>) {
    if any(id.xy>=textureDimensions(output)) {return;}
    textureStore(output,vec2<i32>(id.xy),filtered(id.xy,vec2<f32>(1.0,0.0)));
}
@compute @workgroup_size(8,8)
fn vertical(@builtin(global_invocation_id) id:vec3<u32>) {
    if any(id.xy>=textureDimensions(output)) {return;}
    let c=filtered(id.xy,vec2<f32>(0.0,1.0));
    textureStore(output,vec2<i32>(id.xy),c);
    // The first round (extract uniform) starts the sum at zero.
    var acc=vec4<f32>(0.0);
    if params.x==0.0 && params.y==0.0 {acc=textureLoad(acc_in,vec2<i32>(id.xy),0)+c*params.z;}
    textureStore(acc_out,vec2<i32>(id.xy),acc);
}
