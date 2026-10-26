// Appended to the page consumer with a writable decision buffer. A complete
// certificate is per CURRENT view, after opaque depth and before root lists.
// No representative point tap or previous-frame-only decision can remove roots.
var<workgroup> meadow_certificate_rect:vec4<i32>;
var<workgroup> meadow_certificate_failed:atomic<u32>;
var<workgroup> meadow_certificate_visible:atomic<u32>;
var<workgroup> meadow_certificate_pixels:atomic<u32>;
@compute @workgroup_size(64)
fn certify_meadow_pages(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    if group.x>=meadow_pages.control.y {return;}
    let slot=meadow_pages.resident_slots[group.x/4u][group.x%4u];
    if lane==0u {
        atomicStore(&meadow_certificate_failed,0u);atomicStore(&meadow_certificate_visible,0u);atomicStore(&meadow_certificate_pixels,0u);
        meadow_certificate_rect=vec4<i32>(0,0,-1,-1);
        atomicAdd(&meadow_page_decisions[0],1u);
        let rectangle=meadow_page_rectangle(slot);
        let page=meadow_pages.pages[slot];let radius=length(page.upper.xyz-page.lower.xyz)*0.5;
        let fog_uncertain=view.fog_cull.y>0.5 && distance((page.lower.xyz+page.upper.xyz)*0.5,view.cam.xyz)+radius>view.fog_cull.x;
        if !rectangle.valid || fog_uncertain || any(rectangle.high<vec2<f32>(0.0)) || any(rectangle.low>=view.size.xy) {
            atomicStore(&meadow_certificate_failed,1u);
        } else {
            // Scan ALL pixel centres in the same expanded raster rectangle.
            // floor/ceil deliberately include a rounding border; never truncate
            // a large footprint into a cheaper partial certificate.
            let last=vec2<i32>(view.size.xy)-1;
            let first=clamp(vec2<i32>(floor(rectangle.low)),vec2<i32>(0),last);
            let end=clamp(vec2<i32>(ceil(rectangle.high)),vec2<i32>(0),last);
            let size=end-first+1;
            if any(size<=vec2<i32>(0)) || any(size>vec2<i32>(64)) {
                atomicStore(&meadow_certificate_failed,1u);
            } else {meadow_certificate_rect=vec4<i32>(first,end);}
        }
    }
    workgroupBarrier();
    let width=u32(max(0,meadow_certificate_rect.z-meadow_certificate_rect.x+1));
    let height=u32(max(0,meadow_certificate_rect.w-meadow_certificate_rect.y+1));
    var examined=0u;
    for(var index=lane;index<width*height;index+=64u) {
        if atomicLoad(&meadow_certificate_failed)!=0u {break;}
        let pixel=meadow_certificate_rect.xy+vec2<i32>(i32(index%width),i32(index/width));
        let sample=meadow_page_geometry(slot,vec2<f32>(pixel)+0.5);examined++;
        if !sample.valid {atomicStore(&meadow_certificate_failed,1u);break;}
        if sample.reveal<1.0 {atomicStore(&meadow_certificate_visible,1u);}
    }
    atomicAdd(&meadow_certificate_pixels,examined);
    workgroupBarrier();
    if lane==0u {
        atomicAdd(&meadow_page_decisions[3],atomicLoad(&meadow_certificate_pixels));
        if atomicLoad(&meadow_certificate_failed)==0u {
            atomicStore(&meadow_page_decisions[8u+slot],1u+2u*atomicLoad(&meadow_certificate_visible));
            atomicAdd(&meadow_page_decisions[1],1u);
        } else {atomicAdd(&meadow_page_decisions[5],1u);}
    }
}
// Runs after root suppression, producing a real compact page draw. A merely
// resident/certified page with no removed eligible roots cannot add geometry.
@compute @workgroup_size(1)
fn finalize_meadow_pages() {
    var useful=0u;
    for(var i=0u;i<meadow_pages.control.y;i++) {
        let slot=meadow_pages.resident_slots[i/4u][i%4u];
        let flags=atomicLoad(&meadow_page_decisions[8u+slot]);
        if flags==3u && atomicLoad(&meadow_page_decisions[264u+slot])>0u {
            atomicStore(&meadow_page_decisions[520u+useful],slot);useful++;
        }
    }
    atomicStore(&meadow_page_decisions[2],useful);
    atomicStore(&meadow_page_decisions[776],6u);
    atomicStore(&meadow_page_decisions[777],useful);
    atomicStore(&meadow_page_decisions[778],0u);
    atomicStore(&meadow_page_decisions[779],0u);
}
