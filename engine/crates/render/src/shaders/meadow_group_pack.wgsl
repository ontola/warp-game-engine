@group(0) @binding(0) var material:texture_2d_array<f32>;
@group(0) @binding(1) var nearest:texture_depth_2d;
@group(0) @binding(2) var furthest:texture_depth_2d;
@group(0) @binding(3) var motion:texture_2d<f32>;
@group(0) @binding(4) var pages:texture_storage_2d_array<rgba16float,write>;
@group(0) @binding(5) var<uniform> destination:vec4<u32>;
@compute @workgroup_size(8,8)
fn pack_page(@builtin(global_invocation_id) id:vec3<u32>) {
    if any(id.xy>=vec2<u32>(64u)) {return;}
    let p=vec2<i32>(id.xy);let layer=i32(destination.x);
    textureStore(pages,p,layer,textureLoad(material,p,0,0));
    textureStore(pages,p+vec2<i32>(64,0),layer,textureLoad(material,p,1,0));
    textureStore(pages,p+vec2<i32>(128,0),layer,vec4<f32>(textureLoad(material,p,2,0).x,textureLoad(nearest,p,0),textureLoad(furthest,p,0),0.0));
    textureStore(pages,p+vec2<i32>(192,0),layer,textureLoad(motion,p,0));
}
