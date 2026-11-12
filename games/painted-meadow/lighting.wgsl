// All colours here are linear; the output is radiance before fog and exposure.
fn game_lighting(s: LightingSample) -> StyledLight {
    let light_side = smoothstep(0.16, 0.23, dot(s.normal, s.sun) * s.shadow);
    let daylight = max(1000.0, s.sun_lux * 0.30);
    if s.kind >= 1 {
        let night_blue = vec3<f32>(0.39, 0.51, 0.68);
        let sun_gold = vec3<f32>(1.0, 0.94, 0.79);
        let color = s.albedo * mix(night_blue, sun_gold, light_side);
        return StyledLight(color * daylight, u32(s.kind));
    }
    var pigment = s.albedo;
    // Long, broken marks on the hillside, anchored in metres as the camera moves.
    if s.normal.y > 0.9 && pigment.g > pigment.r * 1.15 {
        let uv = s.position.xz * vec2<f32>(2.1, 6.5);
        let flow = sin(uv.y + 1.7 * sin(uv.x * 0.47));
        let stroke = smoothstep(0.62, 0.95, flow) * smoothstep(-0.25, 0.75, sin(uv.x + uv.y * 0.19));
        pigment *= 1.0 + 0.15 * stroke;
    }
    let value = mix(vec3<f32>(0.65,0.73,0.83),vec3<f32>(1.0,0.96,0.84),light_side);
    return StyledLight(pigment * value * daylight, 0u);
}
