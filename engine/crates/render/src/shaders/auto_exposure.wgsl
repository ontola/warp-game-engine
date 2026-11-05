// Automatic exposure: meter one view's final HDR picture and adapt like an
// eye, entirely on the GPU. `histogram` counts a sparse grid of samples into
// 128 bins of log2 luminance (a quarter stop each); `adapt` runs as one
// workgroup, averages the bins between two percentiles, moves the current
// exposure towards that target and clears the bins for the next frame.
// The tone, bloom and TAA passes copy `state.exposure` into their own
// uniforms, so the frame being drawn already uses the adapted value.

struct Params {
    min_ev: f32,        // EV100 range the meter may follow
    max_ev: f32,
    compensation: f32,  // EV, positive brightens
    dt: f32,            // seconds since this view's previous metered frame
    speed_light: f32,   // 1/s towards a brighter scene (0 or less: at once)
    speed_dark: f32,    // 1/s towards a darker scene
    low: f32,           // share of the darkest samples ignored, 0..1
    high: f32,          // share of the samples below which to average, 0..1
    center: f32,        // 0: every sample counts the same, 1: the centre counts 16x
    initial_ev: f32,    // EV100 used before the first measurement
    reset: f32,         // 1: snap to the target, no adaptation
    pad: f32,
    samples: vec2<u32>, // the sample grid over the view
    pad2: vec2<u32>,
};

struct State {
    exposure: f32,      // 1 / (1.2 * 2^ev100), what tone and bloom multiply by
    ev100: f32,         // the adapted exposure value
    metered_ev100: f32, // the meter's reading before range and compensation
    ready: f32,         // 0 until the first measurement
};

const BINS: u32 = 128u;
const MIN_LOG2: f32 = -10.0; // log2 luminance of the lowest bin
const RANGE_LOG2: f32 = 32.0;

@group(0) @binding(0) var hdr: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> bins: array<atomic<u32>, 128>;
@group(0) @binding(2) var<uniform> p: Params;
@group(0) @binding(3) var<storage, read_write> state: State;

var<workgroup> local_bins: array<atomic<u32>, 128>;

@compute @workgroup_size(16, 16)
fn histogram(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    if li < BINS { atomicStore(&local_bins[li], 0u); }
    workgroupBarrier();
    if all(gid.xy < p.samples) {
        let size = textureDimensions(hdr);
        let uv = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(p.samples);
        let px = min(vec2<u32>(uv * vec2<f32>(size)), size - 1u);
        let raw = textureLoad(hdr, vec2<i32>(px), 0).rgb;
        let c = select(vec3<f32>(0.0), clamp(raw, vec3<f32>(0.0), vec3<f32>(65504.0)), raw == raw);
        let luminance = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
        let t = clamp((log2(max(luminance, 1e-12)) - MIN_LOG2) / RANGE_LOG2, 0.0, 0.99999);
        let d = uv * 2.0 - 1.0;
        let weight = 1u + u32(round(clamp(p.center, 0.0, 1.0) * 15.0 * exp(-2.0 * dot(d, d))));
        atomicAdd(&local_bins[u32(t * f32(BINS))], weight);
    }
    workgroupBarrier();
    if li < BINS {
        let n = atomicLoad(&local_bins[li]);
        if n > 0u { atomicAdd(&bins[li], n); }
    }
}

var<workgroup> counts: array<u32, 128>;

// log2 luminance of a bin's centre.
fn bin_log2(i: u32) -> f32 {
    return MIN_LOG2 + (f32(i) + 0.5) / f32(BINS) * RANGE_LOG2;
}

@compute @workgroup_size(128)
fn adapt(@builtin(local_invocation_index) li: u32) {
    counts[li] = atomicLoad(&bins[li]);
    atomicStore(&bins[li], 0u);
    workgroupBarrier();
    if li != 0u { return; }
    let fresh = p.reset > 0.5 || state.ready < 0.5;
    var ev = select(state.ev100, clamp(p.initial_ev, p.min_ev, p.max_ev) - p.compensation, fresh);
    var metered = state.metered_ev100;
    var total = 0.0;
    for (var i = 0u; i < BINS; i++) { total += f32(counts[i]); }
    let low = total * clamp(p.low, 0.0, 1.0);
    let high = max(total * clamp(p.high, 0.0, 1.0), low + 1.0);
    var below = 0.0;
    var sum = 0.0;
    var weight = 0.0;
    for (var i = 0u; i < BINS; i++) {
        let n = f32(counts[i]);
        // The part of this bin that lies between the two percentiles.
        let take = max(min(below + n, high) - max(below, low), 0.0);
        sum += take * bin_log2(i);
        weight += take;
        below += n;
    }
    if weight > 0.0 {
        // Average luminance to EV100 with the usual meter constant:
        // EV100 = log2(L * 100 / 12.5).
        metered = sum / weight + 3.0;
        let target_ev = clamp(metered, p.min_ev, p.max_ev) - p.compensation;
        if fresh || p.dt <= 0.0 {
            ev = target_ev;
        } else {
            // A higher EV means a brighter scene: adapt with the light speed.
            let speed = select(p.speed_dark, p.speed_light, target_ev > ev);
            let k = select(1.0 - exp(-p.dt * speed), 1.0, speed <= 0.0);
            ev = ev + (target_ev - ev) * k;
        }
    }
    state.ev100 = ev;
    state.metered_ev100 = metered;
    state.exposure = 1.0 / (1.2 * exp2(ev));
    state.ready = 1.0;
}
