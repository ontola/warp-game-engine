fn game_glow(p: vec3<f32>, instance: u32, material: u32) -> vec3<f32> {
    let b = mk_record(instance);
    if b == 0u { return vec3<f32>(0.0); }
    let d = mk_depth(p, b);
    if d < 0.0 { return vec3<f32>(0.0); }
    let c = vec3<f32>(bitcast<f32>(aux_word(b + 5u)), bitcast<f32>(aux_word(b + 6u)), bitcast<f32>(aux_word(b + 7u)));
    // A thin white-hot edge, then its colour cooling over a forearm's length.
    // Wide enough to read from the game's high camera (Mancave shots).
    let edge = 1.0 - smoothstep(0.0, 0.06, d);
    let warm = 1.0 - smoothstep(0.0, 0.6, d);
    return (c * (warm * warm * 1.1 + edge * 1.2) + vec3<f32>(1.0, 0.75, 0.4) * edge * 0.5) * light.anim.z;
}
