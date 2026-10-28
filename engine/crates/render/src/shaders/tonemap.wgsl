// Physical exposure, then AgX (Troy Sobotka's view transform, polynomial
// fit), then sRGB encoding into an 8-bit image.

struct Tone {
    exposure: f32,  // 1 / (1.2 * 2^EV100)
    contrast: f32,
    saturation: f32,
    sharpen: f32,   // after TAA, which softens: 0 off
    offset: vec2<u32>, // where this view goes in the output image
    bloom_strength: f32,
    screen: u32,    // 1: bloom is Godot's glow, screen-blended after tonemapping
};

@group(0) @binding(0) var hdr: texture_2d<f32>;
@group(0) @binding(1) var ldr: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(2) var<uniform> tone: Tone;
@group(0) @binding(3) var bloom: texture_2d<f32>;
@group(0) @binding(4) var bloom_sampler: sampler;

fn agx_contrast(x: vec3<f32>) -> vec3<f32> {
    let x2 = x * x;
    let x4 = x2 * x2;
    return 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.00232;
}

fn agx(c: vec3<f32>) -> vec3<f32> {
    let m = mat3x3<f32>(
        vec3<f32>(0.842479062253094, 0.0423282422610123, 0.0423756549057051),
        vec3<f32>(0.0784335999999992, 0.878468636469772, 0.0784336),
        vec3<f32>(0.0792237451477643, 0.0791661274605434, 0.879142973793104),
    );
    let min_ev = -12.47393;
    let max_ev = 4.026069;
    var v = m * c;
    v = clamp(log2(max(v, vec3<f32>(1e-10))), vec3<f32>(min_ev), vec3<f32>(max_ev));
    v = (v - min_ev) / (max_ev - min_ev);
    return agx_contrast(v);
}

fn agx_eotf(v: vec3<f32>) -> vec3<f32> {
    let m = mat3x3<f32>(
        vec3<f32>(1.19687900512017, -0.0528968517574562, -0.0529716355144438),
        vec3<f32>(-0.0980208811401368, 1.15190312990417, -0.0980434501171241),
        vec3<f32>(-0.0990297440797205, -0.0989611768448433, 1.15107367264116),
    );
    return pow(max(m * v, vec3<f32>(0.0)), vec3<f32>(2.2));
}

fn srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

