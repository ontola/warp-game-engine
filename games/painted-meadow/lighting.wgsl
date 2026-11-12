fn painted_field_hash(cell: vec2<f32>) -> f32 {
    let q = fract(vec3<f32>(cell.xyx)*vec3<f32>(0.1031,0.1137,0.0973));
    let h = q + dot(q, q.yzx+33.33);
    return fract((h.x+h.y)*h.z);
}

fn painted_field_wash(p: vec2<f32>) -> f32 {
    let cell = floor(p);
    let f = smoothstep(vec2<f32>(0), vec2<f32>(1), fract(p));
    return mix(mix(painted_field_hash(cell), painted_field_hash(cell+vec2<f32>(1,0)), f.x),
               mix(painted_field_hash(cell+vec2<f32>(0,1)), painted_field_hash(cell+vec2<f32>(1)), f.x), f.y);
}

// All colours here are linear; the output is radiance before fog and exposure.
fn game_lighting(s: LightingSample) -> StyledLight {
    let light_side = smoothstep(0.16, 0.23, dot(s.normal, s.sun) * s.shadow);
    let daylight = max(1000.0, s.sun_lux * 0.30);
    if s.kind == 40 || s.kind == 41 {
        let p = s.position.xz;
        let field = painted_field_wash(p * vec2<f32>(.17,.28));
        let sweep = painted_field_wash(vec2<f32>(p.x*.85+p.y*.27, p.y*1.8+field*2.4));
        var color = mix(vec3<f32>(.155,.335,.063),vec3<f32>(.325,.505,.091),field);
        if s.kind == 40 {
            color *= .94 + .13*smoothstep(.28,.74,sweep);
        } else {
            let hills = clamp((-p.y-16.0)/40.0,0.0,1.0);
            let floor_y = p.y*.055+hills*(2.6*sin(p.x*.063+p.y*.03)+1.8*cos(p.y*.12));
            let tip = smoothstep(.015,.32,s.position.y-floor_y);
            color = mix(color, s.albedo, .53) * (.95+.14*sweep);
            let facing = abs(dot(s.normal,s.sun));
            color *= mix(vec3<f32>(.53,.66,.62), vec3<f32>(1.04,1.03,.94),tip);
            color *= .86 + .18*smoothstep(.14,.75,facing);
        }
        let shade = mix(vec3<f32>(.35,.51,.61),vec3<f32>(1.0,.99,.9),s.shadow);
        return StyledLight(color * shade * daylight,0u);
    }
    if s.kind >= 1 {
        let night_blue = vec3<f32>(0.39, 0.51, 0.68);
        let sun_gold = vec3<f32>(1.0, 0.94, 0.79);
        let color = s.albedo * mix(night_blue, sun_gold, light_side);
        return StyledLight(color * daylight, u32(s.kind));
    }
    let value = mix(vec3<f32>(0.65,0.73,0.83),vec3<f32>(1.0,0.96,0.84),light_side);
    return StyledLight(s.albedo * value * daylight, 0u);
}
