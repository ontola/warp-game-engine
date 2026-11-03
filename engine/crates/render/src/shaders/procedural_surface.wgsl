// Procedural surfaces, after Growing Guns' rock_surface.gdshader and
// lava_flow.gdshader: no textures, only noise in world space.
//
// ROCK (material flag 2048): each face of a unit-cube block gets pits,
// hairline cracks (more and darker with damage, `Material::scale` 0..1),
// colour variation and moss on top, sampled along the block's own axes so a
// rotated or stretched block keeps even, two-dimensional grain.
// LAVA (flag 4096): slow flowing molten rock in world XZ, unlit.
//
// The Godot shaders sample a 256 px FastNoiseLite FBM texture; here the
// same frequencies are evaluated directly (gradient noise, five octaves).

fn ps_ihash(x: i32, y: i32) -> u32 {
    var h = (u32(x) * 0x8da6b343u) ^ (u32(y) * 0xd8163841u);
    h ^= h >> 15u; h *= 0x2c1b3c6du;
    h ^= h >> 12u; h *= 0x297a2d39u;
    h ^= h >> 15u;
    return h;
}

fn ps_grad(x: i32, y: i32, d: vec2<f32>) -> f32 {
    let a = f32(ps_ihash(x, y) & 0xffffu) * (6.2831853 / 65536.0);
    return dot(vec2<f32>(cos(a), sin(a)), d);
}

// Gradient noise, about -1..1.
fn ps_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let ix = i32(i.x);
    let iy = i32(i.y);
    let a = ps_grad(ix, iy, f);
    let b = ps_grad(ix + 1, iy, f - vec2<f32>(1.0, 0.0));
    let c = ps_grad(ix, iy + 1, f - vec2<f32>(0.0, 1.0));
    let d = ps_grad(ix + 1, iy + 1, f - vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y) * 1.414;
}

// FastNoiseLite FBM (gain 0.5, lacunarity 2, normalised), mapped to 0..1
// like the Godot noise textures (`v * 0.5 + 0.5`).
// `fw`: the pixel footprint in noise cells; octaves finer than about two
// pixels fade to their mean instead of aliasing.
fn ps_fbm(p: vec2<f32>, octaves: i32, fw: f32) -> f32 {
    var v = 0.0;
    var amp = 1.0;
    var total = 0.0;
    var q = p;
    var f = fw;
    for (var i = 0; i < octaves; i++) {
        let keep = 1.0 - smoothstep(0.25, 0.5, f);
        v += amp * ps_noise(q) * keep;
        total += amp;
        amp *= 0.5;
        f *= 2.0;
        q = q * 2.0 + vec2<f32>(17.3, 9.1);
    }
    // x0.57: the same spread as the Godot textures (std 0.12 about 0.5).
    return clamp(v / total * 0.57 + 0.5, 0.0, 1.0);
}

