fn game_glow(p: vec3<f32>, instance: u32, material: u32) -> vec3<f32> {
    let b = mk_record(instance);
    if b == 0u { return vec3<f32>(0.0); }
    let d = mk_depth(p, b);
    if d < 0.0 { return vec3<f32>(0.0); }
    let c = vec3<f32>(bitcast<f32>(aux_word(b + 5u)), bitcast<f32>(aux_word(b + 6u)), bitcast<f32>(aux_word(b + 7u)));
    // A thin white-hot edge, then its colour cooling over a forearm's length.
    let edge = 1.0 - smoothstep(0.0, 0.05, d);
    let warm = 1.0 - smoothstep(0.0, 0.4, d);
    return (c * warm * warm * 1.8 + vec3<f32>(1.0, 0.85, 0.6) * edge * 2.5) * light.anim.z;
}
