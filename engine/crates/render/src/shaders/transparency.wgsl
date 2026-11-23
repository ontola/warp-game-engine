// Appended to shade.wgsl so glass uses the same sun, environment, terrain and
// canopy visibility as opaque surfaces. No noisy coverage. Refraction comes
// from transparency_refraction.wgsl (group 3) or its stub, appended after this.
struct BlendTriangle {
    vertices: vec3<u32>,
    owner: u32,
    material: u32,
    alpha: f32,
    // Bit 0 double sided, 1 additive, 2 refracts.
    flags: u32,
    // Refraction: low 16 bits (ior - 1) / 2, high 16 bits thickness in 0.1 mm
    // (65535: a solid that reaches the background).
    optics: u32,
};
const BLEND_DOUBLE_SIDED: u32 = 1u;
const BLEND_ADDITIVE: u32 = 2u;
const BLEND_REFRACTS: u32 = 4u;
@group(2) @binding(0) var<storage, read> blend_triangles: array<BlendTriangle>;
@group(2) @binding(1) var<storage, read> blend_order: array<u32>;

struct BlendVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) triangle: u32,
    // Draw phase from the order entry's top two bits (transparency.rs PHASE_*).
    @location(4) @interpolate(flat) phase: u32,
};

@vertex
fn transparent_vs(@builtin(vertex_index) corner: u32, @builtin(instance_index) index: u32) -> BlendVertex {
    let entry = blend_order[index];
    let id = entry & 0x3fffffffu;
    let tri = blend_triangles[id];
    let inst = instances[tri.owner];
    let v = vtx(tri.vertices[corner]);
    let local_normal = vec3<f32>(vertex_f32(v + 3u), vertex_f32(v + 4u), vertex_f32(v + 5u));
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
    out.uv = vec2<f32>(vertex_f32(v + 6u), vertex_f32(v + 7u));
    out.triangle = id;
    out.phase = entry >> 30u;
    return out;
}

