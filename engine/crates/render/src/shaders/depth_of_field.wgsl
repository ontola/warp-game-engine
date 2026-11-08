// Thin-lens depth of field over final HDR, before exposure, bloom and tone.
// `tiles` finds the largest blur radius per 8x8 tile; `gather` then blurs each
// pixel with a scatter-as-gather disk sized by the largest radius nearby, so
// a blurred foreground also spreads over a sharp background. No storage
// buffers: textures and one uniform only (WebGPU minimum limits).
struct Params {
    // Autofocus point in the view, 0..1 from the top left.
    focus_uv: vec2<f32>,
    // Fixed focus distance in metres; zero or less focuses on `focus_uv`.
    focus_m: f32,
    // Camera near plane: reversed infinite Z stores depth = near / distance.
    near: f32,
    // Blur radius in output pixels is lens * (1 - focus / distance) / (focus - focal).
    lens: f32,
    focal_m: f32,
    max_radius: f32,
    // Tiles searched each side for nearby large radii.
    reach: f32,
};
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var scene_depth: texture_depth_2d;
@group(0) @binding(2) var tiles_out: texture_storage_2d<r32float, write>;
@group(0) @binding(3) var tiles: texture_2d<f32>;
@group(0) @binding(4) var output: texture_storage_2d<rgba16float, write>;
@group(0) @binding(5) var linear_sampler: sampler;
@group(0) @binding(6) var<uniform> params: Params;

const SKY: f32 = 1e7;
const TAPS: u32 = 64u;

var<workgroup> focus_shared: f32;
var<workgroup> radius_shared: atomic<u32>;

fn distance_m(d: f32) -> f32 {
    if d <= 0.0 { return SKY; }
    return min(params.near / d, SKY);
}

fn depth_pixel(uv: vec2<f32>) -> vec2<i32> {
    let size = vec2<i32>(textureDimensions(scene_depth));
    return clamp(vec2<i32>(uv * vec2<f32>(size)), vec2<i32>(0), size - 1);
}

// The nearest surface in a small cross around the focus point, so a thin
// subject edge does not focus on the background behind it.
fn focus_distance() -> f32 {
    if params.focus_m > 0.0 { return params.focus_m; }
    let p = depth_pixel(params.focus_uv);
    let last = vec2<i32>(textureDimensions(scene_depth)) - 1;
    var d = textureLoad(scene_depth, p, 0);
    let cross = array<vec2<i32>, 4>(vec2(3, 0), vec2(-3, 0), vec2(0, 3), vec2(0, -3));
    for (var i = 0; i < 4; i++) {
        d = max(d, textureLoad(scene_depth, clamp(p + cross[i], vec2<i32>(0), last), 0));
    }
    return distance_m(d);
}

// Signed blur radius in output pixels: negative in front of focus.
fn radius(uv: vec2<f32>, focus: f32) -> f32 {
    let s = max(focus, params.focal_m * 1.01);
    let distance = distance_m(textureLoad(scene_depth, depth_pixel(uv), 0));
    let r = params.lens * (1.0 - s / max(distance, 1e-4)) / (s - params.focal_m);
    return clamp(r, -params.max_radius, params.max_radius);
}

@compute @workgroup_size(8, 8)
fn tile_max(@builtin(global_invocation_id) id: vec3<u32>, @builtin(local_invocation_index) li: u32,
        @builtin(workgroup_id) group: vec3<u32>) {
    if li == 0u { focus_shared = focus_distance(); atomicStore(&radius_shared, 0u); }
    workgroupBarrier();
    let size = textureDimensions(source);
    if all(id.xy < size) {
        let uv = (vec2<f32>(id.xy) + 0.5) / vec2<f32>(size);
        // Positive floats order like their bit patterns.
        atomicMax(&radius_shared, bitcast<u32>(abs(radius(uv, focus_shared))));
    }
    workgroupBarrier();
    if li == 0u {
        textureStore(tiles_out, vec2<i32>(group.xy), vec4<f32>(bitcast<f32>(atomicLoad(&radius_shared)), 0.0, 0.0, 0.0));
    }
}

fn hash(p: vec2<u32>) -> f32 {
    // Interleaved gradient noise: rotates each pixel's disk to trade rings for grain.
    return fract(52.9829189 * fract(dot(vec2<f32>(p), vec2<f32>(0.06711056, 0.00583715))));
}

@compute @workgroup_size(8, 8)
fn gather(@builtin(global_invocation_id) id: vec3<u32>, @builtin(local_invocation_index) li: u32,
        @builtin(workgroup_id) group: vec3<u32>) {
    if li == 0u { focus_shared = focus_distance(); atomicStore(&radius_shared, 0u); }
    workgroupBarrier();
    let tile_count = vec2<i32>(textureDimensions(tiles));
    let reach = i32(params.reach);
    let side = 2 * reach + 1;
    for (var i = i32(li); i < side * side; i += 64) {
        let t = clamp(vec2<i32>(group.xy) + vec2<i32>(i % side - reach, i / side - reach), vec2<i32>(0), tile_count - 1);
        atomicMax(&radius_shared, bitcast<u32>(textureLoad(tiles, t, 0).r));
    }
    workgroupBarrier();
    let size = textureDimensions(source);
    if any(id.xy >= size) { return; }
    let pixel = vec2<i32>(id.xy);
    let original = textureLoad(source, pixel, 0);
    let search = bitcast<f32>(atomicLoad(&radius_shared));
    if search < 0.5 { textureStore(output, pixel, original); return; }
    let fsize = vec2<f32>(size);
    let uv = (vec2<f32>(id.xy) + 0.5) / fsize;
    let focus = focus_shared;
    let centre = radius(uv, focus);
    // Each tap stands for search^2 * pi / TAPS pixels, the centre for one; a
    // tap's light spreads over its own disk, pi * r^2 (pi cancels).
    var w = 1.0 / (3.14159265 * max(centre * centre, 0.25));
    var sum = original.rgb * w;
    var total = w;
    let tap_area = search * search / f32(TAPS);
    let spin = hash(id.xy) * 6.2831853;
    for (var i = 0u; i < TAPS; i++) {
        let r = sqrt((f32(i) + 0.5) / f32(TAPS)) * search;
        let a = f32(i) * 2.39996323 + spin;
        let at = clamp(uv + vec2<f32>(cos(a), sin(a)) * r / fsize, 0.5 / fsize, vec2<f32>(1.0) - 0.5 / fsize);
        let s = radius(at, focus);
        // A sample behind this pixel never blurs over it more than this
        // pixel's own blur: sharp subjects keep their edges.
        let e = select(abs(s), min(abs(s), abs(centre)), s > centre);
        let cover = clamp(e - r + 1.0, 0.0, 1.0);
        if cover <= 0.0 { continue; }
        let tw = cover * tap_area / max(e * e, 0.25);
        let c = textureSampleLevel(source, linear_sampler, at, 0.0).rgb;
        if all(c == c) && all(abs(c) < vec3<f32>(65000.0)) { sum += c * tw; total += tw; }
    }
    textureStore(output, pixel, vec4<f32>(sum / total, original.a));
}
