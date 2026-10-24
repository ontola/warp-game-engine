// Appended to shade.wgsl and grass_common.wgsl. Only partially transparent
// cards reach this pass; the unchanged cutout core writes normal opaque depth.
struct GrassRangeUniform { indices: vec4<u32>, shape: vec4<f32> };
@group(2) @binding(0) var<uniform> grass_range: GrassRangeUniform;
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
    let owner = grass_range.indices.x + index;
    let inst = instances[owner];
    if inst.cluster_count == 0u || (inst.pad & 8u) == 0u { return out; }
    let card = vertex / 6u;
    let root = vec3<f32>(inst.r0.w, inst.r1.w, inst.r2.w);
    let opacity = grass_card_opacity(root, view.cam.xyz, view.extra.y > 0.5);
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
@fragment
fn grass_fs(in: GrassVertex) -> @location(0) vec4<f32> {
    let dx = dpdx(in.uv);
    let dy = dpdy(in.uv);
    let m = materials[grass_range.indices.z];
    let sample = textureSampleGrad(albedo, plant_samp, in.uv, m.layer, dx, dy);
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
    let sun_e = light.sun_color.rgb * light.sun_dir.w * shadow;
    let sky_vis = sky_visibility(p,n);
    let refl = reflect(-v,n);
    let rough = clamp(m.roughness,0.04,1.0);
    let f0 = vec3<f32>(0.016);
    let ao = mix(0.45,1.0,height);
    let contact = contact_visibility(p+n*0.025,n,refl,in.owner);
    let bounce = gi.bounce.rgb * (gi.bounce.w*sky_vis + light.sun_dir.w*max(light.sun_dir.y,0.0)*shadow);
    let ambient = (sky_irradiance(n)*sky_vis + bounce*(1.0-sky_vis))*ao*contact.x;
    let spec_contact = mix(contact.y,contact.x,smoothstep(0.08,0.6,rough));
    var reflection_vis = sky_vis;
    if (m.flags & 32u) != 0u || rough<0.55 { reflection_vis=reflected_visibility(p,n,refl); }
    let env = mix(bounce/PI,environment(refl,rough,bounce),reflection_vis)*spec_contact;
    let back = vec3<f32>(0.12,0.16,0.05)*max(-dot(n,l),0.0);
    let diffuse = base*(1.0-m.metal)/PI;
    let direct = diffuse*sun_e*max(dot(n,l),0.0) + sun_e*back/PI + sun_e*ggx(n,v,l,rough,f0);
    let indirect = diffuse*ambient;
    let specular = env*env_brdf(f0,rough,max(dot(n,v),1e-4))*ao;
    let dist = distance(p,view.cam.xyz);
    let fog = max(clamp((dist-light.fog_color.w)/max(light.fog.x-light.fog_color.w,1.0),0.0,1.0),1.0-exp(-light.fog.y*dist));
    var rad = mix(direct+indirect+specular,light.fog_color.rgb,fog);
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
