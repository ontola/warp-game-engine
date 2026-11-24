// Spellstorm's own surfaces (render::set_game_surface), ported from the
// Godot game's materials line by line:
//   0 PAINT  a plain colour times the vertex colours (Shape.flat, Fx.solid
//            on code-built parts, the robe's skirt)
//   1 NOISE  Shape.material: a noise picture through a colour ramp, laid on
//            from three sides in the model's space, bumped by a second
//            noise, times the vertex colours (darker towards the floor)
//   2 STONE  stone.gdshader   3 CLOTH  cloth.gdshader   4 FLOOR floor.gdshader
// Material::layer holds -2 - (kind | aux << 3): `aux` is where the
// material's numbers start in Scene::aux. Vertex colours ride in the
// corner uvs: x = r + 256 g, y = b + 256 a, 8 bits each.

// Phones (Joep's Tab S9+: 53 of 68 ms shading here at full cost): the
// game installs this file with SS_LITE true (live.rs surface): two noise
// octaves instead of four, no bump from extra noise samples (the normal
// stays the mesh's), one triplanar tap. Same colours, ramps and darkness.
const SS_LITE: bool = false;
const SS_OCT: i32 = select(4, 2, SS_LITE);

struct SurfaceIn {
    kind: u32,
    instance: u32,
    p: vec3<f32>,
    local: vec3<f32>,
    n: vec3<f32>,
    dir: vec3<f32>,
    bary: vec3<f32>,
    uv0: vec2<f32>,
    uv1: vec2<f32>,
    uv2: vec2<f32>,
    dx: vec3<f32>,
    dy: vec3<f32>,
    tint: vec3<f32>,
    rough: f32,
    metal: f32,
    scale: f32,
    time: f32,
};

struct SurfaceOut {
    albedo: vec3<f32>,
    normal: vec3<f32>,
    rough: f32,
    metal: f32,
    emission: vec3<f32>,
};

fn ss_f(i: u32) -> f32 { return bitcast<f32>(aux_word(i)); }
fn ss_v3(i: u32) -> vec3<f32> { return vec3<f32>(ss_f(i), ss_f(i + 1u), ss_f(i + 2u)); }

fn ss_unpack(uv: vec2<f32>) -> vec4<f32> {
    let x = u32(uv.x + 0.5);
    let y = u32(uv.y + 0.5);
    return vec4<f32>(f32(x & 255u), f32(x >> 8u), f32(y & 255u), f32(y >> 8u)) / 255.0;
}

fn ss_vcol(s: SurfaceIn) -> vec4<f32> {
    return ss_unpack(s.uv0) * s.bary.x + ss_unpack(s.uv1) * s.bary.y + ss_unpack(s.uv2) * s.bary.z;
}

fn ss_srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

fn ss_linear_to_srgb(c: f32) -> f32 {
    return select(1.055 * pow(c, 1.0 / 2.4) - 0.055, c * 12.92, c <= 0.0031308);
}

// The instance's object-to-world transform, and back (for directions).
fn ss_to_world(inst: u32, v: vec3<f32>) -> vec3<f32> {
    let i = instances[inst];
    return vec3<f32>(dot(i.r0.xyz, v), dot(i.r1.xyz, v), dot(i.r2.xyz, v));
}

fn ss_world_normal_to_local(inst: u32, n: vec3<f32>) -> vec3<f32> {
    let i = instances[inst];
    return normalize(i.r0.xyz * n.x + i.r1.xyz * n.y + i.r2.xyz * n.z);
}

fn ss_world_dir_to_local(inst: u32, v: vec3<f32>) -> vec3<f32> {
    let i = instances[inst];
    let m = mat3x3<f32>(vec3<f32>(i.r0.x, i.r1.x, i.r2.x), vec3<f32>(i.r0.y, i.r1.y, i.r2.y), vec3<f32>(i.r0.z, i.r1.z, i.r2.z));
    let c0 = m[0]; let c1 = m[1]; let c2 = m[2];
    let det = dot(c0, cross(c1, c2));
    let inv = transpose(mat3x3<f32>(cross(c1, c2), cross(c2, c0), cross(c0, c1))) * (1.0 / det);
    return inv * v;
}

// Godot's bump by screen-space derivatives: the height's change across the
// pixel (dhx, dhy) along the surface's footprint (dx, dy).
fn ss_bump(n: vec3<f32>, dx: vec3<f32>, dy: vec3<f32>, dhx: f32, dhy: f32) -> vec3<f32> {
    let r1 = cross(dy, n);
    let r2 = cross(n, dx);
    let det = dot(dx, r1);
    let grad = sign(det) * (dhx * r1 + dhy * r2);
    let out = abs(det) * n - grad;
    return select(n, normalize(out), dot(out, out) > 1e-20);
}

