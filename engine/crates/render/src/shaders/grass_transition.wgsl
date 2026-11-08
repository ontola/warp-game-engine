// Appended to shade.wgsl and grass_common.wgsl. Only partially transparent
// cards reach this pass; the unchanged cutout core writes normal opaque depth.
struct GrassRangeUniform { indices: vec4<u32>, shape: vec4<f32> };
@group(2) @binding(0) var<uniform> grass_range: GrassRangeUniform;
@group(3) @binding(3) var<storage,read> grass_owners: array<u32>;
struct GrassVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) owner: u32,
    @location(3) @interpolate(flat) opacity: f32,
};
@vertex
fn grass_vs(@builtin(vertex_index) vertex: u32, @builtin(instance_index) index: u32) -> GrassVertex {
    var out: GrassVertex;
    out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    let owner = grass_owners[index];
    let inst = instances[owner];
    if inst.cluster_count == 0u || (inst.flags & 8u) == 0u { return out; }
    let card = vertex / 6u;
    let root = vec3<f32>(inst.r0.w, inst.r1.w, inst.r2.w);
    let opacity = grass_card_opacity(root, view.cam.xyz, view.grass.xy, view.extra.w);
    if opacity <= 0.0 || opacity >= GRASS_OPAQUE_THRESHOLD { return out; }
    let corners = array<u32,6>(0u,2u,1u,1u,2u,3u);
    let corner = corners[vertex % 6u];
    let angle = PI * f32(card) / 3.0;
    let side = vec3<f32>(cos(angle), 0.0, sin(angle)) * grass_range.shape.x * 0.5;
    let local = select(-side, side, (corner & 1u) != 0u) + vec3<f32>(0.0, select(0.0, grass_range.shape.y, corner >= 2u), 0.0);
    out.world = wind_position(owner, inst.r1.w, xform(inst, local));
    out.position = view.view_proj * vec4<f32>(out.world, 1.0);
    out.uv = vec2<f32>(f32(corner & 1u), select(1.0, 0.0, corner >= 2u));
    out.owner = owner;
    out.opacity = opacity;
    return out;
}
// Both passes use identical derivatives, mip bias and alpha cutoff, including
// wind, jitter and dynamic owner transforms from the same grass_vs invocation.
fn grass_sample(in: GrassVertex) -> vec4<f32> {
    let mip_scale = select(1.0, view.extra.z, view.extra.z > 0.0);
    let dx = dpdx(in.uv) * mip_scale;
    let dy = dpdy(in.uv) * mip_scale;
    let m = material(grass_range.indices.z);
    let k = albedo_grad(m.layer, dx, dy);
    return textureSampleGrad(albedo, plant_samp, in.uv, m.layer, dx * k, dy * k);
}
@fragment fn grass_depth_fs(in: GrassVertex) {
    if grass_sample(in).a < 0.5 || in.opacity <= 0.0 { discard; }
}
fn grass_color(in: GrassVertex) -> vec4<f32> {
    let sample = grass_sample(in);
    let m = material(grass_range.indices.z);
    // Identical photo silhouette at the opaque/transition boundary. Opacity
    // fades its covered pixels continuously instead of eroding their shape.
    if sample.a < 0.5 || in.opacity <= 0.0 { discard; }
    let inst = instances[in.owner];
    let p = in.world;
    let height = clamp((p.y - inst.r1.w) / max(inst.scale * 0.5, 1e-4), 0.0, 1.0);
    let base = sample.rgb * m.tint * mix(0.8, 1.0, height) * 1.25 * (1.0 - light.fog.w * 0.25);
    let n = vec3<f32>(0.0,1.0,0.0);
    let v = normalize(view.cam.xyz - p);
    let l = light.sun_dir.xyz;
    var shadow = 1.0;
    if dot(n,l)>0.0 || gi.vol_dims.w != 0u || (m.flags & 32u) != 0u || m.roughness<0.55 {
        shadow = mix(1.0, shadow_at(p, n * sign(dot(n,l)+1e-4)), light.sun_color.w);
    }
    shadow*=mix(1.0,grass_sun_transmittance(p,l),light.sun_color.w);
    let sun_e = light.sun_color.rgb * light.sun_dir.w * shadow;
    let sky_vis = sky_visibility(p,n);
    let refl = reflect(-v,n);
    let rough = clamp(m.roughness,0.04,1.0);
    let contact = contact_visibility(p+n*0.025,n,refl,in.owner);
    var reflection_vis = sky_vis;
    if (m.flags & 32u) != 0u || rough<0.55 { reflection_vis=reflected_visibility(p,n,refl); }
    let response=grass_leaf_response(base,height,v,sun_e,shadow,sky_vis,contact,rough);
    let direct=response.direct+headlight_grass_radiance(p,v,base,rough);let indirect=response.indirect;let specular=response.specular;
    var rad = apply_distance_fog(direct+indirect+specular,distance(p,view.cam.xyz),-v);
    if light.sky.z>0.5 && light.sky.z<1.5 {rad=vec3<f32>(sky_vis)*6000.0;}
    else if light.sky.z>1.5 && light.sky.z<2.5 {rad=base*6000.0;}
    else if light.sky.z>2.5 && light.sky.z<3.5 {rad=direct;}
    else if light.sky.z>3.5 && light.sky.z<4.5 {rad=indirect;}
    else if light.sky.z>4.5 && light.sky.z<5.5 {rad=specular;}
    else if light.sky.z>5.5 && light.sky.z<6.5 {rad=vec3<f32>(contact.x)*6000.0;}
    else if light.sky.z>6.5 && light.sky.z<7.5 {rad=vec3<f32>(reflection_vis)*6000.0;}
    // A private depth buffer keeps the nearest covered fragment. Its opaque
    // limit is the same frontmost colour as the visibility pass, including
    // differently lit overlapping cards. Deeper partial layers are omitted.
    return vec4<f32>(rad*in.opacity,in.opacity);
}
@fragment fn grass_fs(in: GrassVertex) -> @location(0) vec4<f32> { return grass_color(in); }
struct GrassMotionOut { @location(0) color: vec4<f32>, @location(1) metadata: vec2<u32> };
@fragment fn grass_motion_fs(in: GrassVertex) -> GrassMotionOut {
    var out: GrassMotionOut;
    out.color = grass_color(in);
    // Same nearest-depth winner as colour. Owner+1 reserves zero for no layer.
    let coverage = u32(round(clamp(out.color.a,0.0,1.0)*255.0));
    out.metadata = vec2<u32>(((in.owner+1u)<<8u)|coverage,bitcast<u32>(in.position.z));
    return out;
}
