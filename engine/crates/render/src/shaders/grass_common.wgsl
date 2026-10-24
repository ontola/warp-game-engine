// Shared by the opaque visibility pass and the true-opacity transition pass.
// Source plants never move or change size. All decisions are per-view and use
// the continuous camera position; CPU residency extends beyond zero opacity.
const GRASS_OPAQUE_THRESHOLD: f32 = 1.0;

fn grass_distance_opacity(root_distance: f32, fade_range: vec2<f32>) -> f32 {
    // Every world cell retains all three source cards. Only its final opacity
    // changes; a sparser subset is not a coverage-preserving representation.
    return 1.0 - smoothstep(fade_range.x, fade_range.y, root_distance);
}
fn grass_card_opacity(root: vec3<f32>, camera: vec3<f32>, fade_range: vec2<f32>, reference: f32) -> f32 {
    // Explicit diagnostic only: compare the SAME resident plants and field.
    // Production reference=0 retains its exact distance fade.
    if reference > 0.5 && reference < 1.5 { return 1.0; }
    if reference >= 1.5 { return 0.0; }
    return grass_distance_opacity(distance(root.xz, camera.xz), fade_range);
}