// ---- Noise pictures (Shape.material) ---------------------------------------
// Each 256 px picture is one cell of an 8 x 8 atlas in a 2048 px albedo layer:
// tile = layer * 64 + cell. Pictures hold grey noise (sRGB-encoded bytes).

fn ss_tile_uv(tile: u32, uv: vec2<f32>) -> vec2<f32> {
    let cell = tile & 63u;
    return (vec2<f32>(f32(cell & 7u), f32(cell >> 3u)) + fract(uv)) / 8.0;
}

fn ss_grey(tile: u32, uv: vec2<f32>, gx: vec2<f32>, gy: vec2<f32>) -> f32 {
    let v = textureSampleGrad(albedo, samp, ss_tile_uv(tile, uv), i32(tile >> 6u), gx / 8.0, gy / 8.0).r;
    return ss_linear_to_srgb(v);
}

// One sample of the Godot normal map made from height tile `tile`
// (Image.bump_map_to_normal_map at `strength`), as -1..1 xyz.
fn ss_bump_normal(tile: u32, uv: vec2<f32>, strength: f32, gx: vec2<f32>, gy: vec2<f32>) -> vec3<f32> {
    let t = 1.0 / 256.0;
    let here = ss_grey(tile, uv, gx, gy);
    let right = ss_grey(tile, uv + vec2<f32>(t, 0.0), gx, gy);
    let below = ss_grey(tile, uv + vec2<f32>(0.0, t), gx, gy);
    let up = vec3<f32>(0.0, 1.0, (here - below) * strength);
    let across = vec3<f32>(1.0, 0.0, (right - here) * strength);
    // Stored as n * 0.5 + 0.5 in the picture, read back the same way;
    // Godot's normal maps put +y down (OpenGL vs. its image rows).
    let nm = normalize(cross(across, up));
    return vec3<f32>(nm.x, -nm.y, nm.z);
}

fn ss_ramp(o: u32, g: f32) -> vec3<f32> {
    let n = aux_word(o);
    var prev_t = ss_f(o + 1u);
    var prev_c = ss_v3(o + 2u);
    if g <= prev_t { return prev_c; }
    for (var i = 1u; i < n; i++) {
        let t = ss_f(o + 1u + i * 4u);
        let c = ss_v3(o + 2u + i * 4u);
        if g <= t { return mix(prev_c, c, (g - prev_t) / max(t - prev_t, 1e-6)); }
        prev_t = t;
        prev_c = c;
    }
    return prev_c;
}

fn ss_noise(s: SurfaceIn, o: u32) -> SurfaceOut {
    // aux: albedo tile, bump tile, bump strength, uv scale, normal scale,
    // then the ramp (count, then offset + sRGB colour per point).
    let a_tile = aux_word(o);
    let b_tile = aux_word(o + 1u);
    let strength = ss_f(o + 2u);
    let scale = ss_f(o + 3u);
    let depth = ss_f(o + 4u);
    let ln = ss_world_normal_to_local(s.instance, s.n);
    var w = pow(abs(ln), vec3<f32>(2.0));
    w /= dot(w, vec3<f32>(1.0));
    let tp = s.local * scale * vec3<f32>(1.0, -1.0, 1.0);
    let dxl = ss_world_dir_to_local(s.instance, s.dx) * scale * vec3<f32>(1.0, -1.0, 1.0);
    let dyl = ss_world_dir_to_local(s.instance, s.dy) * scale * vec3<f32>(1.0, -1.0, 1.0);
    let uz = tp.xy;
    let uy = tp.xz;
    let ux = tp.zy * vec2<f32>(-1.0, 1.0);
    if SS_LITE {
        // The side the surface faces most, one tap, no bump.
        let an = abs(ln);
        var g1 = ss_grey(a_tile, uz, dxl.xy, dyl.xy);
        if an.y >= an.x && an.y >= an.z { g1 = ss_grey(a_tile, uy, dxl.xz, dyl.xz); }
        else if an.x >= an.z { g1 = ss_grey(a_tile, ux, dxl.zy * vec2<f32>(-1.0, 1.0), dyl.zy * vec2<f32>(-1.0, 1.0)); }
        return SurfaceOut(s.tint * ss_srgb_to_linear(ss_ramp(o + 5u, g1)) * ss_vcol(s).rgb, s.n, s.rough, s.metal, vec3<f32>(0.0));
    }
    let g = ss_grey(a_tile, uz, dxl.xy, dyl.xy) * w.z + ss_grey(a_tile, uy, dxl.xz, dyl.xz) * w.y
        + ss_grey(a_tile, ux, dxl.zy * vec2<f32>(-1.0, 1.0), dyl.zy * vec2<f32>(-1.0, 1.0)) * w.x;
    // The ramp is applied per texel in Godot (sRGB), then filtered.
    let tex = ss_srgb_to_linear(ss_ramp(o + 5u, g));
    let vc = ss_vcol(s).rgb;
    var out = SurfaceOut(s.tint * tex * vc, s.n, s.rough, s.metal, vec3<f32>(0.0));
    if b_tile != 0xffffffffu {
        let nm = ss_bump_normal(b_tile, uz, strength, dxl.xy, dyl.xy) * w.z + ss_bump_normal(b_tile, uy, strength, dxl.xz, dyl.xz) * w.y
            + ss_bump_normal(b_tile, ux, strength, dxl.zy * vec2<f32>(-1.0, 1.0), dyl.zy * vec2<f32>(-1.0, 1.0)) * w.x;
        // Godot's triplanar tangent frame, in the model's space.
        let an = abs(ln);
        let t = vec3<f32>(0.0, 0.0, -1.0) * an.x + vec3<f32>(1.0, 0.0, 0.0) * an.y + vec3<f32>(1.0, 0.0, 0.0) * an.z;
        let b = vec3<f32>(0.0, 1.0, 0.0) * an.x + vec3<f32>(0.0, 0.0, -1.0) * an.y + vec3<f32>(0.0, 1.0, 0.0) * an.z;
        let tw = normalize(ss_to_world(s.instance, t));
        let bw = normalize(ss_to_world(s.instance, b));
        let mapped = normalize(tw * nm.x + bw * nm.y + s.n * nm.z);
        out.normal = normalize(mix(s.n, mapped, depth));
    }
    return out;
}

