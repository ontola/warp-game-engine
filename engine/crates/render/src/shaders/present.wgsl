// LDR output already contains sRGB-encoded numbers. The live surface either
// stores them directly (unorm) or requires conversion before its sRGB encoder.
struct Surface { srgb: u32, pad0: u32, pad1: u32, pad2: u32 };
@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;
@group(0) @binding(2) var<uniform> surface: Surface;
struct Out { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs(@builtin(vertex_index) vertex: u32) -> Out {
    let points = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var out: Out;
    out.position = vec4<f32>(points[vertex], 0.0, 1.0);
    out.uv = points[vertex] * vec2<f32>(0.5, -0.5) + 0.5;
    return out;
}
@fragment fn fs(in: Out) -> @location(0) vec4<f32> {
    var color = textureSample(picture, linear_sampler, in.uv).rgb;
    if surface.srgb != 0u {
        color = select(pow((color + 0.055) / 1.055, vec3<f32>(2.4)), color / 12.92, color <= vec3<f32>(0.04045));
    }
    return vec4<f32>(color, 1.0);
}
