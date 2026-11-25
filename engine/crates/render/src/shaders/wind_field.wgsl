// Deterministic per-root loading, shared by affine trees, grass and articulated
// limbs. Keep in step with wind::gust_at. The 128 storm events repeat after 32 minutes.
struct PlantGust {
    load: vec2<f32>, breeze_age: f32, storm_age: f32, storm_release: vec2<f32>, storm_mix: f32,
};
fn pulse_trait(bits: u32, shift: u32) -> f32 {
    return f32((bits >> shift) & 15u) / 15.0;
}
fn plant_gust(root: vec2<f32>, direction: vec2<f32>, phase: f32, strength: f32) -> PlantGust {
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
    let span = duration - 4.0;
    let start = span * (0.025 + 0.14 * pulse_trait(bits, 0u));
    let rise = span * (0.13 + 0.12 * pulse_trait(bits, 4u));
    let crest = start + rise;
    let double_crest = duration > 16.0;
    let hold = span * (select(0.06, 0.33, double_crest) + 0.06 * pulse_trait(bits, 8u));
    let falling = crest + hold;
    let fall = span * (0.14 + 0.11 * pulse_trait(bits, 12u));
    var pulse = smoothstep(0.0, rise, local - start) * (1.0 - smoothstep(0.0, fall, local - falling));
    if double_crest {
        let t = (local - crest) / hold;
        pulse *= 1.0 - 0.53 * smoothstep(0.0, 0.42, t) * (1.0 - smoothstep(0.51, 1.0, t));
    }
    let breadth = 0.93 + 0.07 * cos(0.031 * across + 6.28318530718 * pulse_trait(bits, 24u));
    let peak = (0.68 + 0.32 * pulse_trait(bits, 16u)) * breadth;
    let heading = normalize(direction + crosswind * (1.30 * pulse_trait(bits, 20u) - 0.65));
    let background = direction * (0.18 + 0.06 * sin(11.0 * advected - 0.016 * across))
        + crosswind * (0.045 * sin(15.0 * advected + 0.022 * across));
    gust.load = mix(gust.load, mix(background, heading * peak, pulse), storm_mix);
    gust.storm_age = max(local - falling - 0.5 * fall, 0.0);
    gust.storm_release = (heading * peak - direction * 0.18) / max(sqrt(fall), 1.0);
    return gust;
}
