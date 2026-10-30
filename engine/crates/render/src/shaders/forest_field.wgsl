// The forest as a function (forest_field.rs, line for line the same; keep
// them together). Needs terrain_field.wgsl before it: where trees grow and
// what the forest looks like from far away.

// Metres between tree spots.
const FF_CELL: f32 = 8.0;
// How a forest looks, the game artist's choice (forest_field.rs ForestStyle):
// needles.rgb sunlit needle albedo, .w per-tree tone variation; floor.rgb
// gap albedo, .w gap occlusion; far.rgb crown colour from afar, .w crown
// top height (m); crown.x radius per unit width (m), .y full stand's crown
// share, .z normal lean (cone steepness).
struct ForestStyle {
    needles: vec4<f32>,
    floor: vec4<f32>,
    far: vec4<f32>,
    crown: vec4<f32>,
};
// The terrain detail cut the forest samples at (forest_field.rs CUT).
const FF_CUT: f32 = 0.05;

struct ForestSample {
    // Linear crown albedo here.
    colour: vec3<f32>,
    // Share of the ground's tree spots that grow a tree, 0..1.
    density: f32,
    // Height of the crowns above the ground (metres), 0 without forest.
    crown_height: f32,
};

struct ForestTree {
    pos: vec3<f32>,
    yaw: f32,
    scale: vec3<f32>,
    variant: u32,
    // 1 when the cell grows a tree.
    present: u32,
};

// A uniform 0..1 number for grid cell (i, j), channel k.
fn ff_hash(i: i32, j: i32, k: u32, seed: u32) -> f32 {
    var h = ((bitcast<u32>(i) * 0x8da6b343u) ^ (bitcast<u32>(j) * 0xd8163841u)) ^ ((seed * 0xcb1ab31fu) ^ (k * 0x165667b1u));
    h = h ^ (h >> 15u);
    h = h * 0x2c1b3c6du;
    h = h ^ (h >> 12u);
    return f32(h & 0xffffffu) / 16777216.0;
}

// Stands and clearings hundreds of metres across, -1..1.
fn ff_stand(x: f32, z: f32, seed: u32) -> f32 {
    return tf_noised(vec2<f32>(x, z) / 300.0, seed + 77u).x;
}

// The chance a tree spot in this stand grows a tree.
fn ff_chance(stand: f32, density: f32) -> f32 {
    return clamp((stand + 0.15) * 2.0, 0.0, 1.0) * density;
}

// Whether ground can carry a tree.
fn ff_grows(s: TerrainSample, st: TerrainStyle) -> bool {
    return !(s.rock > 0.3 || s.scree > 0.3 || s.water > 0.01 || s.snow > 0.05 || s.up < 0.82 || s.pos.y > st.snow_line - 250.0);
}

// The forest at (x, z).
fn forest_at(x: f32, z: f32, seed: u32, st: TerrainStyle, density: f32, fs: ForestStyle) -> ForestSample {
    let stand = ff_stand(x, z, seed);
    let s = terrain_eval(x, z, FF_CUT, seed, st);
    let cover = select(0.0, ff_chance(stand, density), ff_grows(s, st));
    let tone = clamp(stand * 0.5 + 0.5, 0.0, 1.0);
    var f: ForestSample;
    f.density = cover;
    f.colour = fs.far.rgb * (0.8 + 0.4 * tone);
    f.crown_height = select(0.0, fs.far.w * (0.85 + 0.3 * tone), cover > 0.0);
    return f;
}

// The tree of grid cell (i, j); present 0 when it grows none.
fn forest_tree(i: i32, j: i32, seed: u32, st: TerrainStyle, density: f32) -> ForestTree {
    var t: ForestTree;
    let x = (f32(i) + ff_hash(i, j, 1u, seed)) * FF_CELL;
    let z = (f32(j) + ff_hash(i, j, 2u, seed)) * FF_CELL;
    if ff_hash(i, j, 3u, seed) >= ff_chance(ff_stand(x, z, seed), density) { return t; }
    let s = terrain_eval(x, z, FF_CUT, seed, st);
    if !ff_grows(s, st) { return t; }
    let width = 0.75 + 0.5 * ff_hash(i, j, 4u, seed);
    let height = width * (0.9 + 0.25 * ff_hash(i, j, 5u, seed));
    t.pos = vec3<f32>(x, s.pos.y - 0.2, z);
    t.yaw = ff_hash(i, j, 6u, seed) * 6.2831855;
    t.scale = vec3<f32>(width, height, width);
    t.variant = u32(ff_hash(i, j, 7u, seed) * 3.0) % 3u;
    t.present = 1u;
    return t;
}

// The canopy seen from above (forest_field.rs Canopy, canopy).
struct ForestCanopy {
    albedo: vec3<f32>,
    occlusion: f32,
    normal: vec3<f32>,
    cover: f32,
};

