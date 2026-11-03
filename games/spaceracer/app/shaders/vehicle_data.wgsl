// Game-owned aux block, referenced from the reserved header word 5.
// Header: wear texture layer, followed by six 24-word vehicle records.
fn sr_vehicle(instance:u32)->u32 {
    if arrayLength(&aux)<8u || aux[5]==0u {return 0u;}
    let base=aux[5]+1u;
    if instance<aux[base] || instance>=aux[base+121u] {return 0u;}
    for(var i=0u;i<6u;i++) {let b=base+i*24u;if instance>=aux[b] && instance<aux[b+1u] {return b;}}
    return 0u;
}
fn sr_v3(b:u32)->vec3<f32>{return vec3<f32>(bitcast<f32>(aux[b]),bitcast<f32>(aux[b+1u]),bitcast<f32>(aux[b+2u]));}
fn sr_hull(p:vec3<f32>,b:u32)->vec3<f32>{
    let q=p-sr_v3(b+4u);
    return vec3<f32>(dot(q,sr_v3(b+8u)),dot(q,sr_v3(b+12u)),dot(q,sr_v3(b+16u)));
}