// ---- stone.gdshader -----------------------------------------------------

fn ss_hash3(q: vec3<f32>) -> f32 {
    var p = fract(q * 0.3183099 + 0.1);
    p *= 17.0;
    return fract(p.x * p.y * p.z * (p.x + p.y + p.z));
}

fn ss_vnoise3(x: vec3<f32>) -> f32 {
    let i = floor(x);
    var f = fract(x);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(mix(ss_hash3(i), ss_hash3(i + vec3<f32>(1.0, 0.0, 0.0)), f.x), mix(ss_hash3(i + vec3<f32>(0.0, 1.0, 0.0)), ss_hash3(i + vec3<f32>(1.0, 1.0, 0.0)), f.x), f.y),
        mix(mix(ss_hash3(i + vec3<f32>(0.0, 0.0, 1.0)), ss_hash3(i + vec3<f32>(1.0, 0.0, 1.0)), f.x), mix(ss_hash3(i + vec3<f32>(0.0, 1.0, 1.0)), ss_hash3(i + vec3<f32>(1.0, 1.0, 1.0)), f.x), f.y), f.z);
}

fn ss_fbm3(q: vec3<f32>) -> f32 {
    var p = q;
    var a = 0.5;
    var s = 0.0;
    for (var i = 0; i < SS_OCT; i++) {
        s += a * ss_vnoise3(p);
        p = p * 2.03 + vec3<f32>(1.7, 9.2, 3.1);
        a *= 0.5;
    }
    return s;
}

struct StoneAt { col: vec3<f32>, h: f32, grain: f32 };

fn ss_stone_at(wp: vec3<f32>, wn: vec3<f32>, o: u32, cw: f32) -> StoneAt {
    let color = ss_v3(o);
    let scale = ss_f(o + 3u);
    let moss = ss_v3(o + 5u);
    let moss_amount = ss_f(o + 8u);
    let p = wp * scale;
    let big = ss_fbm3(p * 0.35);
    let grain = ss_fbm3(p * 3.0);
    let c = ss_fbm3(p * vec3<f32>(0.9, 1.8, 0.9) + 4.0);
    let crack = (1.0 - smoothstep(0.0, cw, abs(c - 0.5))) * (0.025 / cw);
    let h = big * 0.6 + grain * 0.4 - crack * 0.3;
    var col = color * (0.65 + 0.6 * big) * (0.85 + 0.3 * grain);
    col *= 1.0 - crack * 0.25;
    let up = smoothstep(0.35, 0.85, wn.y);
    let m = up * smoothstep(0.45, 0.65, ss_fbm3(p * 0.8 + 7.0)) * moss_amount;
    col = mix(col, moss * (0.7 + 0.5 * grain), m);
    return StoneAt(col, h, grain);
}

