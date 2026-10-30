// A grass field seen past its blades: the ground's albedo and normal shaded
// as a canopy, so the meadow can stop drawing blades a few tens of metres out
// and the field still reads as grass to the horizon. Shared by every terrain
// material (Sideways' stage terrain and the procedural field terrain).
//
// Unlike meadow_field.wgsl this depends on the view and the sun: a canopy
// is brighter seen edge-on and back to the sun, darker from above.

struct MeadowCanopy { albedo: vec3<f32>, normal: vec3<f32> };

// `ground`: the albedo where the canopy stands; `n`: the terrain normal;
// `dir`: eye to point; `footprint`: metres per pixel; `seed`: the field's.
// `weight` 0..1 is how much of the ground is grass.
fn meadow_canopy(ground: vec3<f32>, nn: vec3<f32>, n: vec3<f32>, dir: vec3<f32>, xz: vec2<f32>,
        footprint: f32, time: f32, seed: u32, weight: f32) -> MeadowCanopy {
    if weight <= 0.0 { return MeadowCanopy(ground, nn); }
    let v = -dir;
    let ndv = clamp(dot(n, v), 0.0, 1.0);
    let sun = light.sun_dir.xyz;
    // From above you look into the gaps and blade shadows; at a glancing
    // angle you see blade sides only, greener and more even.
    let side = pow(1.0 - ndv, 2.0);
    var g = ground * mix(0.72, 0.98, side);
    // Back to the sun the blades hide their own shadows (the bright hotspot
    // of a field); facing it the rims glow yellow-green.
    let back = pow(clamp(dot(v, sun), 0.0, 1.0), 4.0);
    let front = pow(clamp(dot(dir, sun), 0.0, 1.0), 6.0) * side;
    g *= 1.0 + 0.25 * back;
    g = mix(g, g * vec3<f32>(1.15, 1.25, 0.8), front * 0.6);
    // Clumps a metre across while a pixel can still show them, and gusts
    // rolling over the field.
    let clump_w = 1.0 - smoothstep(0.3, 1.2, footprint);
    let clump = meadow_noise(xz / 1.1 + vec2<f32>(23.0, 5.0), seed) - 0.5;
    let tuft = meadow_noise(xz / 4.5 + vec2<f32>(-9.0, 14.0), seed + 3u) - 0.5;
    let gust = meadow_noise(xz / 23.0 + vec2<f32>(time * 0.19, time * 0.07), seed + 7u) - 0.5;
    g *= (1.0 + 0.35 * clump * clump_w) * (1.0 + 0.3 * tuft) * (1.0 + gust * 0.18 * side);
    // A canopy faces the sky more than the ground under it does.
    let up = normalize(mix(nn, vec3<f32>(0.0, 1.0, 0.0), 0.2));
    return MeadowCanopy(mix(ground, g, weight), normalize(mix(nn, up, weight)));
}
