// The terrain hex tiling before warp 7663ba89: a struct with an array of
// mat2x2 that naga's HLSL wraps in a layout type DXC has no mul() for.
// Firefox on Windows (Zen) failed the whole shade pass on it, so the room
// drew nothing. hlsl-check.py must reject this module.
@group(0) @binding(0) var<storage, read_write> out: array<vec2<f32>>;

struct HexTap { uv: array<vec2<f32>, 3>, r: array<mat2x2<f32>, 3>, w: vec3<f32> }

fn taps(uv: vec2<f32>) -> HexTap {
    var t: HexTap;
    for (var i = 0; i < 3; i++) {
        let a = f32(i) * 2.0;
        t.r[i] = mat2x2<f32>(vec2<f32>(cos(a), sin(a)), vec2<f32>(-sin(a), cos(a)));
        t.uv[i] = t.r[i] * uv;
    }
    t.w = vec3<f32>(1.0 / 3.0);
    return t;
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let t = taps(vec2<f32>(f32(id.x), 1.0));
    var g = vec2<f32>(0.0);
    for (var i = 0; i < 3; i++) { g += transpose(t.r[i]) * t.uv[i] * t.w[i]; }
    out[id.x] = g;
}
