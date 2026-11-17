fn game_glow(p: vec3<f32>, instance: u32, material: u32) -> vec4<f32> {
    let b = mk_record(instance);
    if b == 0u { return vec4<f32>(0.0); }
    let d = mk_depth(p, b);
    if d < 0.0 { return vec4<f32>(0.0); }
    let c = vec3<f32>(bitcast<f32>(aux_word(b + 5u)), bitcast<f32>(aux_word(b + 6u)), bitcast<f32>(aux_word(b + 7u)));
    // The build front is hot iron: white-yellow at the edge (well above
    // white so it blooms), orange, then a dark red crust that cools back to
    // bone. Medium-bright red read as pink flesh under the crypt grade, so
    // the brightness falls off steeply and the lower band stays dark.
    let t = d / 0.3;
    if t >= 1.0 { return vec4<f32>(0.0); }
    let white = vec3<f32>(1.0, 0.62, 0.25) * 7.0;
    let orange = c * 1.6;
    let red = vec3<f32>(0.6, 0.09, 0.0) * 0.45;
    let crust = vec3<f32>(0.12, 0.01, 0.0) * 0.15;
    var e = mix(white, orange, smoothstep(0.0, 0.08, t));
    e = mix(e, red, smoothstep(0.15, 0.45, t));
    e = mix(e, crust, smoothstep(0.4, 0.8, t));
    let a = 1.0 - smoothstep(0.6, 1.0, t);
    return vec4<f32>(e * a * light.anim.z, a);
}
