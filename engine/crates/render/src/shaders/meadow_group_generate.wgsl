// Only the certified far-root pipeline includes this dependency. Layout and
// integer world-group identity match the page consumer, including negatives.
struct MeadowPage {
    vp:mat4x4<f32>, inverse:mat4x4<f32>,
    lower:vec4<f32>, upper:vec4<f32>, direction:vec4<f32>, identity:vec4<i32>,
};
struct MeadowPages {
    control:vec4<u32>, lookup:array<vec4<u32>,256>, resident_slots:array<vec4<u32>,64>, pages:array<MeadowPage,256>,
};
@group(2) @binding(0) var<uniform> meadow_replacement_pages:MeadowPages;
@group(2) @binding(1) var<storage,read_write> meadow_replacement_decisions:array<atomic<u32>>;
fn meadow_retain_certified_root(cell:vec2<i32>,root:vec3<f32>)->bool {
    if meadow_replacement_pages.control.w==0u {return true;}
    // Right shift of signed integers performs floor division by eight. Using
    // truncating signed division would misassign negative world cells.
    let key=cell>>vec2<u32>(3u);
    let hash=((bitcast<u32>(key.x)*0x9e3779b1u)^(bitcast<u32>(key.y)*0x85ebca77u))&1023u;
    for(var probe=0u;probe<16u;probe++) {
        let address=(hash+probe)&1023u;
        let encoded=meadow_replacement_pages.lookup[address/4u][address%4u];
        if encoded==0u {return true;}
        let slot=encoded-1u;if slot>=256u {return true;}
        if all(meadow_replacement_pages.pages[slot].identity.xy==key) {
            if (atomicLoad(&meadow_replacement_decisions[8u+slot])&1u)==0u {return true;}
            atomicAdd(&meadow_replacement_decisions[4],1u);
            atomicAdd(&meadow_replacement_decisions[264u+slot],1u);
            return false;
        }
    }
    return true;
}