fn ps_to_srgb(c: vec3<f32>) -> vec3<f32> { return pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2)); }
fn ps_to_linear(c: vec3<f32>) -> vec3<f32> { return pow(max(c, vec3<f32>(0.0)), vec3<f32>(2.2)); }
// The exact sRGB curve (Godot's conversion of `source_color` uniforms).
fn ps_srgb_decode(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

struct ProcRock { base: vec3<f32>, rough: f32, normal: vec3<f32> };

// One shared, prefiltered quarry surface. Reuse the instance's axes (metres,
// not stretched unit-cube UVs) and a stable slot seed. Translating/rotating a
// rubble shard therefore moves its texture with it, independent of the view.
// Albedo alpha stays opaque; roughness is stored in the LINEAR normal alpha,
// so foliage alpha-coverage mip correction cannot corrupt the roughness.
// Height is packed in normal B (XY still reconstruct the shading normal).
// A bounded ray march with linear intersection refinement, using the same
// metric projection and mip derivatives as the color and normal samples.
fn rock_parallax(uv: vec2<f32>, gx: vec2<f32>, gy: vec2<f32>, layer: i32,
    p: vec3<f32>, n: vec3<f32>, gu: vec3<f32>, gv: vec3<f32>) -> vec2<f32> {
    let eye = safe_normalize(view.cam.xyz - p);
    let facing = dot(eye, n);
    let footprint = max(length(gx), length(gy)) * 1.5;
    let fade = (1.0 - smoothstep(0.008, 0.025, footprint))
        * (1.0 - smoothstep(12.0, 24.0, distance(view.cam.xyz, p)))
        * smoothstep(0.06, 0.18, facing);
    if fade <= 0.0 { return uv; }
    let tangent_eye = eye - n * facing;
    let travel = vec2<f32>(dot(tangent_eye, gu), dot(tangent_eye, gv))
        * (0.032 * fade / (1.5 * max(facing, 0.06)));
    if length(travel) < max(length(gx), length(gy)) * 0.35 { return uv; }
    let steps = mix(20.0, 8.0, clamp(facing, 0.0, 1.0));
    let step_depth = 1.0 / steps;
    var depth = 0.0;
    var height = textureSampleGrad(normal_tex, samp, uv, layer, gx, gy).b;
    var previous_gap = 1.0 - height;
    if previous_gap <= 0.001 { return uv; }
    for (var i = 0u; i < 20u; i++) {
        depth += step_depth;
        height = textureSampleGrad(normal_tex, samp, uv - travel * depth, layer, gx, gy).b;
        let gap = 1.0 - height - depth;
        if gap <= 0.0 {
            let fraction = previous_gap / max(previous_gap - gap, 1e-5);
            return uv - travel * (depth - step_depth + step_depth * fraction);
        }
        previous_gap = gap;
    }
    return uv - travel;
}

fn textured_rock(inst: Instance, instance_id: u32, p: vec3<f32>, n: vec3<f32>,
    tint: vec3<f32>, damage: f32, roughness: f32, layer: i32,
    dx: vec3<f32>, dy: vec3<f32>) -> ProcRock {
    let ax = vec3<f32>(inst.r0.x, inst.r1.x, inst.r2.x);
    let ay = vec3<f32>(inst.r0.y, inst.r1.y, inst.r2.y);
    let az = vec3<f32>(inst.r0.z, inst.r1.z, inst.r2.z);
    let d = p - vec3<f32>(inst.r0.w, inst.r1.w, inst.r2.w);
    let ux = safe_normalize(ax); let uy = safe_normalize(ay); let uz = safe_normalize(az);
    let lp = vec3<f32>(dot(d, ax) / max(dot(ax, ax), 1e-8),
        dot(d, ay) / max(dot(ay, ay), 1e-8), dot(d, az) / max(dot(az, az), 1e-8));
    let ap = abs(lp);
    var u = ux; var v = uy;
    if ap.x >= ap.y && ap.x >= ap.z { u = uy; v = uz; }
    else if ap.y >= ap.x && ap.y >= ap.z { u = ux; v = uz; }
    let h = ps_ihash(i32(instance_id), 9137);
    var gu = u; var gv = v;
    switch h & 3u {
        case 1u: { gu = -v; gv = u; }
        case 2u: { gu = -u; gv = -v; }
        case 3u: { gu = v; gv = -u; }
        default: {}
    }
    // Keep in sync with stone_surface::REPEAT_METRES, shared by every block.
    let repeat_m = 1.5;
    let offset = vec2<f32>(f32((h >> 2u) & 1023u), f32((h >> 12u) & 1023u)) / 1024.0;
    var uv = vec2<f32>(dot(d, gu), dot(d, gv)) / repeat_m + offset;
    let gx = vec2<f32>(dot(dx, gu), dot(dx, gv)) / repeat_m;
    let gy = vec2<f32>(dot(dy, gu), dot(dy, gv)) / repeat_m;
    uv = rock_parallax(uv, gx, gy, layer, p, n, gu, gv);
    let sampled = textureSampleGrad(albedo, samp, uv, layer, gx, gy);
    let footprint_m = max(length(gx), length(gy)) * repeat_m;
    // Fade unresolved millimetre relief continuously; beyond this band only
    // one albedo mip sample remains. No distance switch or per-stone CPU LOD.
    let relief = 1.0 - smoothstep(0.025, 0.10, footprint_m);
    var normal = n;
    var grain_roughness = 0.93;
    if relief > 0.0 {
        let packed = textureSampleGrad(normal_tex, samp, uv, layer, gx, gy);
        let dressing = mix(0.65, 1.0, f32((h >> 22u) & 255u) / 255.0);
        let xy = (packed.xy * 2.0 - 1.0) * (relief * dressing);
        // Preserve the metric UV Jacobian on sloped fracture/bevel faces;
        // normalizing the projected axes would amplify the physical relief.
        normal = rock_mapped_normal(n, xy, gu, gv);
        grain_roughness = mix(grain_roughness, packed.a, relief);
    }
    let dmg = clamp(damage, 0.0, 1.0);
    var result: ProcRock;
    result.base = tint * sampled.rgb * (1.0 - dmg * 0.24);
    result.rough = clamp(roughness * grain_roughness + dmg * 0.06, 0.55, 1.0);
    result.normal = normal;
    return result;
}

// Rock noise: uv = world * 0.085 into a 256 px texture of frequency 0.032.
const PS_ROCK_FREQ: f32 = 0.6963;

fn proc_rock(inst: Instance, p: vec3<f32>, n: vec3<f32>, tint: vec3<f32>, damage: f32, footprint: f32) -> ProcRock {
    let ax = vec3<f32>(inst.r0.x, inst.r1.x, inst.r2.x);
    let ay = vec3<f32>(inst.r0.y, inst.r1.y, inst.r2.y);
    let az = vec3<f32>(inst.r0.z, inst.r1.z, inst.r2.z);
    let t = vec3<f32>(inst.r0.w, inst.r1.w, inst.r2.w);
    let d = p - t;
    // Block-local position in the unit cube (orthogonal axes: rotation and scale).
    let lp = vec3<f32>(dot(d, ax) / max(dot(ax, ax), 1e-8), dot(d, ay) / max(dot(ay, ay), 1e-8), dot(d, az) / max(dot(az, az), 1e-8));
    let ap = abs(lp);
    let ux = normalize(ax);
    let uy = normalize(ay);
    let uz = normalize(az);
    // Each block its own pattern (Godot INSTANCE_CUSTOM seed).
    let seed = f32(ps_ihash(i32(floor(t.x * 8.0)) + i32(floor(t.y * 8.0)) * 7919, i32(floor(t.z * 8.0))) & 1023u);
    let sp = p + vec3<f32>(seed * 0.31, seed * 0.53, seed * 0.71);
    var uv: vec2<f32>;
    var top = 0.0;
    if ap.x >= ap.y && ap.x >= ap.z {
        uv = vec2<f32>(dot(sp, uy), dot(sp, uz));
    } else if ap.y >= ap.x && ap.y >= ap.z {
        uv = vec2<f32>(dot(sp, ux), dot(sp, uz)) + vec2<f32>(3.7, 1.9);
        top = step(0.0, lp.y);
    } else {
        uv = vec2<f32>(dot(sp, ux), dot(sp, uy)) + vec2<f32>(7.1, 4.3);
    }
    let fw = footprint * PS_ROCK_FREQ;
    let rock = ps_fbm(uv * PS_ROCK_FREQ, 5, fw);
    let detail = ps_fbm(uv * PS_ROCK_FREQ * 2.0 + vec2<f32>(31.0, 11.0), 4, fw * 2.0);
    let combined = rock * 0.58 + detail * 0.42;
    let cornerness = min(ap.x, min(ap.y, ap.z)) / max(max(ap.x, ap.y), max(ap.z, 1e-4));
    let bevel = smoothstep(0.58, 0.86, cornerness);

    var pit = 1.0 - smoothstep(0.14, 0.38, combined);
    let pit2 = 1.0 - smoothstep(0.22, 0.48, detail);
    pit = max(pit, pit2 * 0.72);
    let dmg = clamp(damage, 0.0, 1.0);
    let crack_raw = max(1.0 - abs(rock * 2.0 - 1.0), 1.0 - abs(detail * 2.0 - 1.0));
    let base_cracks = smoothstep(0.80, 0.97, crack_raw);
    let dmg_cracks = smoothstep(mix(0.82, 0.62, dmg), 0.96, crack_raw) * dmg;
    let crack_line = clamp(max(base_cracks, dmg_cracks), 0.0, 1.0);

    // The Godot colours are authored in sRGB: work there, return linear.
    let base_color = ps_to_srgb(tint);
    var face = base_color * (0.76 + combined * 0.28);
    face *= 1.0 - dmg * 0.28;
    face = mix(face * 0.62, face, 1.0 - pit * 0.48);
    face = mix(face, face * 0.12, crack_line * clamp(0.52 + dmg * 0.4, 0.0, 0.95));
    let edge_dist = min(min(0.5 - ap.x, 0.5 - ap.y), 0.5 - ap.z);
    var moss = top * smoothstep(0.10, 0.18, edge_dist);
    moss *= 1.0 - smoothstep(0.30, 0.56, combined);
    moss *= 1.0 - crack_line * 0.85;
    face = mix(face, vec3<f32>(0.36, 0.56, 0.30), moss * 0.38);
    let col = mix(face, base_color * 0.92, bevel);
    var r: ProcRock;
    r.base = ps_to_linear(col);
    r.rough = clamp(0.96 + (1.0 - bevel) * (pit * 0.18 + crack_line * 0.12 + (1.0 - combined) * 0.08), 0.0, 1.0);
    r.normal = n;
    return r;
}

// Lava noise. Godot's sample_n reads its 256 px FastNoiseLite texture
// (frequency 0.07) at uv * 0.01, repeating, with trilinear mipmaps: the
// scene's noise field when it has one (`lod` is the mip level), else a
// gradient noise of the same statistics (no mips).
fn ps_lava_n(q: vec2<f32>, lod: f32, field: bool) -> f32 {
    if field { return textureSampleLevel(noise_tex, samp, q * 0.01, lod).r; }
    return ps_fbm(q * 0.1792, 3, 0.0);
}

fn ps_lava_fbm(p: vec2<f32>, lod: f32, field: bool) -> f32 {
    var v = 0.0;
    var amp = 0.52;
    var q = p;
    var l = lod;
    for (var i = 0; i < 4; i++) {
        v += amp * ps_lava_n(q, l, field);
        q *= 2.04;
        amp *= 0.5;
        l += 1.0285; // log2(2.04)
    }
    return v;
}

// Linear radiance relative to white. Godot's material is `unshaded`, which
// shows ALBEDO alone (EMISSION is ignored in that mode); its `source_color`
// uniforms are converted to linear before the mixes, so mix in linear too.
// `footprint`: metres one pixel covers, for the mip level.
fn proc_lava(p: vec3<f32>, time: f32, footprint: f32) -> vec3<f32> {
    let w = p.xz * 0.048;
    let t = time * 0.035;
    let drift = vec2<f32>(t * 0.42, t * 0.42 * 0.58);
    let field = textureDimensions(noise_tex).x > 1u;
    // Texels per pixel at the first octave: 256 * 0.01 * 0.048 per metre.
    let lod = log2(max(footprint * f32(textureDimensions(noise_tex).x) * 0.00048, 1e-6));
    let base = ps_lava_fbm(w + drift, lod, field);
    let detail = ps_lava_fbm(w * 1.85 - drift * 1.25 + vec2<f32>(4.2, 1.1), lod + 0.8875, field); // log2(1.85)
    let veins = pow(1.0 - abs(base * 2.0 - 1.0), 3.2);
    let crust = pow(1.0 - abs(detail * 2.0 - 1.0), 2.4);
    var heat = clamp(base * 0.58 + detail * 0.42, 0.0, 1.0);
    heat = smoothstep(0.18, 0.82, heat + veins * 0.38 - crust * 0.22);
    let cool = ps_srgb_decode(vec3<f32>(0.14, 0.05, 0.01));
    let hot = ps_srgb_decode(vec3<f32>(0.92, 0.30, 0.05));
    let bright = ps_srgb_decode(vec3<f32>(1.0, 0.68, 0.22));
    let col = mix(mix(cool, hot, heat), bright, pow(heat, 2.6) * veins);
    return col;
}