fn ss_stone(s: SurfaceIn, o: u32) -> SurfaceOut {
    // aux: colour (linear), scale, bump, moss (linear), moss amount, rough.
    let scale = ss_f(o + 3u);
    let bump = ss_f(o + 4u);
    let rough = ss_f(o + 9u);
    // The crack's width follows how fast its noise changes across a pixel.
    if SS_LITE {
        let lite = ss_stone_at(s.p, s.n, o, 0.06);
        return SurfaceOut(lite.col, s.n, rough - lite.grain * 0.15, 0.0, vec3<f32>(0.0));
    }
    let p = s.p * scale;
    let cq = p * vec3<f32>(0.9, 1.8, 0.9) + 4.0;
    let c0 = ss_fbm3(cq);
    let fw = abs(ss_fbm3(cq + s.dx * scale * vec3<f32>(0.9, 1.8, 0.9)) - c0) + abs(ss_fbm3(cq + s.dy * scale * vec3<f32>(0.9, 1.8, 0.9)) - c0);
    let cw = 0.025 + fw * 1.5;
    let here = ss_stone_at(s.p, s.n, o, cw);
    let hx = ss_stone_at(s.p + s.dx, s.n, o, cw).h;
    let hy = ss_stone_at(s.p + s.dy, s.n, o, cw).h;
    let n = ss_bump(s.n, s.dx, s.dy, (hx - here.h) * bump, (hy - here.h) * bump);
    return SurfaceOut(here.col, n, rough - here.grain * 0.15, 0.0, vec3<f32>(0.0));
}

// ---- cloth.gdshader -----------------------------------------------------

fn ss_cloth_h(op: vec3<f32>, weave: f32, fade: f32) -> vec2<f32> {
    let dye = ss_vnoise3(op * 6.0) * 0.6 + ss_vnoise3(op * 17.0) * 0.4;
    var w = sin(op.y * weave) * sin((op.x + op.z) * weave);
    w *= fade;
    return vec2<f32>(dye, w);
}

fn ss_cloth(s: SurfaceIn, o: u32) -> SurfaceOut {
    // aux: colour (linear), weave, bump.
    let color = ss_v3(o);
    let weave = ss_f(o + 3u);
    let bump = ss_f(o + 4u);
    let op = s.local;
    let dxl = ss_world_dir_to_local(s.instance, s.dx);
    let dyl = ss_world_dir_to_local(s.instance, s.dy);
    let fade = 1.0 - smoothstep(0.15, 0.5, (abs(dxl.y) + abs(dyl.y)) * weave);
    let dw = ss_cloth_h(op, weave, fade);
    let grime = smoothstep(0.55, 0.8, ss_vnoise3(op * 3.0 + 11.0));
    let tint = pow(ss_vcol(s).rgb, vec3<f32>(2.2));
    var col = color * tint * (0.78 + 0.35 * dw.x) * (0.97 + 0.03 * dw.y);
    col = mix(col, col * 0.55, grime * 0.5);
    let rim = pow(1.0 - clamp(dot(s.n, -s.dir), 0.0, 1.0), 3.0);
    if SS_LITE { return SurfaceOut(col, s.n, 0.92, 0.0, col * rim * 0.15); }
    let h = dw.x * 0.6 + dw.y * 0.15;
    let hx = ss_cloth_h(op + dxl, weave, fade);
    let hy = ss_cloth_h(op + dyl, weave, fade);
    let n = ss_bump(s.n, s.dx, s.dy, (hx.x * 0.6 + hx.y * 0.15 - h) * bump, (hy.x * 0.6 + hy.y * 0.15 - h) * bump);
    return SurfaceOut(col, n, 0.92, 0.0, col * rim * 0.15);
}

// ---- floor.gdshader -----------------------------------------------------
// aux at o: the level's numbers (see view.rs FloorData), then this chunk's
// path segments, islands and plazas.

// An integer hash of the cell: exact everywhere. The old
// fract(sin(dot(p, k)) * 43758.5) lost its precision away from the origin
// (large sine arguments), so neighbouring cells got the same value and the
// noise broke into flat facets with straight seams (Joep's floor photo).
fn ss_hash2(p: vec2<f32>) -> f32 {
    let q = bitcast<vec2<u32>>(vec2<i32>(floor(p)));
    var h = (q.x * 1597334673u) ^ (q.y * 3812015801u);
    h = (h ^ (h >> 16u)) * 2246822519u;
    h = (h ^ (h >> 13u)) * 3266489917u;
    h = h ^ (h >> 16u);
    return f32(h >> 8u) * (1.0 / 16777216.0);
}

fn ss_noise2(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    return mix(mix(ss_hash2(i), ss_hash2(i + vec2<f32>(1.0, 0.0)), u.x),
        mix(ss_hash2(i + vec2<f32>(0.0, 1.0)), ss_hash2(i + vec2<f32>(1.0, 1.0)), u.x), u.y);
}

const SS_ROT: mat2x2<f32> = mat2x2<f32>(vec2<f32>(0.8, 0.6), vec2<f32>(-0.6, 0.8));

fn ss_fbm2(q: vec2<f32>) -> f32 {
    var p = q;
    var v = 0.0;
    var a = 0.5;
    for (var i = 0; i < SS_OCT; i++) {
        v += a * ss_noise2(p);
        p = SS_ROT * p * 2.03;
        a *= 0.5;
    }
    return v;
}