fn display(px: vec2<i32>) -> vec3<f32> {
    let size = vec2<i32>(textureDimensions(hdr));
    let p=clamp(px,vec2<i32>(0),size-1);
    // Additive overlap can exceed fp16 even when individual effects are
    // finite. Contain it before AgX mixes channels and sharpening samples
    // neighbours. This cannot recover radiance already lost to overflow.
    let raw = textureLoad(hdr, p, 0).rgb;
    let finite = select(vec3<f32>(0.0), clamp(raw, vec3<f32>(0.0), vec3<f32>(65504.0)), raw == raw);
    var c = finite * tone.exposure;
    var glow = vec3<f32>(0.0);
    if tone.bloom_strength>0.0 {
        // Bloom already uses this frame's exposure. Never expose it twice.
        // Separate per-view images and clamp sampling prohibit split bleed.
        let uv=(vec2<f32>(p)+0.5)/vec2<f32>(size);
        glow=textureSampleLevel(bloom,bloom_sampler,uv,0.0).rgb*tone.bloom_strength;
        if tone.screen==0u { c+=glow; glow=vec3<f32>(0.0); }
    }
    var v = agx(c);
    // Look: a touch of contrast and saturation around mid grey.
    let luma = dot(v, vec3<f32>(0.2126, 0.7152, 0.0722));
    v = mix(vec3<f32>(luma), v, tone.saturation);
    v = clamp((v - 0.5) * tone.contrast + 0.5, vec3<f32>(0.0), vec3<f32>(1.0));
    let out = clamp(agx_eotf(v), vec3<f32>(0.0), vec3<f32>(1.0));
    if tone.screen==0u { return out; }
    // Godot: tonemap the glow on its own, screen it over the image in sRGB.
    let g = srgb(clamp(agx_eotf(agx(glow)), vec3<f32>(0.0), vec3<f32>(1.0)));
    let o = srgb(out);
    return srgb_to_linear(o + g - o * g);
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

// The sharpened pass needs display() of each pixel's four neighbours: the
// workgroup computes its 8x8 tile plus a one-pixel border once into shared
// memory (100 evaluations instead of 320), then sharpens from there.
var<workgroup> tile: array<vec3<f32>, 100>;

fn tile_at(l: vec2<i32>) -> vec3<f32> { return tile[u32(l.y * 10 + l.x)]; }

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wg: vec3<u32>) {
    let size = textureDimensions(hdr);
    let inside = gid.x < size.x && gid.y < size.y;
    let p = vec2<i32>(gid.xy);
    if tone.sharpen <= 0.0 {
        if inside { textureStore(ldr, vec2<i32>(gid.xy + tone.offset), vec4<f32>(srgb(display(p)), 1.0)); }
        return;
    }
    // Uniform branch: every invocation reaches both barriers.
    let origin = vec2<i32>(wg.xy) * 8 - 1;
    for (var k = li; k < 100u; k += 64u) {
        tile[k] = display(origin + vec2<i32>(i32(k % 10u), i32(k / 10u)));
    }
    workgroupBarrier();
    if !inside { return; }
    let l = vec2<i32>(gid.xy) - origin;
    var out = tile_at(l);
    // Contrast-adaptive sharpening (after AMD's CAS): sharpen less
    // where the neighbourhood already has contrast; never overshoot it.
    let n = tile_at(l + vec2<i32>(0, -1));
    let s = tile_at(l + vec2<i32>(0, 1));
    let w = tile_at(l + vec2<i32>(-1, 0));
    let e = tile_at(l + vec2<i32>(1, 0));
    let lo = min(out, min(min(n, s), min(w, e)));
    let hi = max(out, max(max(n, s), max(w, e)));
    let room = min(lo, 1.0 - hi) / max(hi, vec3<f32>(1e-4));
    let amp = sqrt(clamp(room, vec3<f32>(0.0), vec3<f32>(1.0))) * tone.sharpen;
    out = clamp(out + (out * 4.0 - (n + s + w + e)) * amp * 0.25, lo, hi);
    textureStore(ldr, vec2<i32>(gid.xy + tone.offset), vec4<f32>(srgb(out), 1.0));
}

// The same pass evaluating every neighbour itself (KERN_TILED_POST=0),
// for comparison.
@compute @workgroup_size(8, 8)
fn main_direct(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = textureDimensions(hdr);
    if gid.x >= size.x || gid.y >= size.y {
        return;
    }
    let p = vec2<i32>(gid.xy);
    var out = display(p);
    if tone.sharpen > 0.0 {
        // Contrast-adaptive sharpening (after AMD's CAS): sharpen less
        // where the neighbourhood already has contrast; never overshoot it.
        let n = display(p + vec2<i32>(0, -1));
        let s = display(p + vec2<i32>(0, 1));
        let w = display(p + vec2<i32>(-1, 0));
        let e = display(p + vec2<i32>(1, 0));
        let lo = min(out, min(min(n, s), min(w, e)));
        let hi = max(out, max(max(n, s), max(w, e)));
        let room = min(lo, 1.0 - hi) / max(hi, vec3<f32>(1e-4));
        let amp = sqrt(clamp(room, vec3<f32>(0.0), vec3<f32>(1.0))) * tone.sharpen;
        out = clamp(out + (out * 4.0 - (n + s + w + e)) * amp * 0.25, lo, hi);
    }
    textureStore(ldr, vec2<i32>(gid.xy + tone.offset), vec4<f32>(srgb(out), 1.0));
}
