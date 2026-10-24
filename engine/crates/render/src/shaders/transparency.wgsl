// Appended to shade.wgsl so glass uses the same sun, environment, terrain and
// canopy visibility as opaque surfaces. No noisy coverage or refraction hack.
struct BlendTriangle {
    vertices: vec3<u32>,
    owner: u32,
    material: u32,
    alpha: f32,
    double_sided: u32,
    padding: u32,
};
@group(2) @binding(0) var<storage, read> blend_triangles: array<BlendTriangle>;
@group(2) @binding(1) var<storage, read> blend_order: array<u32>;

struct BlendVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) triangle: u32,
};

@vertex
fn transparent_vs(@builtin(vertex_index) corner: u32, @builtin(instance_index) index: u32) -> BlendVertex {
    let id = blend_order[index];
    let tri = blend_triangles[id];
    let inst = instances[tri.owner];
    let v = vtx(tri.vertices[corner]);
    let local_normal = vec3<f32>(vertices[v + 3u], vertices[v + 4u], vertices[v + 5u]);
    let ax = vec3<f32>(inst.r0.x, inst.r1.x, inst.r2.x);
    let ay = vec3<f32>(inst.r0.y, inst.r1.y, inst.r2.y);
    let az = vec3<f32>(inst.r0.z, inst.r1.z, inst.r2.z);
    // Inverse transpose for the importer/instance's orthogonal scaled axes.
    let normal = ax * local_normal.x / max(dot(ax, ax), 1e-8)
        + ay * local_normal.y / max(dot(ay, ay), 1e-8)
        + az * local_normal.z / max(dot(az, az), 1e-8);
    var out: BlendVertex;
    out.world = wind_position(tri.owner, inst.r1.w, xform(inst, pos_of(v)));
    out.position = view.view_proj * vec4<f32>(out.world, 1.0);
    out.normal = safe_normalize(wind_normal(tri.owner, normal));
    out.uv = vec2<f32>(vertices[v + 6u], vertices[v + 7u]);
    out.triangle = id;
    return out;
}

fn transparent_color(in: BlendVertex, front: bool) -> vec4<f32> {
    // Evaluate derivatives before any per-material discard or dynamic branch.
    let mip_scale = select(1.0, view.extra.z, view.extra.z > 0.0);
    let dx = dpdx(in.uv) * mip_scale;
    let dy = dpdy(in.uv) * mip_scale;
    let tri = blend_triangles[in.triangle];
    let m = materials[tri.material];
    if !front && tri.double_sided == 0u { discard; }
    var sample = vec4<f32>(1.0);
    if m.layer >= 0 { sample = textureSampleGrad(albedo, samp, in.uv, m.layer, dx, dy); }
    let alpha = clamp(sample.a * tri.alpha, 0.0, 1.0);
    if alpha <= 0.0 { discard; }
    let base = sample.rgb * m.tint;
    let p = in.world;
    let v = normalize(view.cam.xyz - p);
    var n = safe_normalize(in.normal);
    if tri.double_sided != 0u && dot(n, v) < 0.0 { n = -n; }
    // The interpolated authored normal keeps GI/bias continuous across a
    // smooth pane's triangles, matching the opaque base-normal policy.
    let visibility_n = n;
    let l = light.sun_dir.xyz;
    let s_map = shadow_at(p, visibility_n * sign(dot(visibility_n, l) + 1e-4));
    let shadow = mix(1.0, s_map, light.sun_color.w);
    let sun_e = light.sun_color.rgb * light.sun_dir.w * shadow;
    let sky_vis = sky_visibility(p, visibility_n);
    let refl = reflect(-v, n);
    let rough = clamp(m.roughness, 0.04, 1.0);
    let f0 = mix(vec3<f32>(0.04), base, m.metal);
    let diffuse = base * (1.0 - m.metal) / PI;
    let contact = contact_visibility(p + visibility_n * 0.025, visibility_n, refl, tri.owner);
    let bounce = gi.bounce.rgb * (gi.bounce.w * sky_vis + light.sun_dir.w * max(light.sun_dir.y, 0.0) * shadow);
    let ambient = (sky_irradiance(n) * sky_vis + bounce * (1.0 - sky_vis)) * contact.x;
    var reflection_vis = sky_vis;
    if (m.flags & 32u) != 0u || rough < 0.55 { reflection_vis = reflected_visibility(p, visibility_n, refl); }
    let spec_contact = mix(contact.y, contact.x, smoothstep(0.08, 0.6, rough));
    let env = mix(bounce / PI, environment(refl, rough, bounce), reflection_vis) * spec_contact;
    let ndv = max(dot(n, v), 1e-4);
    var direct = diffuse * sun_e * max(dot(n, l), 0.0) + sun_e * ggx(n, v, l, rough, f0);
    var indirect = diffuse * ambient;
    var specular = env * env_brdf(f0, rough, ndv);
    if (m.flags & 32u) != 0u {
        let fc = 0.04 + 0.96 * pow(1.0 - ndv, 5.0);
        direct = direct * (1.0 - fc) + sun_e * ggx(n, v, l, 0.04, vec3<f32>(0.04));
        indirect *= 1.0 - fc;
        specular = specular * (1.0 - fc) + mix(bounce / PI, environment(refl, 0.04, bounce), reflection_vis) * contact.y * fc;
    }
    direct += headlight_radiance(p,n,v,diffuse,rough,f0);
    var rad = direct + indirect + specular + car_lamp_emission(m.flags, base);
    rad = apply_distance_fog(rad, distance(p, view.cam.xyz), -v);
    if light.sky.z > 0.5 && light.sky.z < 1.5 { rad = vec3<f32>(sky_vis) * 6000.0; }
    else if light.sky.z > 1.5 && light.sky.z < 2.5 { rad = base * 6000.0; }
    else if light.sky.z > 2.5 && light.sky.z < 3.5 { rad = direct; }
    else if light.sky.z > 3.5 && light.sky.z < 4.5 { rad = indirect; }
    else if light.sky.z > 4.5 && light.sky.z < 5.5 { rad = specular; }
    else if light.sky.z > 5.5 && light.sky.z < 6.5 { rad = vec3<f32>(contact.x) * 6000.0; }
    else if light.sky.z > 6.5 && light.sky.z < 7.5 { rad = vec3<f32>(reflection_vis) * 6000.0; }
    // Premultiplied source-over, in linear HDR before TAA and tonemapping.
    return vec4<f32>(rad * alpha, alpha);
}
@fragment fn transparent_fs(in: BlendVertex, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    return transparent_color(in,front);
}
struct GlassMotionOut { @location(0) color:vec4<f32>, @location(1) metadata:vec2<u32> };
@fragment fn transparent_motion_fs(in: BlendVertex, @builtin(front_facing) front: bool)->GlassMotionOut {
    var out:GlassMotionOut;out.color=transparent_color(in,front);
    let owner=blend_triangles[in.triangle].owner;
    let coverage=u32(round(clamp(out.color.a,0.0,1.0)*255.0));
    out.metadata=vec2<u32>(((owner+1u)<<8u)|coverage,bitcast<u32>(in.position.z));
    return out;
}
