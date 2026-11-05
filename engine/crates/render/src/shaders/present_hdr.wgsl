// An HDR surface (fp16, linear extended sRGB). The scene is the renderer's
// linear HDR output (1.0 = SDR white) or, without one, the SDR picture
// decoded. The HUD layer (premultiplied, drawn at SDR white) goes on top,
// then both are scaled to the OS's SDR white in surface units.
struct Hdr { linear_source: u32, hud: u32, white: f32, pad: u32 };
@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;
@group(0) @binding(2) var<uniform> hdr: Hdr;
@group(0) @binding(3) var scene_hdr: texture_2d<f32>;
@group(0) @binding(4) var hud_layer: texture_2d<f32>;

struct Out { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs(@builtin(vertex_index) vertex: u32) -> Out {
    let points = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var out: Out;
    out.position = vec4<f32>(points[vertex], 0.0, 1.0);
    out.uv = points[vertex] * vec2<f32>(0.5, -0.5) + 0.5;
    return out;
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

@fragment fn fs(in: Out) -> @location(0) vec4<f32> {
    let encoded = textureSample(picture, linear_sampler, in.uv).rgb;
    let linear = textureSample(scene_hdr, linear_sampler, in.uv).rgb;
    let layer = textureSample(hud_layer, linear_sampler, in.uv);
    var scene = select(srgb_to_linear(encoded), max(linear, vec3<f32>(0.0)), hdr.linear_source != 0u);
    if hdr.hud != 0u { scene = scene * (1.0 - layer.a) + layer.rgb; }
    return vec4<f32>(scene * hdr.white, 1.0);
}