fn ss_pebbles(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    var best = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let g = vec2<f32>(f32(x), f32(y));
            let keep = ss_hash2(i + g + 31.0);
            if keep > 0.35 { continue; }
            let o = vec2<f32>(ss_hash2(i + g + 3.0), ss_hash2(i + g + 41.0)) * 0.6 + 0.2;
            let r = 0.12 + 0.14 * ss_hash2(i + g + 9.0);
            let d = (g + o - f) * vec2<f32>(1.0, 1.0 + 0.5 * ss_hash2(i + g + 13.0));
            best = max(best, 1.0 - smoothstep(r * 0.6, r, length(d)));
        }
    }
    return best;
}

fn ss_cells(p: vec2<f32>) -> vec2<f32> {
    let i = floor(p);
    let f = fract(p);
    var d1 = 8.0;
    var d2 = 8.0;
    var id = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let g = vec2<f32>(f32(x), f32(y));
            let o = vec2<f32>(ss_hash2(i + g), ss_hash2(i + g + 17.0));
            let d = length(g + o - f);
            if d < d1 {
                d2 = d1;
                d1 = d;
                id = ss_hash2(i + g + 5.0);
            } else if d < d2 {
                d2 = d;
            }
        }
    }
    return vec2<f32>(d2 - d1, id);
}

// Colours written in the Godot shader's code are linear as they stand;
// only its source_color uniforms (and their defaults) are sRGB.

// Level record offsets (view.rs writes these in this order).
const FL_BASE_A: u32 = 0u;
const FL_BASE_B: u32 = 3u;
const FL_TILES: u32 = 6u;
const FL_HALF: u32 = 7u;
const FL_WET: u32 = 9u;
const FL_PATH_W: u32 = 10u;
const FL_STYLE: u32 = 11u;
const FL_PATH_A: u32 = 12u;
const FL_PATH_B: u32 = 15u;
const FL_ZONE_Z: u32 = 18u;
const FL_TINT0: u32 = 20u;
const FL_FLAGS: u32 = 29u;
const FL_BOUNDS: u32 = 30u;
const FL_SDF: u32 = 34u;

fn ss_bounds_tex(l: u32, i: vec2<i32>) -> f32 {
    let w = i32(ss_f(l + FL_BOUNDS + 2u));
    let h = i32(ss_f(l + FL_BOUNDS + 3u));
    let c = clamp(i, vec2<i32>(0), vec2<i32>(w - 1, h - 1));
    return ss_f(aux_word(l + FL_SDF) + u32(c.y * w + c.x));
}

