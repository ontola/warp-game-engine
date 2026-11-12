// Stochastic screen-space reflections: the resolve, temporal and composite
// passes (sssr.rs). Shade traced one GGX visible-normal ray per pixel and
// wrote, per pixel (rgba32uint): x the surface normal (octahedral, 12+12
// bits) and roughness (8 bits), y the environment fallback (RGB9E5), z the
// specular weight (env_brdf * ao, RGB) and the ray's confidence (4x8 unorm),
// w the hit's screen position this frame (2x16 unorm). Pixels that do not
// take part are zero.
//
// Resolve (Stachowiak, "Stochastic Screen-Space Reflections", 2015): each
// pixel reuses the rays of a few neighbours. A neighbour's hit lights this
// pixel along the direction to it, weighted by this pixel's BRDF over the
// pdf the neighbour sampled it with; the weighted mean of the hits'
// radiance (the ratio estimator) is the reflection, which the specular
// weight scales. Each hit reads last frame's image at the mip whose texels
// match the solid angle the sample stands for (filtered importance
// sampling), so few rays give a smooth lobe.

struct Uniform {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>,
    // xyz camera, w frame index.
    cam: vec4<f32>,
    // width, height, 1 / width, 1 / height.
    size: vec4<f32>,
    // x projection scale (proj[1][1]), y history levels, z resolve radius in
    // pixels, w 1 when the temporal history is valid.
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniform;
@group(0) @binding(1) var rays: texture_2d<u32>;
@group(0) @binding(2) var depth_tex: texture_depth_2d;
@group(0) @binding(3) var history: texture_2d<f32>;
@group(0) @binding(4) var samp: sampler;
@group(0) @binding(5) var resolved_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(6) var resolved_in: texture_2d<f32>;
@group(0) @binding(7) var temporal_prev: texture_2d<f32>;
@group(0) @binding(8) var temporal_out: texture_storage_2d<rgba16float, write>;

const PI: f32 = 3.14159265;
const SAMPLES: u32 = 8u;

fn oct_decode(e: vec2<f32>) -> vec3<f32> {
    var n = vec3<f32>(e.x, e.y, 1.0 - abs(e.x) - abs(e.y));
    if n.z < 0.0 {
        let s = select(vec2<f32>(-1.0), vec2<f32>(1.0), n.xy >= vec2<f32>(0.0));
        n = vec3<f32>((1.0 - abs(n.yx)) * s, n.z);
    }
    return normalize(n);
}

fn unpack_rgb9e5(w: u32) -> vec3<f32> {
    return vec3<f32>(f32(w & 511u), f32((w >> 9u) & 511u), f32((w >> 18u) & 511u)) * exp2(f32(w >> 27u) - 24.0);
}

struct Surface { n: vec3<f32>, rough: f32, env: vec3<f32>, weight: vec3<f32>, conf: f32, hit_uv: vec2<f32>, valid: bool }

fn surface(px: vec2<i32>) -> Surface {
    let r = textureLoad(rays, px, 0);
    var s: Surface;
    let wc = unpack4x8unorm(r.z);
    s.valid = any(wc.rgb > vec3<f32>(0.0));
    if !s.valid { return s; }
    s.n = oct_decode(vec2<f32>(f32(r.x & 4095u), f32((r.x >> 12u) & 4095u)) / 4095.0 * 2.0 - 1.0);
    s.rough = f32(r.x >> 24u) / 255.0;
    s.env = unpack_rgb9e5(r.y);
    s.weight = wc.rgb;
    s.conf = wc.a;
    s.hit_uv = unpack2x16unorm(r.w);
    return s;
}

fn world_at(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    let w = u.inv_view_proj * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    return w.xyz / w.w;
}

fn ggx_d(nh: f32, a: f32) -> f32 {
    let a2 = a * a;
    let d = nh * nh * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}

fn smith_g1(nx: f32, a: f32) -> f32 {
    let a2 = a * a;
    return 2.0 * nx / (nx + sqrt(a2 + (1.0 - a2) * nx * nx));
}

// The pdf of direction `l` under GGX visible-normal sampling from `v`.
fn vndf_pdf(v: vec3<f32>, n: vec3<f32>, a: f32, l: vec3<f32>) -> f32 {
    let h = normalize(v + l);
    let nv = max(dot(n, v), 1e-4);
    return smith_g1(nv, a) * ggx_d(max(dot(n, h), 0.0), a) / (4.0 * nv);
}

// The specular BRDF without Fresnel, times the cosine: D G2 / (4 n.v).
fn brdf_cos(v: vec3<f32>, n: vec3<f32>, a: f32, l: vec3<f32>) -> f32 {
    let h = normalize(v + l);
    let nv = max(dot(n, v), 1e-4);
    let nl = max(dot(n, l), 0.0);
    return ggx_d(max(dot(n, h), 0.0), a) * smith_g1(nv, a) * smith_g1(nl, a) / (4.0 * nv);
}

fn pcg(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

@compute @workgroup_size(8, 8)
fn resolve(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = vec2<i32>(u.size.xy);
    let px = vec2<i32>(id.xy);
    if any(px >= size) { return; }
    let c = surface(px);
    if !c.valid { textureStore(resolved_out, px, vec4<f32>(0.0)); return; }
    let d = textureLoad(depth_tex, px, 0);
    let uv = (vec2<f32>(px) + 0.5) * u.size.zw;
    let p = world_at(uv, d);
    let v = normalize(u.cam.xyz - p);
    let a = max(c.rough * c.rough, 1e-3);
    // World size of one pixel at this depth (for the plane test below).
    let pixel_m = 2.0 * length(u.cam.xyz - p) / (u.params.x * u.size.y);
    // Rotation of the reuse pattern: white noise per pixel and frame.
    let turn = f32(pcg(u32(px.x) ^ pcg(u32(px.y) ^ pcg(u32(u.cam.w)))) >> 8u) / 16777216.0 * 2.0 * PI;
    var sum = vec3<f32>(0.0);
    var weight = 0.0;
    var hit_dist = 0.0;
    var conf = 0.0;
    var counted = 0.0;
    for (var k = 0u; k < SAMPLES; k++) {
        // The centre first, then a Vogel disc of the resolve radius.
        var q = px;
        if k > 0u {
            let ang = turn + f32(k) * 2.3999632;
            let rad = sqrt(f32(k) / f32(SAMPLES - 1u)) * u.params.z;
            q = clamp(px + vec2<i32>(round(vec2<f32>(cos(ang), sin(ang)) * rad)), vec2<i32>(0), size - 1);
        }
        let s = surface(q);
        if !s.valid { continue; }
        let dq = textureLoad(depth_tex, q, 0);
        let pq = world_at((vec2<f32>(q) + 0.5) * u.size.zw, dq);
        // A neighbour on another surface (across a depth edge) does not
        // share this pixel's reflection: farther from this pixel's plane
        // than a few pixels' size.
        if abs(dot(pq - p, c.n)) > 4.0 * pixel_m || dot(s.n, c.n) < 0.0 { continue; }
        counted += 1.0;
        conf += s.conf;
        if s.conf <= 0.0 { continue; }
        // The neighbour's hit, as this frame's depth places it (the sky:
        // a point far along the same pixel).
        let hd = textureLoad(depth_tex, clamp(vec2<i32>(s.hit_uv * u.size.xy), vec2<i32>(0), size - 1), 0);
        let hit = world_at(s.hit_uv, max(hd, 1e-7));
        let to = hit - p;
        let len = length(to);
        if len < 1e-4 { continue; }
        let l = to / len;
        if dot(l, c.n) <= 0.0 { continue; }
        let lq = normalize(hit - pq);
        let vq = normalize(u.cam.xyz - pq);
        let pdf = vndf_pdf(vq, s.n, max(s.rough * s.rough, 1e-3), lq);
        if pdf <= 0.0 { continue; }
        let w = brdf_cos(v, c.n, a, l) / pdf;
        if w <= 0.0 { continue; }
        // Last frame's image at the hit, at the mip whose texels cover the
        // solid angle this sample stands for (1 / (N pdf)).
        let prev = u.prev_view_proj * vec4<f32>(hit, 1.0);
        if prev.w <= 1e-4 { continue; }
        let puv = vec2<f32>(prev.x / prev.w * 0.5 + 0.5, 0.5 - prev.y / prev.w * 0.5);
        if any(puv < vec2<f32>(0.0)) || any(puv > vec2<f32>(1.0)) { continue; }
        let sample_sr = 1.0 / (f32(SAMPLES) * pdf);
        let texel_m = 2.0 * max(prev.w, 1e-3) / (u.params.x * u.size.y);
        let texel_sr = (texel_m / len) * (texel_m / len);
        let lod = clamp(0.5 * log2(sample_sr / max(texel_sr, 1e-12)), 0.0, u.params.y - 1.0);
        // The mips hold colour times the share that is not a light's face
        // (reflection_mips.rs): divided out, a face reads as its surroundings.
        let hs = textureSampleLevel(history, samp, puv, lod);
        let radiance = select(vec3<f32>(0.0), hs.rgb / hs.a, hs.a > 1e-4);
        if any(radiance != radiance) { continue; }
        sum += radiance * w;
        weight += w;
        hit_dist += len * w;
    }
    let mean_conf = select(0.0, conf / counted, counted > 0.0);
    var reflection = c.env;
    var dist = 0.0;
    if weight > 0.0 {
        reflection = mix(c.env, sum / weight, mean_conf);
        dist = hit_dist / weight;
    }
    textureStore(resolved_out, px, vec4<f32>(min(reflection, vec3<f32>(65504.0)), dist));
}

// Temporal: last frame's result at where this pixel's reflection was
// (reprojected through the virtual point behind the surface at the hit
// distance), clamped to this frame's 3x3 neighbourhood, blended over about
// ten frames.
@compute @workgroup_size(8, 8)
fn temporal(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = vec2<i32>(u.size.xy);
    let px = vec2<i32>(id.xy);
    if any(px >= size) { return; }
    let cur = textureLoad(resolved_in, px, 0);
    if !surface(px).valid { textureStore(temporal_out, px, vec4<f32>(0.0)); return; }
    // Variance clipping (Salvi 2016): the history is clamped to the 3x3
    // neighbourhood's mean plus or minus one standard deviation, which a
    // single noisy pixel moves far less than it moves a min/max box.
    var m1 = vec3<f32>(0.0);
    var m2 = vec3<f32>(0.0);
    var n = 0.0;
    for (var k = 0; k < 9; k++) {
        let q = clamp(px + vec2<i32>(k % 3 - 1, k / 3 - 1), vec2<i32>(0), size - 1);
        if !surface(q).valid { continue; }
        let s = textureLoad(resolved_in, q, 0).rgb;
        m1 += s;
        m2 += s * s;
        n += 1.0;
    }
    let mean = m1 / max(n, 1.0);
    let sigma = sqrt(max(m2 / max(n, 1.0) - mean * mean, vec3<f32>(0.0)));
    let lo = mean - sigma;
    let hi = mean + sigma;
    var out = cur.rgb;
    if u.params.w > 0.5 {
        let d = textureLoad(depth_tex, px, 0);
        let p = world_at((vec2<f32>(px) + 0.5) * u.size.zw, d);
        let ray = p - u.cam.xyz;
        let virtual_p = u.cam.xyz + ray * ((length(ray) + cur.a) / max(length(ray), 1e-4));
        let prev = u.prev_view_proj * vec4<f32>(virtual_p, 1.0);
        if prev.w > 1e-4 {
            let puv = vec2<f32>(prev.x / prev.w * 0.5 + 0.5, 0.5 - prev.y / prev.w * 0.5);
            if all(puv >= vec2<f32>(0.0)) && all(puv <= vec2<f32>(1.0)) {
                let h = clamp(textureSampleLevel(temporal_prev, samp, puv, 0.0).rgb, lo, hi);
                out = mix(h, cur.rgb, 0.1);
            }
        }
    }
    textureStore(temporal_out, px, vec4<f32>(out, cur.a));
}

// Composite: added onto the shaded image (additive blend). Shade left the
// specular environment term out where it wrote a ray.
struct VsOut { @builtin(position) pos: vec4<f32> };

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
    var o: VsOut;
    let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    o.pos = vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
    return o;
}

@fragment
fn fs(i: VsOut) -> @location(0) vec4<f32> {
    let px = vec2<i32>(i.pos.xy);
    let s = surface(px);
    if !s.valid { return vec4<f32>(0.0); }
    return vec4<f32>(s.weight * textureLoad(resolved_in, px, 0).rgb, 0.0);
}
