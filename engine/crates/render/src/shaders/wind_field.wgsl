// Deterministic per-root loading, shared by affine trees, grass and articulated
// limbs. Keep in step with wind::gust_at. The 128 storm events repeat after 32 minutes.
struct PlantGust {
    load: vec2<f32>, breeze_age: f32, storm_mix: f32,
};
fn pulse_trait(bits: u32, shift: u32) -> f32 {
    return f32((bits >> shift) & 15u) / 15.0;
}
fn crown_impulse(base: vec2<f32>, heading: vec2<f32>, peak: f32, age: f32, rise: f32,
    omega: f32, decay: f32, side_gain: f32, tail: f32, pressure_only: bool) -> vec2<f32> {
    if age <= 0.0 || (pressure_only && age >= rise) { return base; }
    var along: f32;
    var side: f32;
    if age < rise {
        let angle = 3.14159265359 * age / rise;
        let s = sin(angle);
        if pressure_only { return mix(base, heading * peak, s * s); }
        along = 0.5 - 0.5 * cos(angle);
        side = s * along;
    } else {
        let t = age - rise;
        let angle = omega * t;
        let envelope = exp(-decay * t) * tail;
        along = envelope * (cos(angle) + decay / omega * sin(angle));
        side = -envelope * sin(angle) * (3.14159265359 / (rise * omega));
    }
    let offset = heading * along + vec2<f32>(-heading.y, heading.x) * (side_gain * side);
    return base * (1.0 - length(offset)) + peak * offset;
}
fn plant_gust(root: vec2<f32>, direction: vec2<f32>, phase: f32, strength: f32, pressure_only: bool) -> PlantGust {
    let crosswind = vec2<f32>(-direction.y, direction.x);
    let across = dot(root, crosswind);
    let advected = dot(root, direction) * (6.28318530718 / (120.0 * 12.0)) - phase;
    let storm_mix = smoothstep(0.65, 1.0, strength);
    var gust: PlantGust;
    gust.storm_mix = storm_mix;
    if storm_mix < 1.0 {
        let leading = 9.0 * advected + 0.012 * across;
        let following = 17.0 * advected - 0.017 * across;
        let lateral = 11.0 * advected + 0.026 * across;
        let load = 0.49 + 0.36 * sin(leading) + 0.14 * sin(following);
        gust.breeze_age = fract(-leading / 6.28318530718) * (120.0 / 9.0);
        gust.load = direction * load + crosswind * (0.09 * sin(lateral));
    }
    if storm_mix == 0.0 { return gust; }
    let time = -advected * (120.0 / 6.28318530718)
        + 1.5 * sin(0.018 * across) + 0.6 * sin(0.041 * across);
    let block = floor(time / 30.0);
    let block_id = u32(block - 64.0 * floor(block / 64.0));
    let block_t = time - block * 30.0;
    let split = 10.5 + (9.0 / 63.0) * f32((13u * block_id + 21u) % 64u);
    let second = block_t >= split;
    let slot = 2u * block_id + u32(second);
    let local = select(block_t, block_t - split, second);
    let duration = select(split, 30.0 - split, second);
    var bits = (slot + 1u) * 0x9e3779b9u;
    bits = (bits ^ (bits >> 16u)) * 0x85ebca6bu;
    bits ^= bits >> 13u;
    let start = (0.02 + 0.10 * pulse_trait(bits, 0u)) * duration;
    let rise = 0.97 + 0.52 * pulse_trait(bits, 4u);
    let omega = (2.56 + 0.56 * pulse_trait(bits, 8u)) / rise;
    let decay = 0.46 + 0.30 * pulse_trait(bits, 12u);
    let breadth = 0.94 + 0.06 * cos(0.031 * across + 6.28318530718 * pulse_trait(bits, 24u));
    let peak = (0.79 + 0.20 * pulse_trait(bits, 16u)) * breadth;
    let heading = normalize(direction + crosswind * (1.30 * pulse_trait(bits, 20u) - 0.65));
    let side = select(-0.28, 0.28, (bits & 0x80000000u) == 0u);
    let background = direction * (0.14 + 0.046 * sin(17.0 * advected - 0.016 * across))
        + crosswind * (0.046 * sin(23.0 * advected + 0.022 * across));
    let tail = smoothstep(0.0, 1.3, duration - local);
    let first = crown_impulse(background, heading, peak, local - start, rise, omega, decay, side, tail, pressure_only);
    let second_start = (0.42 + 0.20 * pulse_trait(bits, 28u)) * duration;
    let side_heading = normalize(heading + crosswind * (0.92 * pulse_trait(bits, 8u) - 0.46));
    let side_peak = (0.41 + 0.44 * pulse_trait(bits, 24u)) * breadth;
    let second_wind = crown_impulse(first, side_heading, side_peak, local - second_start,
        rise * 0.79, omega * 1.15, decay * 1.12, -side, tail, pressure_only);
    gust.load = mix(gust.load, second_wind, storm_mix);
    return gust;
}
