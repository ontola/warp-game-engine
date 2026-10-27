// MIN reversed depth over the actual normalized low-resolution pixel footprint.
// With ceil-sized odd targets the footprint can overlap three source pixels per
// axis. A sky zero must survive; MAX would incorrectly erase visible silhouettes.
@group(0) @binding(0) var opaque_depth:texture_depth_2d;
@vertex fn vs(@builtin(vertex_index)i:u32)->@builtin(position)vec4<f32> {
    let p=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
    return vec4<f32>(p[i],0.0,1.0);
}
@fragment fn fs(@builtin(position)p:vec4<f32>)->@builtin(frag_depth)f32 {
    let full=textureDimensions(opaque_depth);let low=(full+1u)/2u;let pixel=vec2<u32>(p.xy);
    let first=pixel*full/low;let end=min(((pixel+1u)*full+low-1u)/low,full);
    var depth=1.0;
    for(var y=first.y;y<end.y;y++){for(var x=first.x;x<end.x;x++){
        depth=min(depth,textureLoad(opaque_depth,vec2<i32>(i32(x),i32(y)),0));
    }}
    return depth;
}
