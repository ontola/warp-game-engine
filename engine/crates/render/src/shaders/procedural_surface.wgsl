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

struct ProcRock { base: vec3<f32>, rough: f32 };

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
    return r;
}

// Lava noise: sample(uv) reads the 256 px texture (frequency 0.07) at uv * 0.01.
fn ps_lava_n(q: vec2<f32>) -> f32 { return ps_fbm(q * 0.1792, 3, 0.0); }

fn ps_lava_fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var amp = 0.52;
    var q = p;
    for (var i = 0; i < 4; i++) {
        v += amp * ps_lava_n(q);
        q *= 2.04;
        amp *= 0.5;
    }
    return v;
}

// Linear radiance relative to white (Godot: ALBEDO + EMISSION * 1.35, unlit).
fn proc_lava(p: vec3<f32>, time: f32) -> vec3<f32> {
    let w = p.xz * 0.048;
    let t = time * 0.035;
    let drift = vec2<f32>(t * 0.42, t * 0.42 * 0.58);
    let base = ps_lava_fbm(w + drift);
    let detail = ps_lava_fbm(w * 1.85 - drift * 1.25 + vec2<f32>(4.2, 1.1));
    let veins = pow(1.0 - abs(base * 2.0 - 1.0), 3.2);
    let crust = pow(1.0 - abs(detail * 2.0 - 1.0), 2.4);
    var heat = clamp(base * 0.58 + detail * 0.42, 0.0, 1.0);
    heat = smoothstep(0.18, 0.82, heat + veins * 0.38 - crust * 0.22);
    var col = mix(vec3<f32>(0.14, 0.05, 0.01), vec3<f32>(0.92, 0.30, 0.05), heat);
    col = mix(col, vec3<f32>(1.0, 0.68, 0.22), pow(heat, 2.6) * veins);
    return ps_to_linear(col) * 2.35;
}
