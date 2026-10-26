// Experimental world-page consumer. No member loop: one bounded hash lookup
// and one depth-reprojected page sample. Uncertain visibility keeps the normal
// per-root producer. Pages contain unlit OIT sums, not baked illumination.
struct MeadowPage {
    vp:mat4x4<f32>, inverse:mat4x4<f32>,
    lower:vec4<f32>, upper:vec4<f32>, direction:vec4<f32>, identity:vec4<i32>,
};
struct MeadowPages {
    control:vec4<u32>, lookup:array<vec4<u32>,256>, resident_slots:array<vec4<u32>,64>, pages:array<MeadowPage,256>,
};
@group(2) @binding(7) var meadow_pages_texture:texture_2d_array<f32>;
@group(2) @binding(8) var meadow_pages_opaque:texture_depth_2d;
@group(2) @binding(9) var<uniform> meadow_pages:MeadowPages;
// Eight counters, 256 certificate flags, 256 removed-root counters, 256
// compact useful-page slots, and a four-word indirect draw. Per-view only.
@group(2) @binding(10) var<storage,read> meadow_page_decisions:array<u32>;
fn meadow_decision(index:u32)->u32 {return meadow_page_decisions[index];}
fn meadow_page_hash(key:vec2<i32>)->u32 {
    return ((bitcast<u32>(key.x)*0x9e3779b1u)^(bitcast<u32>(key.y)*0x85ebca77u))&1023u;
}
fn meadow_page_slot(root:vec3<f32>)->u32 {
    let key=vec2<i32>(floor(root.xz/(meadow.field.w*8.0)));
    let hash=meadow_page_hash(key);
    for(var probe=0u;probe<16u;probe++) {
        let address=(hash+probe)&1023u;
        let encoded=meadow_pages.lookup[address/4u][address%4u];
        if encoded==0u {return 256u;}
        let slot=encoded-1u;if slot>=256u {return 256u;}
        if all(meadow_pages.pages[slot].identity.xy==key) {return slot;}
    }
    return 256u;
}
struct MeadowPageSample {
    color:vec4<f32>, normal:vec4<f32>, reveal:f32,
    world:vec3<f32>, previous:vec3<f32>, depth:f32, valid:bool, coordinate:vec2<i32>,
};
fn meadow_page_geometry(slot:u32,pixel:vec2<f32>)->MeadowPageSample {
    var out:MeadowPageSample;
    if meadow_pages.control.x!=1u || slot>=256u {return out;}
    let page=meadow_pages.pages[slot];if page.identity.w==0 {return out;}
    let ndc=pixel/view.size.xy*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0);
    let far=view.inv_view_proj*vec4<f32>(ndc,0.00001,1.0);
    let ray=meadow_safe_normal(far.xyz/far.w-view.cam.xyz);
    // Rectangle corners need not hit the actual support box. The producer's
    // entire world-space wind envelope is inside this box; a miss is empty.
    if meadow_pages.control.w!=0u {
      var enter=0.0;var leave=1e30;
      for(var axis=0u;axis<3u;axis++) {
        if abs(ray[axis])<1e-12 {
            if view.cam[axis]<page.lower[axis] || view.cam[axis]>page.upper[axis] {
                out.valid=true;out.reveal=1.0;return out;
            }
        } else {
            let a=(page.lower[axis]-view.cam[axis])/ray[axis];
            let b=(page.upper[axis]-view.cam[axis])/ray[axis];
            enter=max(enter,min(a,b));leave=min(leave,max(a,b));
        }
      }
      if leave<enter {out.valid=true;out.reveal=1.0;return out;}
    }
    let center=(page.lower.xyz+page.upper.xyz)*0.5;
    let denominator=dot(ray,page.direction.xyz);
    if abs(denominator)<0.05 {return out;}
    var point=view.cam.xyz+ray*(dot(center-view.cam.xyz,page.direction.xyz)/denominator);
    var coordinate=vec2<i32>(0);var data=vec4<f32>(0.0);var projected=vec4<f32>(0.0);
    // Two bounded corrections account for translation using cached depth.
    // Angular/parallax eligibility was conservatively bounded on the CPU.
    for(var iteration=0u;iteration<3u;iteration++) {
        projected=page.vp*vec4<f32>(point,1.0);
        let uv=projected.xy*vec2<f32>(0.5,-0.5)+0.5;
        if any(uv<vec2<f32>(0.0)) || any(uv>=vec2<f32>(1.0)) {return out;}
        coordinate=vec2<i32>(uv*64.0);
        data=textureLoad(meadow_pages_texture,coordinate+vec2<i32>(128,0),i32(slot),0);
        if data.y<=0.0 {out.valid=true;out.reveal=1.0;return out;}
        let reconstructed=page.inverse*vec4<f32>(projected.xy,data.y,1.0);
        let t=dot(reconstructed.xyz/reconstructed.w-view.cam.xyz,ray);
        if t<=0.0 {return out;}
        point=view.cam.xyz+ray*t;
    }
    let nearest=page.inverse*vec4<f32>(projected.xy,min(data.y+0.001,1.0),1.0);
    let furthest=page.inverse*vec4<f32>(projected.xy,max(data.z-0.001,0.0),1.0);
    let a=dot(nearest.xyz/nearest.w-view.cam.xyz,ray);
    let b=dot(furthest.xyz/furthest.w-view.cam.xyz,ray);
    let lo=min(a,b);let hi=max(a,b);
    let opaque=textureLoad(meadow_pages_opaque,vec2<i32>(pixel),0);
    if opaque>0.0 {
        let world=view.inv_view_proj*vec4<f32>(ndc,opaque,1.0);
        let t=dot(world.xyz/world.w-view.cam.xyz,ray);
        // An opaque cut through the group cannot be represented by aggregate
        // moments: keep every member and its original opaque-depth test.
        if t>=lo-0.01 && t<=hi+0.01 {return out;}
        if t<lo {out.valid=true;out.reveal=1.0;return out;}
    }
    out.reveal=clamp(data.x,0.0,1.0);out.world=point;out.previous=point;out.coordinate=coordinate;
    let clip=view.view_proj*vec4<f32>(point,1.0);out.depth=clip.z/clip.w;
    out.valid=all(abs(clip)<vec4<f32>(1e30)) && clip.w>0.0 && out.depth>0.0 && out.depth<1.0;
    return out;
}
fn meadow_page_read(slot:u32,pixel:vec2<f32>)->MeadowPageSample {
    var out=meadow_page_geometry(slot,pixel);
    if !out.valid || out.reveal>=1.0 {return out;}
    let page=meadow_pages.pages[slot];let coordinate=out.coordinate;
    out.color=textureLoad(meadow_pages_texture,coordinate,i32(slot),0);
    out.normal=textureLoad(meadow_pages_texture,coordinate+vec2<i32>(64,0),i32(slot),0);
    if page.upper.w>0.5 {
        out.previous+=textureLoad(meadow_pages_texture,coordinate+vec2<i32>(192,0),i32(slot),0).xyz;
    }
    return out;
}
fn meadow_group_replaces(pixel:vec2<f32>,root:vec3<f32>)->bool {
    if meadow_pages.control.x!=1u {return false;}
    // Certified mode removed all eligible members during generation. Every
    // remaining root belongs to a complete fallback group, with no pixel mask.
    if meadow_pages.control.w!=0u {return false;}
    return meadow_page_read(meadow_page_slot(root),pixel).valid;
}
struct MeadowPageRectangle {low:vec2<f32>,high:vec2<f32>,valid:bool};
fn meadow_page_rectangle(slot:u32)->MeadowPageRectangle {
    var result:MeadowPageRectangle;var low=vec2<f32>(1e30);var high=vec2<f32>(-1e30);
    let page=meadow_pages.pages[slot];
    for(var k=0u;k<8u;k++) {
        let p=vec3<f32>(select(page.lower.x,page.upper.x,(k&1u)!=0u),select(page.lower.y,page.upper.y,(k&2u)!=0u),select(page.lower.z,page.upper.z,(k&4u)!=0u));
        let c=view.view_proj*vec4<f32>(p,1.0);
        if !all(abs(c)<vec4<f32>(1e30)) || c.w<=1e-5 || c.z<0.0 || c.z>=c.w-1e-6 {return result;}
        let pixel=(c.xy/c.w*vec2<f32>(0.5,-0.5)+0.5)*view.size.xy;
        low=min(low,pixel);high=max(high,pixel);
    }
    result.low=low-vec2<f32>(1.001);result.high=high+vec2<f32>(1.001);result.valid=true;return result;
}
@vertex fn meadow_page_vs(@builtin(vertex_index) vertex:u32,@builtin(instance_index) instance:u32)->MeadowAggregateVertex {
    var out:MeadowAggregateVertex;out.position=vec4<f32>(2.0,2.0,2.0,1.0);
    if instance>=meadow_pages.control.y {return out;}
    var slot=meadow_pages.resident_slots[instance/4u][instance%4u];
    if meadow_pages.control.w!=0u {slot=meadow_decision(520u+instance);}
    out.owner=slot;
    if slot>=256u || meadow_pages.pages[slot].identity.w==0 {return out;}
    if meadow_pages.control.w!=0u {
        let rectangle=meadow_page_rectangle(slot);if !rectangle.valid {return out;}
        let corners=array<vec2<f32>,6>(vec2<f32>(0.0,0.0),vec2<f32>(1.0,0.0),vec2<f32>(0.0,1.0),vec2<f32>(0.0,1.0),vec2<f32>(1.0,0.0),vec2<f32>(1.0,1.0));
        let pixel=mix(rectangle.low,rectangle.high,corners[vertex]);
        out.position=vec4<f32>(pixel/view.size.xy*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0),0.5,1.0);return out;
    }
    let page=meadow_pages.pages[slot];var lo=vec2<f32>(1e30);var hi=vec2<f32>(-1e30);
    for(var k=0u;k<8u;k++) {
        let p=vec3<f32>(select(page.lower.x,page.upper.x,(k&1u)!=0u),select(page.lower.y,page.upper.y,(k&2u)!=0u),select(page.lower.z,page.upper.z,(k&4u)!=0u));
        let c=view.view_proj*vec4<f32>(p,1.0);if c.w<=0.0 {return out;}
        lo=min(lo,c.xy/c.w);hi=max(hi,c.xy/c.w);
    }
    let corners=array<vec2<f32>,6>(vec2<f32>(0.0,0.0),vec2<f32>(1.0,0.0),vec2<f32>(0.0,1.0),vec2<f32>(0.0,1.0),vec2<f32>(1.0,0.0),vec2<f32>(1.0,1.0));
    out.position=vec4<f32>(mix(lo-2.0/view.size.xy,hi+2.0/view.size.xy,corners[vertex]),0.5,1.0);return out;
}
@fragment fn meadow_page_depth_fs(in:MeadowAggregateVertex)->@builtin(frag_depth) f32 {
    let p=meadow_page_read(in.owner,in.position.xy);if !p.valid || p.reveal>=1.0 {discard;}return p.depth;
}
@fragment fn meadow_page_motion_fs(in:MeadowAggregateVertex)->MeadowAggregateMotion {
    let p=meadow_page_read(in.owner,in.position.xy);if !p.valid || p.reveal>=1.0 {discard;}
    var out:MeadowAggregateMotion;out.depth=p.depth;out.reprojection=vec4<f32>(p.previous-p.world,distance(p.world,view.cam.xyz));return out;
}
@fragment fn meadow_page_fs(in:MeadowAggregateVertex)->MeadowAggregateAccumulation {
    let p=meadow_page_read(in.owner,in.position.xy);if !p.valid || p.reveal>=1.0 {discard;}
    var out:MeadowAggregateAccumulation;out.color=p.color;out.normal=p.normal;out.reveal=vec4<f32>(1.0-p.reveal);out.depth=p.depth;return out;
}
