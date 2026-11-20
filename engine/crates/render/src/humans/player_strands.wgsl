const PI:f32=3.14159265;
const STRAND_LIMIT:u32=16384u;
const VERTICES_PER_HEAD:u32=STRAND_LIMIT*54u;
struct Groom { fit:mat4x4<f32>, weights:vec4<f32>, tint:vec4<f32>, info:vec4<u32> }
struct Rows { x:vec4<f32>, y:vec4<f32>, z:vec4<f32> }
struct Owner { r:Rows, sphere:vec4<f32>, first:u32, count:u32, scale:f32, flags:u32 }
struct HairView { vp:mat4x4<f32>, focal:f32, slot:u32, clock:f32, heads:u32 }
@group(0) @binding(0) var<uniform> cam:HairView;
@group(0) @binding(1) var<storage,read> heads:array<Groom>;
@group(0) @binding(2) var<storage,read> owners:array<Owner>;
@group(0) @binding(3) var<storage,read> bones:array<Rows>;
@group(0) @binding(4) var<storage,read_write> arguments:array<vec4<u32>>;
// Grown hair (assets warp_hair): ten points per strand, [x, y, z, shade] in
// the groom's fit space. A groom of style 3 draws these; info.w is its first
// strand, weights.x how many it has, weights.y how wide they are drawn.
@group(0) @binding(9) var<storage,read> grown:array<vec4<f32>>;
fn grown_fibre(vid:u32,first:u32,width:f32)->Fibre {
    let corner=vid%6u;
    let upper=corner==1u || corner==4u || corner==5u;
    let side=select(-1.,1.,corner==2u || corner==3u || corner==5u);
    let k=(vid/6u)%9u+u32(upper);
    let base=(first+vid/54u)*10u;
    let at=grown[base+k];
    let before=grown[base+max(k,1u)-1u].xyz; let after=grown[base+min(k+1u,9u)].xyz;
    // A strand cut short repeats its last point: no direction, no width.
    let along=after-before;
    let tangent=along*inverseSqrt(max(dot(along,along),1.e-12));
    // Lit as part of a rounded head of hair, not as a lone cylinder.
    let normal=normalize(at.xyz-vec3<f32>(0.,.925,-.008));
    let t=f32(k)/9.;
    let radius=mix(.00013,.00024,strand_hash(base))*(1.-.85*t*t*t*t)*width;
    return Fibre(at.xyz,normal,cross(tangent,normal),tangent,at.w,side,radius);
}
@group(1) @binding(0) var<uniform> view:View;
@group(1) @binding(2) var<uniform> light:Light;
@group(1) @binding(5) var shadow_tex:texture_depth_2d;
@group(1) @binding(6) var shadow_samp:sampler_comparison;
@group(1) @binding(7) var sky_tex:texture_2d<f32>;
fn affine(r:Rows,p:vec4<f32>)->vec4<f32> {
    return vec4<f32>(dot(r.x,p),dot(r.y,p),dot(r.z,p),p.w);
}
@compute @workgroup_size(32) fn choose(@builtin(global_invocation_id) gid:vec3<u32>) {
    let i=gid.x; if i>=cam.heads{return;}
    let g=heads[i]; let owner=owners[g.info.x];
    let world=affine(owner.r,affine(bones[g.info.y],g.fit*vec4<f32>(0.,.94,0.,1.)));
    let clip=cam.vp*world;
    var count=0u;
    if owner.count>0u && (owner.flags & (256u<<cam.slot))==0u && clip.w>.08 && all(abs(clip.xy)<vec2<f32>(clip.w+.45)) {
        let head_pixels=.24*cam.focal/clip.w;
        let close_bob=select(0.,10240.*smoothstep(130.,280.,head_pixels),g.info.z==1u);
        count=u32(6144.*smoothstep(24.,142.,head_pixels)+close_bob);
        // Grown hair has no groom mesh under it: every strand it has, from
        // as far as a head is a few pixels.
        if g.info.z==3u {count=u32(g.weights.x*smoothstep(4.,40.,head_pixels));}
    }
    arguments[i]=vec4<u32>(count*54u,1u,i*VERTICES_PER_HEAD,0u);
}
struct StrandOut { @builtin(position) clip:vec4<f32>, @location(0) world:vec3<f32>, @location(1) normal:vec3<f32>, @location(2) color:vec3<f32>, @location(3) tangent:vec3<f32>, @location(4) edge:vec2<f32> }
@vertex fn player_strand(@builtin(vertex_index) id:u32)->StrandOut {
    let g=heads[id/VERTICES_PER_HEAD]; let r=owners[g.info.x].r; let b=bones[g.info.y];
    var f:Fibre;
    if g.info.z==3u {f=grown_fibre(id%VERTICES_PER_HEAD,g.info.w,g.weights.y);}
    else {f=grow_strand(id%VERTICES_PER_HEAD,g.info.z,g.info.w,g.weights,cam.clock);}
    let world=affine(r,affine(b,g.fit*vec4<f32>(f.p,1.)));
    let normal=normalize(affine(r,affine(b,g.fit*vec4<f32>(f.normal,0.))).xyz);
    let tangent=normalize(affine(r,affine(b,g.fit*vec4<f32>(f.tangent,0.))).xyz);
    let projected=cross(normalize(world.xyz-view.cam.xyz),tangent);
    let across=projected*inverseSqrt(max(dot(projected,projected),1.e-8));
    let pixel=max((cam.vp*world).w,.01)/cam.focal;
    let radius=f.radius*length(g.fit[1].xyz);
    let filtered=max(radius,.76*pixel);
    let edge=world.xyz+across*(f.side*filtered);
    return StrandOut(cam.vp*vec4<f32>(edge,1.),world.xyz,normal,mix(g.tint.rgb,vec3<f32>(.72,.71,.69),floor(f.tint*.5)/8.)*(f.tint-2.*floor(f.tint*.5)),tangent,vec2<f32>(f.side,radius/filtered));
}
@fragment fn strand_light(v:StrandOut)->@location(0) vec4<f32> {
    let n=normalize(v.normal);
    let sc=light.shadow_view_proj*vec4<f32>(v.world+n*.03,1.);
    let q=sc.xyz/sc.w;
    var visibility=1.;
    if all(abs(q.xy)<vec2<f32>(1.)) && q.z>0. && q.z<1. {
        visibility=mix(1.,textureSampleCompareLevel(shadow_tex,shadow_samp,q.xy*vec2<f32>(.5,-.5)+.5,q.z),light.sun_color.w);
    }
    let tangent=normalize(v.tangent);
    let longitudinal=dot(tangent,light.sun_dir.xyz);
    let wrap=(.19+.81*max(0.,dot(n,light.sun_dir.xyz)))*sqrt(max(.03,1.-longitudinal*longitudinal));
    let sun=wrap*light.sun_dir.w*visibility;
    let ambient=max(light.sh[0].rgb*.282095,vec3<f32>(light.fog_sampling.z));
    let eye=normalize(view.cam.xyz-v.world);
    let half_dir=normalize(light.sun_dir.xyz+eye);
    let along=dot(normalize(v.tangent),half_dir);
    let sheen=.021*pow(max(0.,1.-(along+.09)*(along+.09)),25.);
    let pigment=mix(v.color,vec3<f32>(dot(v.color,vec3<f32>(.3,.59,.11))),.19);
    let radiance=(pigment*ambient+(pigment+sheen)*sun*light.sun_color.rgb)/PI;
    let ray=v.world-view.cam.xyz; let d=length(ray);
    let alpha=.88*v.edge.y*(1.-smoothstep(.26,1.,abs(v.edge.x)));
    return vec4<f32>(alpha*apply_distance_fog(radiance,d,ray/max(d,.001)),alpha);
}