// Bilinear like Godot's filter_linear over texel centres, at uv in 0..1.
fn ss_bounds_lin(l: u32, uv: vec2<f32>) -> f32 {
    let sz = vec2<f32>(ss_f(l + FL_BOUNDS + 2u), ss_f(l + FL_BOUNDS + 3u));
    let t = uv * sz - 0.5;
    let i = floor(t);
    let f = t - i;
    let ii = vec2<i32>(i);
    let a = ss_bounds_tex(l, ii);
    let b = ss_bounds_tex(l, ii + vec2<i32>(1, 0));
    let c = ss_bounds_tex(l, ii + vec2<i32>(0, 1));
    let d = ss_bounds_tex(l, ii + vec2<i32>(1, 1));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

fn ss_bounds_at(l: u32, p: vec2<f32>) -> f32 {
    let rect = vec4<f32>(ss_f(l + FL_BOUNDS), ss_f(l + FL_BOUNDS + 1u), ss_f(l + FL_BOUNDS + 2u), ss_f(l + FL_BOUNDS + 3u));
    let sz = rect.zw;
    let c = (p - rect.xy) - 0.5;
    let i = floor(c);
    let f = c - i;
    let f2 = f * f;
    let f3 = f2 * f;
    let w0 = (1.0 - 3.0 * f + 3.0 * f2 - f3) / 6.0;
    let w1 = (4.0 - 6.0 * f2 + 3.0 * f3) / 6.0;
    let w2 = (1.0 + 3.0 * f + 3.0 * f2 - 3.0 * f3) / 6.0;
    let w3 = f3 / 6.0;
    let g0 = w0 + w1;
    let g1 = w2 + w3;
    let h0 = (i - 1.0 + w1 / g0 + 0.5) / sz;
    let h1 = (i + 1.0 + w3 / g1 + 0.5) / sz;
    return g0.y * (g0.x * ss_bounds_lin(l, vec2<f32>(h0.x, h0.y)) + g1.x * ss_bounds_lin(l, vec2<f32>(h1.x, h0.y)))
        + g1.y * (g0.x * ss_bounds_lin(l, vec2<f32>(h0.x, h1.y)) + g1.x * ss_bounds_lin(l, vec2<f32>(h1.x, h1.y)));
}

struct FloorAt { col: vec3<f32>, h: f32, rough: f32, glow: vec3<f32> };

fn ss_floor_at(p: vec2<f32>, o: u32, time: f32) -> FloorAt {
    // The chunk: level record offset, then counts and the lists.
    let l = aux_word(o);
    let seg_n = aux_word(o + 1u);
    let isle_n = aux_word(o + 2u);
    let plaza_n = aux_word(o + 3u);
    let segs = o + 4u;
    let isles = segs + seg_n * 5u;
    let plazas = isles + isle_n * 4u;
    let base_a = ss_v3(l + FL_BASE_A);
    let base_b = ss_v3(l + FL_BASE_B);
    let tiles = ss_f(l + FL_TILES);
    let half_size = vec2<f32>(ss_f(l + FL_HALF), ss_f(l + FL_HALF + 1u));
    let wet = ss_f(l + FL_WET);
    let path_w = ss_f(l + FL_PATH_W);
    let path_style = i32(ss_f(l + FL_STYLE));
    let path_a = ss_v3(l + FL_PATH_A);
    let path_b = ss_v3(l + FL_PATH_B);
    let zone_z = vec2<f32>(ss_f(l + FL_ZONE_Z), ss_f(l + FL_ZONE_Z + 1u));
    let flags = aux_word(l + FL_FLAGS);
    let water = (flags & 1u) != 0u;
    let bounded = (flags & 2u) != 0u;
    let square_plazas = (flags & 4u) != 0u;
    let outside_lava = (flags & 8u) != 0u;
    let water_a = vec3<f32>(0.0012, 0.0040, 0.0056);
    let water_b = vec3<f32>(0.0031, 0.0152, 0.0170);
    let sand = vec3<f32>(0.0331, 0.0273, 0.0152);

    let n = ss_fbm2(p * 0.35);
    let fine = ss_fbm2(p * 2.5);
    var col: vec3<f32>;
    var h = fine * 0.5 + n * 0.3;
    if tiles > 0.5 {
        col = mix(base_a, base_b, smoothstep(0.3, 0.7, n));
        col *= 0.85 + 0.3 * fine;
    } else {
        let patch_k = ss_fbm2(p * 0.13 + 4.0) * 0.75 + ss_fbm2(p * 0.5) * 0.25;
        col = mix(base_b, base_a, smoothstep(0.32, 0.68, patch_k));
        col *= 0.9 + 0.2 * fine;
        col *= 0.93 + 0.14 * ss_noise2(SS_ROT * p * 13.0);
        let tuft = smoothstep(0.62, 0.8, ss_fbm2(p * 3.1 + 9.0)) * smoothstep(0.45, 0.65, patch_k);
        col = mix(col, base_a * 1.35, tuft * 0.5);
        h += tuft * 0.4 + ss_fbm2(p * 9.0 + 3.0) * 0.3;
    }
    var rough = mix(0.95, 0.25, wet * smoothstep(0.4, 0.6, ss_fbm2(p * 0.6 + 3.0)));
    var glow = vec3<f32>(0.0);
    if tiles > 0.5 {
        var t = p / 1.6;
        t.x += step(1.0, (floor(t.y) - 2.0 * floor(floor(t.y) / 2.0))) * 0.5;
        let f = fract(t);
        let id = floor(t);
        let edge = min(min(f.x, 1.0 - f.x), min(f.y, 1.0 - f.y));
        let grout = smoothstep(0.0, 0.05, edge);
        col *= mix(0.35, 1.0, grout);
        col *= 0.8 + 0.4 * ss_hash2(id);
        h += grout * 1.5 + ss_hash2(id) * 0.3;
    }
    let k1 = smoothstep(zone_z.x + 6.0, zone_z.x - 6.0, p.y);
    let k2 = smoothstep(zone_z.y + 6.0, zone_z.y - 6.0, p.y);
    col *= mix(mix(ss_v3(l + FL_TINT0), ss_v3(l + FL_TINT0 + 3u), k1), ss_v3(l + FL_TINT0 + 6u), k2);
    let cr = ss_fbm2(p * 1.1 + 7.0);
    let crack = smoothstep(0.48, 0.5, cr) * smoothstep(0.52, 0.5, cr);
    col *= 1.0 - crack * 0.35 * tiles;

    var shore = -1e5;
    if water {
        var d = 1e5;
        for (var i = 0u; i < isle_n; i++) {
            let e = vec4<f32>(ss_f(isles + i * 4u), ss_f(isles + i * 4u + 1u), ss_f(isles + i * 4u + 2u), ss_f(isles + i * 4u + 3u));
            let q = (p - e.xy) / e.zw;
            d = min(d, (length(q) - 1.0) * min(e.z, e.w));
        }
        shore = d + (ss_fbm2(p * 0.9) - 0.5) * 0.9;
        col = mix(col, sand * (0.8 + 0.4 * fine), smoothstep(-1.2, -0.1, shore));
    }

    var pz = 1e5;
    for (var i = 0u; i < plaza_n; i++) {
        let c = ss_v3(plazas + i * 3u);
        let dq = abs(p - c.xy);
        pz = min(pz, select(length(dq), max(dq.x, dq.y), square_plazas) - c.z);
    }
    pz += (ss_fbm2(p * 0.7 + 5.0) - 0.5) * select(1.6, 0.7, square_plazas);
    if pz < 0.0 && shore < 0.0 {
        var t = p / 1.5;
        t.x += step(1.0, (floor(t.y) - 2.0 * floor(floor(t.y) / 2.0))) * 0.5;
        let f = fract(t);
        let edge = min(min(f.x, 1.0 - f.x), min(f.y, 1.0 - f.y));
        var flag = mix(path_a, path_b, ss_hash2(floor(t))) * (0.8 + 0.35 * fine);
        flag = mix(flag, vec3<f32>(0.2, 0.2, 0.19) * (0.8 + 0.4 * fine), 0.35);
        flag *= mix(0.4, 1.0, smoothstep(0.0, 0.06, edge));
        let worn = smoothstep(-1.4, 0.0, pz) * smoothstep(0.35, 0.7, ss_fbm2(p * 1.3));
        col = mix(flag, col, worn);
        h = mix(h + smoothstep(0.0, 0.06, edge) * 1.5, h, worn);
    }

    // The path: distance to it, distance along it.
    var best = 1e5;
    var along = 0.0;
    for (var i = 0u; i < seg_n; i++) {
        let s = segs + i * 5u;
        let a = vec2<f32>(ss_f(s), ss_f(s + 1u));
        let ab = vec2<f32>(ss_f(s + 2u), ss_f(s + 3u)) - a;
        let len = max(length(ab), 0.001);
        let t = clamp(dot(p - a, ab) / (len * len), 0.0, 1.0);
        let d = length(p - (a + ab * t));
        if d < best {
            best = d;
            along = ss_f(s + 4u) + t * len;
        }
    }
    let half_w = path_w * 0.5 + (ss_fbm2(p * 1.7) - 0.5) * 0.5;
    var on_path = select(0.0, 1.0 - smoothstep(half_w - 0.25, half_w + 0.1, best), seg_n > 0u);
    let bridge = water && shore > -0.2;
    // A plaza is paved over the path: the dirt runs up to its flagstones and
    // stops (Joep, 24 Nov: a dark band across the plaza read as smeared mist).
    if !bridge {
        on_path *= smoothstep(-1.2, 0.2, pz);
    }

    if water && shore > 0.0 && !(bridge && best < path_w * 0.5 + 0.1) {
        let w1 = ss_fbm2(p * 0.5 + vec2<f32>(time * 0.05, time * 0.03));
        let w2 = ss_fbm2(p * 1.6 - vec2<f32>(time * 0.11, -time * 0.07));
        var wc = mix(water_a, water_b, w1 * 0.8 + w2 * 0.4);
        wc *= mix(1.0, 0.55, smoothstep(0.5, 6.0, shore));
        var foam = (1.0 - smoothstep(0.0, 0.45, shore)) * (0.55 + 0.45 * sin(shore * 12.0 - time * 2.0));
        foam += smoothstep(0.72, 0.8, w2) * 0.25;
        wc = mix(wc, vec3<f32>(0.42, 0.52, 0.52), clamp(foam, 0.0, 1.0) * 0.55);
        col = wc;
        rough = 0.08;
        h = w2 * 0.4;
        glow = water_b * 0.12 * (0.6 + w2);
    } else if on_path > 0.0 {
        var pc: vec3<f32>;
        if bridge {
            let al = along / 0.42;
            let plank = fract(al);
            let pid = floor(al);
            pc = mix(vec3<f32>(0.11, 0.08, 0.05), vec3<f32>(0.065, 0.047, 0.03), ss_hash2(vec2<f32>(pid, 3.0)));
            pc *= 0.8 + 0.35 * ss_fbm2(vec2<f32>(best * 6.0, pid * 3.0));
            pc *= mix(0.25, 1.0, smoothstep(0.0, 0.08, plank) * smoothstep(1.0, 0.92, plank));
            let rail = smoothstep(path_w * 0.5 - 0.22, path_w * 0.5 - 0.12, best);
            pc = mix(pc, vec3<f32>(0.2, 0.13, 0.08), rail);
            h = smoothstep(0.0, 0.08, plank) * smoothstep(1.0, 0.92, plank) * 1.4 + rail;
            on_path = step(best, path_w * 0.5);
            rough = mix(rough, 0.9, on_path);
        } else if path_style == 1 {
            let c = ss_cells(p * 1.3);
            pc = mix(path_a, path_b, c.y) * (0.85 + 0.3 * fine);
            pc *= mix(0.3, 1.0, smoothstep(0.02, 0.12, c.x));
            h = mix(h, fine * 0.4 + smoothstep(0.02, 0.12, c.x) * 1.6, on_path);
        } else if path_style == 2 {
            let edge = smoothstep(half_w - 0.45, half_w - 0.35, best);
            pc = mix(path_a, vec3<f32>(0.3, 0.22, 0.09), edge * (1.0 - smoothstep(half_w - 0.2, half_w - 0.1, best)));
            pc *= 0.75 + 0.4 * ss_fbm2(p * 4.0);
            pc = mix(pc, col, smoothstep(0.62, 0.7, ss_fbm2(p * 0.8 + 11.0)) * 0.8);
        } else if path_style == 3 {
            let c = ss_cells(p * 1.1);
            pc = mix(path_a, path_b, c.y) * (0.8 + 0.3 * fine);
            let heat = smoothstep(0.42, 0.72, ss_fbm2(p * 0.45 + vec2<f32>(time * 0.03, 3.0)));
            let seam = 1.0 - smoothstep(0.0, 0.025 + 0.035 * heat, c.x);
            pc = mix(pc, vec3<f32>(0.03, 0.02, 0.02), seam);
            h = mix(h, fine * 0.5 + (1.0 - seam) * 1.5, on_path);
            let core = 1.0 - smoothstep(0.0, 0.012 + 0.02 * heat, c.x);
            glow = vec3<f32>(1.0, 0.28, 0.03) * (seam * 0.35 + core) * heat * on_path * (0.45 + 0.2 * sin(time * 1.5 + p.x));
        } else {
            let mid = 1.0 - smoothstep(0.0, half_w, best);
            pc = mix(path_b, path_a, mid * 0.6 + 0.4 * fine);
            let pebble = ss_pebbles(p * 3.0) * smoothstep(half_w * 0.3, half_w * 0.8, best);
            pc = mix(pc, path_a * 1.1, pebble * 0.4);
            h = mix(h, fine * 0.3 + pebble * 0.6, on_path);
        }
        col = mix(col, pc, on_path);
    }

    let e = abs(p) / half_size;
    var edge_dark = smoothstep(0.75, 1.05, max(e.x, e.y));
    if bounded {
        let sd = ss_bounds_at(l, p);
        col *= 1.0 - 0.3 * smoothstep(-2.5, 0.3, sd);
        col = mix(col, base_b * (0.75 + 0.4 * fine), smoothstep(0.0, 1.5, sd));
        edge_dark = smoothstep(1.0, 14.0, sd) * 0.75;
        glow += base_b * (0.6 + 0.5 * fine) * 0.22 * smoothstep(0.5, 3.0, sd);
        if outside_lava {
            let out_k = smoothstep(0.6, 2.2, sd);
            let ln = ss_fbm2(p * 0.35 + vec2<f32>(0.0, time * 0.02));
            let seam = pow(1.0 - abs(ln * 2.0 - 1.0), 6.0);
            let pool = smoothstep(0.62, 0.8, ss_fbm2(p * 0.12 + 9.0));
            let heat = clamp(seam * 0.8 + pool, 0.0, 1.0) * (0.75 + 0.25 * sin(time * 1.3 + p.x * 0.4 + p.y * 0.3));
            col = mix(col, vec3<f32>(0.03, 0.015, 0.012) * (0.7 + 0.6 * fine), out_k);
            glow += vec3<f32>(1.0, 0.3, 0.04) * heat * out_k * 0.9;
            edge_dark *= 1.0 - out_k * 0.7;
        }
    }
    col *= 1.0 - min(edge_dark, 1.0) * 0.6 - max(edge_dark - 1.0, 0.0) * 0.8;
    return FloorAt(col * 2.4, h, rough, glow);
}

fn ss_floor(s: SurfaceIn, o: u32) -> SurfaceOut {
    let here = ss_floor_at(s.p.xz, o, s.time);
    if SS_LITE { return SurfaceOut(here.col, s.n, here.rough, 0.0, here.glow); }
    let hx = ss_floor_at(s.p.xz + s.dx.xz, o, s.time).h;
    let hy = ss_floor_at(s.p.xz + s.dy.xz, o, s.time).h;
    let bump = 0.035;
    let n = ss_bump(s.n, s.dx, s.dy, (hx - here.h) * bump, (hy - here.h) * bump);
    return SurfaceOut(here.col, n, here.rough, 0.0, here.glow);
}

fn game_surface(s: SurfaceIn) -> SurfaceOut {
    let kind = s.kind & 7u;
    let o = s.kind >> 3u;
    if kind == 1u { return ss_noise(s, o); }
    if kind == 2u { return ss_stone(s, o); }
    if kind == 3u { return ss_cloth(s, o); }
    if kind == 4u { return ss_floor(s, o); }
    return SurfaceOut(s.tint * ss_vcol(s).rgb, s.n, s.rough, s.metal, vec3<f32>(0.0));
}