// Mean occlusion over a crown (forest_field.rs CROWN_OCCLUSION).
const FF_CROWN_OCCLUSION: f32 = 0.45 + 0.55 * (1.0 / 3.0) * 0.8;

// The spot of cell (i, j) if it grows a tree, without the terrain:
// xy centre, z width, w tint; w < 0 when there is none.
fn ff_spot(i: i32, j: i32, seed: u32, density: f32, variation: f32) -> vec4<f32> {
    let c = vec2<f32>((f32(i) + ff_hash(i, j, 1u, seed)) * FF_CELL, (f32(j) + ff_hash(i, j, 2u, seed)) * FF_CELL);
    if ff_hash(i, j, 3u, seed) >= ff_chance(ff_stand(c.x, c.y, seed), density) { return vec4<f32>(0.0, 0.0, 0.0, -1.0); }
    let width = 0.75 + 0.5 * ff_hash(i, j, 4u, seed);
    return vec4<f32>(c, width, 1.0 + variation * (ff_hash(i, j, 8u, seed) - 0.5));
}

// The phase of cell (i, j)'s star-shaped crown outline.
fn ff_phase(i: i32, j: i32, seed: u32) -> f32 {
    return ff_hash(i, j, 9u, seed) * 6.2831855;
}

// The canopy at (x, z) for a pixel `footprint` metres across, lit from
// `sun` (unit, towards the sun); `ground` false where the terrain carries
// no forest. Crown cones up close, their average once a crown is subpixel.
fn forest_canopy(x: f32, z: f32, footprint: f32, sun: vec3<f32>, seed: u32, density: f32, ground: bool, fs: ForestStyle) -> ForestCanopy {
    let mean_cover = select(0.0, ff_chance(ff_stand(x, z, seed), density) * fs.crown.y, ground);
    var mean: ForestCanopy;
    // The same light as the crowns and gaps it stands for (albedo x occlusion).
    mean.albedo = mix(fs.floor.rgb, fs.needles.rgb, mean_cover);
    let light = fs.needles.y * FF_CROWN_OCCLUSION * mean_cover + fs.floor.y * fs.floor.w * (1.0 - mean_cover);
    mean.normal = vec3<f32>(0.0, 1.0, 0.0);
    mean.occlusion = light / mean.albedo.y;
    mean.cover = mean_cover;
    // Average once a pixel spans a good part of a crown, before it aliases.
    let far = clamp((footprint - FF_CELL * 0.06) / (FF_CELL * 0.3), 0.0, 1.0);
    if far >= 1.0 || !ground { return mean; }
    let ci = i32(floor(x / FF_CELL));
    let cj = i32(floor(z / FF_CELL));
    var best_h = 0.0;
    var best_d = vec2<f32>(0.0);
    var best_width = 0.0;
    var best_tint = 1.0;
    for (var dj = -1; dj <= 1; dj++) {
        for (var di = -1; di <= 1; di++) {
            let s = ff_spot(ci + di, cj + dj, seed, density, fs.needles.w);
            if s.w < 0.0 { continue; }
            let d = vec2<f32>(x, z) - s.xy;
            // A spruce from above is a star of branch tiers, not a disc.
            let radius = fs.crown.x * s.z * (0.82 + 0.18 * sin(7.0 * atan2(d.y, d.x) + ff_phase(ci + di, cj + dj, seed)));
            let h = (1.0 - length(d) / radius) * s.z;
            if h > best_h { best_h = h; best_d = d; best_width = s.z; best_tint = s.w; }
        }
    }
    var near: ForestCanopy;
    if best_h > 0.0 {
        // A steep cone: its normal leans out from the trunk; sun-facing flanks light.
        let out = best_d / max(length(best_d), 1e-4);
        let normal = normalize(vec3<f32>(out.x * fs.crown.z, 1.0, out.y * fs.crown.z));
        let tip = clamp(best_h / best_width, 0.0, 1.0);
        let lit = max(dot(normal, sun), 0.0);
        near.albedo = fs.needles.rgb * best_tint;
        near.normal = normal;
        near.occlusion = 0.45 + 0.55 * tip * (0.6 + 0.4 * lit);
        near.cover = 1.0;
    } else {
        // A gap: forest floor under the neighbours' shade.
        near.albedo = fs.floor.rgb;
        near.normal = vec3<f32>(0.0, 1.0, 0.0);
        near.occlusion = fs.floor.w;
        near.cover = 0.0;
    }
    var c: ForestCanopy;
    c.albedo = mix(near.albedo, mean.albedo, far);
    c.normal = normalize(mix(near.normal, mean.normal, far));
    c.occlusion = near.occlusion + (mean.occlusion - near.occlusion) * far;
    c.cover = near.cover + (mean.cover - near.cover) * far;
    return c;
}
