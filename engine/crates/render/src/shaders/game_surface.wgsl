// A game's own surface shader (material flag SURFACE, 1 << 15).
//
// A game replaces this file with `render::set_game_surface(source)` before
// the renderer is created. Its `game_surface` turns the shading point into
// albedo, normal, roughness, metalness and emission, so procedural Godot
// shaders can be ported line by line. The material's `layer` holds
// -2 - kind: `kind` tells the game's surfaces apart, and the negative layer
// keeps the texture array and alpha test away from these materials.
//
// The built-in copy keeps the material as it is.

struct SurfaceIn {
    // Which of the game's surfaces, from Material::layer.
    kind: u32,
    // The instance (index into the scene's instances).
    instance: u32,
    // World position, and object-space (rest pose) position.
    p: vec3<f32>,
    local: vec3<f32>,
    // Shading normal (world), and the direction from the eye to the point.
    n: vec3<f32>,
    dir: vec3<f32>,
    // Barycentrics and each corner's raw uv, so a game can pack per-vertex
    // data (such as vertex colours) into uvs and blend it itself.
    bary: vec3<f32>,
    uv0: vec2<f32>,
    uv1: vec2<f32>,
    uv2: vec2<f32>,
    // World-space footprint of one pixel (ray differentials).
    dx: vec3<f32>,
    dy: vec3<f32>,
    // Material tint, roughness, metalness and scale.
    tint: vec3<f32>,
    rough: f32,
    metal: f32,
    scale: f32,
    // Seconds (Lighting::time).
    time: f32,
};

struct SurfaceOut {
    albedo: vec3<f32>,
    normal: vec3<f32>,
    rough: f32,
    metal: f32,
    // Radiance in nits for white, scaled like EMISSIVE.
    emission: vec3<f32>,
};

fn game_surface(s: SurfaceIn) -> SurfaceOut {
    return SurfaceOut(s.tint, s.n, s.rough, s.metal, vec3<f32>(0.0));
}
