// Packed current-frame receiver demand. One extra storage binding reaches the
// portable 16-buffer cull budget. Camera, lamps, pages and cached far maps never
// opt in: only ordinary Sun views set extra.z=-(map_index+1). The negative
// namespace is essential: positive extra.z also controls raster alpha mips.
@group(1) @binding(14) var<storage,read_write> receiver_demand:array<atomic<u32>>;
fn receiver_shadow_full_map(tiles:u32,requested:u32)->bool {
    return tiles>0u && tiles<=64u && requested==tiles*tiles;
}
fn receiver_shadow_keep(center:vec3<f32>,radius:f32)->bool {
    if view.params.z<0.5 || !(view.extra.z>=-5.0 && view.extra.z<=-1.0) {return true;}
    if atomicLoad(&receiver_demand[0])==0u {return true;}
    let generation=atomicLoad(&receiver_demand[1]);
    if generation==0u || atomicLoad(&receiver_demand[2])!=generation ||
        atomicLoad(&receiver_demand[3])!=0u {return true;}
    let map=u32(-view.extra.z)-1u;
    if map>=atomicLoad(&receiver_demand[4]) {return true;}
    let base=16u+map*32u;
    let size=atomicLoad(&receiver_demand[base+16u]);
    let tiles=atomicLoad(&receiver_demand[base+17u]);
    let levels=atomicLoad(&receiver_demand[base+18u]);
    if size==0u || tiles==0u || levels==0u {return true;}
    // A published full map retains every caster without any projection/OR
    // query. This is safe even if the projection differs: it cannot omit one.
    // Count separately; word30 remains actual projection/OR query work.
    if receiver_shadow_full_map(tiles,atomicLoad(&receiver_demand[base+28u])) {
        atomicAdd(&receiver_demand[base+19u],1u);return true;
    }
    // Fail open if the selected map is not the projection used by the builder.
    for(var col=0u;col<4u;col++) {for(var row=0u;row<4u;row++) {
        if bitcast<u32>(view.view_proj[col][row])!=atomicLoad(&receiver_demand[base+col*4u+row]) {return true;}
    }}
    let rect=demand_project(view.view_proj,center,vec3<f32>(radius),size,1.0);
    if rect.kind==2u {return true;}
    atomicAdd(&receiver_demand[base+30u],1u);
    var demanded=false;
    if rect.kind==1u {
        // Ascend until an inclusive rectangle spans at most two cells per axis.
        // The OR hierarchy can only retain extra casters, never omit demand.
        var level=0u;
        loop {
            let first=rect.first>>vec2<u32>(level);let last=rect.last>>vec2<u32>(level);
            if all(last-first<=vec2<u32>(1u)) || level+1u>=levels {
                let dim=max(tiles>>level,1u);
                let offset=atomicLoad(&receiver_demand[base+20u+level]);
                for(var y=first.y;y<=last.y;y++) {for(var x=first.x;x<=last.x;x++) {
                    demanded=demanded || atomicLoad(&receiver_demand[offset+y*dim+x])!=0u;
                }}
                break;
            }
            level++;
        }
    }
    if !demanded {atomicAdd(&receiver_demand[base+31u],1u);}
    return demanded;
}
