// Unit-strength loading shared by affine crowns, grass and growth branches.
// Integer time harmonics repeat continuously every 120 s. A front travels
// along the wind at 12 m/s. Crosswind variation breaks up parallel bands.
fn plant_gust(root: vec2<f32>, direction: vec2<f32>, phase: f32) -> vec2<f32> {
    let crosswind = vec2<f32>(-direction.y, direction.x);
    let across = dot(root, crosswind);
    let advected = dot(root, direction) * (6.28318530718 / (120.0 * 12.0)) - phase;
    let leading = 9.0 * advected + 0.012 * across;
    let following = 17.0 * advected - 0.017 * across;
    let lateral = 11.0 * advected + 0.026 * across;
    let load = 0.49 + 0.36 * sin(leading) + 0.14 * sin(following);
    return direction * load + crosswind * (0.09 * sin(lateral));
}