fn transparent_color(in: BlendVertex, front: bool) -> vec4<f32> {
    // Evaluate derivatives before any per-material discard or dynamic branch.
    let mip_scale = select(1.0, view.extra.z, view.extra.z > 0.0);
    let dx = dpdx(in.uv) * mip_scale;
    let dy = dpdy(in.uv) * mip_scale;
    let tri = blend_triangles[in.triangle];
    let m = material(tri.material);
    if !front && (tri.flags & BLEND_DOUBLE_SIDED) == 0u { discard; }
    // With refracting glass in view the list is drawn in phases around the
    // nearest refracting surface; only that surface bends what is behind it.
    let refract_here = REFRACTION_ENABLED && in.phase == 1u;
    if REFRACTION_ENABLED && in.phase != 3u && !refraction_phase_keeps(in.phase, in.position.xy, in.position.z) { discard; }
    var sample = vec4<f32>(1.0);
    if m.layer >= 0 { let k = albedo_grad(m.layer, dx, dy); sample = textureSampleGrad(albedo, samp, in.uv, m.layer, dx * k, dy * k); }
    let alpha = clamp(sample.a * tri.alpha, 0.0, 1.0);
    let optical = (tri.flags & BLEND_REFRACTS) != 0u;
    if alpha <= 0.0 && (!optical || sample.a <= 0.0) { discard; }
    if (m.flags & 1024u) != 0u {
        // Emissive surfaces are unlit, blended (signs, labels) or added. Fog may absorb their light, but adding
        // its scattering colour here would make the fog itself glow.
        let ray_length = distance(in.world, view.cam.xyz);
        var transmission = 1.0 - distance_fog_amount(ray_length);
        if light.fog.z > 0.0 {
            let direction = normalize(in.world - view.cam.xyz);
            let column = fog_column(ray_length, view.cam.y, direction.y, light.fog.z);
            transmission = exp(-max(light.fog.y, 0.0) * column);
        }
        // The froxel fog's medium in front absorbs it too (its in-scattering
        // is already in the opaque image behind).
        transmission *= froxel_fog_at(in.world).a;
        // With a texture layer the textured colour glows (signs, logos).
        let glow = select(m.tint, sample.rgb * m.tint, m.layer >= 0);
        return vec4<f32>(glow * abs(m.scale) * light.anim.y * transmission * alpha, alpha);
    }
    let base = sample.rgb * m.tint;
    let p = in.world;
    let v = normalize(view.cam.xyz - p);
    var n = safe_normalize(in.normal);
    if (tri.flags & BLEND_DOUBLE_SIDED) != 0u && dot(n, v) < 0.0 { n = -n; }
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
    var ambient = (sky_irradiance(n) * sky_vis + bounce * (1.0 - sky_vis)) * contact.x;
    var reflection_vis = sky_vis;
    if (m.flags & 32u) != 0u || rough < 0.55 { reflection_vis = reflected_visibility(p, visibility_n, refl); }
    let spec_contact = mix(contact.y, contact.x, smoothstep(0.08, 0.6, rough));
    var env = mix(bounce / PI, environment(refl, rough, bounce), reflection_vis) * spec_contact;
    if optical {
        // A window is set into a wall. Sample the medium on the viewing side
        // of its reveal, keeping the outdoor reflection on its street face.
        let side = p + visibility_n * .20;
        let root = u32(gi.params.w);
        let indoor = irradiance_sample(side, n);
        if indoor.value.w > 0.0 && aux_word(root + 7u) >= 1u {
            let sky_in = sky_irradiance(n) * sky_vis * clamp(indoor.value.x, 0.0, 1.0);
            let returned = sky_irradiance(vec3<f32>(0.0,1.0,0.0)) * indoor.value.y
                + light.sun_color.rgb * light.sun_dir.w * max(indoor.value.z, 0.0) + indoor.emission * light.anim.y;
            ambient = mix(ambient, (sky_in + returned) * contact.x, indoor.value.w);
        }
        if root == 0u || aux_word(root + 7u) != 8u || indoor.value.w > 0.0 {
            let room = glass_room_reflection(side, refl, rough);
            env = mix(env, room.rgb * spec_contact, room.a);
        }
    }
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
    var rad = direct + indirect + specular + car_lamp_emission(m.flags, base, instances[tri.owner], p) * light.anim.y;
    if (m.flags & 1024u) != 0u { rad += m.tint * abs(m.scale) * light.anim.y; }
    let diagnostic_view = light.sky.z > 0.5 && light.sky.z < 7.5;
    // Optical layers behind the first pane (and adapters without refraction)
    // still reflect at grazing angles and transmit the opaque scene normally.
    if optical && !refract_here && !diagnostic_view {
        let fresnel = env_brdf(f0, rough, ndv);
        let reflected = clamp(dot(fresnel, vec3<f32>(1.0/3.0)), 0.0, 1.0);
        let cover = alpha + (1.0 - alpha) * reflected;
        let spec_only = sun_e * ggx(n, v, l, rough, f0) + specular;
        let surface = rad * alpha + (1.0 - alpha) * spec_only;
        let froxel = froxel_fog_at(p);
        let fogged = apply_distance_fog(surface / max(cover, 1e-4), distance(p, view.cam.xyz), -v) * froxel.a + froxel.rgb;
        return vec4<f32>(fogged * cover, cover);
    }
    if refract_here && !diagnostic_view {
        // The surface layer keeps its diffuse/scattering share (alpha) and
        // all of its reflection; the rest of the light comes through, bent.
        let spec_only = sun_e * ggx(n, v, l, rough, f0) + specular;
        let fresnel = env_brdf(f0, rough, ndv);
        let transmission = (1.0 - alpha) * clamp(1.0 - (fresnel.r + fresnel.g + fresnel.b) / 3.0, 0.0, 1.0);
        let cover = 1.0 - transmission;
        let surface = rad * alpha + (1.0 - alpha) * spec_only;
        // Fogged at the pane's own depth: the distance fog, then the froxel
        // medium between the camera and the pane.
        let froxel = froxel_fog_at(p);
        let fogged = (apply_distance_fog(surface / max(cover, 1e-4), distance(p, view.cam.xyz), -v) * froxel.a + froxel.rgb) * cover;
        let ior = 1.0 + f32(tri.optics & 65535u) / 65535.0 * 2.0;
        let thickness_q = tri.optics >> 16u;
        let thickness = select(f32(thickness_q) * 1e-4, -1.0, thickness_q == 65535u);
        // A solid's front already bends the light to the background: its
        // inside faces would bend the same light a second time.
        if thickness_q == 65535u && !front { discard; }
        // The copy holds the opaque scene and every glass layer behind this one.
        let behind = refraction_background(in.position.xy, in.position.z, p, v, n, ior, thickness, rough);
        // Hue of the glass, softened: alpha already darkens it.
        let hue = sqrt(base / max(max(base.r, max(base.g, base.b)), 1e-3));
        // Output alpha 1 replaces the destination: the copy already holds it.
        // Motion coverage below still uses the surface's own cover.
        return vec4<f32>(fogged + transmission * hue * behind.bent, cover);
    }
    rad = apply_distance_fog(rad, distance(p, view.cam.xyz), -v);
    // The froxel medium in front of the pane (froxel_fog.wgsl), as shade
    // fogs opaque surfaces: blended over the image behind, which holds the
    // fog to its own depth, a pane gets it to the pane's.
    let froxel = froxel_fog_at(p);
    rad = rad * froxel.a + froxel.rgb;
    if light.sky.z > 0.5 && light.sky.z < 1.5 { rad = vec3<f32>(sky_vis) * 6000.0; }
    else if light.sky.z > 1.5 && light.sky.z < 2.5 { rad = base * 6000.0; }
    else if light.sky.z > 2.5 && light.sky.z < 3.5 { rad = direct; }
    else if light.sky.z > 3.5 && light.sky.z < 4.5 { rad = indirect; }
    else if light.sky.z > 4.5 && light.sky.z < 5.5 { rad = specular; }
    else if light.sky.z > 5.5 && light.sky.z < 6.5 { rad = vec3<f32>(contact.x) * 6000.0; }
    else if light.sky.z > 6.5 && light.sky.z < 7.5 { rad = vec3<f32>(reflection_vis) * 6000.0; }
    // Keep true alpha here for motion coverage, independently of blending.
    return vec4<f32>(rad * alpha, alpha);
}
fn transparent_blend(color: vec4<f32>, additive: u32) -> vec4<f32> {
    // Premultiplied source-over uses 1 - source alpha for the destination.
    // Zero output alpha makes the same pipeline retain all destination light.
    return vec4<f32>(color.rgb, select(color.a, 0.0, additive != 0u));
}
fn transparent_output(color: vec4<f32>, flags: u32, phase: u32) -> vec4<f32> {
    if REFRACTION_ENABLED && phase == 1u { return vec4<f32>(color.rgb, 1.0); }
    return transparent_blend(color, flags & BLEND_ADDITIVE);
}
@fragment fn transparent_fs(in: BlendVertex, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let color = transparent_color(in,front);
    return transparent_output(color, blend_triangles[in.triangle].flags, in.phase);
}
struct GlassMotionOut { @location(0) color:vec4<f32>, @location(1) metadata:vec2<u32> };
@fragment fn transparent_motion_fs(in: BlendVertex, @builtin(front_facing) front: bool)->GlassMotionOut {
    let color=transparent_color(in,front);
    let tri=blend_triangles[in.triangle];
    var out:GlassMotionOut;out.color=transparent_output(color,tri.flags,in.phase);
    let owner=tri.owner;
    let coverage=u32(round(clamp(color.a,0.0,1.0)*255.0));
    out.metadata=vec2<u32>(((owner+1u)<<8u)|coverage,bitcast<u32>(in.position.z));
    return out;
}
