// Game image effects operate on the opaque image of one view, in linear HDR.
@group(0) @binding(0) var scene_color: texture_2d<f32>;
@group(0) @binding(1) var scene_depth: texture_depth_2d;
@group(0) @binding(2) var finished: texture_storage_2d<rgba16float, write>;

struct FinishSample { color: vec3<f32>, depth: f32, tag: u32 }

fn scene_sample(pixel: vec2<i32>) -> FinishSample {
    let limit = vec2<i32>(textureDimensions(scene_color)) - 1;
    let at = clamp(pixel, vec2<i32>(0), limit);
    let value = textureLoad(scene_color, at, 0);
    return FinishSample(value.rgb, textureLoad(scene_depth, at, 0), u32(max(abs(value.a) - 1.0, 0.0)));
}

@compute @workgroup_size(8, 8)
fn apply_finish(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= textureDimensions(finished)) { return; }
    let at = vec2<i32>(id.xy);
    let color = game_finish(at, scene_sample(at));
    let coverage = select(0.0, 1.0, textureLoad(scene_color, at, 0).a > 0.0);
    textureStore(finished, at, vec4<f32>(clamp(color, vec3<f32>(0), vec3<f32>(65504)), coverage));
}
