// Exposed-linear highlight bloom, isolated to one reconstructed view.
// Never modify scene HDR/history. Storage is bounded well below half-float max.
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var output: texture_storage_2d<rgba16float,write>;
@group(0) @binding(2) var<uniform> params: vec4<f32>; // exposure, threshold, knee, cap
@group(0) @binding(3) var linear_sampler: sampler;

fn highlight(raw:vec3<f32>)->vec3<f32> {
    let finite=select(vec3<f32>(0.0),clamp(raw,vec3<f32>(0.0),vec3<f32>(65504.0)),raw==raw);
    let c=min(finite*params.x,vec3<f32>(params.w));
    // Peak-channel threshold also preserves saturated red lamp highlights.
    let brightness=max(c.x,max(c.y,c.z));
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
    let size=vec2<f32>(textureDimensions(source));let uv=(vec2<f32>(p)+0.5)/size;let d=axis/size;
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
    textureStore(output,vec2<i32>(id.xy),filtered(id.xy,vec2<f32>(0.0,1.0)));
}
