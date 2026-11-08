// Portable temporal upscaler (UpscaleMode::Temporal*). The scene renders at
// a lower resolution with a sub-pixel Halton jitter; every output pixel
// gathers the jittered render samples around it (Gaussian weights by how
// close each sample landed), reprojects its own output-resolution history
// with the motion vectors (motion.rs) and blends the two. History is
// variance clipped to the current samples' colour range (YCoCg of
// range-compressed colour, as in taa.wgsl). Only textures: no storage
// buffers, one storage texture, so it fits WebGPU's minimum limits.

struct Upscale {
    render: vec4<f32>,  // render w, h, 1/w, 1/h
    output: vec4<f32>,  // output w, h, 1/w, 1/h
    jitter: vec4<f32>,  // sample offset in render pixels (x, y down), output/render scale x, y
    params: vec4<f32>,  // exposure, maximum history weight, 1: reset, clip gamma
};

@group(0) @binding(0) var color: texture_2d<f32>;
// Dominant surface depth from motion.rs (R32Float, reversed Z).
@group(0) @binding(1) var depth_tex: texture_2d<f32>;
// Previous minus current position, unjittered render pixels.
@group(0) @binding(2) var motion_vec: texture_2d<f32>;
// 1 where the surface has no trustworthy history.
@group(0) @binding(3) var motion_bias: texture_2d<f32>;
@group(0) @binding(4) var hist: texture_2d<f32>;
@group(0) @binding(5) var out_tex: texture_storage_2d<rgba16float, write>;
@group(0) @binding(6) var<uniform> up: Upscale;
@group(0) @binding(7) var lin: sampler;

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn compress(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0)) * up.params.x;
    return x / (1.0 + luma(x));
}

fn expand(x: vec3<f32>) -> vec3<f32> {
    return x / max(1.0 - luma(x), 1e-4) / up.params.x;
}

fn ycocg(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(0.25 * c.r + 0.5 * c.g + 0.25 * c.b, 0.5 * c.r - 0.5 * c.b, -0.25 * c.r + 0.5 * c.g - 0.25 * c.b);
}

fn rgb(y: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(y.x + y.y - y.z, y.x + y.z, y.x - y.y - y.z);
}

// Catmull-Rom through 5 bilinear taps of the output-resolution history.
fn history(uv: vec2<f32>) -> vec3<f32> {
    let pos = uv * up.output.xy;
    let c = floor(pos - 0.5) + 0.5;
    let f = pos - c;
    let w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    let w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    let w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    let w3 = f * f * (-0.5 + 0.5 * f);
    let w12 = w1 + w2;
    let t0 = (c - 1.0) * up.output.zw;
    let t3 = (c + 2.0) * up.output.zw;
    let t12 = (c + w2 / w12) * up.output.zw;
    var s = textureSampleLevel(hist, lin, vec2<f32>(t12.x, t0.y), 0.0).rgb * (w12.x * w0.y);
    s += textureSampleLevel(hist, lin, vec2<f32>(t0.x, t12.y), 0.0).rgb * (w0.x * w12.y);
    s += textureSampleLevel(hist, lin, t12, 0.0).rgb * (w12.x * w12.y);
    s += textureSampleLevel(hist, lin, vec2<f32>(t3.x, t12.y), 0.0).rgb * (w3.x * w12.y);
    s += textureSampleLevel(hist, lin, vec2<f32>(t12.x, t3.y), 0.0).rgb * (w12.x * w3.y);
    let wsum = w12.x * w0.y + w0.x * w12.y + w12.x * w12.y + w3.x * w12.y + w12.x * w3.y;
    return max(s / wsum, vec3<f32>(0.0));
}

