// Conservative world bounds -> dirty shadow tiles. Shared by source and voxel
// work generation. The caller supplies shadow_damage_words in its own group.
fn shadow_damage_word(i:u32)->u32 {return shadow_damage_words[i>>2u][i&3u];}
fn shadow_damage_rect(a:vec2<u32>,b:vec2<u32>)->bool {
    // Each 64-tile row is exactly two words; never loop over individual tiles.
    for(var y=a.y;y<=b.y;y++) {
        for(var word=a.x>>5u;word<=b.x>>5u;word++) {
            let first=max(a.x,word*32u)-word*32u;
            let last=min(b.x,word*32u+31u)-word*32u;
            let bits=(0xffffffffu<<first)&(0xffffffffu>>(31u-last));
            if (shadow_damage_word(y*2u+word)&bits)!=0u {return true;}
        }
    }
    return false;
}
fn shadow_damage_bounds(vp:mat4x4<f32>,size:f32,low:vec3<f32>,high:vec3<f32>)->bool {
    var lo=vec2<f32>(1e30);var hi=vec2<f32>(-1e30);
    for(var k=0u;k<8u;k++) {
        let p=vec3<f32>(select(low.x,high.x,(k&1u)!=0u),select(low.y,high.y,(k&2u)!=0u),select(low.z,high.z,(k&4u)!=0u));
        let q=vp*vec4<f32>(p,1.);
        // Unsupported projection: keep geometry, never incorrectly reject it.
        if !(q.w>0.000001) {return true;}
        let uv=vec2<f32>(q.x/q.w*.5+.5,.5-q.y/q.w*.5);
        lo=min(lo,uv);hi=max(hi,uv);
    }
    let margin=vec2<f32>(2./max(size,1.));lo-=margin;hi+=margin;
    if any(hi<vec2<f32>(0.)) || any(lo>vec2<f32>(1.)) {return false;}
    let a=vec2<u32>(clamp(floor(lo*64.),vec2<f32>(0.),vec2<f32>(63.)));
    let b=vec2<u32>(clamp(floor(hi*64.),vec2<f32>(0.),vec2<f32>(63.)));
    return shadow_damage_rect(a,b);
}
