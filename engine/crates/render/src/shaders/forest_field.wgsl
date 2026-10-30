// The forest as a function (forest_field.rs, line for line the same; keep
// them together). Needs terrain_field.wgsl before it: where trees grow and
// what the forest looks like from far away.

// Metres between tree spots.
const FF_CELL: f32 = 8.0;
// Linear albedo of a conifer crown seen from afar.
const FF_CROWN: vec3<f32> = vec3<f32>(0.045, 0.085, 0.035);
// Height of a full-size tree's crown top above its foot (metres).
const FF_TREE_HEIGHT: f32 = 15.0;
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
fn forest_at(x: f32, z: f32, seed: u32, st: TerrainStyle, density: f32) -> ForestSample {
    let stand = ff_stand(x, z, seed);
    let s = terrain_eval(x, z, FF_CUT, seed, st);
    let cover = select(0.0, ff_chance(stand, density), ff_grows(s, st));
    let tone = clamp(stand * 0.5 + 0.5, 0.0, 1.0);
    var f: ForestSample;
    f.density = cover;
    f.colour = FF_CROWN * (0.8 + 0.4 * tone);
    f.crown_height = select(0.0, FF_TREE_HEIGHT * (0.85 + 0.3 * tone), cover > 0.0);
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
