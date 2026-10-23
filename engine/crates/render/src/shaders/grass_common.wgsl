// Shared by the opaque visibility pass and the true-opacity transition pass.
// Source plants never move or change size. All decisions are per-view and use
// the continuous camera position; CPU residency extends beyond zero opacity.
const GRASS_OPAQUE_THRESHOLD: f32 = 1.0;

fn grass_distance_opacity(root_distance: f32, split: bool) -> f32 {
    // Every world cell retains all three source cards. Only its final opacity
    // changes; a sparser subset is not a coverage-preserving representation.
    let fade_range = select(vec2<f32>(40.0, 60.0), vec2<f32>(20.0, 38.0), split);
    return 1.0 - smoothstep(fade_range.x, fade_range.y, root_distance);
}
fn grass_card_opacity(root: vec3<f32>, camera: vec3<f32>, split: bool) -> f32 {
    return grass_distance_opacity(distance(root.xz, camera.xz), split);
}
