fn game_glow(p: vec3<f32>, instance: u32, material: u32) -> vec4<f32> {
    let b = mk_record(instance);
    if b == 0u { return vec4<f32>(0.0); }
    let d = mk_depth(p, b);
    if d < 0.0 { return vec4<f32>(0.0); }
    let c = vec3<f32>(bitcast<f32>(aux_word(b + 5u)), bitcast<f32>(aux_word(b + 6u)), bitcast<f32>(aux_word(b + 7u)));
    // The build front is molten: the bone there is replaced by a saturated
    // ember band (adding light only washed the pale bone out to cream), a
    // hot core right at the edge, then it cools back to bone over 0.5 m.
    let edge = 1.0 - smoothstep(0.0, 0.13, d);
    let cool = 1.0 - smoothstep(0.0, 0.5, d);
    let core = 1.0 - smoothstep(0.0, 0.03, d);
    let a = clamp(edge * 0.95 + cool * 0.3, 0.0, 1.0);
    // The crypt grade turns plain orange to tan; a red-heavy ember reads
    // as orange on screen, and the core goes hotter towards yellow.
    // The core is well above white so it blooms into a thin molten line.
    let hot = c * (edge * 0.9 + cool * 0.12) + vec3<f32>(1.0, 0.28, 0.03) * core * 3.0;
    return vec4<f32>(hot * light.anim.z, a);
}
