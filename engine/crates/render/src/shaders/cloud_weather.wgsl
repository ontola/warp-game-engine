// Cloud weather, shared by shade.wgsl (cloud shadows on surfaces) and
// sky_volume.wgsl (the raymarched clouds and the shadowed air): one 2D field
// says how much cloud stands above each point, so the shadows on the ground
// fall where the clouds are. No bindings: callers pass their parameters.
//
// cloud: x coverage 0..1, y base altitude (m), z thickness (m), w extinction per m.
// motion: xy wind offset (m, already times time), z shadow strength, w enabled.

fn cloud_hash2(c: vec2<i32>) -> f32 {
    var h = (bitcast<u32>(c.x) * 0x8da6b343u) ^ (bitcast<u32>(c.y) * 0xd8163841u);
    h = (h ^ (h >> 16u)) * 0x7feb352du;
    h = (h ^ (h >> 15u)) * 0x846ca68bu;
    h = h ^ (h >> 16u);
    return f32(h & 0xffffffu) / 16777215.0;
}

fn cloud_value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = cloud_hash2(c);
    let b = cloud_hash2(c + vec2<i32>(1, 0));
    let d = cloud_hash2(c + vec2<i32>(0, 1));
    let e = cloud_hash2(c + vec2<i32>(1, 1));
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}

// How much cloud stands over world point xz (m), 0..1: three octaves of
// value noise from ~4 km cells, thresholded so that about `coverage` of the
// sky is cloud. Callers add the wind offset to xz.
fn cloud_weather_noise(p: vec2<f32>) -> f32 {
    return 0.55 * cloud_value_noise(p) + 0.3 * cloud_value_noise(p * 2.03 + vec2<f32>(17.1, 3.7))
        + 0.15 * cloud_value_noise(p * 4.11 + vec2<f32>(5.3, 41.9));
}

fn cloud_weather(xz: vec2<f32>, coverage: f32) -> f32 {
    if coverage <= 0.0 { return 0.0; }
    let n = cloud_weather_noise(xz / 4000.0);
    let threshold = 1.0 - coverage;
    return smoothstep(threshold - 0.08, threshold + 0.08, n);
}

// The cloud's footprint for shadows: the weather with two finer octaves
// (~400 and ~190 m) and a narrow edge, centred where the raymarched cloud
// actually starts (its 3D shape only clears the threshold well inside the
// weather ramp). The plain weather ramp cast a shadow a km soft.
fn cloud_shadow_cover(xz: vec2<f32>, coverage: f32) -> f32 {
    if coverage <= 0.0 { return 0.0; }
    let p = xz / 4000.0;
    let fine = 0.65 * cloud_value_noise(p * 9.7 + vec2<f32>(31.7, 12.9))
        + 0.35 * cloud_value_noise(p * 21.3 + vec2<f32>(7.1, 63.3)) - 0.5;
    let n = cloud_weather_noise(p) + 0.1 * fine;
    let threshold = 1.0 - coverage + 0.03;
    return smoothstep(threshold - 0.02, threshold + 0.02, n);
}

// Mean cloud density along a vertical column, as a share of the full
// extinction (the 3D shape and erosion leave about this much on average).
const CLOUD_COLUMN_FILL: f32 = 0.3;

// The share of direct sunlight the cloud layer lets through to world point
// p (m), with `l` towards the sun. 1 without clouds, above them, or when the
// sun is down.
fn cloud_sun_transmittance(p: vec3<f32>, l: vec3<f32>, cloud: vec4<f32>, motion: vec4<f32>) -> f32 {
    if motion.w < 0.5 || motion.z <= 0.0 || l.y <= 0.01 { return 1.0; }
    let mid = cloud.y + 0.5 * cloud.z;
    if p.y >= cloud.y + cloud.z { return 1.0; }
    let t = max(mid - p.y, 0.0) / l.y;
    let w = cloud_shadow_cover(p.xz + l.xz * t + motion.xy, cloud.x);
    // Inside the layer only the part above p shades it.
    let above = clamp((cloud.y + cloud.z - p.y) / max(cloud.z, 1.0), 0.0, 1.0);
    let tau = cloud.w * cloud.z * above * CLOUD_COLUMN_FILL * w / max(l.y, 0.15);
    return mix(1.0, exp(-tau), clamp(motion.z, 0.0, 1.0));
}