// Pull `h` towards the box centre until it lies inside (Playdead).
fn clip_box(h: vec3<f32>, lo: vec3<f32>, hi: vec3<f32>) -> vec3<f32> {
    let centre = 0.5 * (hi + lo);
    let extent = 0.5 * (hi - lo) + 1e-5;
    let v = h - centre;
    let a = abs(v / extent);
    let m = max(a.x, max(a.y, a.z));
    return select(h, centre + v / m, m > 1.0);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let out_size = vec2<i32>(up.output.xy);
    let o = vec2<i32>(gid.xy);
    if o.x >= out_size.x || o.y >= out_size.y {
        return;
    }
    let rsize = vec2<i32>(up.render.xy);
    let po = vec2<f32>(o) + 0.5;
    // This output pixel's centre in render pixels, and the render pixel
    // whose jittered sample landed closest to it.
    let pr = po * up.render.xy * up.output.zw;
    let centre = vec2<i32>(floor(pr - up.jitter.xy));
    // Two gathers: a wide one (radius about one render pixel) for a frame
    // without history, and a narrow one (about one output pixel) whose
    // samples are accumulated into the history one by one.
    var sum = vec3<f32>(0.0);
    var wsum = 0.0;
    var fine = vec3<f32>(0.0);
    var fine_w = 0.0;
    var m1 = vec3<f32>(0.0);
    var m2 = vec3<f32>(0.0);
    var best = clamp(centre, vec2<i32>(0), rsize - 1);
    var best_d = -1.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let q = clamp(centre + vec2<i32>(x, y), vec2<i32>(0), rsize - 1);
            let c = compress(textureLoad(color, q, 0).rgb);
            // Where this sample was taken, in unjittered render pixels.
            let d = vec2<f32>(q) + 0.5 + up.jitter.xy - pr;
            // Gaussians close to a Blackman-Harris of radius 1.
            let w = exp(-2.29 * dot(d, d));
            sum += c * w;
            wsum += w;
            let e = d * up.jitter.zw;
            let wf = exp(-2.29 * dot(e, e));
            fine += c * wf;
            fine_w += wf;
            let s = ycocg(c);
            m1 += s;
            m2 += s * s;
            let z = textureLoad(depth_tex, q, 0).r;
            if z > best_d {
                best_d = z;
                best = q;
            }
        }
    }
    let current = sum / max(wsum, 1e-6);
    let mean = m1 / 9.0;
    let sigma = sqrt(max(m2 / 9.0 - mean * mean, vec3<f32>(0.0)));
    // Motion of the nearest surface around (edges of moving geometry
    // reproject with the foreground), scaled to output pixels.
    let vector = textureLoad(motion_vec, best, 0).xy * up.jitter.zw;
    let prev_uv = (po + vector) * up.output.zw;
    let inside = all(prev_uv >= vec2<f32>(0.0)) && all(prev_uv <= vec2<f32>(1.0));
    let trusted = textureLoad(motion_bias, best, 0).r < 0.5;
    var n = min(fine_w, 1.0);
    var result = current;
    if inside && trusted && up.params.z < 0.5 {
        var hn = min(textureSampleLevel(hist, lin, prev_uv, 0.0).a, up.params.y);
        let gamma = mix(up.params.w, 1.0, smoothstep(0.05, 0.5, length(vector)));
        let h = ycocg(compress(history(prev_uv)));
        let clipped = clip_box(h, mean - gamma * sigma, mean + gamma * sigma);
        // History that had to be pulled far was wrong: start counting again.
        let pulled = length(clipped - h) / (length(sigma) + 1e-3);
        hn = select(hn, min(hn, 2.0), pulled > 1.0);
        // Each sample adds as much weight as it landed close to this pixel:
        // a pixel no sample landed near keeps mostly history. A little of
        // the wide estimate keeps young history from being speckled.
        let wide = 0.05;
        result = (rgb(clipped) * hn + fine + current * wide) / (hn + fine_w + wide);
        n = min(hn + fine_w, up.params.y);
    }
    textureStore(out_tex, o, vec4<f32>(expand(result), n));
}
