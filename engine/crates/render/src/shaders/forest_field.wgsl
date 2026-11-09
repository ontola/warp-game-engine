// The forest as a function (forest_field.rs, line for line the same; keep
// them together). Needs terrain_field.wgsl before it: where trees grow and
// what the forest looks like from far away.

// Metres between tree spots.
const FF_CELL: f32 = 8.0;
// How a forest looks, the game artist's choice (forest_field.rs ForestStyle):
// needles.rgb sunlit needle albedo, .w per-tree tone variation; floor.rgb
// gap albedo, .w gap occlusion; far.rgb second canopy's albedo (patches),
// .w crown top height (m); crown.x radius per unit width (m), .y crown
// area per spot over FF_CELL^2, .z normal lean (cone steepness), .w second canopy's share.
struct ForestStyle {
    needles: vec4<f32>,
    floor: vec4<f32>,
    far: vec4<f32>,
    crown: vec4<f32>,
};
// The terrain detail cut the forest samples at (forest_field.rs CUT).
const FF_CUT: f32 = 0.05;
// A site where no forest grows (forest_field.rs NO_FOREST).
const FF_NO_FOREST: f32 = -10.0;

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
fn ff_mix(v: u32) -> u32 {
    var x = v;
    x = x ^ (x >> 16u); x = x * 0x7feb352du; x = x ^ (x >> 15u); x = x * 0x846ca68bu;
    return x ^ (x >> 16u);
}
// forest_field.rs hash: each input mixed in turn.
fn ff_hash(i: i32, j: i32, k: u32, seed: u32) -> f32 {
    let h = ff_mix(ff_mix(ff_mix((seed * 0x9e3779b9u) ^ bitcast<u32>(i)) ^ bitcast<u32>(j)) ^ k);
    return f32(h >> 8u) / 16777216.0;
}

