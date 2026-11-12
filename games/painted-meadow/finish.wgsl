fn game_finish(pixel: vec2<i32>, center: FinishSample) -> vec3<f32> {
    if center.depth == 0.0 { return center.color; }
    if center.tag != 0u {
        let steps = array(vec2<i32>(-1,0),vec2<i32>(1,0),vec2<i32>(0,-1),vec2<i32>(0,1));
        for (var i = 0u; i < 4u; i++) {
            let other = scene_sample(pixel + steps[i]);
            // Reversed Z: the farther neighbour has the smaller value.
            if other.tag != center.tag && other.depth <= center.depth {
                return center.color * vec3<f32>(0.32,0.29,0.34);
            }
        }
        return center.color;
    }
    // Choose the quietest four-texel brush footprint. Keep strokes away from
    // figure edges, the skyline, and abrupt depth changes.
    var best = center.color;
    var lowest = 1e30;
    for (var i = 0; i < 4; i++) {
        let dir = vec2<i32>(select(-1,1,(i & 1)!=0),select(-1,1,(i & 2)!=0));
        var sum = center.color;
        var variance = 0.0;
        var valid = true;
        for (var k=1; k<4; k++) {
            let offset = dir * vec2<i32>(k & 1,k >> 1);
            let tap = scene_sample(pixel + offset);
            valid = valid && tap.tag == 0u && abs(tap.depth-center.depth) < 0.04*center.depth;
            sum += tap.color;
            let delta = tap.color - center.color;
            variance += dot(delta,delta);
        }
        if valid && variance < lowest { lowest=variance; best=sum*0.25; }
    }
    return best;
}
