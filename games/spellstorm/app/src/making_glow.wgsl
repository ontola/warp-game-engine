fn game_glow(p: vec3<f32>, instance: u32, material: u32) -> vec4<f32> {
    let b = mk_record(instance);
    if b == 0u { return vec4<f32>(0.0); }
    let d = mk_depth(p, b);
    if d < 0.0 { return vec4<f32>(0.0); }
    let c = vec3<f32>(bitcast<f32>(aux_word(b + 5u)), bitcast<f32>(aux_word(b + 6u)), bitcast<f32>(aux_word(b + 7u)));
    // The build front is hot iron: white-yellow at the edge (well above
    // white so it blooms), orange, a thin red, then a charcoal crust that cools back to
    // bone. Medium-bright red read as pink flesh under the crypt grade, so
    // the brightness falls off steeply and the lower band stays dark.
    let t = d / 0.3;
    if t >= 1.0 { return vec4<f32>(0.0); }
    let white = vec3<f32>(1.0, 0.62, 0.25) * 7.0;
    let orange = c * 1.1 + vec3<f32>(0.12, 0.03, 0.0);
    let red = vec3<f32>(0.55, 0.12, 0.01) * 0.3;
    let crust = vec3<f32>(0.05, 0.035, 0.03) * 0.5;
    var e = mix(white, orange, smoothstep(0.0, 0.12, t));
    e = mix(e, red, smoothstep(0.25, 0.42, t));
    e = mix(e, crust, smoothstep(0.38, 0.55, t));
    let a = 1.0 - smoothstep(0.5, 0.85, t);
    return vec4<f32>(e * a * light.anim.z, a);
}