fn ff_smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = clamp((x - a) / (b - a), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// How much of a pattern `scale` metres across a pixel `footprint` metres
// across shows: all of it over four pixels, none under two (its mean, 0).
fn ff_band(scale: f32, footprint: f32) -> f32 {
    return clamp(scale / max(footprint, 1e-3) * 0.5 - 1.0, 0.0, 1.0);
}

// The ground's own leaning towards forest: slopes and ridges carry it,
// flat and damp ground is meadow.
fn ff_site(up: f32, wet: f32, slopes: f32) -> f32 {
    let steep = 1.0 - ff_smooth(0.86, 0.97, up);
    // slopes 1: the valley sides carry the forest, the gentle ground is pasture.
    let slope = mix(-0.7 * steep, 0.1 * steep - 0.5 * ff_smooth(0.96, 0.995, up), slopes);
    return 0.05 + 0.6 * wet - 1.6 * max(wet - 0.75, 0.0) + slope;
}

// Hedgerows: stretches of the edges of 110 by 80 m fields (on a grid
// turned by the seed), about a tree wide, 0..1.
fn ff_hedgerow(x: f32, z: f32, seed: u32) -> f32 {
    let a = ff_hash(0, 0, 10u, seed) * 1.5;
    // Meandering edges, as forest_field.rs.
    let p = vec2<f32>(x, z);
    let wu = 12.0 * tf_noised(p / 90.0 + vec2<f32>(5.3, 2.9), seed + 83u).x;
    let wv = 12.0 * tf_noised(p / 90.0 + vec2<f32>(1.7, 9.1), seed + 84u).x;
    let u = x * cos(a) + z * sin(a) + wu;
    let v = z * cos(a) - x * sin(a) + wv;
    let fu = u / 110.0;
    let fv = v / 80.0;
    let iu = round(fu);
    let iv = round(fv);
    // Distance to the nearest field edge across u and across v (metres), and
    // whether that stretch of edge carries a hedge.
    let du = select(1e3, abs(fu - iu) * 110.0, ff_hash(i32(iu), i32(floor(fv)), 11u, seed) < 0.3);
    let dv = select(1e3, abs(fv - iv) * 80.0, ff_hash(i32(floor(fu)), i32(iv), 12u, seed) < 0.3);
    return clamp(1.0 - min(du, dv) / 4.0, 0.0, 1.0);
}

// Stands and clearings: 300 m stands with ragged edges down to 12 m, on
// the ground's site. Octaves under two footprints are left out.
fn ff_stand(x: f32, z: f32, seed: u32, site: f32, footprint: f32) -> f32 {
    let p = vec2<f32>(x, z);
    var s = tf_noised(p / 300.0, seed + 77u).x + site;
    var b = ff_band(95.0, footprint);
    if b > 0.0 { s += 0.45 * b * tf_noised(p / 95.0 + vec2<f32>(3.7, 1.3), seed + 78u).x; }
    b = ff_band(33.0, footprint);
    if b > 0.0 { s += 0.25 * b * tf_noised(p / 33.0 + vec2<f32>(8.1, 4.9), seed + 79u).x; }
    b = ff_band(12.0, footprint);
    if b > 0.0 { s += 0.14 * b * tf_noised(p / 12.0 + vec2<f32>(2.3, 6.2), seed + 80u).x; }
    return s;
}

// The share of tree spots at (x, z) that grow a tree, before density: 1 in
// a stand, 0 a few metres past its edge, with loose trees and hedgerows in
// the clearings; 0 on an FF_NO_FOREST site. `footprint`: the pixel's size
// in metres (0 for trees).
fn ff_share(x: f32, z: f32, seed: u32, site: f32, footprint: f32) -> f32 {
    if site <= FF_NO_FOREST { return 0.0; }
    let inside = clamp((ff_stand(x, z, seed, site, footprint) + 0.05) * 5.0, 0.0, 1.0);
    let rows = 0.8 * ff_band(16.0, footprint) * ff_hedgerow(x, z, seed);
    return max(max(inside, 0.04), rows);
}

// Whether ground can carry a tree.
fn ff_grows(s: TerrainSample, st: TerrainStyle) -> bool {
    return !(s.rock > 0.3 || s.scree > 0.3 || s.water > 0.01 || s.snow > 0.05 || s.up < 0.82 || s.pos.y > st.snow_line - 250.0);
}

// Where the second canopy grows instead, 0..1.
fn ff_patch(x: f32, z: f32, seed: u32, amount: f32) -> f32 {
    if amount <= 0.0 { return 0.0; }
    let n = tf_noised(vec2<f32>(x, z) / 70.0 + vec2<f32>(1.9, 7.3), seed + 82u).x;
    let t = 0.3 - 0.6 * amount;
    return ff_smooth(t - 0.04, t + 0.04, n);
}

// The forest at (x, z).
fn forest_at(x: f32, z: f32, seed: u32, st: TerrainStyle, density: f32, fs: ForestStyle) -> ForestSample {
    let s = terrain_eval(x, z, FF_CUT, seed, st);
    let site = select(FF_NO_FOREST, ff_site(s.up, s.wet, st.forest_slopes), ff_grows(s, st));
    let cover = ff_share(x, z, seed, site, 0.0) * density;
    let tone = clamp(ff_stand(x, z, seed, max(site, -1.0), 0.0) * 0.5 + 0.5, 0.0, 1.0);
    let colour = mix(fs.needles.rgb, fs.far.rgb, ff_patch(x, z, seed, fs.crown.w));
    var f: ForestSample;
    f.density = cover;
    f.colour = colour * (0.8 + 0.4 * tone);
    f.crown_height = select(0.0, fs.far.w * (0.85 + 0.3 * tone), cover > 0.0);
    return f;
}

// The tree of grid cell (i, j); present 0 when it grows none.
fn forest_tree(i: i32, j: i32, seed: u32, st: TerrainStyle, density: f32) -> ForestTree {
    var t: ForestTree;
    let roll = ff_hash(i, j, 3u, seed);
    if roll >= density { return t; }
    let x = (f32(i) + ff_hash(i, j, 1u, seed)) * FF_CELL;
    let z = (f32(j) + ff_hash(i, j, 2u, seed)) * FF_CELL;
    let s = terrain_eval(x, z, FF_CUT, seed, st);
    if !ff_grows(s, st) { return t; }
    if roll >= ff_share(x, z, seed, ff_site(s.up, s.wet, st.forest_slopes), 0.0) * density { return t; }
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
const FF_CROWN_OCCLUSION: f32 = 0.6 + 0.4 * (1.0 / 3.0) * 0.8;

// Cell (i, j)'s tree in a stand where `chance` of the spots grow one,
// without the terrain: xy centre, z width, w tint; w < 0 when there is none.
fn ff_spot(i: i32, j: i32, seed: u32, chance: f32, variation: f32) -> vec4<f32> {
    if ff_hash(i, j, 3u, seed) >= chance { return vec4<f32>(0.0, 0.0, 0.0, -1.0); }
    let c = vec2<f32>((f32(i) + ff_hash(i, j, 1u, seed)) * FF_CELL, (f32(j) + ff_hash(i, j, 2u, seed)) * FF_CELL);
    let width = 0.75 + 0.5 * ff_hash(i, j, 4u, seed);
    return vec4<f32>(c, width, 1.0 + variation * (ff_hash(i, j, 8u, seed) - 0.5));
}

// The phase of cell (i, j)'s star-shaped crown outline.
fn ff_phase(i: i32, j: i32, seed: u32) -> f32 {
    return ff_hash(i, j, 9u, seed) * 6.2831855;
}

// How much wider a crown looks seen from `eye` (unit, towards the viewer)
// than from straight above: its side, crown length x cot(elevation), adds
// to its top's area (capped where the 3x3 spots end).
fn ff_crown_growth(eye: vec3<f32>, fs: ForestStyle) -> f32 {
    let cot = min(length(eye.xz) / max(eye.y, 1e-3), 4.0);
    return min(sqrt(1.0 + 0.8 * fs.far.w * cot / (3.14159265 * fs.crown.x)), 7.0 / fs.crown.x);
}

// The canopy at (x, z) for a pixel `footprint` metres across, lit from
// `sun` and seen from `eye` (units, towards each), on ground of `site`
// (ff_site, or FF_NO_FOREST). Crown cones up close, as wide as their side
// looks from `eye`, their average once a crown is subpixel, textured by
// clumps and gaps while those span a few pixels.
fn forest_canopy(x: f32, z: f32, footprint: f32, sun: vec3<f32>, eye: vec3<f32>, seed: u32, density: f32, site: f32, fs: ForestStyle) -> ForestCanopy {
    let needles = mix(fs.needles.rgb, fs.far.rgb, ff_patch(x, z, seed, fs.crown.w));
    let crowd = ff_share(x, z, seed, site, footprint) * density;
    let grow = ff_crown_growth(eye, fs);
    // Crowns overlap at random: the share of ground they hide.
    let mean_cover = 1.0 - exp(-crowd * fs.crown.y * grow * grow);
    // Gaps sit in their neighbours' shade only where the stand is dense.
    let gap = 1.0 - (1.0 - fs.floor.w) * min(crowd * 1.25, 1.0);
    var mean: ForestCanopy;
    // The same light as the crowns and gaps it stands for (albedo x occlusion).
    mean.albedo = mix(fs.floor.rgb, needles, mean_cover);
    let light = needles.y * FF_CROWN_OCCLUSION * mean_cover + fs.floor.y * gap * (1.0 - mean_cover);
    // Clumps of crowns and the gaps between them, a few metres high.
    let p = vec2<f32>(x, z);
    let wa = ff_band(23.0, footprint);
    let wb = ff_band(9.0, footprint);
    var a = vec3<f32>(0.0);
    if wa > 0.0 { a = tf_noised(p / 23.0 + vec2<f32>(4.4, 0.7), seed + 83u) * wa; }
    var b = vec3<f32>(0.0);
    if wb > 0.0 { b = tf_noised(p / 9.0 + vec2<f32>(0.3, 5.8), seed + 84u) * wb; }
    let rise = min(crowd, 1.0);
    let slope = (a.yz * (3.0 / 23.0) + b.yz * (1.5 / 9.0)) * rise;
    let clump = max(1.0 + 0.6 * (a.x + 0.5 * b.x) * rise, 0.2);
    mean.normal = normalize(vec3<f32>(-slope.x, 1.0, -slope.y));
    mean.occlusion = light / mean.albedo.y * clump;
    mean.cover = mean_cover;
    // Average once a pixel spans a good part of a crown, before it aliases.
    let far = clamp((footprint - FF_CELL * 0.08) / (FF_CELL * 0.25), 0.0, 1.0);
    if far >= 1.0 || crowd <= 0.0 { return mean; }
    let ci = i32(floor(x / FF_CELL));
    let cj = i32(floor(z / FF_CELL));
    var best_h = 0.0;
    var best_d = vec2<f32>(0.0);
    var best_width = 0.0;
    var best_tint = 1.0;
    for (var dj = -1; dj <= 1; dj++) {
        for (var di = -1; di <= 1; di++) {
            let s = ff_spot(ci + di, cj + dj, seed, crowd, fs.needles.w);
            if s.w < 0.0 { continue; }
            let d = p - s.xy;
            // A spruce from above is a star of branch tiers, not a disc.
            let radius = fs.crown.x * grow * s.z * (0.82 + 0.18 * sin(7.0 * atan2(d.y, d.x) + ff_phase(ci + di, cj + dj, seed)));
            let h = (1.0 - length(d) / radius) * s.z * grow;
            if h > best_h { best_h = h; best_d = d; best_width = s.z; best_tint = s.w; }
        }
    }
    var near: ForestCanopy;
    if best_h > 0.0 {
        // A steep cone: its normal leans out from the trunk; sun-facing flanks light.
        let out = best_d / max(length(best_d), 1e-4);
        let normal = normalize(vec3<f32>(out.x * fs.crown.z, 1.0, out.y * fs.crown.z));
        let tip = clamp(best_h / (best_width * grow), 0.0, 1.0);
        let lit = max(dot(normal, sun), 0.0);
        near.albedo = needles * best_tint;
        near.normal = normal;
        near.occlusion = 0.6 + 0.4 * tip * (0.6 + 0.4 * lit);
        near.cover = 1.0;
    } else {
        // A gap: forest floor under the neighbours' shade.
        near.albedo = fs.floor.rgb;
        near.normal = vec3<f32>(0.0, 1.0, 0.0);
        near.occlusion = gap;
        near.cover = 0.0;
    }
    var c: ForestCanopy;
    c.albedo = mix(near.albedo, mean.albedo, far);
    c.normal = normalize(mix(near.normal, mean.normal, far));
    c.occlusion = near.occlusion + (mean.occlusion - near.occlusion) * far;
    c.cover = near.cover + (mean.cover - near.cover) * far;
    return c;
}
