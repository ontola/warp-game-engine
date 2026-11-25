fn material_kind(m: Material) -> i32 {
    return select(-1, -2 - source_layer(m.layer), (m.flags & 32768u) != 0u && (m.flags & 2u) == 0u);
}
fn styled_light(kind: i32, flags: u32, p: vec3<f32>, n: vec3<f32>, base: vec3<f32>, rad: vec3<f32>, shadow: f32) -> StyledLight {
    return game_lighting(LightingSample(kind, flags, p, n, base, rad, light.sun_dir.xyz, light.sun_dir.w, shadow, light.anim.x));
}
// Untagged sky and ordinary passes already write alpha 1. Half floats
// represent integers through 2048 exactly. The finish restores reflection coverage.
fn style_alpha(tag: u32) -> f32 {
    return select(1.0, 1.0 + f32(min(tag, 2047u)), WRITE_STYLE_TAGS);
}
// Deferred shading from the visibility buffer: per pixel, find the
// triangle, intersect the view ray with it for exact barycentrics, and
// light the surface in physical units (lux, cd/m2). Output is linear HDR.

struct Cluster {
    cull: vec4<f32>,
    parent: vec4<f32>,
    own: vec4<f32>,
    parent_error: f32,
    own_error: f32,
    first_tri: u32,
    tri_count: u32,
    first_vert: u32,
    material: u32,
    depth: u32,
    pad: u32,
};

struct Instance {
    r0: vec4<f32>,
    r1: vec4<f32>,
    r2: vec4<f32>,
    sphere: vec4<f32>,
    first_cluster: u32,
    cluster_count: u32,
    scale: f32,
    flags: u32,
};

struct Material {
    tint: vec3<f32>,
    layer: i32,
    roughness: f32,
    flags: u32,
    scale: f32,
    metal: f32,
};

// Cluster geometry in one buffer (topology.rs): vertices from word 0, then
// cluster vertex indices and triangles from scene table vec4 1.z and 1.w.
@group(0) @binding(0) var<storage, read> topology: array<u32>;
fn vertex_f32(i: u32) -> f32 { return bitcast<f32>(topology[i]); }
fn cluster_vert(i: u32) -> u32 { return topology[scene_tables[1].z + i]; }
fn tri_word(i: u32) -> u32 { return topology[scene_tables[1].w + i]; }
@group(0) @binding(3) var<storage, read> clusters: array<Cluster>;
@group(0) @binding(4) var<storage, read> instances: array<Instance>;

// Address/deformation composition is shared in virtual_member.wgsl.
fn instance_at(id: u32) -> Instance {
    if (id & VIRTUAL_MEMBER) == 0u { return instances[id]; }
    let owner = instances[member_owner(id)];
    return compose_member(owner, instances[member_template(owner,id)]);
}
// Read-only scene tables in one buffer (scene_tables.rs): vec4 0 holds where
// the materials, car materials and solid occluders start (in vec4s), vec4 1.x
// where the aux words start.
@group(0) @binding(5) var<storage, read> scene_tables: array<vec4<u32>>;
fn material(i: u32) -> Material {
    let at = scene_tables[0].x + 2u * i;
    let a = scene_tables[at];
    let b = scene_tables[at + 1u];
    return Material(bitcast<vec3<f32>>(a.xyz), bitcast<i32>(a.w), bitcast<f32>(b.x), b.y, bitcast<f32>(b.z), bitcast<f32>(b.w));
}
@group(0) @binding(6) var albedo: texture_2d_array<f32>;
@group(0) @binding(7) var samp: sampler;
// Scene aux words (Scene::aux) live after the tables; vec4 1.x is their start,
// 1.y their count.
fn aux_word(i: u32) -> u32 {
    let v = scene_tables[scene_tables[1].x + (i >> 2u)];
    return v[i & 3u];
}
fn aux_len() -> u32 { return scene_tables[1].y; }
@group(0) @binding(9) var normal_tex: texture_2d_array<f32>;

// Material extension in the scene tables from vec4 2.w (material_maps.rs):
// a header (layer maps at, material layers at, their counts), per texture
// layer x: 1 + its material map layer in normal_tex (0: none), y/z: least
// resident albedo/normal LOD while textures stream; per material x: wet,
// snow, moss and porosity as bytes.
fn layer_map(layer: i32) -> vec4<u32> {
    let h = scene_tables[scene_tables[2].w];
    if layer < 0 || u32(layer) >= h.z { return vec4<u32>(0u); }
    return scene_tables[h.x + u32(layer)];
}
fn albedo_lod_floor(layer: i32) -> f32 { return bitcast<f32>(layer_map(layer).y); }
fn normal_lod_floor(layer: i32) -> f32 { return bitcast<f32>(layer_map(layer).z); }
// Scale for texture gradients so a sample never reads below LOD `floor`
// (mips that have not streamed in): measured on the shorter gradient,
// which bounds the LOD anisotropic filtering picks from below.
fn lod_floor_scale(gx: vec2<f32>, gy: vec2<f32>, size: f32, floor: f32) -> f32 {
    if floor <= 0.0 { return 1.0; }
    let minor = max(min(length(gx), length(gy)) * size, 1e-8);
    return exp2(max(floor - log2(minor), 0.0));
}
fn albedo_grad(layer: i32, gx: vec2<f32>, gy: vec2<f32>) -> f32 {
    return lod_floor_scale(gx, gy, f32(textureDimensions(albedo).x), albedo_lod_floor(layer));
}
fn material_weather(i: u32) -> vec4<f32> {
    let h = scene_tables[scene_tables[2].w];
    if i >= h.w { return vec4<f32>(1.0, 1.0, 0.0, 0.5); }
    return unpack4x8unorm(scene_tables[h.y + i].x);
}
// The same for a normal_tex layer of texture layer `layer` (its normal map
// or its material map).
fn normal_grad(layer: i32, gx: vec2<f32>, gy: vec2<f32>) -> f32 {
    return lod_floor_scale(gx, gy, f32(textureDimensions(normal_tex).x), normal_lod_floor(layer));
}
@group(0) @binding(10) var plant_samp: sampler;
// Sparse persistent tyre material data: no geometry, emission or extra lights.
@group(0) @binding(20) var track_atlas: texture_2d<f32>;
@group(0) @binding(21) var track_lookup: texture_2d<i32>;
// One source-geometry sun map shared by every view. Unlike a density field,
// this preserves the open sky immediately above an exposed tree crown.
struct FarSun {
    view_proj: mat4x4<f32>,
    params: vec4<f32>, // world texel, edge blend start, enabled, reserved
};
@group(0) @binding(22) var<uniform> far_sun: FarSun;
@group(0) @binding(23) var far_sun_tex: texture_depth_2d_array;
// Only a material approximation beneath/beyond the actual directional grass
// aggregate, plus opt-in analytic ground-contact AO. Neither is a cast shadow.
struct MeadowSubstrate {
    control: vec4<u32>, // albedo enabled, seed, f32 top-coverage, f32 AO strength
    leaf_base: vec4<f32>, // source linear RGB (height tint included), roughness
    field_bounds: vec4<f32>, // minXZ/maxXZ
    contact: vec4<f32>, // live density, basal radius, fixed anchor, blade reach
};
@group(0) @binding(27) var<uniform> meadow_substrate: MeadowSubstrate;
// Scene::noise, a tiling scalar field for procedural materials (1x1 0.5 without one).
@group(0) @binding(28) var noise_tex: texture_2d<f32>;
// Four small camera-local caches of source-accepted basal occupancy. This is
// a bounded contact model, not a per-blade or long-range shadow map.
@group(0) @binding(29) var<storage,read> meadow_edge_field:array<vec4<f32>>;

@group(1) @binding(0) var<uniform> view: View;
@group(1) @binding(1) var<storage, read> visible: array<vec2<u32>>;
@group(1) @binding(2) var<uniform> light: Light;
@group(1) @binding(3) var vis_tex: texture_2d<u32>;
@group(1) @binding(4) var depth_tex: texture_depth_2d;
@group(1) @binding(5) var shadow_tex: texture_depth_2d;
@group(1) @binding(6) var shadow_samp: sampler_comparison;
@group(1) @binding(7) var sky_tex: texture_2d<f32>;
@group(1) @binding(8) var out_hdr: texture_storage_2d<rgba16float, write>;
@group(1) @binding(10) var heightfield: texture_2d<f32>;
@group(1) @binding(11) var<uniform> gi: Gi;
struct SolidOccluder {
    center: vec4<f32>,
    half_extent: vec4<f32>,
    identity: vec4<u32>,
};
fn solid_occluder(i: u32) -> SolidOccluder {
    let at = scene_tables[0].z + 3u * i;
    return SolidOccluder(bitcast<vec4<f32>>(scene_tables[at]), bitcast<vec4<f32>>(scene_tables[at + 1u]), scene_tables[at + 2u]);
}
// The world terrain hierarchy (terrain_trace.wgsl) and the foliage sun page
// table (foliage_sun_pages.wgsl) are copied into the scene tables; vec4 2.x
// and 2.y are where (scene_tables.rs).
fn terrain_world() -> TerrainWorld {
    let b = scene_tables[2].x;
    return TerrainWorld(bitcast<vec4<f32>>(scene_tables[b]), scene_tables[b + 1u]);
}
fn terrain_level(i: u32) -> vec4<u32> { return scene_tables[scene_tables[2].x + 2u + i]; }
fn terrain_info() -> vec4<u32> { return scene_tables[scene_tables[2].x + 18u]; }
fn terrain_bounds(i: u32) -> vec2<f32> {
    let v = scene_tables[scene_tables[2].x + 19u + (i >> 1u)];
    return bitcast<vec2<f32>>(select(v.xy, v.zw, (i & 1u) == 1u));
}
fn sun_pages_world_to_light() -> mat4x4<f32> {
    let b = scene_tables[2].y;
    return mat4x4<f32>(bitcast<vec4<f32>>(scene_tables[b]), bitcast<vec4<f32>>(scene_tables[b + 1u]),
        bitcast<vec4<f32>>(scene_tables[b + 2u]), bitcast<vec4<f32>>(scene_tables[b + 3u]));
}
fn sun_pages_geometry() -> vec4<f32> { return bitcast<vec4<f32>>(scene_tables[scene_tables[2].y + 4u]); }
fn sun_pages_counts() -> vec4<u32> { return scene_tables[scene_tables[2].y + 5u]; }
fn sun_page_entry(i: u32) -> vec4<i32> { return bitcast<vec4<i32>>(scene_tables[scene_tables[2].y + 6u + i]); }
@group(1) @binding(13) var canopy_tex: texture_3d<f32>;
@group(1) @binding(14) var canopy_sampler: sampler;
@group(1) @binding(15) var near_shadow_tex: texture_depth_2d_array;
// The shade pass's one read-write buffer (shade_scratch.rs). The texture
// binding budget is already full, so certified meadow source IDs are copied
// into `words` (two words per pixel, rows of row_pairs) and the opaque
// lighting pass reads one owner word per pixel. Sky tiles: `classify` lists
// the 8x8 tiles that need shading from words[tiles_at]; tiles_x/y/z are the
// indirect dispatch.
struct ShadeScratch {
    skipped:atomic<u32>, width:u32, height:u32, row_pairs:u32,
    tiles_x:atomic<u32>, tiles_y:u32, tiles_z:u32, tiles_at:u32,
    padding:array<u32,56>, words:array<u32>,
};
@group(1) @binding(16) var<storage,read_write> shade_scratch:ShadeScratch;
// Scene features, specialised per pipeline. All default to true: every
// shader that includes this file behaves as before. The main shade pipeline
// sets a feature false when its scene cannot use it (Renderer::new), so a
// game without cars, grass, tracks or forests does not compile their code.
override FEATURE_CARS: bool = true;
// Available to game_surface on desktop and compact WebGPU renderers.
override SURFACE_LOW_DETAIL: bool = false;
// Procedural rock and lava surfaces (procedural_surface.wgsl).
override FEATURE_PROCEDURAL: bool = true;
override FEATURE_GRASS: bool = true;
override FEATURE_GRASS_SHADE: bool = true;
override FEATURE_TRACKS: bool = true;
override FEATURE_WORLD_TERRAIN: bool = true;
override FEATURE_FOLIAGE_PAGES: bool = true;
override FEATURE_MEADOW: bool = true;
override FEATURE_TERRAIN: bool = true;
override FEATURE_FOLIAGE: bool = true;
// Sun that reaches a source needle through its own brush, on average. The sun
// maps resolve a voxel crown cell, not the needles of one brush, so source
// brushes went unshaded and read ~1.55x brighter than the same crown as
// voxels, which carry (1-exp(-tau))/tau per cell (Joep, 8 November: near
// crowns pale, darker as trees switch to voxels).
override NEEDLE_BRUSH_OWN: f32 = 0.42;
// Spot and local lights (Renderer::set_headlights, set_local_lights). A frame
// with none uses a pipeline without their loop: fewer registers per pixel.
override FEATURE_LOCAL_LIGHTS: bool = true;
// The indoor irradiance volume. Without one gi.params.w is 0 and the lookup
// returns at once, but its code still costs registers (0.37 ms of land
// shading at the hill view on an M5 Pro).
override FEATURE_IRRADIANCE: bool = true;
// Procedural terrain with region ground colours, and with roads drawn into
// its pixels (shade_features.rs). Without them both lookups return at once,
// but cost registers: about 0.3-0.5 and 0.2 ms of land shading on an M5 Pro.
override FEATURE_FIELD_REGIONS: bool = true;
override FEATURE_FIELD_ROADS: bool = true;
// SKIN and HAIR materials (scene.rs, skin_hair below). Off unless a scene
// has one, so other scenes compile none of it.
override FEATURE_SKIN_HAIR: bool = true;
override SKIP_HIDDEN_MEADOW:bool=false;
var<workgroup> hidden_meadow_count:atomic<u32>;
// Sky tiles: `classify` writes the sky of 8x8 tiles without geometry and
// lists the others; `shade` then runs (indirectly) over listed tiles only,
// so pure sky never occupies the heavy shading kernel.
override SKY_TILES: bool = false;
// Rasterized voxel trees (voxel_forest.rs Stage::Splats): their depth,
// reversed Z, 0 where there is none. A pixel they cover in front of the
// opaque surface is theirs; its composite replaces whatever shade wrote.
override VOXEL_OCCLUSION: bool = false;
@group(1) @binding(18) var voxel_depth: texture_depth_2d;
// Rasterized voxel trees in the scene's own visibility buffer (bit 31 of the
// id, voxel_forest Stage::Splats): their surface, packed by
// voxel_splat_gbuffer, lit here like the leaves they stand for.
override VOXEL_SURFACE: bool = false;
// Voxel pixels in their own small pass (shade_voxels) instead of inside
// the big shade kernel, whose register footprint they would share.
override VOXEL_PASS: bool = false;
// Which pixels this shade pipeline lights (frame.rs): 0 all, 1 only terrain
// (TERRAIN materials, procedural or not: a lean specialisation without cars,
// foliage, grass lighting or procedural rock), 2 everything else (both
// terrain paths compiled out).
// Like the voxel pass: the kernel's size, not its work, set the pace.
override SHADE_CLASS: u32 = 0u;
override SHADE_DIAGNOSTICS: bool = true;
// KERN_SHADE_PROBE=N (frame.rs): the simple/compact pass stops after stage N
// and stores what it has, to time each stage on a device. 1 the visibility
// and depth reads, 2 + triangle and normal, 3 + material, surfaces and sun,
// 4 + albedo texture and ambient/GI, 5 + local lights, 6 + the game's light
// styling. 0 the whole pass (6 + flash, glow and fog).
override SHADE_PROBE: u32 = 0u;
// KERN_PHONE_GI=1 (the Android phone host's default): the irradiance volume
// blends its eight probes by distance and facing only, without the DDGI
// reach masks, probe visibility and gradients. 7.6 of the compact pass's
// 20.6 ns a pixel on an Adreno 740 were texture plus this lookup. Light can
// leak through a thin wall where a probe sits behind it.
override GI_PHONE: bool = false;
// Voxel leaves light like the needle brushes they stand for (KERN_VOXEL_LEAF_PARITY,
// 1 default, 0 = before): the brushes' own sub-cell shade (NEEDLE_BRUSH_OWN) on sun
// and sky, and sky visibility read at the surface as for a mesh needle.
override VOXEL_LEAF_PARITY: f32 = 1.0;
const VOXEL_SKY_LIFT_OLD: f32 = 1.0;
@group(1) @binding(20) var voxel_surface: texture_2d<u32>;

// A voxel pixel: the same sun, sky, contact and fog terms as a procedural
// leaf in `shade`, from the voxel's albedo, picked normal lobe and leaf
// fraction instead of a triangle's.
// How deep a point lies inside the canopy along the sun, from the first
// caster a sun map holds above it: metres between that depth and the
// point's own (the map is orthographic, so depth is linear along the sun).
// Branch voxel volumes carry no crown of their own (only whole-plant
// proxies march one), so at 25-200 m crowns lit as if every leaf faced the
// sun: ~2x the same tree as a mesh (Mancave tree series, 6 November).
// Off: wherever this estimate applied, a sun map (clipmap or shared) holds
// the crown's own voxels too and already shades it from inside, so the two
// counted its depth twice: a dark band of trees between the clipmap and the
// shared map's edge (Joep playtest 5; Mancave band spot 0.62 of far trees at
// 0.35). Mesh and voxel crowns now take one lighting source, the maps.
const CROWN_EXTINCTION_PER_M: f32 = 0.0;
fn crown_depth_in(map: texture_depth_2d, projection: mat4x4<f32>, p: vec3<f32>) -> f32 {
    let c = projection * vec4<f32>(p, 1.0);
    let uv = vec2<f32>(c.x * 0.5 + 0.5, 0.5 - c.y * 0.5);
    if any(uv <= vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0)) || c.z < 0.0 || c.z > 1.0 { return -1.0; }
    let rate = abs(dot(vec3<f32>(projection[0].z, projection[1].z, projection[2].z), light.sun_dir.xyz));
    if rate <= 1e-9 { return -1.0; }
    let size = vec2<f32>(textureDimensions(map));
    // A 3x3 grid of taps 0.6 m apart, bilinear in between: the casters are
    // voxel cells half a metre wide, and one texel per point printed them
    // as squares across the crown that flickered as the camera moved
    // (Joep, playtest 3).
    let texel_m = 2.0 / max(length(vec3<f32>(projection[0].x, projection[1].x, projection[2].x)) * size.x, 1e-9);
    let step = 0.6 / texel_m;
    let f = uv * size - 0.5;
    let base = floor(f);
    let frac = f - base;
    var sum = 0.0;
    var weight = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let w = (2.0 - abs(f32(x))) * (2.0 - abs(f32(y)));
            let o = vec2<f32>(f32(x), f32(y)) * step;
            let t = clamp(base + o, vec2<f32>(0.0), size - 2.0);
            let i = vec2<i32>(t);
            let d00 = textureLoad(map, i, 0);
            let d10 = textureLoad(map, i + vec2<i32>(1, 0), 0);
            let d01 = textureLoad(map, i + vec2<i32>(0, 1), 0);
            let d11 = textureLoad(map, i + vec2<i32>(1, 1), 0);
            let d = mix(mix(d00, d10, frac.x), mix(d01, d11, frac.x), frac.y);
            sum += w * abs(c.z - d) / rate;
            weight += w;
        }
    }
    return sum / weight;
}
// Metres below the first caster along the sun (0 where no map covers p).
// The shared map only: switching to the near map at a radius round the
// camera made a crown's darkness jump as you walked.
fn crown_depth(p: vec3<f32>) -> f32 {
    let depth = crown_depth_in(shadow_tex, light.shadow_view_proj, p);
    return max(depth - 0.25, 0.0);
}

fn shade_voxel(px: vec2<i32>, size: vec2<u32>, dir: vec3<f32>, depth: f32) {
    let ndc = vec2<f32>((f32(px.x) + 0.5) / view.size.x * 2.0 - 1.0, 1.0 - (f32(px.y) + 0.5) / view.size.y * 2.0);
    let hit = view.inv_view_proj * vec4<f32>(ndc, depth, 1.0);
    let p = hit.xyz / hit.w;
    let dist = distance(p, view.cam.xyz);
    // voxel_splat_gbuffer: albedo, roughness and leaf fraction (unorm8), the
    // normal, the tree's own sun transmittance (unorm8) and where the sun ray
    // leaves its box (quarter metres).
    let g = textureLoad(voxel_surface, px, 0);
    let base = vec3<f32>(unpack2x16float(g.x), unpack2x16float(g.y & 0xffffu).x);
    var n = normalize(vec3<f32>(unpack2x16float(g.z), unpack2x16float(g.w & 0xffffu).x));
    let rough = clamp(f32((g.y >> 16u) & 255u) / 255.0, 0.04, 1.0);
    let leaf = f32(g.y >> 24u) / 255.0;
    // A proxy's own march, or for branch volumes (own 1) the canopy depth
    // the sun maps see above this point; the smaller of the two. The sky
    // term counts at most 1.5 m of it: grass under a tree is metres below
    // the crown, and lost its sky fill too (shadows went nearly black).
    let marched = f32((g.w >> 16u) & 255u) / 255.0;
    let canopy = select(crown_depth(p), 0.0, (g.w >> 24u) == 255u);
    // Inside the sun clipmap the maps hold every caster (mesh leaves and
    // voxel cells) at a fine texel, so the crown shades itself through
    // shadow_at_surface exactly as a mesh tree does: one lighting source at
    // every distance. The canopy estimate only covers the gap beyond it.
    let estimate = 1.0 - sun_clipmap_cover(p);
    let own = min(marched, exp(-CROWN_EXTINCTION_PER_M * canopy * estimate));
    let own_sky = min(marched, exp(-CROWN_EXTINCTION_PER_M * min(canopy, 1.5) * estimate));
    let exit = f32(g.w >> 24u) / 4.0;
    let l = light.sun_dir.xyz;
    let v = -dir;
    // Other casters from the sun maps past this tree, its own crown from
    // its cells: the maps hold it too, at their texel, and lit its shadow
    // side through their bias.
    // A whole plant's maps hold it at a coarse level with a texel of bias:
    // they shade it too little on their own, its cells too little alone.
    // Past the shared sun map a crown's light is its own cells' business:
    // the coarse canopy march that would stand in for its neighbours cost a
    // quarter of the frame at forest views (4 ms of 37 at the playtest's
    // camera 4). Hills still shade it.
    var others = 1.0;
    // A camera-anchored far map (far_sun.rs, streamed forests) holds the
    // neighbours past the shared map too: crowns keep the maps' pairing.
    let anchored = far_sun.params.z > 0.5 && far_sun.params.w < 0.5;
    if sun_coverage_fade(light.shadow_view_proj, p, 0.45) < 1.0 || anchored {
        // Lift the receiver toward the sun by most of a sun-map voxel cell:
        // the pixel lies on its own cell's splat, which the map holds at
        // the same depth, so every crown half-shadowed itself. With cells
        // growing per clipmap level that darkened mid-distance trees most
        // (Joep, playtest 5).
        others = shadow_at_surface(p + l * 0.75 * sun_voxel_cell(p), n * sign(dot(n, l) + 1e-4), vec3<f32>(0.0), 0.0);
    } else if FEATURE_WORLD_TERRAIN && sun_visibility_required() {
        others = world_terrain_sun_visibility(p, n, 0.0);
    } else if far_sun.params.z > 0.5 && far_sun.params.w < 0.5 && sun_coverage_fade(far_sun.view_proj, p, far_sun.params.y) < 1.0 {
        // Past the shared map the ground reads the far map; the crowns on it
        // did not, and caught full sun on a mountain's shadow side, ~3x the
        // ground beneath them (Mancave sky test, Joep playtest 5). One
        // compare tap, lifted a few metres up the ray past the crown's own
        // coarse copy; no march.
        let c = far_sun.view_proj * vec4<f32>(p + l * 3.0, 1.0);
        others = textureSampleCompareLevel(far_sun_tex, shadow_samp, c.xy * vec2<f32>(0.5, -0.5) + 0.5, 0, c.z);
    }
    // Out there its neighbours' shade is folded into its own: the plant's
    // full transmittance instead of the square root the maps pair with.
    var outside = sun_coverage_fade(light.shadow_view_proj, p, 0.45);
    if anchored {outside *= sun_coverage_fade(far_sun.view_proj, p, far_sun.params.y);}
    // Inside the maps the crown's own transmittance counts in full: with
    // sqrt(own) (the maps were meant to supply the rest, but at 1 m cells and
    // a texel of bias they barely shade a crown) voxel crowns caught ~1.4x
    // the sun of the same tree as a mesh (Mancave tree series, 6 November).
    // Where the sun clipmap covers the cell it already holds this tree's own
    // cells and leaves: its own march on top shaded it twice, and a voxel
    // trunk read 1.64x darker than the same trunk as a mesh (Joep, 9
    // November). VOXEL_OWN_IN_CLIPMAP keeps that share of the march (1 = old).
    let own_sun = mix(own, 1.0, sun_clipmap_cover(p) * (1.0 - VOXEL_OWN_IN_CLIPMAP));
    var shadow = mix(1.0, others * mix(own_sun, own_sun * own_sun, outside), light.sun_color.w);
    // A cell's leaves are needle brushes: their own needles shade each other below
    // the cell size, as NEEDLE_BRUSH_OWN does for the mesh (outside the shadow
    // strength, like the mesh). Bark (leaf 0) keeps full sun.
    let brush_own = mix(1.0, NEEDLE_BRUSH_OWN, leaf * VOXEL_LEAF_PARITY);
    shadow *= brush_own;
    shadow *= cloud_sun_transmittance(p, l, light.clouds, light.cloud_motion);
    let sun_e = light.sun_color.rgb * light.sun_dir.w * shadow;
    // Sky reaches a cell through its own crown too. sky_visibility only sees
    // what lies past the lift, so inner cells lit as if under open sky and
    // a voxel crown read ~3.5x brighter than the same tree as a mesh (Joep,
    // 6 November). The crown's sun transmittance stands in for the sky's.
    let crown_sky = 0.25 + 0.75 * sqrt(min(own_sky, brush_own));
    let sky_lift = VOXEL_SKY_LIFT_OLD * (1.0 - VOXEL_LEAF_PARITY);
    let sky_vis = sky_visibility(p + n * sky_lift, n) * crown_sky;
    let back_sky_vis = sky_visibility(p - n * sky_lift, -n) * crown_sky;
    let refl = reflect(dir, n);
    // No screen-space AO: a voxel's blocky depth darkened its own crown to
    // 60 % of the leaves' sky light (forest_ladder, sun_lux=0).
    let contact = contact_visibility(p + n * 0.025, n, refl, 0xffffffffu);
    let bounce = gi.bounce.rgb * (gi.bounce.w * sky_vis + light.sun_dir.w * max(light.sun_dir.y, 0.0) * shadow);
    let front = (sky_irradiance(n) * sky_vis + bounce * (1.0 - sky_vis)) * contact.x;
    let back = (sky_irradiance(-n) * back_sky_vis + bounce * (1.0 - back_sky_vis)) * contact.x;
    let spec_contact = mix(contact.y, contact.x, smoothstep(0.08, 0.6, rough));
    let env = mix(bounce / PI, environment(refl, rough, bounce), sky_vis) * spec_contact;
    let response = thin_foliage_response(base, n, v, rough, leaf, sun_e, front, back, env);
    var rad = response.direct + response.indirect + response.specular;
    let style = styled_light(-1, 512u, p, n, base, rad, shadow);
    rad = apply_distance_fog(style.radiance, dist, dir);
    if light.sky.z > 1.5 && light.sky.z < 2.5 { rad = base * 6000.0; }
    textureStore(out_hdr, px, vec4<f32>(min(rad, vec3<f32>(65504.0)), style_alpha(style.tag)));
}
// Coarse terrain material (TERRAIN_COARSE): field_surface once per 2x2
// pixels, at each quad's centre, packed by terrain_coarse below. Where a
// pixel spans more than COARSE_FOOTPRINT_M of ground, shade interpolates it
// instead of evaluating the material itself; lighting stays per pixel.
// 8 cm: from about 100 m at 1440p, also straight down from 280 m (a 0.3 m
// threshold never triggered there). A/B at the playtest cameras: PSNR 44-46
// dB, under 0.01 % of pixels off by more than 3 %.
override TERRAIN_COARSE: bool = false;
// KERN_COARSE_FOOTPRINT_M trades ground detail for frame time (A/B only).
override COARSE_FOOTPRINT_M: f32 = 0.08;
@group(1) @binding(19) var terrain_coarse_tex: texture_2d<u32>;
// Group 3, beside the bakes' outputs: shade.wgsl is shared, and group 2
// belongs to the passes that include it.
@group(3) @binding(32) var terrain_coarse_out: texture_storage_2d<rgba32uint, write>;
var<workgroup> tile_has_geometry: atomic<u32>;

struct Gi {
    vol_origins: array<vec4<f32>, 3>,
    vol_anchors: array<vec4<f32>, 3>,
    vol_dims: vec4<u32>,
    hf: vec4<f32>,
    hf_dims: vec4<u32>,
    bounce: vec4<f32>,
    params: vec4<f32>, // contact on, reflection on, sun-free sky cap, irradiance volume aux offset (0: none)
    // Traced reflections (dynamic-gi.md): last frame's unjittered camera,
    // and x: on (a valid history this frame), y: roughness cut, z: reach (m).
    prev_view_proj: mat4x4<f32>,
    trace: vec4<f32>,
    // Reflective shadow maps (rsm.wgsl): x the surface voxels' aux offset (0:
    // none), y 0 off, 1 on, 2 on with last frame's gather, z the frame, w
    // texels per light and pixel; and last frame's view-projection.
    rsm: vec4<f32>,
    rsm_prev: mat4x4<f32>,
    // Reflection probes (reflection_probes.rs): x 1 inside a probe's
    // capture, y the box count, z 1 when they reflect; per box capture (w 1
    // once complete), min, max.
    probes: vec4<f32>,
    probe_boxes: array<vec4<f32>, 24>,
    // Froxel fog (froxel_fog.wgsl): density, anisotropy, far, 0 off / 1 on /
    // 2 with last frame's cells; albedo and the frame; last frame's camera.
    fog: vec4<f32>,
    fog_albedo: vec4<f32>,
    fog_prev: mat4x4<f32>,
    // Where the medium is: min (w 1 a box, 2 the probes' room boxes), max.
    fog_bounds: array<vec4<f32>, 2>,
};
// Last frame's resolved radiance (the TAA history), for traced reflections.
@group(1) @binding(21) var reflection_history: texture_2d<f32>;

fn meadow_edge_covers(p:vec3<f32>,normal:vec3<f32>,sun_query:vec2<f32>,
    block:u32,origin:vec2<i32>,dimension:u32,radius:f32)->vec2<f32> {
    // Both visibility terms share25 records. Radius is bounded on the host
    // using the anchor extent, jitter and half the maximum sun displacement,
    // so no omitted root can influence either side of this midpoint.
    let midpoint=(p.xz+sun_query)*0.5;
    let first=vec2<i32>(floor(midpoint/MEADOW_ROOT_CELL))-vec2<i32>(2);
    var gap=vec2<f32>(1.0);
    for(var z=0i;z<5i;z++){for(var x=0i;x<5i;x++) {
        let cell=first+vec2<i32>(x,z)-origin;
        if any(cell<vec2<i32>(0)) || any(cell>=vec2<i32>(i32(dimension))) {continue;}
        let sample=meadow_edge_field[block+4u+u32(cell.y)*dimension+u32(cell.x)];
        if sample.w<=0.0 {continue;}
        let sky_delta=p.xz-sample.xz;let sun_delta=sun_query-sample.xz;
        let distances=vec2<f32>(dot(sky_delta,sky_delta),dot(sun_delta,sun_delta));
        let footprint=vec2<f32>(1.0)-smoothstep(vec2<f32>(0.0625*radius*radius),
            vec2<f32>(radius*radius),distances);
        // Same-plane adjacent terrain and road can receive contact. Raised
        // road decks or a different-height surface cannot borrow the roots.
        let separation=abs(dot(p-sample.xyz,normal));
        let height_gate=1.0-smoothstep(0.04,0.18,separation);
        gap*=vec2<f32>(1.0)-clamp(sample.w,0.0,1.0)*footprint*height_gate;
    }}
    return vec2<f32>(1.0)-gap;
}

fn meadow_edge_visibility(p:vec3<f32>,normal:vec3<f32>,sun:vec3<f32>)->vec2<f32> {
    if view.grass.w<0.5 || view.grass.z<1.0 || view.grass.z>4.0 || normal.y<=0.45 {
        return vec2<f32>(1.0);
    }
    // Dimensions are fixed by the producer; a tiny empty scene binding is
    // safe even for shaders that do not prepare this optional contact field.
    // Camera IDs are one-based, shared with the instance-hide mask in cull.
    let stride=4u+151u*151u;let block=(u32(view.grass.z)-1u)*stride;
    if block+stride>arrayLength(&meadow_edge_field) {return vec2<f32>(1.0);}
    let header=meadow_edge_field[block];let params=meadow_edge_field[block+1u];
    let fades=meadow_edge_field[block+2u];let camera=meadow_edge_field[block+3u];
    if header.w<0.5 || header.z!=151.0 || params.x<=0.0 || distance(camera.xyz,view.cam.xyz)>0.001 {
        return vec2<f32>(1.0);
    }
    let distance_to_view=distance(p.xz,view.cam.xz);
    let fade=(1.0-smoothstep(fades.x,fades.y,distance_to_view))
        *(1.0-smoothstep(fades.z,fades.w,distance_to_view));
    if fade<=0.0 {return vec2<f32>(1.0);}
    let origin=bitcast<vec2<i32>>(header.xy);let dimension=u32(header.z);
    let sun_active=light.sun_dir.w>0.0 && sun.y>0.0;
    var offset=vec2<f32>(0.0);
    if sun_active {
        // Only a short basal sun footprint. Clip at35cm even at low sun;
        // full-height blade silhouettes and their wind motion are not traced.
        offset=sun.xz*(0.20/max(sun.y,0.15));
        offset*=min(1.0,params.w/max(length(offset),0.0001));
    }
    let covers=meadow_edge_covers(p,normal,p.xz+offset,block,origin,dimension,params.x);
    let sun_cover=select(0.0,covers.y,sun_active);
    return vec2<f32>(1.0-params.y*covers.x*fade,
        1.0-params.z*sun_cover*fade*light.sun_color.w);
}

// Extinction density on three absolute world grids. Blend the densities
// before tracing, so overlapping levels never count the same forest twice.
// The continuous anchors control the handoff; snapped storage windows do
// not. An origin shift therefore cannot move a blend boundary in one frame.
fn bounded_canopy_density(q:vec3<f32>,level:u32,d:vec3<f32>)->f32 {
    // Texture samples represent cell centres. Source-bounded Y tents retain
    // the same [-1,+1] neighbour support, so exactly eight loads suffice.
    let grid=q-0.5;let base=vec3<i32>(floor(grid));
    var result=0.0;
    for(var z=0i;z<2i;z++) {for(var y=0i;y<2i;y++) {for(var x=0i;x<2i;x++) {
        let cell=base+vec3<i32>(x,y,z);
        if any(cell<vec3<i32>(0)) || any(cell>=vec3<i32>(d)) {continue;}
        let delta=grid-vec3<f32>(cell);
        let data=textureLoad(canopy_tex,cell+vec3<i32>(0,i32(f32(level)*d.y),0),0);
        if data.r<=0.0 || data.a<=0.0 || delta.y<=data.g || delta.y>=data.b {continue;}
        let tent=max(vec3<f32>(1.0)-abs(delta),vec3<f32>(0.0));
        let lower=select(1.0,smoothstep(data.g,data.g+data.a,delta.y),data.g>-1.0);
        let upper=select(1.0,1.0-smoothstep(data.b-data.a,data.b,delta.y),data.b<1.0);
        let window=lower*upper;
        result+=data.r*tent.x*tent.y*tent.z*window;
    }}}
    return result;
}

fn foliage(p: vec3<f32>) -> f32 {
    let d = vec3<f32>(gi.vol_dims.xyz);
    var remaining = 1.0;
    var sigma = 0.0;
    for (var level = 0u; level < 3u; level++) {
        let origin = gi.vol_origins[level];
        let delta = abs((p - gi.vol_anchors[level].xyz) / origin.w);
        let weights = 1.0 - smoothstep(d * 0.5 - 13.0, d * 0.5 - 5.0, delta);
        let weight = min(weights.x, min(weights.y, weights.z));
        if weight <= 0.0 { continue; }
        let q = (p - origin.xyz) / origin.w;
        var value=0.0;
        if (gi.vol_dims.w&2u)!=0u {
            value=bounded_canopy_density(q,level,d);
        } else {
            // Each level occupies one Y slab. Clamp inside its own texel
            // centres so filtering cannot sample a different level's slab.
            let local = clamp(q, vec3<f32>(0.5), d - 0.5);
            let uv = vec3<f32>(local.x / d.x,
                (local.y + f32(level) * d.y) / (CANOPY_SLABS * d.y), local.z / d.z);
            value=textureSampleLevel(canopy_tex, canopy_sampler, uv, 0.0).r;
        }
        sigma += remaining * weight * value;
        remaining *= 1.0 - weight;
        if remaining <= 0.0001 { break; }
    }
    return sigma;
}

// Fast distance lookup in the packed 3D unsigned Euclidean distance field
// (`dfao.rs`) stored in `aux` at offset `u32(gi.hf.w)`.
fn dfao_byte(idx: u32, o: u32) -> f32 {
    return f32((aux_word(o + 8u + (idx >> 2u)) >> ((idx & 3u) * 8u)) & 0xffu);
}

fn dfao_sample(q: vec3<f32>, o: u32, origin: vec3<f32>, inv_voxel: f32, dims: vec3<u32>, scale: f32) -> f32 {
    let f = (q - origin) * inv_voxel - 0.5;
    let last = vec3<f32>(dims - vec3<u32>(1u));
    if any(f < vec3<f32>(-0.5)) || any(f > last + vec3<f32>(0.5)) { return 255.0 * scale; }
    let fc = clamp(f, vec3<f32>(0.0), last);
    let i0 = vec3<u32>(min(floor(fc), max(last - vec3<f32>(1.0), vec3<f32>(0.0))));
    let t = fc - vec3<f32>(i0);
    let sy = dims.x;
    let sz = dims.x * dims.y;
    let b00 = i0.x + i0.y * sy + i0.z * sz;
    let b10 = b00 + sy;
    let b01 = b00 + sz;
    let b11 = b10 + sz;
    let x00 = mix(dfao_byte(b00, o), dfao_byte(b00 + 1u, o), t.x);
    let x10 = mix(dfao_byte(b10, o), dfao_byte(b10 + 1u, o), t.x);
    let x01 = mix(dfao_byte(b01, o), dfao_byte(b01 + 1u, o), t.x);
    let x11 = mix(dfao_byte(b11, o), dfao_byte(b11 + 1u, o), t.x);
    let y0 = mix(x00, x10, t.y);
    let y1 = mix(x01, x11, t.y);
    return mix(y0, y1, t.z) * scale;
}

fn dfao_visibility(p: vec3<f32>, n: vec3<f32>, refl: vec3<f32>) -> vec2<f32> {
    let o = u32(gi.hf.w);
    if o == 0u { return vec2<f32>(1.0); }
    let origin = vec3<f32>(aux_f32(o), aux_f32(o + 1u), aux_f32(o + 2u));
    let voxel = aux_f32(o + 3u);
    let dims = vec3<u32>(aux_word(o + 4u), aux_word(o + 5u), aux_word(o + 6u));
    let extent = origin + vec3<f32>(dims) * voxel;
    if any(p < origin) || any(p > extent) { return vec2<f32>(1.0); }
    let max_dist = aux_f32(o + 7u);
    let inv_voxel = 1.0 / max(voxel, 0.01);
    let scale = max_dist / 255.0;
    let d1 = dfao_sample(p + n * (voxel * 0.85), o, origin, inv_voxel, dims, scale);
    let d2 = dfao_sample(p + n * (voxel * 2.15), o, origin, inv_voxel, dims, scale);
    let v1 = clamp(d1 / (voxel * 0.95), 0.22, 1.0);
    let v2 = clamp(d2 / (voxel * 2.25), 0.30, 1.0);
    let diffuse = min(v1, v2) * 0.65 + (v1 * v2) * 0.35;
    let dr = dfao_sample(p + refl * (voxel * 1.4), o, origin, inv_voxel, dims, scale);
    let specular = clamp(dr / (voxel * 1.45), 0.20, 1.0);
    return vec2<f32>(diffuse, specular);
}

// Fast 4-tap Screen-Space Ambient Occlusion (SSAO) in reversed-Z hardware depth,
// with thickness rejection so foreground edges never cast dark halos.
// Horizon-based ambient occlusion (the idea of GTAO, as Call of Duty uses
// it): along two screen directions, rotated per pixel and frame for TAA to
// average, four depth samples each way within 0.6 m of p. Each side's
// horizon is the steepest sample above the surface plane; its elevation
// occludes. Samples further away than the radius fade out, so a silhouette
// in front (a wall edge, the gun) does not darken what is behind it, and
// flat surfaces never occlude themselves. Room corners, the floor under a
// couch and creases darken; a three-way ceiling corner stays continuous.
fn screen_space_ao(px: vec2<i32>, size: vec2<u32>, p: vec3<f32>, n: vec3<f32>, dist: f32) -> f32 {
    if gi.params.x < 0.5 || dist >= 48.0 { return 1.0; }
    let d0 = textureLoad(depth_tex, px, 0);
    if d0 <= 0.0001 { return 1.0; }
    let fade = 1.0 - smoothstep(30.0, 48.0, dist);
    let radius = 0.6;
    let radius_px = min(radius * view.cam.w / max(dist, 0.3), 64.0);
    if radius_px < 1.5 { return 1.0; }
    let max_px = vec2<i32>(size) - vec2<i32>(1);
    let xi = trace_hash(px, light.fog_sampling.x * 16.0 + 7.0);
    var occlusion = 0.0;
    for (var dir_i = 0u; dir_i < 2u; dir_i++) {
        let angle = (xi.x + f32(dir_i) * 0.5) * PI;
        let dir = vec2<f32>(cos(angle), sin(angle));
        for (var side = 0u; side < 2u; side++) {
            let sign = select(-1.0, 1.0, side == 0u);
            var horizon = 0.0;
            for (var k = 0u; k < 4u; k++) {
                // Steps jittered along the ray, denser near the pixel.
                let t = (f32(k) + xi.y) / 4.0;
                let offset = dir * sign * max(t * t * radius_px, 1.0);
                let q_px = clamp(px + vec2<i32>(round(offset)), vec2<i32>(0), max_px);
                let depth = textureLoad(depth_tex, q_px, 0);
                if depth <= 0.0001 { continue; }
                let ndc = (vec2<f32>(q_px) + 0.5) / vec2<f32>(size) * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);
                let h = view.inv_view_proj * vec4<f32>(ndc, depth, 1.0);
                let w = h.xyz / h.w - p;
                let len = length(w);
                if len < 1e-4 { continue; }
                // sin of the sample's elevation above the surface plane, less
                // a small bias against depth precision on flat ground.
                let elevation = dot(n, w) / len - 0.1;
                let falloff = clamp(1.0 - (len * len) / (radius * radius), 0.0, 1.0);
                horizon = max(horizon, elevation * falloff);
            }
            occlusion += horizon;
        }
    }
    // Four sides; sin^2 of the horizon is the share of the cosine-weighted
    // hemisphere it hides, sin alone looks closer to the bake's softness.
    let ao = 1.0 - clamp(occlusion * 0.25 * 1.1, 0.0, 1.0);
    return mix(1.0, max(ao, 0.45), fade);
}

// Short directional contact shadow supplements the finite spotlight map.
// Four depth taps, only nearby receivers; no ambient darkening of lit faces.
fn local_contact_shadow(p:vec3<f32>, normal:vec3<f32>, towards:vec3<f32>, size:vec2<u32>, dist:f32)->f32 {
    if gi.params.x<0.5 || dist>=32.0 {return 1.0;}
    let origin=p+normal*0.018;
    var occlusion=0.0;
    for(var i=0u;i<4u;i++) {
        let travel=0.045*exp2(f32(i));
        let q=origin+towards*travel;
        let clip=view.view_proj*vec4<f32>(q,1.0);
        if clip.w<=0.0 {continue;}
        let uv=clip.xy/clip.w*vec2<f32>(0.5,-0.5)+vec2<f32>(0.5);
        if any(uv<=vec2<f32>(0.0)) || any(uv>=vec2<f32>(1.0)) {continue;}
        let pixel=vec2<i32>(uv*vec2<f32>(size));
        let depth=textureLoad(depth_tex,pixel,0);
        if depth<=0.0 {continue;}
        let ndc=(vec2<f32>(pixel)+0.5)/vec2<f32>(size)*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0);
        let hit=view.inv_view_proj*vec4<f32>(ndc,depth,1.0);
        let gap=distance(q,view.cam.xyz)-distance(hit.xyz/hit.w,view.cam.xyz);
        // Reject distant foreground silhouettes rather than stretching shadows.
        let blocked=smoothstep(0.012,0.035,gap)*(1.0-smoothstep(0.07,0.13,gap));
        occlusion=max(occlusion,blocked);
    }
    return 1.0-occlusion*(1.0-smoothstep(22.0,32.0,dist));
}

fn contact_headlight_radiance(p:vec3<f32>,n:vec3<f32>,surface_n:vec3<f32>,v:vec3<f32>,diffuse:vec3<f32>,rough:f32,f0:vec3<f32>,size:vec2<u32>,dist:f32)->vec3<f32> {
    var result=vec3<f32>(0.0);
    for(var index=0u;index<incident_light_count(p);index++) {
        if dot(n,incident_light_position(index)-p)<=0.0 {continue;}
        let sample=incident_light_sample(index,p,surface_n);
        if all(sample.irradiance==vec3<f32>(0.0)) {continue;}
        var contact=1.0;
        if index<min(headlights.count.x,8u) {contact=local_contact_shadow(p,surface_n,sample.direction,size,dist);}
        result+=sample.irradiance*contact*(diffuse*max(dot(n,sample.direction),0.0)+ggx(n,v,sample.direction,rough,f0));
    }
    return result;
}

// Smooth projected-area approximation for diffuse contact, and opaque ray
// intersection for glossy visibility. Proxies follow the live car transform.
// These small local bounds never share the coarse, translucent foliage grid.
fn contact_visibility(p: vec3<f32>, n: vec3<f32>, refl: vec3<f32>, owner: u32) -> vec2<f32> {
    if gi.vol_dims.w == 0u || gi.params.x < 0.5 { return vec2<f32>(1.0); }
    let dfao = dfao_visibility(p, n, refl);
    var diffuse = dfao.x;
    var specular = dfao.y;
    for (var i = 0u; i < gi.hf_dims.z; i++) {
        let bounds = solid_occluder(i);
        // A coarse interior proxy must never shade its own visible body:
        // segment boundaries otherwise appear as planes across the paint.
        if bounds.identity.x == owner {
            i += max(bounds.identity.y, 1u) - 1u;
            continue;
        }
        let obj = instances[bounds.identity.x];
        let world_delta = p - obj.sphere.xyz;
        let reach_radius = obj.sphere.w + 8.0;
        if dot(world_delta, world_delta) > reach_radius * reach_radius {
            // All remaining boxes of this body share its broad-phase sphere.
            i += max(bounds.identity.y, 1u) - 1u;
            continue;
        }
        let ax = vec3<f32>(obj.r0.x, obj.r1.x, obj.r2.x);
        let ay = vec3<f32>(obj.r0.y, obj.r1.y, obj.r2.y);
        let az = vec3<f32>(obj.r0.z, obj.r1.z, obj.r2.z);
        let scale2 = max(vec3<f32>(dot(ax, ax), dot(ay, ay), dot(az, az)), vec3<f32>(1e-8));
        let offset = p - vec3<f32>(obj.r0.w, obj.r1.w, obj.r2.w);
        let local = vec3<f32>(dot(ax, offset), dot(ay, offset), dot(az, offset)) / scale2 - bounds.center.xyz;
        let extent3 = bounds.half_extent.xyz;
        let delta = max(abs(local) - extent3, vec3<f32>(0.0));
        let distance2 = dot(delta * delta, scale2);
        if distance2 > 64.0 { continue; }
        let scale = sqrt(scale2);
        let toward = -local * scale;
        let d2 = max(dot(toward, toward), 1e-6);
        let v = toward / sqrt(d2);
        let nn = vec3<f32>(dot(ax, n), dot(ay, n), dot(az, n)) / scale;
        let h = extent3 * scale;
        let area = 4.0 * dot(vec3<f32>(h.y * h.z, h.x * h.z, h.x * h.y), abs(v));
        let coverage = area / (area + 3.14159265 * d2);
        let reach = 1.0 - smoothstep(16.0, 64.0, distance2);
        diffuse *= 1.0 - coverage * max(dot(nn, v), 0.0) * reach;
        let rd = vec3<f32>(dot(ax, refl), dot(ay, refl), dot(az, refl)) / scale2;
        let safe_rd = select(vec3<f32>(-1e-6), vec3<f32>(1e-6), rd >= vec3<f32>(0.0));
        let inv = 1.0 / select(rd, safe_rd, abs(rd) < vec3<f32>(1e-6));
        let a = (-extent3 - local) * inv;
        let b = (extent3 - local) * inv;
        let lo = min(a, b);
        let hi = max(a, b);
        let enter = max(max(lo.x, lo.y), lo.z);
        let leave = min(min(hi.x, hi.y), hi.z);
        if leave > max(enter, 0.025) && enter < 8.0 { specular = 0.0; }
    }
    return vec2<f32>(clamp(diffuse, 0.0, 1.0), specular);
}

// Terrain height at (x, z), on the same diagonal as the terrain mesh.
// A bilinear patch can rise above its two rendered triangles and make the
// ground falsely occlude its own indirect light. -1e9 outside the field.
fn ground_at(xz: vec2<f32>) -> f32 {
    let g = (xz - gi.hf.xy) / gi.hf.z;
    let d = vec2<i32>(gi.hf_dims.xy);
    let i = vec2<i32>(floor(g));
    if any(i < vec2<i32>(0)) || any(i >= d - 1) {
        return -1e9;
    }
    let f = g - vec2<f32>(i);
    let a = textureLoad(heightfield, i, 0).r;
    let b = textureLoad(heightfield, i + vec2<i32>(1, 0), 0).r;
    let c = textureLoad(heightfield, i + vec2<i32>(0, 1), 0).r;
    let e = textureLoad(heightfield, i + vec2<i32>(1, 1), 0).r;
    if f.x + f.y <= 1.0 {
        return a + (b - a) * f.x + (c - a) * f.y;
    }
    return e + (c - e) * (1.0 - f.x) + (b - e) * (1.0 - f.y);
}

// How much light gets from direction `dir` to p, through the canopy and
// past the terrain: steps grow with distance, like a cone widening.
fn transmittance(p: vec3<f32>, dir: vec3<f32>, max_t: f32) -> f32 {
    var t = 0.5;
    var tr = 1.0;
    for (var k = 0; k < 12; k++) {
        if t >= max_t { break; }
        let q = p + dir * t;
        if q.y < ground_at(q.xz) - 0.3 {
            return 0.0;
        }
        // Left-endpoint samples retain their existing positions, but the last
        // interval ends at max_t. A full final step over-attenuated short rays.
        let step = min(max(t * 0.45, 0.6), max_t - t);
        tr *= exp(-foliage(q) * 0.9 * step);
        t += step;
        if t >= max_t || tr < 0.02 {
            break;
        }
    }
    // Bounded 12-sample estimate only. Unmarched space after this budget is
    // unknown, not certified empty; this does not solve canopy shape/coverage.
    return tr;
}

// The canopy texture's Y slabs: three extinction levels, then (canopy.rs)
// the cached directional sky transmittance of each level.
const CANOPY_SLABS: f32 = 6.0;

// Cached sky transmittance at p (gi.vol_dims.w bit 3): per canopy level,
// L1 coefficients of the transmittance towards each direction w,
// T(w) = a0 + dot(b, w), baked by `bake_sky_visibility` whenever a level's
// extinction is rebuilt, and blended across levels as `foliage` blends.
fn cached_transmittance(p: vec3<f32>) -> vec4<f32> {
    let d = vec3<f32>(gi.vol_dims.xyz);
    var remaining = 1.0;
    var sum = vec4<f32>(0.0);
    for (var level = 0u; level < 3u; level++) {
        let origin = gi.vol_origins[level];
        let delta = abs((p - gi.vol_anchors[level].xyz) / origin.w);
        let weights = 1.0 - smoothstep(d * 0.5 - 13.0, d * 0.5 - 5.0, delta);
        let weight = min(weights.x, min(weights.y, weights.z));
        if weight <= 0.0 { continue; }
        let local = clamp((p - origin.xyz) / origin.w, vec3<f32>(0.5), d - 0.5);
        let uv = vec3<f32>(local.x / d.x, (local.y + (3.0 + f32(level)) * d.y) / (CANOPY_SLABS * d.y), local.z / d.z);
        sum += remaining * weight * textureSampleLevel(canopy_tex, canopy_sampler, uv, 0.0);
        remaining *= 1.0 - weight;
        if remaining <= 0.0001 { break; }
    }
    // Outside every level nothing was traced: open sky, as the march returns.
    return sum + vec4<f32>(remaining, 0.0, 0.0, 0.0);
}

// Fraction of the sky a surface sees: five cones around its normal.
fn sky_visibility(p: vec3<f32>, n: vec3<f32>) -> f32 {
    if gi.vol_dims.w == 0u || (gi.vol_dims.w & 4u) != 0u {
        return 1.0;
    }
    if (gi.vol_dims.w & 8u) != 0u {
        // Cosine-weighted hemisphere integral of the L1 transmittance.
        let c = cached_transmittance(p + n * 0.15);
        return clamp(c.x + dot(c.yzw, n) * (2.0 / 3.0), 0.0, 1.0);
    }
    let t1 = normalize(select(cross(n, vec3<f32>(0.0, 0.0, 1.0)), cross(n, vec3<f32>(1.0, 0.0, 0.0)), abs(n.x) < 0.9));
    let t2 = cross(n, t1);
    let o = p + n * 0.15;
    var v = transmittance(o, n, 60.0) * 2.0;
    v += transmittance(o, normalize(n + t1 * 1.2), 40.0);
    v += transmittance(o, normalize(n - t1 * 1.2), 40.0);
    v += transmittance(o, normalize(n + t2 * 1.2), 40.0);
    v += transmittance(o, normalize(n - t2 * 1.2), 40.0);
    return v / 6.0;
}

// Only glossy surfaces need this longer direction-aware query. A diffuse
// hemisphere visibility cannot tell whether a mountain hides a reflection.
fn reflected_visibility(p: vec3<f32>, n: vec3<f32>, r: vec3<f32>) -> f32 {
    if gi.vol_dims.w == 0u || gi.params.y < 0.5 || (gi.vol_dims.w & 4u) != 0u { return 1.0; }
    let o = p + n * 0.06;
    if (gi.vol_dims.w & 8u) != 0u {
        // The canopy from the cache along r; the terrain still marched (cheap:
        // a few heightfield loads, none outside the field).
        let c = cached_transmittance(o);
        var t = 0.25;
        for (var k = 0; k < 20; k++) {
            let q = o + r * t;
            if q.y < ground_at(q.xz) - 0.12 { return 0.0; }
            t += max(0.6, t * 0.40);
            if t > 250.0 { break; }
        }
        return clamp(c.x + dot(c.yzw, r), 0.0, 1.0);
    }
    var t = 0.25;
    var tr = 1.0;
    for (var k = 0; k < 20; k++) {
        let q = o + r * t;
        if q.y < ground_at(q.xz) - 0.12 { return 0.0; }
        let step = max(0.6, t * 0.40);
        tr *= exp(-foliage(q) * 0.9 * step);
        t += step;
        if t > 250.0 || tr < 0.02 { break; }
    }
    return tr;
}

const PI: f32 = 3.14159265;

// GGX microfacet specular for a light from l (Karis 2013): D * Vis * F * n.l.
fn ggx(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, rough: f32, f0: vec3<f32>) -> vec3<f32> {
    let h = normalize(v + l);
    let ndl = max(dot(n, l), 0.0);
    let ndv = max(dot(n, v), 1e-4);
    let ndh = max(dot(n, h), 0.0);
    let a = max(rough * rough, 2e-3);
    let a2 = a * a;
    let d = a2 / (PI * pow(ndh * ndh * (a2 - 1.0) + 1.0, 2.0));
    let vis = 0.5 / (ndl * sqrt(ndv * ndv * (1.0 - a2) + a2) + ndv * sqrt(ndl * ndl * (1.0 - a2) + a2) + 1e-5);
    let f = f0 + (1.0 - f0) * pow(1.0 - max(dot(v, h), 0.0), 5.0);
    return d * vis * f * ndl;
}

// Split-sum environment BRDF, analytic fit (Karis, mobile).
fn env_brdf(f0: vec3<f32>, rough: f32, ndv: f32) -> vec3<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = rough * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * ndv)) * r.x + r.y;
    let ab = vec2<f32>(-1.04, 1.04) * a004 + r.zw;
    return f0 * ab.x + ab.y;
}

// Radiance arriving along r: the sky above the horizon (blurred into its
// irradiance as the surface gets rougher), lit ground below it.
fn environment(r: vec3<f32>, rough: f32, bounce: vec3<f32>) -> vec3<f32> {
    // The sun is already evaluated by GGX and its shadow map. Match the SH
    // sun exclusion here so the HDRI does not add a second, unshadowed sun.
    let ground = bounce / PI;
    let sky_weight = smoothstep(-0.08, 0.04, r.y);
    // Exact smoothstep endpoints: do not sample a contribution weighted zero.
    if sky_weight <= 0.0 { return ground; }
    var sky: vec3<f32>;
    if rough >= 0.55 {
        sky = sky_irradiance(r) / PI;
    } else if rough <= 0.05 {
        sky = min(sky_radiance(r), vec3<f32>(gi.params.z));
    } else {
        sky = mix(min(sky_radiance(r), vec3<f32>(gi.params.z)), sky_irradiance(r) / PI, smoothstep(0.05, 0.55, rough));
    }
    return mix(ground, sky, sky_weight);
}

// Traced reflections, stage 1 of dynamic GI (dynamic-gi.md). The reflected
// ray marches this frame's depth buffer with steps that grow with distance;
// where it passes behind a surface by less than the step it hit it, and the
// hit's radiance is read from last frame's resolved image at the hit's
// reprojected position: whatever the frame shows, lit by everything, trees
// and cars included. A ray that leaves the screen (it cannot come back: the
// frustum is convex) returns confidence 0 and keeps the environment and its
// heightfield occlusion. The ray is the mirror ray; rough surfaces blur the
// hit's radiance over the lobe's footprint (rotated taps, TAA-averaged) and
// the reflection fades out by roughness. rgb: radiance, a: confidence.
fn trace_hash(px: vec2<i32>, salt: f32) -> vec2<f32> {
    // Interleaved gradient noise, shifted by the frame sequence.
    let q = vec2<f32>(px) + salt * vec2<f32>(5.588238, 5.588238);
    let a = fract(52.9829189 * fract(dot(q, vec2<f32>(0.06711056, 0.00583715))));
    let b = fract(52.9829189 * fract(dot(q + vec2<f32>(47.0, 17.0), vec2<f32>(0.00583715, 0.06711056))));
    return vec2<f32>(a, b);
}

fn trace_jitter(r: vec3<f32>, rough: f32, xi: vec2<f32>) -> vec3<f32> {
    // A GGX lobe around the mirror direction (an approximation that treats
    // the reflection as the lobe axis; exact enough under the TAA average).
    let a = rough * rough;
    let cos_t = sqrt((1.0 - xi.x) / (1.0 + (a * a - 1.0) * xi.x));
    let sin_t = sqrt(max(1.0 - cos_t * cos_t, 0.0));
    let phi = 2.0 * PI * xi.y;
    let t1 = normalize(select(cross(r, vec3<f32>(0.0, 0.0, 1.0)), cross(r, vec3<f32>(1.0, 0.0, 0.0)), abs(r.x) < 0.9));
    let t2 = cross(r, t1);
    // The half-vector spread doubles in the reflected direction.
    let s = sin_t * 2.0 * cos_t;
    let c = cos_t * cos_t * 2.0 - 1.0;
    return normalize(r * c + (t1 * cos(phi) + t2 * sin(phi)) * s);
}

// Water optics (Lighting::water, terrain_surface.wgsl water_body).
fn water_optics_on() -> bool { return light.fog_sampling.w > 0.5; }

// The depth a reflection ray is tested against: the opaque depth and, in
// front of it, the rasterized voxel trees when they keep their own depth
// (VOXEL_OCCLUSION), so far forests show in water and glossy paint.
fn trace_depth(c: vec2<i32>) -> f32 {
    var d = textureLoad(depth_tex, c, 0);
    if VOXEL_OCCLUSION { d = max(d, textureLoad(voxel_depth, c, 0)); }
    return d;
}

// One level of the reflection mips at uv: one bilinear tap at level 0, a
// tent of four bilinear taps half a texel apart above it, so a coarse
// level's texels do not show as squares.
// The mips hold colour times the share of each texel that is not a light's
// face (reflection_mips.rs); dividing it out reads a face as what is around it.
fn reflection_level(uv: vec2<f32>, lod: f32) -> vec3<f32> {
    var s: vec4<f32>;
    if lod < 0.5 { s = textureSampleLevel(reflection_history, canopy_sampler, uv, 0.0); }
    else {
        let o = 0.5 / vec2<f32>(textureDimensions(reflection_history, i32(lod)));
        s = 0.25 * (textureSampleLevel(reflection_history, canopy_sampler, uv + vec2<f32>(-o.x, -o.y), lod)
            + textureSampleLevel(reflection_history, canopy_sampler, uv + vec2<f32>(o.x, -o.y), lod)
            + textureSampleLevel(reflection_history, canopy_sampler, uv + vec2<f32>(-o.x, o.y), lod)
            + textureSampleLevel(reflection_history, canopy_sampler, uv + vec2<f32>(o.x, o.y), lod));
    }
    return select(vec3<f32>(0.0), s.rgb / s.a, s.a > 1e-4);
}

// The result of marching one reflected ray through this frame's depth
// buffer: the point it reached (or, for a ray that left through an opening,
// a point in front of what it escaped into), the distance along the ray and
// a confidence (0: no hit; fades at the screen edge and the reach).
struct MarchHit { hit: vec3<f32>, t: f32, conf: f32 }

// Whether the pixel shows a light's face: an EMISSIVE material with a
// negative scale (scene.rs), whose emission an analytic light stands for.
fn light_face_at(px: vec2<i32>) -> bool {
    let id = textureLoad(vis_tex, px, 0).r;
    if (id & 0x80000000u) != 0u { return false; }
    let m = material(clusters[visible[id >> 7u].y].material);
    return (m.flags & 1024u) != 0u && m.scale < 0.0;
}

// The depth buffer's surface at pixel c: its world position and view depth
// (1e9 for the sky).
fn trace_world(c: vec2<i32>, size: vec2<u32>) -> vec4<f32> {
    let q = clamp(c, vec2<i32>(0), vec2<i32>(size) - 1);
    let d = trace_depth(q);
    if d <= 0.0 { return vec4<f32>(0.0, 0.0, 0.0, 1e9); }
    let pc = (vec2<f32>(q) + 0.5) / vec2<f32>(size);
    let h = view.inv_view_proj * vec4<f32>(pc.x * 2.0 - 1.0, 1.0 - pc.y * 2.0, d, 1.0);
    let w = h.xyz / h.w;
    return vec4<f32>(w, (view.view_proj * vec4<f32>(w, 1.0)).w);
}

fn trace_view_depth(c: vec2<i32>, size: vec2<u32>) -> f32 { return trace_world(c, size).w; }

fn trace_screen(q: vec3<f32>, fsize: vec2<f32>) -> vec4<f32> {
    let c = view.view_proj * vec4<f32>(q, 1.0);
    let ndc = c.xyz / c.w;
    return vec4<f32>(vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5) * fsize, ndc.z, c.w);
}

fn march_hit(p: vec3<f32>, n: vec3<f32>, r: vec3<f32>, px: vec2<i32>, size: vec2<u32>, dist: f32) -> MarchHit {
    var none: MarchHit;
    if dot(r, n) <= 0.0 { return none; }
    let fsize = vec2<f32>(size);
    // Leave the surface by its depth precision and pixel footprint.
    let o = p + n * (0.02 + dist * 0.002);
    let reach = gi.trace.z;
    let jitter = trace_hash(px.yx, light.fog_sampling.x * 16.0 + 3.0).x;
    var lo = 0.0;
    var t = max(0.06, dist * 0.003) * (0.75 + 0.5 * jitter);
    var found = false;
    var escape = 0.0;
    var hi = t;
    var hit = vec3<f32>(0.0);
    for (var k = 0; k < 40; k++) {
        let c = view.view_proj * vec4<f32>(o + r * t, 1.0);
        if c.w <= 1e-4 { break; }
        let ndc = c.xyz / c.w;
        if abs(ndc.x) >= 1.0 || abs(ndc.y) >= 1.0 { break; }
        let sp = vec2<i32>(vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5) * fsize);
        let d = trace_depth(sp);
        // The scene here is far beyond the ray (sky, or the field through a
        // doorway): remember it. A ray that leaves through an opening then
        // projects onto the wall around it and never hits; it reads what it
        // escaped into instead.
        if d < ndc.z * 0.66 { escape = t; }
        // Reversed infinite Z: the scene is in front of the ray point when
        // d > ndc.z. A candidate: the ray went behind it during this step.
        if d > ndc.z {
            let behind = c.w * (1.0 - ndc.z / d);
            if behind < max(0.1, (t - lo) * 1.2 + c.w * 0.005) {
                // Bisect to the crossing.
                var a = lo;
                var b = t;
                for (var j = 0; j < 6; j++) {
                    let mid = 0.5 * (a + b);
                    let sm = trace_screen(o + r * mid, fsize);
                    if trace_depth(vec2<i32>(sm.xy)) > sm.z { b = mid; } else { a = mid; }
                }
                // A hit only where the ray meets the depth buffer's surface:
                // at a and b it lies within what the ray's own depth change,
                // the surface's depth change across the pixels between them
                // and the depth precision allow. Passing behind a nearer
                // object (its silhouette in front of the ray) leaves a gap of
                // the object's thickness and is no hit.
                let sa = trace_screen(o + r * a, fsize);
                let sb = trace_screen(o + r * b, fsize);
                let cb = clamp(vec2<i32>(sb.xy), vec2<i32>(0), vec2<i32>(size) - 1);
                let surf = trace_world(cb, size);
                let w_surf = surf.w;
                var slope = 0.0;
                for (var e = 0; e < 4; e++) {
                    let off = select(vec2<i32>(select(-1, 1, e == 1), 0), vec2<i32>(0, select(-1, 1, e == 3)), e >= 2);
                    let wn = trace_view_depth(cb + off, size);
                    if wn < 1e8 { slope = max(slope, abs(wn - w_surf)); }
                }
                let moved = length(sb.xy - sa.xy) + 1.0;
                let allowed = abs(sb.w - sa.w) + slope * moved + w_surf * 2e-3;
                let back = sb.w - w_surf;
                let front = trace_view_depth(vec2<i32>(sa.xy), size) - sa.w;
                // The surface must also face the ray (not be seen from behind).
                var facing = true;
                if w_surf < 1e8 {
                    let ns = rsm_normal(cb, size, surf.xyz);
                    facing = all(ns == vec3<f32>(0.0)) || dot(ns, r) < 0.0;
                }
                if back <= allowed && front <= allowed && w_surf < 1e8 && facing {
                    found = true;
                    hi = b;
                    hit = surf.xyz;
                    break;
                }
            }
        }
        lo = t;
        // 20 % per step: 40 steps reach past 250 m.
        t += max(0.06, t * 0.2);
        if t > reach { break; }
    }
    if found {
        // A light's face (EMISSIVE with negative scale): its light reaches
        // this surface as that light's analytic specular, on screen or off,
        // so the ray counts as a miss instead of adding it a second time.
        let sh = trace_screen(o + r * hi, fsize);
        if light_face_at(clamp(vec2<i32>(sh.xy), vec2<i32>(0), vec2<i32>(size) - 1)) { return none; }
    } else if escape <= 0.0 {
        return none;
    } else {
        hi = escape;
        hit = o + r * escape;
    }
    let c = view.view_proj * vec4<f32>(o + r * hi, 1.0);
    let ndc = c.xyz / c.w;
    // Fade at the screen's edges and near the reach.
    var out: MarchHit;
    out.hit = hit;
    out.t = hi;
    out.conf = (1.0 - smoothstep(0.82, 1.0, max(abs(ndc.x), abs(ndc.y)))) * (1.0 - smoothstep(reach * 0.7, reach, hi));
    return out;
}

fn traced_reflection(p: vec3<f32>, n: vec3<f32>, mirror: vec3<f32>, rough: f32, px: vec2<i32>, size: vec2<u32>, dist: f32) -> vec4<f32> {
    if gi.trace.x < 0.5 || rough >= gi.trace.y { return vec4<f32>(0.0); }
    // One mirror ray; the roughness blur is applied at the hit instead, as a
    // cone-traced reflection does. The stochastic path (sssr.wgsl) replaces
    // this for the main surface when it is on; a clear coat keeps it.
    let m = march_hit(p, n, mirror, px, size, dist);
    if m.conf <= 0.0 { return vec4<f32>(0.0); }
    let fsize = vec2<f32>(size);
    let c = view.view_proj * vec4<f32>(m.hit, 1.0);
    let prev = gi.prev_view_proj * vec4<f32>(m.hit, 1.0);
    if prev.w <= 1e-4 { return vec4<f32>(0.0); }
    let prev_ndc = prev.xy / prev.w;
    let edge = max(abs(prev_ndc.x), abs(prev_ndc.y));
    if edge >= 1.0 { return vec4<f32>(0.0); }
    let uv = vec2<f32>(prev_ndc.x * 0.5 + 0.5, 0.5 - prev_ndc.y * 0.5);
    let footprint = 2.5 * rough * rough * m.t / max(c.w, 0.1) * 0.7 * fsize.y;
    let levels = f32(textureNumLevels(reflection_history));
    let lod = clamp(log2(max(footprint, 1.0)), 0.0, levels - 1.0);
    let lo_lod = floor(lod);
    var radiance = reflection_level(uv, lo_lod);
    if lod > lo_lod { radiance = mix(radiance, reflection_level(uv, lo_lod + 1.0), lod - lo_lod); }
    if any(radiance != radiance) { return vec4<f32>(0.0); }
    var confidence = m.conf * (1.0 - smoothstep(0.82, 1.0, edge));
    confidence *= 1.0 - smoothstep(gi.trace.y * 0.75, gi.trace.y, rough);
    return vec4<f32>(min(radiance, vec3<f32>(65504.0)), confidence);
}

// ---- Stochastic screen-space reflections (sssr.wgsl resolves them) ----
// One ray per pixel and frame, sampled from the GGX distribution of visible
// normals (Heitz 2018). The pixel's surface, specular weight, environment
// fallback and the hit's screen position go to `sssr_out`; the resolve
// pass reuses each ray for its neighbours, weighted by their BRDF over this
// ray's pdf (Stachowiak 2015).
@group(1) @binding(22) var sssr_out: texture_storage_2d<rgba32uint, write>;

fn sssr_on() -> bool { return gi.trace.x > 1.5; }

// PCG hash (Jarzynski and Olano, "Hash Functions for GPU Rendering", 2020).
fn pcg_hash(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

// A GGX visible normal for view direction `v` (towards the eye) about `n`.
fn sample_vndf(v: vec3<f32>, n: vec3<f32>, alpha: f32, xi: vec2<f32>) -> vec3<f32> {
    let t1 = normalize(select(cross(n, vec3<f32>(0.0, 1.0, 0.0)), cross(n, vec3<f32>(1.0, 0.0, 0.0)), abs(n.y) > 0.9));
    let t2 = cross(n, t1);
    let vl = vec3<f32>(dot(v, t1), dot(v, t2), dot(v, n));
    let vh = normalize(vec3<f32>(alpha * vl.x, alpha * vl.y, vl.z));
    let lensq = vh.x * vh.x + vh.y * vh.y;
    let b1 = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(-vh.y, vh.x, 0.0) / sqrt(max(lensq, 1e-12)), lensq > 0.0);
    let b2 = cross(vh, b1);
    let r = sqrt(xi.x);
    let phi = 2.0 * PI * xi.y;
    let s = 0.5 * (1.0 + vh.z);
    let tx = r * cos(phi);
    let ty = mix(sqrt(max(1.0 - tx * tx, 0.0)), r * sin(phi), s);
    let nh = tx * b1 + ty * b2 + sqrt(max(0.0, 1.0 - tx * tx - ty * ty)) * vh;
    let hl = normalize(vec3<f32>(alpha * nh.x, alpha * nh.y, max(0.0, nh.z)));
    return normalize(t1 * hl.x + t2 * hl.y + n * hl.z);
}

// Shared-exponent RGB, as irradiance_volume's emission: 9-bit mantissas.
fn pack_rgb9e5(c: vec3<f32>) -> u32 {
    let m = max(max(c.x, c.y), max(c.z, 1e-9));
    var e = clamp(i32(floor(log2(m))) + 16, 0, 31);
    var scale = exp2(f32(e) - 24.0);
    if round(m / scale) >= 512.0 && e < 31 { e += 1; scale *= 2.0; }
    let q = vec3<u32>(clamp(round(c / scale), vec3<f32>(0.0), vec3<f32>(511.0)));
    return q.x | (q.y << 9u) | (q.z << 18u) | (u32(e) << 27u);
}

// The ray for this pixel: march a VNDF sample and store the surface
// (normal, roughness), its specular weight and environment fallback, and
// the hit (its screen position this frame and the march's confidence).
fn sssr_write(px: vec2<i32>, size: vec2<u32>, p: vec3<f32>, n: vec3<f32>, v: vec3<f32>, rough: f32, weight: vec3<f32>, env: vec3<f32>, dist: f32) {
    let alpha = max(rough * rough, 1e-3);
    // White noise per pixel and frame (PCG): interleaved gradient noise is
    // correlated along diagonals, which the neighbour reuse turned into
    // streaks; uncorrelated samples average out in the resolve and over time.
    let seed = pcg_hash(u32(px.x) ^ pcg_hash(u32(px.y) ^ pcg_hash(u32(light.fog_sampling.x * 16.0) + 0x9e3779b9u)));
    let xi = vec2<f32>(f32(seed >> 8u), f32(pcg_hash(seed) >> 8u)) / 16777216.0;
    let h = sample_vndf(v, n, alpha, xi);
    let l = reflect(-v, h);
    let m = march_hit(p, n, l, px, size, dist);
    var hit_uv = vec2<f32>(0.0);
    var conf = 0.0;
    if m.conf > 0.0 {
        let c = view.view_proj * vec4<f32>(m.hit, 1.0);
        hit_uv = clamp(vec2<f32>(c.x / c.w * 0.5 + 0.5, 0.5 - c.y / c.w * 0.5), vec2<f32>(0.0), vec2<f32>(1.0));
        conf = m.conf;
    }
    let e = oct_encode(n) * 0.5 + 0.5;
    let packed_n = u32(round(e.x * 4095.0)) | (u32(round(e.y * 4095.0)) << 12u) | (u32(round(clamp(rough, 0.0, 1.0) * 255.0)) << 24u);
    // Fade out at the roughness cut, as the single-ray path does.
    let fade = 1.0 - smoothstep(gi.trace.y * 0.75, gi.trace.y, rough);
    textureStore(sssr_out, px, vec4<u32>(packed_n, pack_rgb9e5(max(env, vec3<f32>(0.0))),
        pack4x8unorm(vec4<f32>(clamp(weight, vec3<f32>(0.0), vec3<f32>(1.0)), conf * fade)), pack2x16unorm(hit_uv)));
}

fn vtx(i: u32) -> u32 {
    return i * 8u;
}

fn pos_of(v: u32) -> vec3<f32> {
    return vec3<f32>(vertex_f32(v), vertex_f32(v + 1u), vertex_f32(v + 2u));
}

fn xform(inst: Instance, p: vec3<f32>) -> vec3<f32> {
    let q = vec4<f32>(p, 1.0);
    return vec3<f32>(dot(inst.r0, q), dot(inst.r1, q), dot(inst.r2, q));
}

fn xform_dir(inst: Instance, d: vec3<f32>) -> vec3<f32> {
    if (inst.flags & 32u) != 0u {
        // A scaled assembly root followed by rigid child rotations can have
        // non-orthogonal columns. Match the volume's full affine inverse.
        let ax = vec3<f32>(inst.r0.x, inst.r1.x, inst.r2.x);
        let ay = vec3<f32>(inst.r0.y, inst.r1.y, inst.r2.y);
        let az = vec3<f32>(inst.r0.z, inst.r1.z, inst.r2.z);
        let cx = cross(ay, az);
        let cy = cross(az, ax);
        let cz = cross(ax, ay);
        return (cx * d.x + cy * d.y + cz * d.z) / dot(ax, cx);
    }
    // Instance transforms have orthogonal scaled axes. Normals need the
    // inverse transpose, including non-uniformly scaled trees and rocks.
    let ax = vec3<f32>(inst.r0.x, inst.r1.x, inst.r2.x);
    let ay = vec3<f32>(inst.r0.y, inst.r1.y, inst.r2.y);
    let az = vec3<f32>(inst.r0.z, inst.r1.z, inst.r2.z);
    return ax * d.x / max(dot(ax, ax), 1e-8)
        + ay * d.y / max(dot(ay, ay), 1e-8)
        + az * d.z / max(dot(az, az), 1e-8);
}

// A fixed world-direction catalogue, generated from cubemap cells. Neither
// camera translation nor frame/time enters its seed. Sparse points have an
// angular Gaussian footprint, widened by the pixel footprint while preserving
// integrated energy; tiny stars do not randomly blink between single samples.
fn star_hash(value: u32) -> u32 {
    var h = value;
    h = (h ^ (h >> 16u)) * 0x7feb352du;
    h = (h ^ (h >> 15u)) * 0x846ca68bu;
    return h ^ (h >> 16u);
}
fn star_unit(value: u32) -> f32 { return f32(star_hash(value) >> 8u) / 16777216.0; }

fn night_star_radiance(direction: vec3<f32>) -> vec3<f32> {
    if light.night_stars.x <= 0.0 || direction.y <= 0.015 { return vec3<f32>(0.0); }
    let a = abs(direction);
    var face = 0u;
    var plane: vec2<f32>;
    if a.x >= max(a.y, a.z) {
        face = select(0u, 1u, direction.x < 0.0);
        plane = direction.yz / a.x;
    } else if a.y >= a.z {
        face = select(2u, 3u, direction.y < 0.0);
        plane = direction.xz / a.y;
    } else {
        face = select(4u, 5u, direction.z < 0.0);
        plane = direction.xy / a.z;
    }
    let cell = min(vec2<u32>((plane * 0.5 + 0.5) * 64.0), vec2<u32>(63u));
    let id = (face * 4096u + cell.y * 64u + cell.x) ^ u32(light.night_stars.w);
    // Cubemap solid angle correction avoids denser star clusters at corners.
    let cell_plane = ((vec2<f32>(cell) + 0.5) / 64.0) * 2.0 - 1.0;
    let solid_angle = pow(1.0 + dot(cell_plane, cell_plane), -1.5);
    if star_unit(id) > 0.085 * solid_angle { return vec3<f32>(0.0); }
    let jitter = vec2<f32>(star_unit(id ^ 0x68bc21ebu), star_unit(id ^ 0x02e5be93u));
    // Leave an empty margin at cell edges. The tiny Gaussian support lies
    // inside its cell at normal render sizes, including split-screen input.
    let q = ((vec2<f32>(cell) + 0.25 + 0.5 * jitter) / 64.0) * 2.0 - 1.0;
    let sign = select(1.0, -1.0, (face & 1u) != 0u);
    var centre = vec3<f32>(sign, q.x, q.y);
    if face >= 4u { centre = vec3<f32>(q.x, q.y, sign); }
    else if face >= 2u { centre = vec3<f32>(q.x, sign, q.y); }
    let delta = direction - normalize(centre);
    let angular_sigma = 0.00032;
    let pixel_angle = 1.0 / max(view.cam.w, 1.0);
    let variance = angular_sigma * angular_sigma + pixel_angle * pixel_angle / 12.0;
    let footprint = exp(-0.5 * dot(delta, delta) / variance)
        * angular_sigma * angular_sigma / variance;
    let magnitude = 0.06 + 0.94 * pow(star_unit(id ^ 0x967a889bu), 6.0);
    let colour = mix(vec3<f32>(0.72, 0.84, 1.0), vec3<f32>(1.0, 0.86, 0.72), star_unit(id ^ 0x368cc8b7u));
    let transmission = exp(-light.night_stars.y / max(direction.y, 0.05))
        * smoothstep(0.015, 0.12, direction.y);
    return colour * light.night_stars.x * magnitude * footprint * transmission;
}

// Background only: opaque geometry naturally hides the solar disc. The
// environment bake remains disc-free, avoiding double-counted illumination.
fn solar_disc(direction: vec3<f32>) -> vec3<f32> {
    if light.anim.w == 0.0 || light.sun_dir.w <= 0.0 || light.sun_dir.y <= 0.0 { return vec3<f32>(0.0); }
    let radius = max(light.anim.w, 0.0001);
    let pixel = 1.0 / max(view.cam.w, 1.0);
    let angle = length(direction - light.sun_dir.xyz);
    let disc = 1.0 - smoothstep(max(radius - pixel * 0.7, 0.0), radius + pixel * 0.7, angle);
    // Preserve a finite fp16 HDR buffer, including at extreme solar luminance.
    let radiance = min(light.sun_color.rgb * light.sun_dir.w / (PI * radius * radius), vec3<f32>(60000.0));
    return radiance * disc * clamp(view.fog_cull.z, 0.0, 1.0);
}

fn visible_background_radiance(direction: vec3<f32>) -> vec3<f32> {
    return attenuate_background(direction, min(background_sky(direction) + night_star_radiance(direction) + solar_disc(direction), vec3<f32>(65504.0)));
}

// Irradiance (lux) from the sky for a surface facing n.
// Indoor irradiance probes (irradiance_volume.rs): an ambient cube per grid
// point in `aux`. Returns irradiance and a weight, 1 inside the grid, fading
// to 0 over the outer half cell, 0 without a volume.
fn aux_f32(i: u32) -> f32 { return bitcast<f32>(aux_word(i)); }

// Bullet dents (dents.rs): the aux offset of their grid, 0 for none.
fn view_hf_dents() -> u32 { return gi.hf_dims.w; }

// The craters around p on a ROCK face with geometric normal `face`: a
// ragged paraboloid bowl per dent bends `normal` towards its centre and
// darkens the inside (occlusion), with a paler rim of fresh stone.
// Returns (normal, albedo multiplier).
fn rock_dents(p: vec3<f32>, face: vec3<f32>, normal: vec3<f32>) -> vec4<f32> {
    let o = view_hf_dents();
    let buckets = aux_word(o);
    let index_cap = aux_word(o + 2u);
    let cell_m = aux_f32(o + 3u);
    let c = vec3<i32>(floor(p / cell_m));
    let h = (u32(c.x) * 73856093u) ^ (u32(c.y) * 19349663u) ^ (u32(c.z) * 83492791u);
    let entry = aux_word(o + 4u + (h & (buckets - 1u)));
    let count = min(entry & 255u, 15u);
    if count == 0u { return vec4<f32>(normal, 1.0); }
    var nn = normal;
    var dim = 1.0;
    let list = o + 4u + buckets + (entry >> 8u);
    let dent_base = o + 4u + buckets + index_cap;
    for (var k = 0u; k < count; k++) {
        let i = aux_word(list + k);
        let q = dent_base + i * 4u;
        let centre = vec3<f32>(aux_f32(q), aux_f32(q + 1u), aux_f32(q + 2u));
        let r = aux_f32(q + 3u);
        let v = p - centre;
        let along = dot(v, face);
        if abs(along) > r * 0.6 { continue; }
        let t = v - face * along;
        let d = length(t);
        if d >= r * 1.25 { continue; }
        // Ragged outline: the edge wobbles with the angle around the centre.
        let tx = safe_normalize(cross(face, select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(face.y) > 0.9)));
        let ty = cross(face, tx);
        let a = atan2(dot(t, ty), dot(t, tx));
        let seed = f32(i % 97u) * 1.7;
        let edge = r * (0.82 + 0.13 * sin(a * 3.0 + seed) + 0.08 * sin(a * 7.0 + seed * 2.3) + 0.05 * sin(a * 13.0 + seed * 0.7));
        let x = d / edge;
        if x < 1.0 {
            // Bowl depth 45% of the radius; its slope tips the normal inwards.
            let depth = 0.45;
            let slope = 2.0 * depth * x;
            let inward = select(vec3<f32>(0.0), -t / d, d > 1e-5);
            nn = safe_normalize(nn + inward * slope * 1.6);
            // Pits inside the bowl: chipped facets.
            let grain = sin(a * 5.0 + x * 9.0 + seed) * 0.5 + 0.5;
            dim *= mix(0.42, 1.0, x * x) * mix(0.9, 1.05, grain);
        } else if x < 1.25 {
            // The rim of freshly broken stone, paler than the weathered face.
            let rim = 1.0 - abs(x - 1.08) / 0.17;
            dim *= 1.0 + 0.22 * clamp(rim, 0.0, 1.0);
        }
    }
    return vec4<f32>(nn, dim);
}
// Visibility stores the same scalar in RGB; avoid fetching its duplicate channels.
fn probe_axis(base: u32, visibility: bool) -> vec3<f32> {
    if visibility { return vec3<f32>(aux_f32(base)); }
    return vec3<f32>(aux_f32(base),aux_f32(base+1u),aux_f32(base+2u));
}
fn unpack_sky_transport(word:u32)->vec3<f32> {
    // 9-bit mantissas, shared 4-bit exponent (irradiance_volume::pack_transport).
    return vec3<f32>(f32(word&511u),f32((word>>9u)&511u),f32((word>>18u)&511u))*exp2(f32((word>>27u)&15u)-23.0);
}
struct IrradianceSample { value: vec4<f32>, emission: vec3<f32> }
// The last visibility-tested lookup's coverage: the share of its trilinear
// weight (over probes outside geometry) whose probes see the surface.
var<private> irradiance_coverage: f32;
fn unpack_emission(word: u32) -> vec3<f32> {
    return vec3<f32>(f32(word&511u),f32((word>>9u)&511u),f32((word>>18u)&511u))*exp2(f32(word>>27u)-24.0);
}
// Mode 8 contains a directory of separately streamed world-space mode-7
// grids. The sample, its wall planes and its probes all use the same space.
// A moving skinned mesh can straddle an open doorway. The room's authored
// wall plane is a streaming boundary, while the diffuse field changes over
// the extent of a body. Keep fixed room surfaces on their exact side.
var<private> irradiance_receiver_radius: f32;
var<private> irradiance_boundary_weight: f32;
fn irradiance_grid_at(p: vec3<f32>) -> u32 {
    irradiance_boundary_weight = 1.0;
    let root=u32(gi.params.w);
    if !FEATURE_IRRADIANCE || root==0u { return 0u; }
    if aux_word(root+7u)!=8u { return root; }
    let rooms=aux_word(root);
    var best=0.0;
    var found=0u;
    for (var i=0u; i<rooms; i++) {
        let at=root+8u+8u*i;
        let grid=aux_word(at+3u);
        if grid==0u { continue; }
        let low=vec3<f32>(aux_f32(at),aux_f32(at+1u),aux_f32(at+2u));
        let high=vec3<f32>(aux_f32(at+4u),aux_f32(at+5u),aux_f32(at+6u));
        if irradiance_receiver_radius <= 0.0 {
            if all(p>=low) && all(p<=high) { return grid; }
        } else if p.y >= low.y && p.y <= high.y {
            let sides=min(p.xz-low.xz,high.xz-p.xz);
            let w=smoothstep(-irradiance_receiver_radius,irradiance_receiver_radius,min(sides.x,sides.y));
            if w>best { best=w; found=grid; }
        }
    }
    irradiance_boundary_weight=best;
    return found;
}

fn irradiance_sample(p: vec3<f32>, n: vec3<f32>) -> IrradianceSample {
    return irradiance_sample_at(irradiance_grid_at(p),p,p,n);
}
// How far open space joins the probe at grid point `at` to `x` within its
// neighbourhood (room_bake::bake_reach: one bit per voxel of the cells
// around the grid point, four voxels per cell, stored after the offsets),
// interpolated trilinearly between voxel centres so the weight changes
// over a voxel instead of switching at its face.
fn probe_reaches(index: u32, at: vec3<f32>, x: vec3<f32>, dims: vec3<u32>, cell: f32) -> f32 {
    let f = (x - at + cell) / (cell * 0.25) - 0.5;
    let base = vec3<i32>(floor(f));
    let t = f - floor(f);
    let words = u32(gi.trace.w) + dims.x * dims.y * dims.z * 65u + index * 16u;
    var sum = 0.0;
    // A row of eight voxels along x is one byte of a word (bit x + 8 (y % 4),
    // word y / 4 + 2 z): the two x neighbours come from one load.
    for (var c = 0u; c < 4u; c++) {
        let y = base.y + i32(c & 1u);
        let z = base.z + i32(c >> 1u);
        if y < 0 || y >= 8 || z < 0 || z >= 8 { continue; }
        let row = (aux_word(words + u32(y / 4 + 2 * z)) >> (8u * u32(y % 4))) & 255u;
        var r = 0.0;
        if base.x >= 0 && base.x < 8 { r += f32((row >> u32(base.x)) & 1u) * (1.0 - t.x); }
        if base.x + 1 >= 0 && base.x + 1 < 8 { r += f32((row >> u32(base.x + 1)) & 1u) * t.x; }
        sum += r * select(1.0 - t.y, t.y, (c & 1u) != 0u) * select(1.0 - t.z, t.z, (c >> 1u) != 0u);
    }
    return sum;
}
// How probe `index`'s three values (q0 0: the sky transport, 3: the
// emission) change over `dp` from its grid point, for the three axes the
// normal picks (`axis`, three words per axis) weighted by n2: its gradients
// (irradiance_volume::pack_gradients, half floats, 54 words per probe).
fn probe_slope(gradients: u32, index: u32, axis: vec3<u32>, q0: u32, n2: vec3<f32>, dp: vec3<f32>) -> vec3<f32> {
    let base = gradients + 1u + index * 54u;
    var out = vec3<f32>(0.0);
    for (var k = 0u; k < 3u; k++) {
        // An axis the normal does not face adds nothing (a wall faces one).
        if n2[k] <= 0.0 { continue; }
        let a = axis[k] / 3u;
        var r = vec3<f32>(0.0);
        for (var j = 0u; j < 9u; j++) {
            let h = (a * 6u + q0) * 3u + j;
            let pair = unpack2x16float(aux_word(base + h / 2u));
            r[j / 3u] += select(pair.x, pair.y, (h & 1u) == 1u) * dp[j % 3u];
        }
        out += r * n2[k];
    }
    return out;
}
// A probe's offset from its grid point (room_bake::pack_offsets), stored
// after the visibility maps: three 10-bit signed fractions of half a cell.
fn probe_offset(index: u32, dims: vec3<u32>, cell: f32) -> vec3<f32> {
    let w = aux_word(u32(gi.trace.w) + dims.x * dims.y * dims.z * 64u + index);
    let q = vec3<f32>(f32(w & 1023u), f32((w >> 10u) & 1023u), f32((w >> 20u) & 1023u));
    return (q - 512.0) / 511.0 * (0.5 * cell);
}

// DDGI-style probe visibility (room_bake::bake_visibility): per probe an
// 8x8 octahedral map of the mean and mean squared distance to the nearest
// surface, in aux at gi.trace.w (0: none). How much probe `index` at `probe`
// sees the point `x`: 1 in front of the nearest surface along that
// direction, falling off by Chebyshev's bound behind it (cubed, as DDGI).
fn probe_sees(index: u32, probe: vec3<f32>, x: vec3<f32>) -> f32 {
    let d = x - probe;
    let dist = length(d);
    if dist < 1e-4 { return 1.0; }
    let dir = d / dist;
    var e = dir.xy / (abs(dir.x) + abs(dir.y) + abs(dir.z));
    if dir.z < 0.0 {
        e = (1.0 - abs(e.yx)) * select(vec2<f32>(-1.0), vec2<f32>(1.0), e >= vec2<f32>(0.0));
    }
    // Bilinear over the 8x8 map (texel centres; clamped at its border): a
    // nearest texel showed its facets as a sawtooth edge on the walls.
    let st = clamp(e * 4.0 + 3.5, vec2<f32>(0.0), vec2<f32>(7.0));
    let t0 = vec2<u32>(min(floor(st), vec2<f32>(6.0)));
    let fr = st - vec2<f32>(t0);
    let at = u32(gi.trace.w) + index * 64u + t0.y * 8u + t0.x;
    let m = mix(mix(unpack2x16float(aux_word(at)), unpack2x16float(aux_word(at + 1u)), fr.x),
        mix(unpack2x16float(aux_word(at + 8u)), unpack2x16float(aux_word(at + 9u)), fr.x), fr.y);
    if m.x <= 0.0 { return 0.0; }
    if dist <= m.x { return 1.0; }
    let variance = abs(m.y - m.x * m.x) + 1e-4;
    let behind = dist - m.x;
    let p = variance / (variance + behind * behind);
    return p * p * p;
}


// `irradiance_sample` with the trilinear weights taken at `p` and the wall
// tests (backface, separation planes) at `surface`: a lookup point moved
// along the surface (jitter) must not validate probes behind a wall the
// move crossed.
fn irradiance_sample_at(o: u32, p: vec3<f32>, surface: vec3<f32>, n: vec3<f32>) -> IrradianceSample {
    // Plane tests blend over 15 cm, but only 2 cm for a moved lookup: its
    // weights reach probes the blend would let through at a corner.
    let soft=select(0.15,0.02,any(p!=surface));
    let ddgi=gi.trace.w>0.5;
    if !FEATURE_IRRADIANCE || o == 0u { return IrradianceSample(vec4<f32>(0.0),vec3<f32>(0.0)); }
    let transport = aux_word(o + 7u) >= 2u;
    let sun_transport = aux_word(o + 7u) == 3u;
    let visibility = aux_word(o + 7u) >= 1u;
    let origin = vec3<f32>(aux_f32(o), aux_f32(o + 1u), aux_f32(o + 2u));
    let cell = aux_f32(o + 3u);
    let dims = vec3<u32>(aux_word(o + 4u), aux_word(o + 5u), aux_word(o + 6u));
    let f = (p - origin) / cell;
    let last = vec3<f32>(dims - vec3<u32>(1u));
    let outside = max(max(-f, f - last), vec3<f32>(0.0));
    let weight = irradiance_boundary_weight * clamp(1.0 - max(outside.x, max(outside.y, outside.z)) * 2.0, 0.0, 1.0);
    if weight <= 0.0 { return IrradianceSample(vec4<f32>(0.0),vec3<f32>(0.0)); }
    let fc = clamp(f, vec3<f32>(0.0), last);
    let cell0 = min(floor(fc), last - 1.0);
    let t = fc - cell0;
    let n2 = n * n;
    let axis = vec3<u32>(select(1u, 0u, n.x >= 0.0), select(3u, 2u, n.y >= 0.0), select(5u, 4u, n.z >= 0.0)) * 3u;
    if visibility {
        // Trilinear over the cell's eight probes. A probe counts only if it
        // is on the shaded side: in front of the surface (backface weight,
        // as DDGI does) and, with separation planes, on the same side of its
        // nearest wall. Without that a point in a room's corner blends in
        // the probes outside the walls and glows with outdoor light.
        var value=vec3<f32>(0.0);
        var emission=vec3<f32>(0.0);
        var valid_weight=0.0;
        // Without the separation planes, as a fallback: a gap behind
        // furniture can have every probe across some plane.
        var loose=vec3<f32>(0.0);
        var loose_emission=vec3<f32>(0.0);
        var loose_weight=0.0;
        let mode=aux_word(o+7u);
        // Probe gradients (irradiance_volume::pack_gradients), after the
        // reach masks: header 1 when uploaded.
        let gradients=u32(gi.trace.w)+dims.x*dims.y*dims.z*81u;
        let smooth_weights=ddgi&&!GI_PHONE&&aux_word(gradients)==1u;
        let ts=t*t*(3.0-2.0*t);
        for(var c=0u;c<8u;c++) {
            let corner=vec3<u32>(c&1u,(c>>1u)&1u,(c>>2u)&1u);
            // With gradients C1 weights (each probe's value is continued
            // along its gradient, below), else trilinear.
            let w3=select(1.0-select(t,ts,smooth_weights),select(t,ts,smooth_weights),corner==vec3<u32>(1u));
            let g=vec3<u32>(cell0)+corner;
            // The probe where it was baked: its grid point, moved out of
            // geometry (room_bake::probe_positions) when visibility is on.
            let probe=origin+vec3<f32>(g)*cell+select(vec3<f32>(0.0),probe_offset(g.x+g.y*dims.x+g.z*dims.x*dims.y,dims,cell),ddgi);
            let to_probe=probe-surface;
            let facing=dot(to_probe,n)/max(length(to_probe),1e-4);
            // Probes just behind the surface still count a little (a wall's
            // own probes sit in it); far behind it they do not.
            let backface=clamp(facing*2.0+1.0,0.0,1.0);
            var blend_weight=w3.x*w3.y*w3.z*(backface*backface+1e-4);
            let base=o+8u+(g.x+g.y*dims.x+g.z*dims.x*dims.y)*18u;
            let index=g.x+g.y*dims.x+g.z*dims.x*dims.y;
            if ddgi {
                // DDGI: a smooth backface weight and the probe's own view of
                // the (normal-biased) point instead of per-cell wall tests.
                let wrap=(facing+1.0)*0.5;
                blend_weight=w3.x*w3.y*w3.z*(wrap*wrap+0.2);
            }
            if mode>=4u {
                var valid=select(1.0,0.0,(aux_word(base)&0x80000000u)!=0u);
                // With visibility, the fallback below uses only probes that reach
                // the point through open space (a probe outdoors behind the wall
                // must not light a room whatever else fails).
                if ddgi && !GI_PHONE { valid*=probe_reaches(index,origin+vec3<f32>(g)*cell,surface+n*(0.3*cell),dims,cell); }
                let inside=valid;
                if ddgi {
                    if !GI_PHONE { valid*=probe_sees(index,probe,surface+n*(0.3*cell)); }
                } else if mode==5u {
                    let plane=vec4<f32>(aux_f32(base+1u),aux_f32(base+2u),aux_f32(base+4u),aux_f32(base+5u));
                    // Zero plane passes through. Blend over 15 cm to avoid a hard edge.
                    valid*=smoothstep(-soft,0.0,dot(plane.xyz,surface)+plane.w);
                }
                if mode==7u {
                    let plane=vec4<f32>(aux_f32(base+2u),aux_f32(base+5u),aux_f32(base+8u),aux_f32(base+11u));
                    if !ddgi { valid*=smoothstep(-soft,0.0,dot(plane.xyz,surface)+plane.w); }
                    var e=unpack_emission(aux_word(base+axis.x+1u))*n2.x
                        +unpack_emission(aux_word(base+axis.y+1u))*n2.y
                        +unpack_emission(aux_word(base+axis.z+1u))*n2.z;
                    if smooth_weights && blend_weight*inside > 0.0 { e+=probe_slope(gradients,index,axis,3u,n2,p-(origin+vec3<f32>(g)*cell)); }
                    emission+=e*blend_weight*valid;
                    loose_emission+=e*blend_weight*inside;
                }
                var v=unpack_sky_transport(aux_word(base+axis.x))*n2.x
                    +unpack_sky_transport(aux_word(base+axis.y))*n2.y
                    +unpack_sky_transport(aux_word(base+axis.z))*n2.z;
                // Only for a probe that counts (those behind a wall do not).
                if smooth_weights && blend_weight*inside > 0.0 { v+=probe_slope(gradients,index,axis,0u,n2,p-(origin+vec3<f32>(g)*cell)); }
                value+=v*blend_weight*valid;
                valid_weight+=blend_weight*valid;
                loose+=v*blend_weight*inside;
                loose_weight+=blend_weight*inside;
                continue;
            }
            let v=vec3<f32>(aux_f32(base+axis.x),aux_f32(base+axis.y),aux_f32(base+axis.z));
            var valid=1.0; var bounce_value=0.0; var sun_value=0.0;
            if transport {
                valid=aux_f32(base+2u);
                if sun_transport {
                    valid=select(0.0,1.0,valid<1.5);
                    sun_value=dot(vec3<f32>(aux_f32(base+axis.x+2u),aux_f32(base+axis.y+2u),aux_f32(base+axis.z+2u)),n2);
                }
                let b=vec3<f32>(aux_f32(base+axis.x+1u),aux_f32(base+axis.y+1u),aux_f32(base+axis.z+1u));
                bounce_value=dot(b,n2);
            }
            value+=vec3<f32>(dot(v,n2),bounce_value,sun_value)*blend_weight*valid;
            valid_weight+=blend_weight*valid;
        }
        irradiance_coverage=select(0.0,valid_weight/loose_weight,loose_weight>0.0);
        if valid_weight<1e-3*max(loose_weight,1e-5) {
            // Every probe is across a wall: use those merely outside
            // geometry, and if none is, call the point unlit (inside a
            // probe volume dark is the safe guess, open sky the worst).
            // A moved (jittered) lookup says so instead (weight -1): the
            // caller looks up the unmoved point, where the walls hold.
            if any(p!=surface) { return IrradianceSample(vec4<f32>(0.0,0.0,0.0,-1.0),vec3<f32>(0.0)); }
            if loose_weight<1e-5 { return IrradianceSample(vec4<f32>(0.0,0.0,0.0,weight),vec3<f32>(0.0)); }
            value=loose; emission=loose_emission; valid_weight=loose_weight;
        }
        value/=max(dot(n2,vec3<f32>(1.0)),1e-12)*valid_weight;
        return IrradianceSample(vec4<f32>(value,weight),emission/(max(dot(n2,vec3<f32>(1.0)),1e-12)*valid_weight));
    }
    var e = vec3<f32>(0.0);
    for (var c = 0u; c < 8u; c++) {
        let corner = vec3<u32>(c & 1u, (c >> 1u) & 1u, (c >> 2u) & 1u);
        let w3 = select(1.0 - t, t, corner == vec3<u32>(1u));
        let g = vec3<u32>(cell0) + corner;
        let base = o + 8u + (g.x + g.y * dims.x + g.z * dims.x * dims.y) * 18u;
        let ex = probe_axis(base + axis.x, visibility);
        let ey = probe_axis(base + axis.y, visibility);
        let ez = probe_axis(base + axis.z, visibility);
        e += (ex * n2.x + ey * n2.y + ez * n2.z) * (w3.x * w3.y * w3.z);
    }
    return IrradianceSample(vec4<f32>(e / max(n2.x + n2.y + n2.z, 1e-12), weight),vec3<f32>(0.0));
}

fn irradiance_volume(p: vec3<f32>,n: vec3<f32>) -> vec4<f32> { return irradiance_sample(p,n).value; }

// A per-pixel, per-frame offset of the probe lookup in the surface's plane,
// up to 0.35 of a cell (with TAA only): trilinear interpolation over 0.5 m
// probes shows the cells as steps and creases where neighbouring probes
// differ; TAA averages the jittered lookups into a smooth, wider filter at
// no extra fetches. The offset stays in the plane and well under the wall
// thickness the probe layout asks for, and the wall tests in
// irradiance_sample run at the offset point.
// The jittered lookup: trilinear weights at the moved point, the wall tests
// at the surface point. Near a corner the offset can cross into the
// neighbouring wall; tested there, probes on that wall's far side (outdoors,
// the next room) would pass and the edge showed as a bright dotted line.
fn jittered_irradiance(p: vec3<f32>, px: vec2<i32>, visibility_n: vec3<f32>, n: vec3<f32>) -> IrradianceSample {
    // A move into another probe cell is kept only if none of that cell's
    // probes is inside geometry; otherwise it stays in the plain point's
    // cell (the same eight probes, only their weights move). At a corner the
    // next cell holds the adjoining wall's probes and those beyond it, which
    // showed as a bright dotted line along every edge of a room; in open
    // space the move still blurs the cell steps.
    let o = irradiance_grid_at(p);
    if o == 0u { return IrradianceSample(vec4<f32>(0.0),vec3<f32>(0.0)); }
    var q = p + irradiance_jitter(o, px, visibility_n);
    if FEATURE_IRRADIANCE && o != 0u && aux_word(o + 7u) >= 4u && gi.trace.w <= 0.5 {
        let origin = vec3<f32>(aux_f32(o), aux_f32(o + 1u), aux_f32(o + 2u));
        let cell = aux_f32(o + 3u);
        let dims = vec3<u32>(aux_word(o + 4u), aux_word(o + 5u), aux_word(o + 6u));
        let last = vec3<f32>(dims - vec3<u32>(1u));
        let c = min(floor(clamp((p - origin) / cell, vec3<f32>(0.0), last)), last - 1.0);
        let cq = min(floor(clamp((q - origin) / cell, vec3<f32>(0.0), last)), last - 1.0);
        if any(cq != c) {
            // Any probe inside geometry in the next cell: stay in this one.
            var buried = false;
            for (var k = 0u; k < 8u; k++) {
                let g = vec3<u32>(cq) + vec3<u32>(k & 1u, (k >> 1u) & 1u, (k >> 2u) & 1u);
                if (aux_word(o + 8u + (g.x + g.y * dims.x + g.z * dims.x * dims.y) * 18u) & 0x80000000u) != 0u { buried = true; }
            }
            if buried { q = origin + clamp((q - origin) / cell, c + 0.001, c + 0.999) * cell; }
        }
    }
    irradiance_coverage = 1.0;
    let moved = irradiance_sample_at(o, q, p, n);
    if moved.value.w < 0.0 { return irradiance_sample_at(o, p, p, n); }
    if gi.trace.w > 0.5 && any(q != p) {
        // DDGI: the moved lookup speaks for p only with the share of its
        // weight whose probes see p; renormalised, a move through a wall
        // (a corner's next wall, the moved cell holding only the probes
        // outdoors) would turn their faint visibility into full daylight.
        // The rest comes from the unmoved lookup, chosen stochastically
        // (TAA averages it). On an open wall the share is 1: the blur stays
        // symmetric and cell boundaries leave no trace.
        if trace_hash(px, light.fog_sampling.x * 16.0 + 29.0).x >= irradiance_coverage { return irradiance_sample_at(o, p, p, n); }
    }
    return moved;
}

fn irradiance_jitter(o: u32, px: vec2<i32>, n: vec3<f32>) -> vec3<f32> {
    if !FEATURE_IRRADIANCE || o == 0u || light.fog_sampling.y < 0.5 { return vec3<f32>(0.0); }
    let cell = aux_f32(o + 3u);
    let xi = trace_hash(px, light.fog_sampling.x * 16.0 + 11.0) * 2.0 - 1.0;
    let t1 = normalize(select(cross(n, vec3<f32>(0.0, 1.0, 0.0)), cross(n, vec3<f32>(1.0, 0.0, 0.0)), abs(n.y) > 0.9));
    let t2 = cross(n, t1);
    // Streamed rooms can use metre-scale cells and a single coarse preview.
    // Keep the temporal footprint near the receiving surface instead of
    // randomly sampling across a doorway or an entire ceiling panel.
    var radius = 0.35 * cell;
    if aux_word(u32(gi.params.w) + 7u) == 8u { radius = min(radius, 0.18); }
    return (t1 * xi.x + t2 * xi.y) * radius;
}

fn sky_irradiance(n: vec3<f32>) -> vec3<f32> {
    let c1 = 0.429043;
    let c2 = 0.511664;
    let c3 = 0.743125;
    let c4 = 0.886227;
    let c5 = 0.247708;
    let s = light.sh;
    let e = c1 * s[8].rgb * (n.x * n.x - n.y * n.y) + c3 * s[6].rgb * n.z * n.z + c4 * s[0].rgb - c5 * s[6].rgb
        + 2.0 * c1 * (s[4].rgb * n.x * n.y + s[7].rgb * n.x * n.z + s[5].rgb * n.y * n.z)
        + 2.0 * c2 * (s[3].rgb * n.x + s[1].rgb * n.y + s[2].rgb * n.z);
    return max(e, vec3<f32>(0.0));
}

// Explicit 4x4 footprint equals the old 3x3 bilinear comparison kernel.
// Each comparison uses the actual receiving triangle's depth at THAT texel,
// instead of pushing every possible caster away by its unbounded slope.
var<private> sun_receiver_tri: array<vec3<f32>,3>;
var<private> sun_receiver_tri_set: bool;
var<private> sun_receiver_error: f32;
// Metres along the sun by which the shaded skin triangle, being flat, can
// lie behind the rounded surface its vertex normals describe: its longest
// edge times how far its own facing of the sun departs from its vertices'.
// Skin in the sun's way within this is the mesh's facets, not the person.
var<private> skin_facet_slack: f32;
// Half of what that triangle spans along the sun, metres.
var<private> skin_facet_reach: f32;
fn shadow_cross2(a:vec2<f32>,b:vec2<f32>)->f32 { return a.x*b.y-a.y*b.x; }
fn shadow_triangle_contains(q:vec2<f32>,a:vec2<f32>,b:vec2<f32>,c:vec2<f32>,orientation:f32)->bool {
    return shadow_cross2(b-a,q-a)*orientation>=0.0
        && shadow_cross2(c-b,q-b)*orientation>=0.0
        && shadow_cross2(a-c,q-c)*orientation>=0.0;
}
fn shadow_filter_plane(map: texture_depth_2d, projection: mat4x4<f32>, world_texel: f32, p: vec3<f32>, plane_n: vec3<f32>) -> f32 {
    let c = projection * vec4<f32>(p, 1.0);
    let uv = c.xy * vec2<f32>(0.5, -0.5) + 0.5;
    if !all(abs(c) < vec4<f32>(1e20)) || any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || c.z < 0.0 || c.z > 1.0 { return 1.0; }
    let size = vec2<i32>(textureDimensions(map));
    let q = uv * vec2<f32>(size) - 0.5;
    let base = vec2<i32>(floor(q));
    let f = fract(q);
    let wx = vec4<f32>(1.0-f.x, 1.0, 1.0, f.x) / 3.0;
    let wy = vec4<f32>(1.0-f.y, 1.0, 1.0, f.y) / 3.0;
    let row_x = vec3<f32>(projection[0].x, projection[1].x, projection[2].x);
    let row_y = vec3<f32>(projection[0].y, projection[1].y, projection[2].y);
    let row_z = vec3<f32>(projection[0].z, projection[1].z, projection[2].z);
    let denominator = dot(plane_n, -normalize(row_z));
    var gradient = vec2<f32>(0.0);
    // Zero means no actual triangle plane was supplied (volume/transparency).
    // A nearly sun-parallel plane has no stable depth function; use a centred
    // comparison, never an arbitrary metre-scale bias or unsafe division.
    if all(abs(plane_n) < vec3<f32>(1e10)) && dot(plane_n,plane_n) > 1e-12
        && abs(denominator) > 1e-4 && dot(row_x,row_x) > 1e-20 && dot(row_y,row_y) > 1e-20 {
        gradient = vec2<f32>(dot(plane_n,normalize(row_x)), -dot(plane_n,normalize(row_y)))
            * (world_texel * length(row_z) / denominator);
    }
    // f32 projection roundoff, in normalized depth. Unlike slope bias this
    // grows only with arithmetic precision, not the receiving/casting slope.
    let epsilon = 4.7683716e-7 * (dot(abs(row_z),abs(p)) + abs(projection[3].z) + 1.0);
    let clipped = sun_receiver_tri_set && dot(plane_n,plane_n)>1e-12;
    var triangle:array<vec2<f32>,3>;
    var orientation=0.0;
    if clipped {
        for(var i=0;i<3;i++) {
            let vertex=projection*vec4<f32>(sun_receiver_tri[i],1.0);
            triangle[i]=(vertex.xy*vec2<f32>(0.5,-0.5)+0.5)*vec2<f32>(size)-0.5;
        }
        orientation=sign(shadow_cross2(triangle[1]-triangle[0],triangle[2]-triangle[0]));
    }
    var lit = 0.0;
    var total = 0.0;
    for (var y=0; y<4; y++) { for (var x=0; x<4; x++) {
        let pixel = clamp(base + vec2<i32>(x-1,y-1), vec2<i32>(0), size-1);
        // A plane is valid only inside its actual triangle. Extrapolating it
        // into a neighbouring concave face creates false triangle-wire shadow
        // even with identical source geometry and infinite numeric precision.
        if clipped && (orientation==0.0 || !shadow_triangle_contains(vec2<f32>(pixel),triangle[0],triangle[1],triangle[2],orientation)) { continue; }
        let delta = vec2<f32>(pixel) - q;
        let reference = c.z + dot(gradient,delta) - epsilon;
        let depth = textureLoad(map,pixel,0);
        let weight=wx[x]*wy[y];
        lit += weight * select(0.0,1.0,reference <= depth);
        total += weight;
    }}
    if total>1e-8 {return lit/total;}
    // Subtexel / edge-on triangles may contain no shadow-map sample centre.
    // Fall back to ONE nearest depth sample at the actual receiver depth.
    // This has <=half-texel quantization per axis and may alias; it cannot
    // invent an arbitrarily extrapolated plane or force an unoccluded result.
    let nearest=clamp(vec2<i32>(floor(q+0.5)),vec2<i32>(0),size-1);
    return select(0.0,1.0,c.z-epsilon<=textureLoad(map,nearest,0));
}

fn shadow_filter(map: texture_depth_2d, projection: mat4x4<f32>, world_texel: f32, p: vec3<f32>, n: vec3<f32>, plane_n: vec3<f32>) -> f32 {
    if light.near_shadow.w > 0.5 { return shadow_filter_plane(map,projection,world_texel,p,plane_n); }
    let biased = p + n * world_texel * 1.5 + light.sun_dir.xyz * world_texel;
    let c = projection * vec4<f32>(biased, 1.0);
    let uv = vec2<f32>(c.x * 0.5 + 0.5, 0.5 - c.y * 0.5);
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || c.z < 0.0 || c.z > 1.0 {
        return 1.0;
    }
    let texel = 1.0 / vec2<f32>(textureDimensions(map));
    var lit = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            lit += textureSampleCompareLevel(map, shadow_samp, uv + vec2<f32>(f32(x), f32(y)) * texel, c.z);
        }
    }
    return lit / 9.0;
}

// The same on a sun clipmap level (near_shadow_tex layer `layer`).
fn shadow_filter_plane_layer(map: texture_depth_2d_array, layer: i32, projection: mat4x4<f32>, world_texel: f32, p: vec3<f32>, plane_n: vec3<f32>) -> f32 {
    let c = projection * vec4<f32>(p, 1.0);
    let uv = c.xy * vec2<f32>(0.5, -0.5) + 0.5;
    if !all(abs(c) < vec4<f32>(1e20)) || any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || c.z < 0.0 || c.z > 1.0 { return 1.0; }
    let size = vec2<i32>(textureDimensions(map));
    let q = uv * vec2<f32>(size) - 0.5;
    let base = vec2<i32>(floor(q));
    let f = fract(q);
    let wx = vec4<f32>(1.0-f.x, 1.0, 1.0, f.x) / 3.0;
    let wy = vec4<f32>(1.0-f.y, 1.0, 1.0, f.y) / 3.0;
    let row_x = vec3<f32>(projection[0].x, projection[1].x, projection[2].x);
    let row_y = vec3<f32>(projection[0].y, projection[1].y, projection[2].y);
    let row_z = vec3<f32>(projection[0].z, projection[1].z, projection[2].z);
    let denominator = dot(plane_n, -normalize(row_z));
    var gradient = vec2<f32>(0.0);
    // Zero means no actual triangle plane was supplied (volume/transparency).
    // A nearly sun-parallel plane has no stable depth function; use a centred
    // comparison, never an arbitrary metre-scale bias or unsafe division.
    if all(abs(plane_n) < vec3<f32>(1e10)) && dot(plane_n,plane_n) > 1e-12
        && abs(denominator) > 1e-4 && dot(row_x,row_x) > 1e-20 && dot(row_y,row_y) > 1e-20 {
        gradient = vec2<f32>(dot(plane_n,normalize(row_x)), -dot(plane_n,normalize(row_y)))
            * (world_texel * length(row_z) / denominator);
    }
    // f32 projection roundoff, in normalized depth. Unlike slope bias this
    // grows only with arithmetic precision, not the receiving/casting slope.
    let epsilon = 4.7683716e-7 * (dot(abs(row_z),abs(p)) + abs(projection[3].z) + 1.0);
    let clipped = sun_receiver_tri_set && dot(plane_n,plane_n)>1e-12;
    var triangle:array<vec2<f32>,3>;
    var orientation=0.0;
    if clipped {
        for(var i=0;i<3;i++) {
            let vertex=projection*vec4<f32>(sun_receiver_tri[i],1.0);
            triangle[i]=(vertex.xy*vec2<f32>(0.5,-0.5)+0.5)*vec2<f32>(size)-0.5;
        }
        orientation=sign(shadow_cross2(triangle[1]-triangle[0],triangle[2]-triangle[0]));
    }
    var lit = 0.0;
    var total = 0.0;
    for (var y=0; y<4; y++) { for (var x=0; x<4; x++) {
        let pixel = clamp(base + vec2<i32>(x-1,y-1), vec2<i32>(0), size-1);
        // A plane is valid only inside its actual triangle. Extrapolating it
        // into a neighbouring concave face creates false triangle-wire shadow
        // even with identical source geometry and infinite numeric precision.
        if clipped && (orientation==0.0 || !shadow_triangle_contains(vec2<f32>(pixel),triangle[0],triangle[1],triangle[2],orientation)) { continue; }
        let delta = vec2<f32>(pixel) - q;
        let reference = c.z + dot(gradient,delta) - epsilon;
        let depth = textureLoad(map,pixel,layer,0);
        let weight=wx[x]*wy[y];
        lit += weight * select(0.0,1.0,reference <= depth);
        total += weight;
    }}
    if total>1e-8 {return lit/total;}
    // Subtexel / edge-on triangles may contain no shadow-map sample centre.
    // Fall back to ONE nearest depth sample at the actual receiver depth.
    // This has <=half-texel quantization per axis and may alias; it cannot
    // invent an arbitrarily extrapolated plane or force an unoccluded result.
    let nearest=clamp(vec2<i32>(floor(q+0.5)),vec2<i32>(0),size-1);
    return select(0.0,1.0,c.z-epsilon<=textureLoad(map,nearest,layer,0));
}

fn shadow_filter_layer(map: texture_depth_2d_array, layer: i32, projection: mat4x4<f32>, world_texel: f32, p: vec3<f32>, n: vec3<f32>, plane_n: vec3<f32>) -> f32 {
    if light.near_shadow.w > 0.5 { return shadow_filter_plane_layer(map,layer,projection,world_texel,p,plane_n); }
    let biased = p + n * world_texel * 1.5 + light.sun_dir.xyz * world_texel;
    let c = projection * vec4<f32>(biased, 1.0);
    let uv = vec2<f32>(c.x * 0.5 + 0.5, 0.5 - c.y * 0.5);
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || c.z < 0.0 || c.z > 1.0 {
        return 1.0;
    }
    let texel = 1.0 / vec2<f32>(textureDimensions(map));
    var lit = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            lit += textureSampleCompareLevel(map, shadow_samp, uv + vec2<f32>(f32(x), f32(y)) * texel, layer, c.z);
        }
    }
    return lit / 9.0;
}

fn sun_coverage_fade(projection: mat4x4<f32>, p: vec3<f32>, blend_start: f32) -> f32 {
    let c = projection * vec4<f32>(p, 1.0);
    let edge = max(abs(c.x), abs(c.y));
    let depth_edge = max(1.0 - c.z / 0.05, 1.0 - (1.0 - c.z) / 0.05);
    return max(smoothstep(blend_start, 1.0, edge), smoothstep(0.0, 1.0, depth_edge));
}

// Stage-wide texels can span metres. Evaluate receiver depth at each tap,
// instead of comparing sloped ground to a single centre depth (striped acne).
fn stage_sun_visibility(p:vec3<f32>,plane_n:vec3<f32>)->f32 {
    let projection=far_sun.view_proj;
    let c=projection*vec4<f32>(p,1.);
    let size=vec2<i32>(textureDimensions(far_sun_tex));
    let q=(c.xy*vec2<f32>(.5,-.5)+.5)*vec2<f32>(size)-.5;
    let base=vec2<i32>(floor(q));let f=fract(q);
    let row_x=vec3<f32>(projection[0].x,projection[1].x,projection[2].x);
    let row_y=vec3<f32>(projection[0].y,projection[1].y,projection[2].y);
    let row_z=vec3<f32>(projection[0].z,projection[1].z,projection[2].z);
    let denominator=dot(plane_n,-normalize(row_z));var gradient=vec2<f32>(0.);
    if dot(plane_n,plane_n)>1e-12 && abs(denominator)>1e-4 {
        gradient=vec2<f32>(dot(plane_n,normalize(row_x)),-dot(plane_n,normalize(row_y)))
            *(far_sun.params.x*length(row_z)/denominator);
    }
    // Include projection XY roundoff propagated through the receiver slope.
    // At grazing sunlight a sub-millipixel XY error is larger than depth ULPs.
    let xy_error=4.7683716e-7*vec2<f32>(size)*(vec2<f32>(dot(abs(row_x),abs(p)),dot(abs(row_y),abs(p)))+abs(projection[3].xy)+1.);
    // Visible and cached meshes can use different cuts of the LOD tree.
    // Their measured error bounds prevent their own hills from becoming dark
    // triangle patches. This disappears with source geometry on both sides.
    let geometry_error=sun_receiver_error+max(0.,far_sun.params.w-1.)*far_sun.params.x;
    let epsilon=4.7683716e-7*(dot(abs(row_z),abs(p))+abs(projection[3].z)+1.)+dot(abs(gradient),xy_error)+length(row_z)*geometry_error;
    let clipped=sun_receiver_tri_set && dot(plane_n,plane_n)>1e-12;
    var triangle:array<vec2<f32>,3>;var orientation=0.;
    if clipped {
        for(var i=0;i<3;i++) {
            let vertex=projection*vec4<f32>(sun_receiver_tri[i],1.);
            triangle[i]=(vertex.xy*vec2<f32>(.5,-.5)+.5)*vec2<f32>(size)-.5;
        }
        orientation=sign(shadow_cross2(triangle[1]-triangle[0],triangle[2]-triangle[0]));
    }
    var lit=0.;var total=0.;
    for(var y=0;y<2;y++){for(var x=0;x<2;x++){
        let pixel=clamp(base+vec2<i32>(x,y),vec2<i32>(0),size-1);
        if clipped && (orientation==0. || !shadow_triangle_contains(vec2<f32>(pixel),triangle[0],triangle[1],triangle[2],orientation)){continue;}
        let weight=select(1.-f.x,f.x,x==1)*select(1.-f.y,f.y,y==1);
        let reference=c.z+dot(gradient,vec2<f32>(pixel)-q)-epsilon;
        lit+=weight*select(0.,1.,reference<=textureLoad(far_sun_tex,pixel,0,0));total+=weight;
    }}
    if total>1e-8 {return lit/total;}
    let nearest=clamp(vec2<i32>(floor(q+.5)),vec2<i32>(0),size-1);
    return select(0.,1.,c.z-epsilon<=textureLoad(far_sun_tex,nearest,0,0));
}

// Distant ground: the sun march of `distant_sun_visibility`, interpolated
// from terrain_coarse's quads (in the terrain shader only).
var<private> coarse_sun_transmittance: f32 = 1.0;
fn distant_sun_visibility(p: vec3<f32>, n: vec3<f32>, plane_n: vec3<f32>) -> f32 {
    if FEATURE_FOLIAGE_PAGES {
        let page=foliage_page_visibility(p,n,plane_n);
        if page.y>0.5 {return page.x;}
    }
    var fade = 1.0;
    var mapped = 1.0;
    if far_sun.params.z > 0.5 {
        fade = sun_coverage_fade(far_sun.view_proj, p, far_sun.params.y);
        if fade < 1.0 {
            // The old per-texel offset becomes metres on a stage-wide map and
            // pushes receivers through their own crowns. Keep numerical bias
            // in world units and use one bilinear comparison footprint: the
            // map already integrates a much larger area than the close maps.
            // Compact architectural caches need half a texel of geometric
            // normal bias on grazing roofs; keep the forest policy unchanged.
            let architecture = far_sun.params.w > 0.5;
            let bias = min(0.06, far_sun.params.x * select(0.1, 0.5, architecture));
            let receiver_n = select(n, plane_n, architecture);
            let c = far_sun.view_proj * vec4<f32>(p + (receiver_n + light.sun_dir.xyz) * bias, 1.0);
            let uv = c.xy * vec2<f32>(0.5,-0.5) + 0.5;
            if far_sun.params.w>0.5 {mapped=stage_sun_visibility(p,plane_n);}
            else {mapped = textureSampleCompareLevel(far_sun_tex, shadow_samp, uv, 0, c.z);}
        }
        if fade <= 0.0 { return mapped; }
    }
    var fallback = 1.0;
    // vol_dims.w bit 4: no foliage and no heightfield, the march returns 1.
    if SHADE_CLASS == 1u && TERRAIN_COARSE {
        // Compiled out of the terrain shader: its pixels take the march from
        // terrain_coarse's quads.
        fallback = coarse_sun_transmittance;
    } else if gi.vol_dims.w != 0u && (gi.vol_dims.w & 4u) == 0u {
        fallback = transmittance(p + n * 0.2, light.sun_dir.xyz, 50.0);
    }
    return mix(mapped, fallback, fade);
}

// Detail follows each camera but its texels remain fixed in world space.
// Blend the shared maps before adding the near detail, so a camera
// near the shared-map edge cannot overwrite a valid car shadow with foliage.
fn near_level_projection(k: i32) -> mat4x4<f32> {
    if k == 0 { return light.near_shadow_view_proj; }
    return light.near_levels_view_proj[k - 1];
}
// How much sun clipmap level k covers p: round, wide fade (light-space
// radius, 0.45 to 0.95 of the half extent): a square edge 20 % wide showed
// the map's outline on the ground where sharp voxel shadows met the soft
// shared map.
fn near_level_weight(k: i32, p: vec3<f32>) -> f32 {
    let c = near_level_projection(k) * vec4<f32>(p, 1.0);
    if c.z < 0.0 || c.z > 1.0 { return 0.0; }
    return 1.0 - smoothstep(light.near_shadow.y, 0.95, length(c.xy));
}
// Half extent of sun clipmap level k in metres, from its projection: the
// levels need not double (frame.rs CLOSE_LADDER).
fn near_level_extent(k: i32) -> f32 {
    let m = near_level_projection(k);
    return 1.0 / max(length(vec3<f32>(m[0].x, m[1].x, m[2].x)), 1e-12);
}
// World size of one of its texels.
fn near_level_texel(k: i32) -> f32 {
    return 2.0 * near_level_extent(k) / f32(textureDimensions(near_shadow_tex).x);
}
fn near_level_visibility(k: i32, p: vec3<f32>, n: vec3<f32>, plane_n: vec3<f32>) -> f32 {
    let texel = near_level_texel(k);
    return shadow_filter_layer(near_shadow_tex, k, near_level_projection(k), texel, p, n, plane_n);
}
// Voxel cell size the sun map that shades p splats (frame.rs
// near_level_voxel_m, SHARED_SUN_VOXEL_M): the finest clipmap level that
// covers p, else the shared map's.
fn sun_voxel_cell(p: vec3<f32>) -> f32 {
    let levels = select(0, i32(light.near_levels.x), light.near_shadow.z > 0.5);
    for (var k = 0; k < levels; k++) {
        if near_level_weight(k, p) > 0.5 {
            // The doubling ladder's level this one stands for (frame.rs
            // extent_level): 24 m is its first.
            let level = i32(max(round(log2(near_level_extent(k) / 24.0)), 0.0));
            return 0.25 * pow(1.41421356, f32(level)) * select(1.0, 1.42, level >= 3);
        }
    }
    return 0.5;
}
// How fully the sun clipmap's levels cover p (0 when near maps are off).
fn sun_clipmap_cover(p: vec3<f32>) -> f32 {
    let levels = select(0, i32(light.near_levels.x), light.near_shadow.z > 0.5);
    var cover = 0.0;
    for (var k = 0; k < levels; k++) { cover = max(cover, near_level_weight(k, p)); }
    return cover;
}
// How fully the sharp sun maps cover p: the near clipmap levels when on,
// else the near part of the shared map (inside its coverage fade).
fn near_sharp_cover(p: vec3<f32>) -> f32 {
    // Within FAR_TREE_SHADOW_MIN_M the sun maps keep the trees' shadows:
    // the per-tree cones up close read as stepped triangles on the grass
    // (Joep, 9 November, 150-500 m from the camera).
    let near = 1.0 - smoothstep(FAR_TREE_SHADOW_MIN_M, FAR_TREE_SHADOW_MIN_M * 1.3, distance(p.xz, view.cam.xz));
    if near >= 1.0 { return 1.0; }
    return max(near, near_sharp_cover_maps(p));
}
fn near_sharp_cover_maps(p: vec3<f32>) -> f32 {
    let levels = select(0, i32(light.near_levels.x), light.near_shadow.z > 0.5);
    var w = 0.0;
    for (var k = 0; k < levels; k++) { w = max(w, near_level_weight(k, p)); }
    if levels > 0 && light.near_levels.y > 0.5 { return w; }
    return max(w, 1.0 - smoothstep(FAR_TREE_SHADOW_FROM, 1.0, max(abs((light.shadow_view_proj * vec4<f32>(p, 1.0)).x), abs((light.shadow_view_proj * vec4<f32>(p, 1.0)).y))));
}
// KERN_FAR_TREE_SHADOWS=0: the coarse maps shade the far forest ground.
override FAR_TREE_SHADOWS: bool = true;
// Where in the shared map (its clip-space radius) per-tree ground shadows
// start to take over.
override FAR_TREE_SHADOW_FROM: f32 = 0.35;
// Metres from the camera within which per-tree ground shadows never draw.
override FAR_TREE_SHADOW_MIN_M: f32 = 800.0;
// Share of a voxel cell's own sun march kept where the sun clipmap covers it
// (shade_voxel; KERN_VOXEL_OWN_IN_CLIPMAP).
override VOXEL_OWN_IN_CLIPMAP: f32 = 0.0;
// far_tree_cast for this pixel's terrain (-1: none, or not evaluated).
var<private> far_tree_shadow: f32 = -1.0;

fn shadow_detail_receiver(p: vec3<f32>, n: vec3<f32>, plane_n: vec3<f32>) -> f32 {
    // The finest clipmap level that covers p in full; coarser ones only
    // matter outside it.
    let levels = select(0, i32(light.near_levels.x), light.near_shadow.z > 0.5);
    var full = levels;
    var weights = array<f32, 8>(0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for (var k = 0; k < levels; k++) {
        weights[k] = near_level_weight(k, p);
        if weights[k] >= 1.0 { full = k; break; }
    }
    var vis = 1.0;
    if full < levels {
        // The clipmap's sun box is shallow: a ridge or forest far up the sun
        // ray lies outside it, so trees on a mountain's shadow side caught
        // full sun (Mancave sky test, Joep playtest 5). The shared map holds
        // those; read it 3 m up the ray so its coarse copy of this very
        // crown does not shade it again.
        // One clipmap (near_levels.y): every level spans the scene's full
        // sun depth, so the covering level already holds those casters.
        vis = near_level_visibility(full, p, n, plane_n);
        if light.near_levels.y < 0.5 {
            vis = min(vis, shadow_filter(shadow_tex, light.shadow_view_proj, light.sky.w, p + light.sun_dir.xyz * 3.0, n, plane_n));
        }
    } else {
        vis = shadow_filter(shadow_tex, light.shadow_view_proj, light.sky.w, p, n, plane_n);
        // The outer map uses actual alpha-tested tree geometry, not the coarse
        // density field that wrongly buried sunlit crowns in their own shadow.
        // Over the outer 55 % of the map: sharp tree shadows thin out into the
        // coarse distant field instead of meeting it at a line.
        let coverage_fade = sun_coverage_fade(light.shadow_view_proj, p, 0.45);
        if coverage_fade > 0.0 && (far_sun.params.z > 0.5 || sun_pages_counts().z != 0u || gi.vol_dims.w != 0u) {
            vis = mix(vis, distant_sun_visibility(p,n,plane_n), coverage_fade);
        }
    }
    // Finer levels that cover p in part, coarsest first.
    for (var k = min(full, levels) - 1; k >= 0; k--) {
        if weights[k] > 0.0 { vis = mix(vis, near_level_visibility(k, p, n, plane_n), weights[k]); }
    }
    return vis;
}

// The renderer reserves night_stars.z for this execution policy. Diagnostics
// retain their shadow queries at zero lux; ambient/contact/reflection paths do
// not use this guard. Keep the predicate aligned with frame.rs.
fn sun_visibility_required() -> bool {
    // The finite lower bound also keeps -infinity on the conservative path;
    // NaN fails the conjunction, exactly as the host's is_finite predicate.
    return light.night_stars.z < 0.5
        || !(light.sun_dir.w <= 0.0 && light.sun_dir.w >= -3.4028234663852886e38)
        || light.sky.z != 0.0;
}

// Coverage of the camera's detailed map never removes a world terrain blocker.
fn shadow_at_surface(p: vec3<f32>, n: vec3<f32>, plane_n: vec3<f32>, representation_error:f32) -> f32 {
    if !sun_visibility_required() { return 1.0; }
    let detail=shadow_detail_receiver(p,n,plane_n);
    if detail<=0.0 {return 0.0;}
    // Disabled world terrain visibility answers 1 (fully visible).
    if !FEATURE_WORLD_TERRAIN { return detail; }
    return min(detail,world_terrain_sun_visibility(p,n,representation_error));
}

fn shadow_at_receiver(p: vec3<f32>, n: vec3<f32>, plane_n: vec3<f32>) -> f32 {
    return shadow_at_surface(p,n,plane_n,0.0);
}

fn shadow_at(p: vec3<f32>, n: vec3<f32>) -> f32 {
    return shadow_at_receiver(p,n,vec3<f32>(0.0));
}
fn shadow_at_plane(p: vec3<f32>, n: vec3<f32>, plane_n: vec3<f32>) -> f32 {
    return shadow_at_receiver(p,n,plane_n);
}

fn tex(layer: i32, uv: vec2<f32>, lod: f32) -> vec4<f32> {
    return textureSampleLevel(albedo, samp, uv, layer, max(lod, albedo_lod_floor(layer)));
}

// The surface's footprint across one pixel, from ray differentials: the
// rays through the next pixel right and down, hit on the triangle's plane.
struct Footprint {
    dx: vec3<f32>,
    dy: vec3<f32>,
};

var<private> fp: Footprint;

fn set_pixel_footprint(ndc:vec2<f32>, p:vec3<f32>, geo_n:vec3<f32>, dist:f32) {
    // Ray differentials: where the neighbouring pixels' rays meet this
    // triangle's plane. Gives exact, anisotropic texture footprints.
    let px_x = view.inv_view_proj * vec4<f32>(ndc + vec2<f32>(2.0 * view.size.z, 0.0), 1e-4, 1.0);
    let px_y = view.inv_view_proj * vec4<f32>(ndc - vec2<f32>(0.0, 2.0 * view.size.w), 1e-4, 1.0);
    let dir_x = normalize(px_x.xyz / px_x.w - view.cam.xyz);
    let dir_y = normalize(px_y.xyz / px_y.w - view.cam.xyz);
    let plane_d = dot(p - view.cam.xyz, geo_n);
    let hit_x = view.cam.xyz + dir_x * (plane_d / select(dot(dir_x, geo_n), 1e-6, abs(dot(dir_x, geo_n)) < 1e-6));
    let hit_y = view.cam.xyz + dir_y * (plane_d / select(dot(dir_y, geo_n), 1e-6, abs(dot(dir_y, geo_n)) < 1e-6));
    let reach = dist * 0.05 + 0.05; // grazing hits run off towards infinity
    fp.dx = clamp(hit_x - p, vec3<f32>(-reach), vec3<f32>(reach));
    fp.dy = clamp(hit_y - p, vec3<f32>(-reach), vec3<f32>(reach));
}

fn track_tile(key:vec2<i32>)->vec4<i32> {
    var h=(bitcast<u32>(key.x)*0x9e3779b9u) ^ (bitcast<u32>(key.y)*0x85ebca6bu);
    h^=h>>16u;h*=0x7feb352du;h^=h>>15u;
    for(var probe=0u;probe<8u;probe++) {
        let index=(h+probe)&0xffffu;
        let entry=textureLoad(track_lookup,vec2<i32>(i32(index&255u),i32(index>>8u)),0);
        if entry.z==0 {return vec4<i32>(0);}
        if entry.z>0 && all(entry.xy==key) {return entry;}
        // z=-1 is an evicted tombstone: a later colliding identity remains
        // reachable, but stale page contents can never match a world tile.
    }
    return vec4<i32>(0);
}
fn track_material(p:vec3<f32>)->vec2<f32> {
    let footprint=max(length(fp.dx.xz),length(fp.dy.xz));
    let filtered=1.0-smoothstep(0.12,0.60,footprint);
    if filtered<=0.0 {return vec2<f32>(0);}
    let q=p.xz/0.08-0.5;let base=vec2<i32>(floor(q));let f=fract(q);
    let key=vec2<i32>(floor(vec2<f32>(base)/128.0));
    let initial=track_tile(key);
    if initial.z==0 && all(base-key*128<vec2<i32>(127)) {return vec2<f32>(0);}
    var result=vec2<f32>(0);
    for(var y=0;y<2;y++) {for(var x=0;x<2;x++) {
        let pixel=base+vec2<i32>(x,y);let tile=vec2<i32>(floor(vec2<f32>(pixel)/128.0));
        var entry=initial;if any(tile!=key) {entry=track_tile(tile);}
        if entry.z<=0 {continue;}
        let slot=u32(entry.z-1);let local=pixel-tile*128;
        let address=vec2<i32>(i32(slot%32u)*128,i32(slot/32u)*128)+local;
        let data=textureLoad(track_atlas,address,0);
        if max(data.r,data.g)<=0.0 {continue;}
        let height=bitcast<f32>(entry.w)+((data.b*255.0+data.a*65280.0)/65535.0*2.0-1.0)*64.0;
        let clip=clamp(1.0-(abs(p.y-height)-0.08)/0.08,0.0,1.0);
        let weight=select(1.0-f.x,f.x,x==1)*select(1.0-f.y,f.y,y==1);
        result+=data.rg*(clip*weight);
    }}
    return result*filtered;
}

// Sample with the true texture-space gradients (anisotropic filtering):
// uv = (world position projected on `ax`, `ay`) / scale.
fn tex_w(layer: i32, uv: vec2<f32>, gx: vec2<f32>, gy: vec2<f32>) -> vec4<f32> {
    let k = albedo_grad(layer, gx, gy);
    return textureSampleGrad(albedo, samp, uv, layer, gx * k, gy * k);
}

fn tex_xz(layer: i32, uv: vec2<f32>, scale: f32) -> vec4<f32> {
    return tex_w(layer, uv, fp.dx.xz / scale, fp.dy.xz / scale);
}

// Tangent-space normal (xy in -1..1) of layer l, sampled like the albedo.
fn nrm(l: i32, uv: vec2<f32>, gx: vec2<f32>, gy: vec2<f32>) -> vec2<f32> {
    let k = normal_grad(l, gx, gy);
    return textureSampleGrad(normal_tex, samp, uv, l, gx * k, gy * k).xy * 2.0 - 1.0;
}

// The same with the map's alpha in z.
fn nrm_a(l: i32, uv: vec2<f32>, gx: vec2<f32>, gy: vec2<f32>) -> vec3<f32> {
    let k = normal_grad(l, gx, gy);
    let t = textureSampleGrad(normal_tex, samp, uv, l, gx * k, gy * k);
    return vec3<f32>(t.xy * 2.0 - 1.0, t.a);
}

// Occlusion, roughness and metalness factors of layer l's material map
// (glTF packing), sampled like the albedo; white when it has none.
fn orm_w(l: i32, uv: vec2<f32>, gx: vec2<f32>, gy: vec2<f32>) -> vec3<f32> {
    let at = layer_map(l).x;
    if at == 0u { return vec3<f32>(1.0); }
    let k = normal_grad(l, gx, gy);
    return textureSampleGrad(normal_tex, samp, uv, i32(at - 1u), gx * k, gy * k).rgb;
}

// The material map in a triplanar mapping (`triplanar` below, unrotated).
fn triplanar_orm(l: i32, p: vec3<f32>, tw: vec3<f32>, scale: f32) -> vec3<f32> {
    if layer_map(l).x == 0u { return vec3<f32>(1.0); }
    let k = 1.0 / scale;
    return orm_w(l, p.zy * k, fp.dx.zy * k, fp.dy.zy * k) * tw.x + orm_w(l, p.xz * k, fp.dx.xz * k, fp.dy.xz * k) * tw.y
        + orm_w(l, p.xy * k, fp.dx.xy * k, fp.dy.xy * k) * tw.z;
}

fn nrm_xz(l: i32, uv: vec2<f32>, scale: f32) -> vec2<f32> {
    return nrm(l, uv, fp.dx.xz / scale, fp.dy.xz / scale);
}

fn safe_normalize(v: vec3<f32>) -> vec3<f32> {
    return v / max(length(v), 1e-6);
}

// Bend n by a tangent-space normal xy. t = dP/du and b = -dP/dv: OpenGL
// normal maps (Godot's) point green towards the image's top row.
fn perturb(n: vec3<f32>, xy: vec2<f32>, t: vec3<f32>, b: vec3<f32>) -> vec3<f32> {
    let tt = safe_normalize(t - n * dot(n, t));
    let bb = safe_normalize(b - n * dot(n, b));
    let z = sqrt(max(1.0 - dot(xy, xy), 0.0));
    return safe_normalize(tt * xy.x + bb * xy.y + n * z);
}

// The normal map in the planar xz mapping: u along +x, v along +z.
fn perturb_xz(n: vec3<f32>, xy: vec2<f32>) -> vec3<f32> {
    return perturb(n, xy, vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, -1.0));
}

// Triplanar normal map, matching `triplanar` below. A sample taken at
// rot * uv turns back with rot (the v flip makes the inverse rot again).
fn triplanar_n(l: i32, p: vec3<f32>, n: vec3<f32>, tw: vec3<f32>, scale: f32, rot: mat2x2<f32>, shift: f32) -> vec3<f32> {
    let k = 1.0 / scale;
    let x = rot * nrm(l, rot * p.zy * k + shift, rot * fp.dx.zy * k, rot * fp.dy.zy * k);
    let y = rot * nrm(l, rot * p.xz * k + shift, rot * fp.dx.xz * k, rot * fp.dy.xz * k);
    let z = rot * nrm(l, rot * p.xy * k + shift, rot * fp.dx.xy * k, rot * fp.dy.xy * k);
    let nx = perturb(n, x, vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, -1.0, 0.0));
    let ny = perturb(n, y, vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, -1.0));
    let nz = perturb(n, z, vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, -1.0, 0.0));
    return safe_normalize(nx * tw.x + ny * tw.y + nz * tw.z);
}

// World directions of the mesh uv on a triangle: (dP/du, -dP/dv).
fn uv_frame(e1: vec3<f32>, e2: vec3<f32>, vi: array<u32, 3>) -> array<vec3<f32>, 2> {
    let uv0 = vec2<f32>(vertex_f32(vi[0] + 6u), vertex_f32(vi[0] + 7u));
    let d1 = vec2<f32>(vertex_f32(vi[1] + 6u), vertex_f32(vi[1] + 7u)) - uv0;
    let d2 = vec2<f32>(vertex_f32(vi[2] + 6u), vertex_f32(vi[2] + 7u)) - uv0;
    let det = d1.x * d2.y - d2.x * d1.y;
    let r = select(1.0 / det, 0.0, abs(det) < 1e-12);
    let dpdu = (e1 * d2.y - e2 * d1.y) * r;
    let dpdv = (e2 * d1.x - e1 * d2.x) * r;
    return array<vec3<f32>, 2>(dpdu, -dpdv);
}

// Triplanar: three planar projections of `scale` metres, blended by tw.
fn triplanar(l: i32, p: vec3<f32>, tw: vec3<f32>, scale: f32, rot: mat2x2<f32>, shift: f32) -> vec3<f32> {
    let k = 1.0 / scale;
    let x = tex_w(l, rot * p.zy * k + shift, rot * fp.dx.zy * k, rot * fp.dy.zy * k).rgb;
    let y = tex_w(l, rot * p.xz * k + shift, rot * fp.dx.xz * k, rot * fp.dy.xz * k).rgb;
    let z = tex_w(l, rot * p.xy * k + shift, rot * fp.dx.xy * k, rot * fp.dy.xy * k).rgb;
    return x * tw.x + y * tw.y + z * tw.z;
}

// The change in the mesh uv for a step d on the triangle (p0, e1, e2).
fn uv_at(d: vec3<f32>, e1: vec3<f32>, e2: vec3<f32>, vi: array<u32, 3>) -> vec2<f32> {
    let d00 = dot(e1, e1);
    let d01 = dot(e1, e2);
    let d11 = dot(e2, e2);
    let d20 = dot(d, e1);
    let d21 = dot(d, e2);
    let den = d00 * d11 - d01 * d01;
    let inv = select(1.0 / den, 0.0, abs(den) < 1e-20);
    let b1 = (d11 * d20 - d01 * d21) * inv;
    let b2 = (d00 * d21 - d01 * d20) * inv;
    let uv0 = vec2<f32>(vertex_f32(vi[0] + 6u), vertex_f32(vi[0] + 7u));
    let uv1 = vec2<f32>(vertex_f32(vi[1] + 6u), vertex_f32(vi[1] + 7u));
    let uv2 = vec2<f32>(vertex_f32(vi[2] + 6u), vertex_f32(vi[2] + 7u));
    return (uv1 - uv0) * b1 + (uv2 - uv0) * b2;
}

const TURN: mat2x2<f32> = mat2x2<f32>(vec2<f32>(0.8, 0.6), vec2<f32>(-0.6, 0.8));

fn fbm(p: vec2<f32>) -> f32 {
    return vnoise(p) * 0.55 + vnoise(p * 2.1) * 0.3 + vnoise(p * 4.3) * 0.15;
}

// Two world-anchored texture variants, shared by color and normal maps.
// The previous second sample enlarged every texture 4.3x as the camera moved
// away. Mountains exposed that repeat, and camera distance changed the soil.
// Adjacent hashed variants use the SAME physical texel scale. A smooth scalar
// noise field chooses their blend, so no square tile boundary is introduced.
// There are still two texture reads per layer, and filtering uses each rotated
// footprint rather than differentiating the discontinuous variant index.
struct TerrainMapping {
    a: mat2x2<f32>,
    b: mat2x2<f32>,
    offset_a: vec2<f32>,
    offset_b: vec2<f32>,
    blend: f32,
};

fn terrain_turn(index: f32) -> mat2x2<f32> {
    let angle = hash2(vec2<f32>(index, 37.0)) * 6.2831853;
    let cs = cos(angle);
    let sn = sin(angle);
    return mat2x2<f32>(cs, sn, -sn, cs);
}

fn terrain_mapping(p: vec3<f32>) -> TerrainMapping {
    let selector = fbm((p.xz + p.yy * vec2<f32>(0.31, -0.23)) / 19.0 + 13.0) * 16.0;
    let index = floor(selector);
    var mapping: TerrainMapping;
    mapping.a = terrain_turn(index);
    mapping.b = terrain_turn(index + 1.0);
    mapping.offset_a = vec2<f32>(hash2(vec2<f32>(index, 11.0)), hash2(vec2<f32>(index, 23.0)));
    mapping.offset_b = vec2<f32>(hash2(vec2<f32>(index + 1.0, 11.0)), hash2(vec2<f32>(index + 1.0, 23.0)));
    mapping.blend = smoothstep(0.15, 0.85, fract(selector));
    return mapping;
}

// Away from a seam between two turns only one of them is sampled.
fn terrain_layer_normal(l: i32, uv: vec2<f32>, scale: f32, mapping: TerrainMapping) -> vec2<f32> {
    var a = vec2<f32>(0.0);
    var b = vec2<f32>(0.0);
    if mapping.blend < 1.0 { a = mapping.a * nrm(l, mapping.a * uv / scale + mapping.offset_a, mapping.a * fp.dx.xz / scale, mapping.a * fp.dy.xz / scale); }
    if mapping.blend > 0.0 { b = mapping.b * nrm(l, mapping.b * uv / scale + mapping.offset_b, mapping.b * fp.dx.xz / scale, mapping.b * fp.dy.xz / scale); }
    return mix(a, b, mapping.blend);
}

fn terrain_layer(l: i32, uv: vec2<f32>, scale: f32, mapping: TerrainMapping) -> vec3<f32> {
    var a = vec3<f32>(0.0);
    var b = vec3<f32>(0.0);
    if mapping.blend < 1.0 { a = tex_w(l, mapping.a * uv / scale + mapping.offset_a, mapping.a * fp.dx.xz / scale, mapping.a * fp.dy.xz / scale).rgb; }
    if mapping.blend > 0.0 { b = tex_w(l, mapping.b * uv / scale + mapping.offset_b, mapping.b * fp.dx.xz / scale, mapping.b * fp.dy.xz / scale).rgb; }
    return mix(a, b, mapping.blend);
}

// The road and terrain use the same source soil texture, metre scale,
// rotation, colour and weather response where their geometry joins. Matching
// positions alone left a bright, ruler-straight gravel/soil material cut.
fn shoulder_soil(l: i32, p: vec3<f32>, mapping: TerrainMapping) -> vec3<f32> {
    return terrain_layer(l, p.xz, 3.0, mapping) * vec3<f32>(0.85, 0.8, 0.75)
        * (0.78 + 0.34 * fbm(p.xz / 31.0));
}

struct RockSurface { color: vec3<f32>, normal: vec3<f32> };
// Explicit diagnostic opt-in. Keep the original material unchanged while
// comparing the independent shadow fix; the shade pipeline specializes this.
override ROCK_VARIATION: bool = false;

// World-cell hash: integer arithmetic keeps negative coordinates and distant
// mountains stable, without the precision loss of sin(large_coordinate).
fn rock_hash(cell: vec2<i32>, seed: u32) -> f32 {
    var h = (bitcast<u32>(cell.x) * 0x9e3779b9u) ^ (bitcast<u32>(cell.y) * 0x85ebca6bu) ^ seed;
    h ^= h >> 16u; h *= 0x7feb352du; h ^= h >> 15u;
    h *= 0x846ca68bu; h ^= h >> 16u;
    return f32(h >> 8u) / 16777216.0;
}

// Signed value and analytic XY gradient. Quintic interpolation gives a C2
// coordinate field, including every cell boundary; its Jacobian is C1.
fn rock_noise_gradient(p: vec2<f32>, seed: u32) -> vec3<f32> {
    let cell = vec2<i32>(floor(p));
    let f = fract(p);
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let du = 30.0 * f * f * (f - 1.0) * (f - 1.0);
    let a = rock_hash(cell, seed);
    let b = rock_hash(cell + vec2<i32>(1, 0), seed);
    let c = rock_hash(cell + vec2<i32>(0, 1), seed);
    let d = rock_hash(cell + vec2<i32>(1, 1), seed);
    return vec3<f32>(mix(mix(a, b, u.x), mix(c, d, u.x), u.y) * 2.0 - 1.0,
        mix(b - a, d - c, u.y) * du.x * 2.0,
        mix(c - a, d - b, u.x) * du.y * 2.0);
}

fn rock_warp_gradient(p: vec2<f32>, seed: u32) -> vec3<f32> {
    let fine = rock_noise_gradient(p / 13.0, seed);
    let broad = rock_noise_gradient(p / 47.0, seed ^ 0x68bc21ebu);
    return vec3<f32>(fine.x * 0.8 + broad.x * 2.6,
        fine.yz * (0.8 / 13.0) + broad.yz * (2.6 / 47.0));
}

struct RockVariant { turn: mat2x2<f32>, offset: vec2<f32> };
struct RockMapping { a: RockVariant, b: RockVariant, blend: f32 };

fn rock_variant(index: i32) -> RockVariant {
    let angle = rock_hash(vec2<i32>(index, 37), 0xc28491d3u) * 6.2831853;
    let cs = cos(angle); let sn = sin(angle);
    var result: RockVariant;
    result.turn = mat2x2<f32>(cs, sn, -sn, cs);
    result.offset = vec2<f32>(rock_hash(vec2<i32>(index, 11), 0x6a09e667u),
        rock_hash(vec2<i32>(index, 23), 0xbb67ae85u));
    return result;
}

fn rock_mapping(p: vec3<f32>) -> RockMapping {
    let field = rock_noise_gradient((p.xz + p.yy * vec2<f32>(0.37, -0.23)) / 17.0, 0x3c6ef372u);
    let selector = (field.x * 0.5 + 0.5) * 12.0;
    let index = i32(floor(selector));
    var result: RockMapping;
    result.a = rock_variant(index);
    result.b = rock_variant(index + 1);
    result.blend = smoothstep(0.15, 0.85, fract(selector));
    return result;
}

// gu/gv are the world gradients of the unscaled, rotated texture coordinates.
// Pull the sampled normal's slope through the SAME Jacobian as its albedo.
// OpenGL normal-map green points against image V. Do not rotate just the UV:
// that leaves relief facing the old direction and produces false lighting.
fn rock_mapped_normal(n: vec3<f32>, xy: vec2<f32>, gu: vec3<f32>, gv: vec3<f32>) -> vec3<f32> {
    let gradient = gu * xy.x - gv * xy.y;
    return safe_normalize(n * sqrt(max(1.0 - dot(xy, xy), 0.0))
        + gradient - n * dot(n, gradient));
}

fn rock_axis_variant(l: i32, uv: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>,
    n: vec3<f32>, gu: vec3<f32>, gv: vec3<f32>, variant: RockVariant) -> RockSurface {
    let q = variant.turn * uv + variant.offset;
    let qx = variant.turn * dx; let qy = variant.turn * dy;
    let ru = gu * variant.turn[0].x + gv * variant.turn[1].x;
    let rv = gu * variant.turn[0].y + gv * variant.turn[1].y;
    var result: RockSurface;
    result.color = tex_w(l, q, qx, qy).rgb;
    result.normal = rock_mapped_normal(n, nrm(l, q, qx, qy), ru, rv);
    return result;
}

fn rock_axis(l: i32, uv: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>,
    n: vec3<f32>, gu: vec3<f32>, gv: vec3<f32>, mapping: RockMapping) -> RockSurface {
    // Endpoints need only one variant. A transition never exceeds two color
    // and two normal reads, and both variants retain the same 3.7 m scale.
    if mapping.blend <= 0.0 { return rock_axis_variant(l, uv, dx, dy, n, gu, gv, mapping.a); }
    if mapping.blend >= 1.0 { return rock_axis_variant(l, uv, dx, dy, n, gu, gv, mapping.b); }
    let a = rock_axis_variant(l, uv, dx, dy, n, gu, gv, mapping.a);
    let b = rock_axis_variant(l, uv, dx, dy, n, gu, gv, mapping.b);
    var result: RockSurface;
    result.color = mix(a.color, b.color, mapping.blend);
    result.normal = safe_normalize(mix(a.normal, b.normal, mapping.blend));
    return result;
}

// World-fixed continuous variants break the remaining recognizable repeat
// of one warped rock asset. At an integer selector boundary the preceding B
// is exactly the following A; there is no tile edge, camera fade or LOD swap.
// Keep the existing non-folding warp: each off-diagonal Jacobian row sums to
// at most 7.5 * (0.8/13 + 2.6/47) < 0.877. Color, normal direction and each
// anisotropic footprint consume the identical warp and variant rotation.
fn rock_surface(l: i32, p: vec3<f32>, n: vec3<f32>, tw: vec3<f32>) -> RockSurface {
    let wx = rock_warp_gradient(p.yz, 0x7314a5d3u);
    let wy = rock_warp_gradient(p.xz, 0x2c6e8b91u);
    let wz = rock_warp_gradient(p.xy, 0x9a35d7f1u);
    let q = p + vec3<f32>(wx.x, wy.x, wz.x);
    let gx = vec3<f32>(1.0, wx.y, wx.z);
    let gy = vec3<f32>(wy.y, 1.0, wy.z);
    let gz = vec3<f32>(wz.y, wz.z, 1.0);
    let dx = vec3<f32>(dot(gx, fp.dx), dot(gy, fp.dx), dot(gz, fp.dx)) / 3.7;
    let dy = vec3<f32>(dot(gx, fp.dy), dot(gy, fp.dy), dot(gz, fp.dy)) / 3.7;
    let uv = q / 3.7;
    if !ROCK_VARIATION {
        var original: RockSurface;
        original.color = tex_w(l, uv.zy, dx.zy, dy.zy).rgb * tw.x
            + tex_w(l, uv.xz, dx.xz, dy.xz).rgb * tw.y
            + tex_w(l, uv.xy, dx.xy, dy.xy).rgb * tw.z;
        let nx = perturb(n, nrm(l, uv.zy, dx.zy, dy.zy), gz, -gy);
        let ny = perturb(n, nrm(l, uv.xz, dx.xz, dy.xz), gx, -gz);
        let nz = perturb(n, nrm(l, uv.xy, dx.xy, dy.xy), gx, -gy);
        original.normal = safe_normalize(nx * tw.x + ny * tw.y + nz * tw.z);
        return original;
    }
    let mapping = rock_mapping(p);
    var result: RockSurface;
    result.color = vec3<f32>(0.0); result.normal = vec3<f32>(0.0);
    // At most 0.01% material weight is discarded per skipped axis. Avoid
    // sampling a ground-facing projection on an almost vertical cliff.
    if tw.x > 0.0001 {
        let x = rock_axis(l, uv.zy, dx.zy, dy.zy, n, gz, gy, mapping);
        result.color += x.color * tw.x; result.normal += x.normal * tw.x;
    }
    if tw.y > 0.0001 {
        let y = rock_axis(l, uv.xz, dx.xz, dy.xz, n, gx, gz, mapping);
        result.color += y.color * tw.y; result.normal += y.normal * tw.y;
    }
    if tw.z > 0.0001 {
        let z = rock_axis(l, uv.xy, dx.xy, dy.xy, n, gx, gy, mapping);
        result.color += z.color * tw.z; result.normal += z.normal * tw.z;
    }
    result.normal = safe_normalize(result.normal);
    return result;
}

struct Bark { albedo: vec3<f32>, normal: vec3<f32>, rough: f32, ao: f32 }

fn bark_hash(p: vec3<i32>) -> f32 {
    var x = (bitcast<u32>(p.x) * 0x8da6b343u) ^ (bitcast<u32>(p.y) * 0xd8163841u) ^ (bitcast<u32>(p.z) * 0xcb1ab31fu);
    x = (x ^ (x >> 16u)) * 0x7feb352du;
    x = (x ^ (x >> 15u)) * 0x846ca68bu;
    return f32(x >> 8u) / 16777216.0;
}

fn bark_noise(p: vec3<f32>) -> f32 {
    let i = vec3<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(mix(bark_hash(i), bark_hash(i + vec3<i32>(1, 0, 0)), u.x), mix(bark_hash(i + vec3<i32>(0, 1, 0)), bark_hash(i + vec3<i32>(1, 1, 0)), u.x), u.y);
    let b = mix(mix(bark_hash(i + vec3<i32>(0, 0, 1)), bark_hash(i + vec3<i32>(1, 0, 1)), u.x), mix(bark_hash(i + vec3<i32>(0, 1, 1)), bark_hash(i + vec3<i32>(1, 1, 1)), u.x), u.y);
    return mix(a, b, u.z);
}

/// Distance to the nearest cell edge of a 3D Voronoi pattern (x) and a
/// per-cell random (y).
fn bark_cells(q: vec3<f32>) -> vec2<f32> {
    let c = vec3<i32>(floor(q));
    var f1 = 1e9;
    var f2 = 1e9;
    var id = 0.0;
    for (var z = -1; z <= 1; z++) {
        for (var y = -1; y <= 1; y++) {
            for (var x = -1; x <= 1; x++) {
                let k = c + vec3<i32>(x, y, z);
                let o = vec3<f32>(bark_hash(k), bark_hash(k + vec3<i32>(71, 13, 5)), bark_hash(k + vec3<i32>(9, 57, 33)));
                let d = length(vec3<f32>(k) + 0.15 + 0.7 * o - q);
                if d < f1 {
                    f2 = f1;
                    f1 = d;
                    id = o.x;
                } else if d < f2 {
                    f2 = d;
                }
            }
        }
    }
    return vec2<f32>(f2 - f1, id);
}

/// Jeffrey pine bark in plate units: long vertical ridges (Voronoi cells
/// six times taller than wide) split by deep furrows, each ridge broken
/// into jigsaw subplates by finer cracks. x: height (1 on a plate, 0 deep in
/// a furrow); y: per-ridge random; z: the furrow (1 deep in it); w: the
/// rim of a peeling flake (1 on it).
fn bark_height(q: vec3<f32>) -> vec4<f32> {
    let warp = vec3<f32>(bark_noise(q * 1.7), bark_noise(q * 1.7 + 7.3), bark_noise(q * 1.7 + 3.1)) - 0.5;
    let ridge = bark_cells(q * vec3<f32>(1.0, 0.16, 1.0) + warp * vec3<f32>(0.35, 0.1, 0.35));
    // Wide, ragged furrows: the edge distance wobbles with fine noise.
    let ragged = ridge.x + (bark_noise(q * vec3<f32>(11.0, 4.0, 11.0)) - 0.5) * 0.06;
    let furrow = 1.0 - smoothstep(0.03, 0.12, ragged);
    // Subplates: ridged noise makes wandering cracks across each ridge.
    let s = q * vec3<f32>(2.6, 1.6, 2.6) + warp * 0.8;
    let crack = 1.0 - smoothstep(0.0, 0.07, abs(2.0 * bark_noise(s) - 1.0));
    // Layered flakes: contour terraces of a warped field give rounded,
    // puzzle-like lobes, each a thin step above the one below.
    let lobes = bark_noise(q * vec3<f32>(3.2, 2.2, 3.2) + warp * 1.4) * 4.0;
    let layer = fract(lobes);
    let rim = 1.0 - smoothstep(0.0, 0.1, layer);
    let plate = (1.0 - furrow) * (0.72 + 0.1 * layer + 0.06 * bark_noise(q * vec3<f32>(9.0, 5.0, 9.0)) - 0.15 * crack);
    return vec4<f32>(plate, ridge.y, furrow, rim * (1.0 - furrow));
}

/// Cinnamon to salmon plates, grey-pink where weathered, between near-black
/// furrows full of orange-brown fibres; relief from the height field.
fn bark_surface(p: vec3<f32>, n: vec3<f32>, plate: f32) -> Bark {
    let q = p / max(plate, 0.01);
    let h = bark_height(q);
    let t1 = normalize(cross(n, vec3<f32>(0.0, 1.0, 0.0)) + vec3<f32>(1e-4, 0.0, 0.0));
    let t2 = cross(n, t1);
    let e = 0.04;
    let d1 = (bark_height(q + t1 * e).x - h.x) / e;
    let d2 = (bark_height(q + t2 * e).x - h.x) / e;
    // Reddish-brown to orange plates, some lobes weathered grey-pink, pale
    // peeling rims; furrows of black and orange-brown fibres.
    // Mostly weathered grey-mauve, with salmon where outer flakes have shed.
    // Darker and even: plates vary a little per ridge and with a few fresh
    // cinnamon patches, so a trunk reads as one bark, not a patchwork.
    let fresh = smoothstep(0.55, 0.75, bark_noise(q * vec3<f32>(2.4, 1.5, 2.4) + vec3<f32>(3.0, 1.0, 7.0))) * 0.6;
    let cinnamon = mix(vec3<f32>(0.16, 0.07, 0.04), vec3<f32>(0.20, 0.09, 0.05), h.y);
    var plates = mix(vec3<f32>(0.12, 0.085, 0.07), cinnamon, fresh) * (0.92 + 0.16 * bark_noise(q * 4.0));
    plates = mix(plates, vec3<f32>(0.20, 0.16, 0.14), h.w * 0.5);
    let fibre = smoothstep(0.35, 0.8, bark_noise(q * vec3<f32>(14.0, 2.0, 14.0)));
    let furrow = mix(vec3<f32>(0.02, 0.015, 0.012), vec3<f32>(0.12, 0.06, 0.03), fibre);
    let albedo = mix(plates, furrow, smoothstep(0.2, 0.7, h.z));
    let normal = normalize(n - (t1 * d1 + t2 * d2) * 0.07);
    return Bark(albedo, normal, mix(0.86, 0.98, h.z), mix(1.0, 0.35, h.z));
}
fn hash2(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash2(i), hash2(i + vec2<f32>(1.0, 0.0)), u.x), mix(hash2(i + vec2<f32>(0.0, 1.0)), hash2(i + vec2<f32>(1.0, 1.0)), u.x), u.y);
}

struct ThinFoliageResponse {direct:vec3<f32>,indirect:vec3<f32>,specular:vec3<f32>};
// Shared by actual procedural blades and their normal-distribution voxels.
// A modest diffuse-transmission fraction is a material approximation, not
// measured plant optics. Reflection/transmission SHARE the coloured diffuse
// budget. There is no fixed green backlight, world-up normal or night glow.
fn thin_foliage_response(base:vec3<f32>,n:vec3<f32>,v:vec3<f32>,rough:f32,leaf_fraction:f32,
    sun_e:vec3<f32>,front_irradiance:vec3<f32>,back_irradiance:vec3<f32>,env:vec3<f32>)->ThinFoliageResponse {
    let f0=vec3<f32>(0.04);
    let specular_weight=clamp(env_brdf(f0,rough,max(dot(n,v),1e-4)),vec3<f32>(0.0),vec3<f32>(1.0));
    let budget=clamp(base,vec3<f32>(0.0),vec3<f32>(1.0))*(1.0-specular_weight)/PI;
    let transmission=0.25*clamp(leaf_fraction,0.0,1.0);
    let reflection=1.0-transmission;
    let ndl=dot(n,light.sun_dir.xyz);
    var result:ThinFoliageResponse;
    // Light through a leaf comes out deeper and more saturated than light
    // off it (it crossed the chlorophyll once more), and most of it keeps
    // going forward: looking toward the sun through a crown it glows
    // (Unreal's two-sided foliage, as Joep asked). A few ALU ops, no pass;
    // the sun's shadow at the leaf already gates it.
    let deep=budget*clamp(base/max(max(base.r,max(base.g,base.b)),1e-4),vec3<f32>(0.0),vec3<f32>(1.0));
    let forward=1.0+3.0*pow(max(dot(-v,light.sun_dir.xyz),0.0),6.0);
    result.direct=sun_e*(budget*reflection*max(ndl,0.0)+deep*transmission*forward*max(-ndl,0.0))
        +sun_e*ggx(n,v,light.sun_dir.xyz,rough,f0);
    result.indirect=budget*(reflection*front_irradiance+transmission*back_irradiance);
    result.specular=env*specular_weight;
    return result;
}

// Skin and hair (material flags SKIN 1<<30 and HAIR 1<<31, scene.rs).
// Physically motivated approximations sized for a deferred compute pass:
// no extra textures, buffers or passes. docs/skin-and-hair.md.
const SKIN_ROUGHNESS: f32 = 0.7;
const HAIR_ROUGHNESS: f32 = 0.6;
// How far the diffuse term wraps past the terminator per channel: red light
// travels furthest under skin (pre-integrated skin, approximated).
const SKIN_WRAP: vec3<f32> = vec3<f32>(0.42, 0.16, 0.09);
// How far light diffuses through skin per channel, metres (Jensen et al.
// 2001, skin1: 3.67, 1.37, 0.68 mm), for light passing through thin parts:
// an ear glows, a cheek two centimetres behind the nose's shadow does not.
// (Four times this, to make up for the coarse sun map's texel of margin, lit
// whole cheeks orange once the map was fine enough to measure them.)
const SKIN_FACET: f32 = 1.5;
const SKIN_PATH: vec3<f32> = vec3<f32>(0.00367, 0.00137, 0.00068);

// x: skin share, y: hair share. SKIN alone: all skin; HAIR alone: all hair.
// SKIN|HAIR: the albedo alpha says per texel, 0 hair, 0.5 skin, 1 neither
// (the plain material), linear between (scene.rs SKIN).
fn skin_hair_shares(flags: u32, alpha: f32) -> vec2<f32> {
    let is_skin = (flags & 0x40000000u) != 0u;
    let is_hair = (flags & 0x80000000u) != 0u;
    if is_skin && is_hair {
        let a = clamp(alpha, 0.0, 1.0);
        return vec2<f32>(1.0 - abs(2.0 * a - 1.0), max(1.0 - 2.0 * a, 0.0));
    }
    return vec2<f32>(select(0.0, 1.0, is_skin), select(0.0, 1.0, is_hair));
}

// How much sunlight reaches p through the skin in front of it, per channel:
// the metres of material between p and the sun come from the sun shadow map
// (orthographic: depth is linear along the light), and the four texels
// round p are blended, so the result follows the surface and not the map's
// grid (one nearest texel printed its squares on a cheek turned from the
// sun). Shadow-map depth is only accurate to about a texel, which is added
// by `margin` texels so that silhouette texels never glow; none where the map
// does not cover p. w: the map's texel in metres at p.
// The maps are read directly (not passed as arguments): shaders that
// include this file declare them differently.
fn sun_through_skin(p: vec3<f32>, plane_n: vec3<f32>, reach: f32, margin: f32, slack: f32) -> vec4<f32> {
    var projection = light.shadow_view_proj;
    var world_texel = light.sky.w;
    // The finest sun clipmap level that covers p, else the shared map.
    var level = -1;
    let levels = select(0, i32(light.near_levels.x), light.near_shadow.z > 0.5);
    for (var k = 0; k < levels; k++) {
        if near_level_weight(k, p) > 0.5 {
            level = k;
            projection = near_level_projection(k);
            world_texel = near_level_texel(k);
            break;
        }
    }
    let c = projection * vec4<f32>(p, 1.0);
    let uv = c.xy * vec2<f32>(0.5, -0.5) + 0.5;
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || c.z < 0.0 || c.z > 1.0 { return vec4<f32>(0.0, 0.0, 0.0, 1e9); }
    var size = vec2<i32>(textureDimensions(shadow_tex));
    if level >= 0 { size = vec2<i32>(textureDimensions(near_shadow_tex)); }
    let q = uv * vec2<f32>(size) - 0.5;
    let base = vec2<i32>(floor(q));
    let f = fract(q);
    let row_z = vec3<f32>(projection[0].z, projection[1].z, projection[2].z);
    let metres = 1.0 / max(length(row_z), 1e-12);
    // Sixteen taps over a disc of 1.8 texels: skin
    // spreads light sideways over millimetres, and a shadow's edge drawn
    // from fewer taps shows the map's grid as teeth where the surface runs
    // along the sun. Each tap compares the map with the receiver's own
    // plane there, so a lit slope is not in its own way. The plane is the
    // surface only as far as the triangle reaches along the sun (`reach`,
    // metres): where the taps would leave that, they close up along the
    // slope and stay spread across it.
    var slope = vec2<f32>(0.0);
    let facing = dot(plane_n, row_z);
    if dot(plane_n, light.sun_dir.xyz) > 0.0 && abs(facing) > 1e-12 {
        let row_x = vec3<f32>(projection[0].x, projection[1].x, projection[2].x);
        let row_y = vec3<f32>(projection[0].y, projection[1].y, projection[2].y);
        let per_clip = -vec2<f32>(dot(plane_n, row_x) / dot(row_x, row_x), dot(plane_n, row_y) / dot(row_y, row_y)) * dot(row_z, row_z) / facing;
        slope = per_clip * vec2<f32>(2.0, -2.0) / vec2<f32>(size) * metres;
    }
    let steep = length(slope);
    let downhill = slope / max(steep, 1e-12);
    let squeeze = 1.0 - min(1.0, reach / max(steep * 1.8, 1e-12));
    var through = vec3<f32>(0.0);
    for (var i = 0; i < 16; i++) {
        // A golden-angle spiral: closed up along any slope, its taps still
        // lie at sixteen different places across it (a square lattice
        // leaves four, and the edge shows as four bands).
        let turn = f32(i) * 2.39996323;
        var offset = 1.8 * sqrt((f32(i) + 0.5) / 16.0) * vec2<f32>(cos(turn), sin(turn));
        offset -= downhill * dot(downhill, offset) * squeeze;
        let at = q + offset;
        let cell = vec2<i32>(floor(at));
        let t = at - vec2<f32>(cell);
        var d: array<f32, 4>;
        for (var k = 0; k < 4; k++) {
            let texel = clamp(cell + vec2<i32>(k & 1, k >> 1), vec2<i32>(0), size - 1);
            if level >= 0 {
                d[k] = textureLoad(near_shadow_tex, texel, level, 0);
            } else {
                d[k] = textureLoad(shadow_tex, texel, 0);
            }
        }
        let occluder = mix(mix(d[0], d[1], t.x), mix(d[2], d[3], t.x), t.y);
        let thickness = max((c.z - occluder) * metres + dot(slope, offset) + world_texel * margin - slack, 0.0);
        through += exp(-vec3<f32>(thickness) / SKIN_PATH) / 16.0;
    }
    return vec4<f32>(through, world_texel);
}

// Kajiya-Kay strand highlight around tangent t, normalised so that its
// peak follows the exponent like a microfacet lobe.
fn strand_lobe(t: vec3<f32>, h: vec3<f32>, exponent: f32) -> f32 {
    let th = dot(t, h);
    let sin_th = sqrt(max(1.0 - th * th, 0.0));
    return pow(sin_th, exponent) * smoothstep(-1.0, 0.0, th) * (exponent + 2.0) / (2.0 * PI);
}

// Sun radiance off skin and hair, already weighted by their shares; the
// caller keeps (1 - skin - hair) of the plain response. `shadow` is the
// sun's visibility, `sun_open` the sun's illuminance without it.
fn skin_hair_direct(albedo: vec3<f32>, n: vec3<f32>, surface_n: vec3<f32>, geo_n: vec3<f32>, v: vec3<f32>, l: vec3<f32>,
    p: vec3<f32>, uv: vec2<f32>, up: vec3<f32>, ahead: vec3<f32>, shadow: f32, far_shadow: f32, sun_open: vec3<f32>,
    skin: f32, hair: f32, gloss: f32) -> vec3<f32> {
    let ndl = dot(n, l);
    var out = vec3<f32>(0.0);
    if skin > 0.0 {
        // Diffuse: per-channel wrap, and a penumbra that lets red through
        // first (the shadow's own edge scatters under the skin too).
        let wrapped = clamp((vec3<f32>(ndl) + SKIN_WRAP) / (1.0 + SKIN_WRAP), vec3<f32>(0.0), vec3<f32>(1.0));
        var soft = pow(vec3<f32>(clamp(shadow, 0.0, 1.0)), vec3<f32>(0.5, 0.85, 1.0));
        // Where the sun map is fine enough to measure skin, the sun's
        // visibility is how much skin lies in its way: none on the lit
        // side, a sliver just past the terminator (which lets the wrap
        // through, red furthest), centimetres in a cast shadow. A lit or
        // dark test of the flat triangles cut the wrapped light off in
        // steps along a cheek. `far_shadow`: what lies beyond the fine map.
        let front = sun_through_skin(p, geo_n * select(-1.0, 1.0, dot(geo_n, surface_n) >= 0.0), skin_facet_reach, -0.25, skin_facet_slack);
        let fine = 1.0 - smoothstep(0.004, 0.010, front.w);
        soft = mix(soft, front.rgb * far_shadow, fine);
        let lit = dot(soft, vec3<f32>(0.2126, 0.7152, 0.0722));
        var rad = albedo / PI * sun_open * soft * wrapped * wrapped;
        // Transmission: light entering the sunlit side of thin parts (ears,
        // nostrils, fingers) leaves through the far side, reddened by the
        // path through the skin. Thickness from the sun shadow map.
        let back = clamp(-dot(surface_n, l), 0.0, 1.0);
        if back > 0.0 {
            rad += albedo / PI * sun_open * sun_through_skin(p, vec3<f32>(0.0), 0.0, 1.5, 0.0).rgb * back * 0.6;
        }
        // Two specular lobes: a thin oily sheen over the broader skin.
        let f0 = vec3<f32>(0.028);
        let spec = mix(ggx(n, v, l, min(SKIN_ROUGHNESS * 1.3, 1.0), f0), ggx(n, v, l, SKIN_ROUGHNESS * 0.6, f0), 0.3);
        rad += sun_open * lit * spec * gloss;
        out += rad * skin;
    }
    if hair > 0.0 {
        // Strands lie over the surface along the body's up axis.
        var t = up - n * dot(up, n);
        if dot(t, t) < 1e-4 { t = ahead - n * dot(ahead, n); }
        t = normalize(t);
        // Per-strand variation: hash across the strands (uv texel scale).
        let cell = floor(uv * 1024.0);
        let jitter = fract(sin(dot(cell, vec2<f32>(12.9898, 78.233))) * 43758.5453) - 0.5;
        let h = normalize(l + v);
        // R: the white cuticle reflection, shifted towards the tip. TRT:
        // light that entered the strand, tinted by it, shifted to the root.
        let r = strand_lobe(normalize(t + n * (-0.08 + jitter * 0.12)), h, 90.0);
        let trt = strand_lobe(normalize(t + n * (0.12 + jitter * 0.2)), h, 24.0)
            * (0.75 + jitter);
        let lit = sun_open * shadow;
        let facing = clamp(ndl + 0.25, 0.0, 1.25) / 1.25;
        var rad = lit * facing * (0.0025 * r + 0.02 * albedo * trt) * gloss;
        // Diffuse: hair scatters light around the strands; wrap generously.
        let diffuse_wrap = clamp((ndl + 0.35) / 1.35, 0.0, 1.0);
        rad += albedo / PI * lit * diffuse_wrap;
        out += rad * hair;
    }
    return out;
}

// Octahedral unit vectors, for the coarse normal.
fn oct_encode(n: vec3<f32>) -> vec2<f32> {
    let m = n / (abs(n.x) + abs(n.y) + abs(n.z));
    let wrapped = (1.0 - abs(m.yx)) * select(vec2<f32>(-1.0), vec2<f32>(1.0), m.xy >= vec2<f32>(0.0));
    return select(wrapped, m.xy, m.z >= 0.0);
}

fn oct_decode(e: vec2<f32>) -> vec3<f32> {
    var n = vec3<f32>(e, 1.0 - abs(e.x) - abs(e.y));
    let t = max(-n.z, 0.0);
    n.x += select(t, -t, n.x >= 0.0);
    n.y += select(t, -t, n.y >= 0.0);
    return normalize(n);
}

// The normal's two low bits hold which pixel of the quad it was taken at.
fn coarse_pack(so: SurfaceOut, sun: f32, sun_transmittance: f32, rep: u32) -> vec4<u32> {
    return vec4<u32>(pack2x16float(so.albedo.rg), pack2x16float(vec2<f32>(so.albedo.b, so.rough)),
        (pack2x16snorm(oct_encode(so.normal)) & ~3u) | rep, pack2x16float(vec2<f32>(sun, sun_transmittance)));
}

fn coarse_rep(c: vec2<i32>, t: vec4<u32>) -> vec2<i32> {
    return min(c * 2 + vec2<i32>(i32(t.z & 1u), i32((t.z >> 1u) & 1u)), vec2<i32>(view.size.xy) - 1);
}

// The coarse sun march at full-resolution pixel `px`: bilinear over the
// quads that sampled the same surface (depth within 2 %), lit if none did.
fn coarse_sun_at(px: vec2<i32>, depth: f32) -> f32 {
    let dims = vec2<i32>(textureDimensions(terrain_coarse_tex));
    let q = (vec2<f32>(px) + 0.5) * 0.5 - 0.5;
    let base = vec2<i32>(floor(q));
    let f = q - floor(q);
    var sum = 0.0;
    var total = 0.0;
    for (var k = 0; k < 4; k++) {
        let o = vec2<i32>(k & 1, k >> 1);
        let c = clamp(base + o, vec2<i32>(0), dims - 1);
        let t = textureLoad(terrain_coarse_tex, c, 0);
        let value = unpack2x16float(t.w).y;
        if value < 0.0 { continue; }
        let d = textureLoad(depth_tex, coarse_rep(c, t), 0);
        if abs(d - depth) > 0.02 * depth { continue; }
        let wgt = max(select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1), 1e-4);
        sum += value * wgt;
        total += wgt;
    }
    return select(1.0, sum / total, total > 0.0);
}

@compute @workgroup_size(8, 8)
fn terrain_coarse(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<u32>(view.size.xy);
    let half = (size + 1u) / 2u;
    if gid.x >= half.x || gid.y >= half.y { return; }
    let cx = vec2<i32>(gid.xy);
    let invalid = vec4<u32>(0u, pack2x16float(vec2<f32>(0.0, -1.0)), 0u, pack2x16float(vec2<f32>(0.0, -1.0)));
    // The quad's first terrain pixel (`rep`, kept in the normal's low bits)
    // stands for it, so ground between tree trunks still has a sample.
    var px = vec2<i32>(-1);
    var id = 0u;
    var rep = 0u;
    for (var k = 0u; k < 4u; k++) {
        let c = min(cx * 2 + vec2<i32>(i32(k & 1u), i32(k >> 1u)), vec2<i32>(size) - 1);
        if textureLoad(depth_tex, c, 0) <= 0.0 { continue; }
        let candidate = textureLoad(vis_tex, c, 0).r;
        if (candidate & 0x80000000u) != 0u { continue; }
        let flags = material(clusters[visible[candidate >> 7u].y].material).flags;
        if (flags & 2u) != 0u && ((flags >> 16u) & 0x3fffu) == 0u { px = c; id = candidate; rep = k; break; }
    }
    if px.x < 0 { textureStore(terrain_coarse_out, cx, invalid); return; }
    let vis = visible[id >> 7u];
    let cl = clusters[vis.y];
    let m = material(cl.material);
    let inst = instance_at(vis.x);
    let packed = tri_word(cl.first_tri + (id & 127u));
    var vi: array<u32, 3>;
    for (var k = 0u; k < 3u; k++) { vi[k] = vtx(cluster_vert(cl.first_vert + ((packed >> (8u * k)) & 0xffu))); }
    // The ray through the quad's centre, on the triangle of its terrain pixel.
    let centre = vec2<f32>(cx * 2) + 1.0;
    let ndc = vec2<f32>(centre.x / view.size.x * 2.0 - 1.0, 1.0 - centre.y / view.size.y * 2.0);
    let far = view.inv_view_proj * vec4<f32>(ndc, 1e-4, 1.0);
    let dir = normalize(far.xyz / far.w - view.cam.xyz);
    let p0 = wind_position(member_slot(vis.x), inst.r1.w, xform(inst, pos_of(vi[0])));
    let p1 = wind_position(member_slot(vis.x), inst.r1.w, xform(inst, pos_of(vi[1])));
    let p2 = wind_position(member_slot(vis.x), inst.r1.w, xform(inst, pos_of(vi[2])));
    let e1 = p1 - p0;
    let e2 = p2 - p0;
    let pv = cross(dir, e2);
    let det = dot(e1, pv);
    let tv = view.cam.xyz - p0;
    let inv = select(1.0 / det, 0.0, abs(det) < 1e-12);
    let b1 = clamp(dot(tv, pv) * inv, 0.0, 1.0);
    let b2 = clamp(dot(dir, cross(tv, e1)) * inv, 0.0, 1.0 - b1);
    let b0 = 1.0 - b1 - b2;
    let p = p0 * b0 + p1 * b1 + p2 * b2;
    var n = vec3<f32>(0.0);
    var uv = vec2<f32>(0.0);
    let w = array<f32, 3>(b0, b1, b2);
    for (var k = 0u; k < 3u; k++) {
        let v = vi[k];
        n += vec3<f32>(vertex_f32(v + 3u), vertex_f32(v + 4u), vertex_f32(v + 5u)) * w[k];
        uv += vec2<f32>(vertex_f32(v + 6u), vertex_f32(v + 7u)) * w[k];
    }
    n = normalize(wind_normal(member_slot(vis.x), xform_dir(inst, n)));
    let geo_n = normalize(cross(e1, e2));
    // One full-resolution pixel's footprint, as `shade` filters there.
    let px_x = view.inv_view_proj * vec4<f32>(ndc + vec2<f32>(2.0 * view.size.z, 0.0), 1e-4, 1.0);
    let px_y = view.inv_view_proj * vec4<f32>(ndc - vec2<f32>(0.0, 2.0 * view.size.w), 1e-4, 1.0);
    let dir_x = normalize(px_x.xyz / px_x.w - view.cam.xyz);
    let dir_y = normalize(px_y.xyz / px_y.w - view.cam.xyz);
    let plane_d = dot(p - view.cam.xyz, geo_n);
    let hit_x = view.cam.xyz + dir_x * (plane_d / select(dot(dir_x, geo_n), 1e-6, abs(dot(dir_x, geo_n)) < 1e-6));
    let hit_y = view.cam.xyz + dir_y * (plane_d / select(dot(dir_y, geo_n), 1e-6, abs(dot(dir_y, geo_n)) < 1e-6));
    let reach = distance(p, view.cam.xyz) * 0.05 + 0.05;
    fp.dx = clamp(hit_x - p, vec3<f32>(-reach), vec3<f32>(reach));
    fp.dy = clamp(hit_y - p, vec3<f32>(-reach), vec3<f32>(reach));
    let mip_scale = select(1.0, view.extra.z, view.extra.z > 0.0);
    fp.dx *= mip_scale;
    fp.dy *= mip_scale;
    // The sun march past the shared map (`distant_sun_visibility`), once per
    // quad for every terrain material: it is smooth at the canopy's cell
    // size, and per pixel in shade it cost a tenth of the frame.
    var sun_transmittance = 1.0;
    let l = light.sun_dir.xyz;
    if gi.vol_dims.w != 0u && (gi.vol_dims.w & 4u) == 0u && sun_coverage_fade(light.shadow_view_proj, p, 0.45) > 0.0 {
        sun_transmittance = transmittance(p + n * sign(dot(n, l) + 1e-4) * 0.2, l, 50.0);
    }
    if (m.flags & 32768u) == 0u {
        var out = invalid;
        out.z = rep;
        out.w = pack2x16float(vec2<f32>(1.0, sun_transmittance));
        textureStore(terrain_coarse_out, cx, out);
        return;
    }
    // As `shade`: the material's own layer relief first (TERRAIN|SURFACE
    // takes its generic textured branch).
    if m.layer >= 0 {
        var frame = uv_frame(e1, e2, vi);
        if dot(geo_n, n) < 0.0 { frame[0] = -frame[0]; frame[1] = -frame[1]; }
        n = perturb(n, nrm(m.layer, uv, uv_at(fp.dx, e1, e2, vi), uv_at(fp.dy, e1, e2, vi)), frame[0], frame[1]);
    }
    var local = vec3<f32>(0.0);
    for (var k = 0u; k < 3u; k++) { local += pos_of(vi[k]) * w[k]; }
    let corner = array<vec2<f32>, 3>(vec2<f32>(vertex_f32(vi[0] + 6u), vertex_f32(vi[0] + 7u)),
        vec2<f32>(vertex_f32(vi[1] + 6u), vertex_f32(vi[1] + 7u)), vec2<f32>(vertex_f32(vi[2] + 6u), vertex_f32(vi[2] + 7u)));
    let si = SurfaceIn(u32(max(-2 - source_layer(m.layer), 0)), vis.x, p, local, n, dir, vec3<f32>(b0, b1, b2),
        corner[0], corner[1], corner[2], fp.dx, fp.dy, m.tint, m.roughness, m.metal, m.scale, light.anim.x);
    field_sun = 1.0;
    let so = field_surface(si, m.layer);
    textureStore(terrain_coarse_out, cx, coarse_pack(so, field_sun, sun_transmittance, rep));
}

struct CoarseSurface { ok: bool, so: SurfaceOut, sun: f32 };
// The coarse material at full-resolution pixel `px` (depth `depth`): the
// four nearest quad centres, bilinear, if all are terrain on the same
// surface (depth within 2 %); else not ok and the pixel evaluates its own.
fn coarse_surface(px: vec2<i32>, depth: f32) -> CoarseSurface {
    var out: CoarseSurface;
    let dims = vec2<i32>(textureDimensions(terrain_coarse_tex));
    let q = (vec2<f32>(px) + 0.5) * 0.5 - 0.5;
    let base = vec2<i32>(floor(q));
    let f = q - floor(q);
    var albedo = vec3<f32>(0.0);
    var normal = vec3<f32>(0.0);
    var rough = 0.0;
    var sun = 0.0;
    for (var k = 0; k < 4; k++) {
        let o = vec2<i32>(k & 1, k >> 1);
        let c = clamp(base + o, vec2<i32>(0), dims - 1);
        let t = textureLoad(terrain_coarse_tex, c, 0);
        let ab = unpack2x16float(t.y);
        if ab.y < 0.0 { return out; }
        let d = textureLoad(depth_tex, coarse_rep(c, t), 0);
        if abs(d - depth) > 0.02 * depth { return out; }
        let wgt = select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1);
        let suns = unpack2x16float(t.w);
        albedo += vec3<f32>(unpack2x16float(t.x), ab.x) * wgt;
        rough += ab.y * wgt;
        normal += oct_decode(unpack2x16snorm(t.z)) * wgt;
        sun += suns.x * wgt;
    }
    out.ok = true;
    out.so = SurfaceOut(albedo, normalize(normal), rough, 0.0, vec3<f32>(0.0));
    out.sun = sun;
    return out;
}

@compute @workgroup_size(8, 8)
fn shade_voxels(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<u32>(view.size.xy);
    if gid.x >= size.x || gid.y >= size.y { return; }
    let px = vec2<i32>(gid.xy);
    let depth = textureLoad(depth_tex, px, 0);
    if depth <= 0.0 { return; }
    if (textureLoad(vis_tex, px, 0).r & 0x80000000u) == 0u { return; }
    let ndc = vec2<f32>((f32(gid.x) + 0.5) / view.size.x * 2.0 - 1.0, 1.0 - (f32(gid.y) + 0.5) / view.size.y * 2.0);
    let far = view.inv_view_proj * vec4<f32>(ndc, 1e-4, 1.0);
    shade_voxel(px, size, normalize(far.xyz / far.w - view.cam.xyz), depth);
}

// Sky for tiles without geometry; tiles with any are listed for `shade`.
// Same sky as `shade` writes for its own background pixels.
@compute @workgroup_size(8, 8)
fn classify(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(local_invocation_index) local: u32, @builtin(workgroup_id) wid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let size = vec2<u32>(view.size.xy);
    if local == 0u { atomicStore(&tile_has_geometry, 0u); }
    workgroupBarrier();
    let inside = all(gid.xy < size);
    var depth = 0.0;
    if inside { depth = textureLoad(depth_tex, vec2<i32>(gid.xy), 0); }
    if depth > 0.0 { atomicOr(&tile_has_geometry, 1u); }
    workgroupBarrier();
    let any_geometry = atomicLoad(&tile_has_geometry);
    if any_geometry != 0u {
        if local == 0u { shade_scratch.words[shade_scratch.tiles_at + atomicAdd(&shade_scratch.tiles_x, 1u)] = wid.y * groups.x + wid.x; }
        return;
    }
    if !inside { return; }
    let ndc = vec2<f32>((f32(gid.x) + 0.5) / view.size.x * 2.0 - 1.0, 1.0 - (f32(gid.y) + 0.5) / view.size.y * 2.0);
    let far = view.inv_view_proj * vec4<f32>(ndc, 1e-4, 1.0);
    let dir = normalize(far.xyz / far.w - view.cam.xyz);
    textureStore(out_hdr, vec2<i32>(gid.xy), vec4<f32>(visible_background_radiance(dir), 1.0));
}

@compute @workgroup_size(8, 8)
fn shade(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>, @builtin(local_invocation_index) local:u32) {
    let size = vec2<u32>(view.size.xy);
    // With sky tiles the dispatch walks the list of tiles with geometry.
    var tile = wid.xy;
    if SKY_TILES {
        let t = shade_scratch.words[shade_scratch.tiles_at + wid.x];
        let across = (size.x + 7u) / 8u;
        tile = vec2<u32>(t % across, t / across);
    }
    let gid = vec3<u32>(tile * 8u + lid.xy, 0u);
    var hidden=false;
    if SKIP_HIDDEN_MEADOW {
        // All invocations participate in barriers, including edge workgroups.
        // Count the actual early returns, with only one global atomic per tile.
        if local==0u {atomicStore(&hidden_meadow_count,0u);}
        workgroupBarrier();
        if light.sky.z==0.0 && all(gid.xy<size) && shade_scratch.width==size.x && shade_scratch.height==size.y {
            let owner=shade_scratch.words[2u*(gid.y*shade_scratch.row_pairs+gid.x)];
            hidden=(owner&0x80000000u)!=0u && textureLoad(depth_tex,vec2<i32>(gid.xy),0)>0.0;
        }
        if hidden {atomicAdd(&hidden_meadow_count,1u);}
        workgroupBarrier();
        if local==0u {let count=atomicLoad(&hidden_meadow_count);if count!=0u {atomicAdd(&shade_scratch.skipped,count);}}
    }
    if gid.x >= size.x || gid.y >= size.y {
        return;
    }
    let px = vec2<i32>(gid.xy);
    if hidden {
        // Later source alpha is exactly1. Keep a finite initialized background
        // for intervening premultiplied layers; no partial coverage is skipped.
        textureStore(out_hdr,px,vec4<f32>(0.0,0.0,0.0,1.0));return;
    }
    let ndc = vec2<f32>((f32(gid.x) + 0.5) / view.size.x * 2.0 - 1.0, 1.0 - (f32(gid.y) + 0.5) / view.size.y * 2.0);
    let far = view.inv_view_proj * vec4<f32>(ndc, 1e-4, 1.0);
    let dir = normalize(far.xyz / far.w - view.cam.xyz);
    let depth = textureLoad(depth_tex, px, 0);
    if depth <= 0.0 {
        if SHADE_CLASS != 1u { textureStore(out_hdr, px, vec4<f32>(visible_background_radiance(dir), 1.0)); }
        return;
    }
    if VOXEL_OCCLUSION && textureLoad(voxel_depth, px, 0) > depth { return; }
    let id = textureLoad(vis_tex, px, 0).r;
    if VOXEL_SURFACE && (id & 0x80000000u) != 0u {
        if !VOXEL_PASS { shade_voxel(px, size, dir, depth); }
        return;
    }
    let vis = visible[id >> 7u];
    let tri = id & 127u;
    let cl = clusters[vis.y];
    // Terrain: a TERRAIN material that is not a car part (car materials keep
    // their index in bits 16.. and may carry low flag bits of their own).
    let terrain_class = (material(cl.material).flags & 2u) != 0u && ((material(cl.material).flags >> 16u) & 0x3fffu) == 0u;
    if SHADE_CLASS != 0u && terrain_class != (SHADE_CLASS == 1u) { return; }
    let inst = instance_at(vis.x);
    irradiance_receiver_radius = select(0.0, 0.5, (inst.flags & 64u) != 0u);
    let packed = tri_word(cl.first_tri + tri);
    var vi: array<u32, 3>;
    for (var k = 0u; k < 3u; k++) {
        vi[k] = vtx(cluster_vert(cl.first_vert + ((packed >> (8u * k)) & 0xffu)));
    }
    // Match the actual deformed raster triangle before ray intersection and
    // texture derivatives. Rest-pose reconstruction would slide UVs and light.
    let p0 = wind_position(member_slot(vis.x), inst.r1.w, xform(inst, pos_of(vi[0])));
    let p1 = wind_position(member_slot(vis.x), inst.r1.w, xform(inst, pos_of(vi[1])));
    let p2 = wind_position(member_slot(vis.x), inst.r1.w, xform(inst, pos_of(vi[2])));
    // Ray-triangle intersection for perspective-correct barycentrics.
    let e1 = p1 - p0;
    let e2 = p2 - p0;
    let pv = cross(dir, e2);
    let det = dot(e1, pv);
    let tv = view.cam.xyz - p0;
    let inv = select(1.0 / det, 0.0, abs(det) < 1e-12);
    let b1 = clamp(dot(tv, pv) * inv, 0.0, 1.0);
    let qv = cross(tv, e1);
    let b2 = clamp(dot(dir, qv) * inv, 0.0, 1.0 - b1);
    let b0 = 1.0 - b1 - b2;
    let p = p0 * b0 + p1 * b1 + p2 * b2;
    var n = vec3<f32>(0.0);
    var uv = vec2<f32>(0.0);
    let w = array<f32, 3>(b0, b1, b2);
    for (var k = 0u; k < 3u; k++) {
        let v = vi[k];
        n += vec3<f32>(vertex_f32(v + 3u), vertex_f32(v + 4u), vertex_f32(v + 5u)) * w[k];
        uv += vec2<f32>(vertex_f32(v + 6u), vertex_f32(v + 7u)) * w[k];
    }
    n = normalize(wind_normal(member_slot(vis.x), xform_dir(inst, n)));
    let geo_n = normalize(cross(e1, e2));
    sun_receiver_tri[0]=p0;sun_receiver_tri[1]=p1;sun_receiver_tri[2]=p2;
    sun_receiver_tri_set=true;
    sun_receiver_error=cl.own_error*inst.scale;
    let m = material(cl.material);
    skin_facet_slack = 0.0;
    skin_facet_reach = 0.0;
    if FEATURE_SKIN_HAIR && (m.flags & 0x40000000u) != 0u {
        let outward = geo_n * select(-1.0, 1.0, dot(geo_n, n) >= 0.0);
        let facing = dot(outward, light.sun_dir.xyz);
        var turn = 0.0;
        for (var k = 0u; k < 3u; k++) {
            let v = vi[k];
            let nk = normalize(xform_dir(inst, vec3<f32>(vertex_f32(v + 3u), vertex_f32(v + 4u), vertex_f32(v + 5u))));
            turn = max(turn, abs(dot(nk, light.sun_dir.xyz) - facing));
        }
        let sun_side = vec3<f32>(dot(p0, light.sun_dir.xyz), dot(p1, light.sun_dir.xyz), dot(p2, light.sun_dir.xyz));
        skin_facet_reach = 0.5 * (max(sun_side.x, max(sun_side.y, sun_side.z)) - min(sun_side.x, min(sun_side.y, sun_side.z)));
        skin_facet_slack = max(length(p1 - p0), max(length(p2 - p1), length(p0 - p2))) * turn * SKIN_FACET;
    }
    // Two-sided cards and backfaces face the viewer.
    if dot(n, dir) > 0.0 && (m.flags & 8u) != 0u && (m.flags & 64u) == 0u {
        n = -n;
    }
    // Keep the smooth authored normal before texture relief. Visibility
    // traced around a flat triangle normal changes abruptly at every edge.
    let surface_n = n;
    let dist = distance(p, view.cam.xyz);
    set_pixel_footprint(ndc, p, geo_n, dist);
    // Reconstruct output-resolution detail rather than selecting coarse
    // input-resolution mip levels. Zero retains native and shadow behavior.
    let mip_scale = select(1.0, view.extra.z, view.extra.z > 0.0);
    fp.dx *= mip_scale;
    fp.dy *= mip_scale;
    let duv_x = uv_at(fp.dx, e1, e2, vi) ;
    let duv_y = uv_at(fp.dy, e1, e2, vi);
    var base = m.tint;
    // Albedo alpha of an opaque textured material: SKIN|HAIR reads it as
    // the texel's skin and hair shares (skin_hair_shares).
    var tex_alpha = 1.0;
    var nn = n; // the normal after normal maps
    var orm = vec3<f32>(1.0); // material map: occlusion, roughness, metalness
    // How much of its sheen a SKIN surface shows: its normal map's alpha
    // (0 where hair stands over it, the scalp of a head of grown hair).
    var gloss = 1.0;
    var meadow_ground_sky = 1.0;
    field_sun = 1.0;
    if FEATURE_TERRAIN && (m.flags & 2u) != 0u && (m.flags & 32768u) == 0u {
        // Terrain (src/shaders/terrain.gdshader): moss, needles, a grassy
        // verge, bare ground beside the road and rock where it is steep.
        // uv.x is metres from the road edge. Layers m.layer .. m.layer+4.
        let wuv = p.xz;
        let mapping = terrain_mapping(p);
        // uv.y=1+macro-up separates broad geology from micro-relief normals.
        // The zero legacy encoding falls back to the old authored normal.
        let up = select(n.y, clamp(uv.y - 1.0, 0.0, 1.0), uv.y >= 1.0);
        let m_rock = smoothstep(0.72, 0.5, up + (fbm(wuv / 3.0) - 0.5) * 0.15);
        var c = vec3<f32>(0.0);
        // A fully exposed rock face has no soil contribution. Reusing the
        // terrain material must not also sample four invisible soil layers.
        if m_rock < 1.0 {
            let moss = terrain_layer(m.layer, wuv, 3.2, mapping) * vec3<f32>(0.78, 0.86, 0.7);
            let needles = terrain_layer(m.layer + 1, wuv, 2.8, mapping) * vec3<f32>(0.72, 0.66, 0.6);
            let verge = terrain_layer(m.layer + 2, wuv, 2.6, mapping) * vec3<f32>(0.68, 0.94, 0.70);
            let ground = terrain_layer(m.layer + 3, wuv, 3.0, mapping) * vec3<f32>(0.85, 0.8, 0.75);
            let road_d = clamp(uv.x / 14.0, 0.0, 1.0);
            let m_needles = smoothstep(0.48, 0.62, fbm(wuv / 11.0 + 7.0));
            // Grass on the verge, thinning into the wood, patchy in clearings.
            let m_verge = clamp((1.0 - smoothstep(6.0, 24.0, uv.x)) * 0.85 + smoothstep(0.55, 0.8, fbm(wuv / 23.0 + 3.0)) * 0.5, 0.0, 1.0);
            var m_bare = 1.0 - smoothstep(0.03, 0.12, road_d + (fbm(wuv / 2.5) - 0.5) * 0.12);
            m_bare = max(m_bare, smoothstep(0.82, 0.62, up) * 0.8);
            c = mix(moss, needles, m_needles);
            c = mix(c, verge, m_verge);
            c = mix(c, ground, m_bare);
            var nt = mix(terrain_layer_normal(m.layer, wuv, 3.2, mapping), terrain_layer_normal(m.layer + 1, wuv, 2.8, mapping), m_needles);
            nt = mix(nt, terrain_layer_normal(m.layer + 2, wuv, 2.6, mapping), m_verge);
            nt = mix(nt, terrain_layer_normal(m.layer + 3, wuv, 3.0, mapping), m_bare);
            nn = perturb_xz(n, nt);
            c *= 0.78 + 0.34 * fbm(wuv / 31.0);
            let contact_strength=bitcast<f32>(meadow_substrate.control.w);
            if FEATURE_MEADOW && contact_strength>0.0 && meadow_substrate.contact.x>0.0 {
                let params=meadow_substrate.contact;
                // Use the same unanimated retained/jittered root identities as
                // source and far grass. No camera, distance, LOD or sun term.
                // Stay inside the field and away from road/support boundaries;
                // this conservative setback deliberately omits edge AO.
                let setback=0.9+params.z+params.w+1.415*params.y;
                let inside=all(wuv>=meadow_substrate.field_bounds.xy+vec2<f32>(setback))
                    && all(wuv<=meadow_substrate.field_bounds.zw-vec2<f32>(setback));
                let habitat=meadow_habitat_coverage(wuv,uv.x,up,meadow_substrate.control.y)
                    *smoothstep(setback,setback+0.5,uv.x)*(1.0-m_rock);
                if inside && habitat>0.0 {
                    // Substrate count scales its average palette coverage;
                    // contact instead selects those actual retained root IDs.
                    let source_cover=clamp(bitcast<f32>(meadow_substrate.control.z)/params.x,0.0,1.0);
                    let cover=meadow_contact_cover(wuv,meadow_substrate.control.y,params.x,params.y,source_cover);
                    meadow_ground_sky=1.0-clamp(contact_strength,0.0,0.7)*habitat*cover;
                }
            }
            if FEATURE_MEADOW && meadow_substrate.control.x != 0u {
                // Use the same habitat beyond the old roadside verge too.
                // Source top-coverage leaves actual soil/needles in the gaps;
                // it approximates rest coverage, not animated blade normals.
                let habitat = meadow_habitat_coverage(wuv, uv.x, up, meadow_substrate.control.y);
                let coverage = habitat * clamp(bitcast<f32>(meadow_substrate.control.z), 0.0, 1.0)
                    * smoothstep(1.5, 2.5, uv.x);
                let cell = vec2<i32>(floor(wuv / 0.45));
                let pigment = meadow_material_tint(wuv, cell, meadow_substrate.control.y);
                // Restrained existing soil texture detail, without importing
                // its unrelated green palette into the shared source albedo.
                let detail = clamp(0.94 + 0.3 * dot(verge, vec3<f32>(0.2126, 0.7152, 0.0722)), 0.94, 1.06);
                c = mix(c, meadow_substrate.leaf_base.rgb * pigment * detail, coverage);
            }
            // Past the meadow's blades the grass reads as a canopy: where the
            // field grows, or on the grassy verge without a field.
            var grassy = m_verge * (1.0 - m_bare);
            if FEATURE_MEADOW && meadow_substrate.control.x != 0u {
                grassy = meadow_habitat_coverage(wuv, uv.x, up, meadow_substrate.control.y) * smoothstep(1.5, 2.5, uv.x);
            }
            let canopy = meadow_canopy(c, nn, n, dir, wuv, max(length(fp.dx), length(fp.dy)), light.anim.x,
                meadow_substrate.control.y, grassy * (1.0 - m_rock));
            c = canopy.albedo;
            nn = canopy.normal;
        }
        if m_rock > 0.0 {
            var tw = pow(abs(n), vec3<f32>(4.0));
            tw /= tw.x + tw.y + tw.z;
            let rp = p.xz + p.yy * 0.7;
            let rl = m.layer + 4;
            let sampled = rock_surface(rl, p, n, tw);
            var rock = sampled.color * vec3<f32>(0.62, 0.63, 0.62);
            // Restrained continuous weathering, never alternating rock types
            // or green soil patches caused by a detailed face pointing up.
            rock *= 0.94 + 0.12 * fbm(rp / 31.0 + 11.0);
            c = mix(c, rock, m_rock);
            nn = safe_normalize(mix(nn, sampled.normal, m_rock));
        }
        let join = 1.0 - smoothstep(1.0, 2.2, uv.x + (vnoise(wuv / 1.1) - 0.5) * 0.18);
        if join > 0.0 {
            c = mix(c, shoulder_soil(m.layer + 3, p, mapping), join);
            nn = safe_normalize(mix(nn, perturb_xz(n, terrain_layer_normal(m.layer + 3, wuv, 3.0, mapping)), join));
        }
        base = c * mix(1.0, 0.7, light.fog.w) * m.tint;
    } else if (m.flags & 4u) != 0u {
        // Road (src/shaders/road.gdshader): uv = (across -1..1, metres
        // along); aux holds the surface per 2 m row (3 earth, 5 asphalt, 6 mud).
        var wuv = p.xz;
        let rows = aux_len();
        let row = u32(clamp(floor(uv.y / 2.0), 0.0, f32(max(rows, 1u) - 1u)));
        let surf = select(0u, aux_word(row), rows > 0u);
        let a = abs(uv.x);
        let firm = 1.0 - smoothstep(0.85, 1.05, a);
        let earth = select(0.0, 1.0, surf == 3u) * firm;
        let muddy = select(0.0, 1.0, surf == 6u) * firm;
        let asphalt = select(0.0, 1.0, surf == 5u) * firm;
        let road_wet = mix(light.fog.w, max(light.fog.w, 0.55), muddy);
        let rut = exp(-pow((a - 0.25) / 0.085, 2.0));
        let ridge = max(exp(-pow(uv.x / 0.07, 2.0)), exp(-pow((a - 0.8) / 0.08, 2.0)));
        let edge = smoothstep(0.62, 0.95, a + (vnoise(wuv / 2.0) - 0.5) * 0.25);
        let shoulder = smoothstep(1.0, 1.2, a);
        let aggregate = mix(2.6, 0.65, asphalt);
        var gr = tex_xz(m.layer, wuv / aggregate, aggregate).rgb;
        let g2 = tex_xz(m.layer, wuv / 11.0, 11.0).rgb;
        gr = mix(gr, g2, 0.3);
        let far_d = smoothstep(10.0, 45.0, dist);
        gr = mix(gr, tex(m.layer, wuv / 2.6, 9.0).rgb, far_d * 0.7);
        let lo = tex_xz(m.layer + 1, wuv / 2.2, 2.2).rgb;
        var c = gr * vec3<f32>(0.66, 0.62, 0.56);
        c = mix(c, c * 0.72, rut * 0.75);
        c = mix(c, mix(gr * vec3<f32>(0.7, 0.66, 0.6), lo * 0.8, 0.5), edge);
        c = mix(c, lo * 0.7, shoulder);
        c = mix(c, mix(gr, lo, 0.3) * vec3<f32>(0.78, 0.74, 0.68), ridge * 0.6);
        c *= 0.9 + 0.2 * vnoise(wuv / 7.0);
        // Vegetation is real ground cover. Painting green into the road's
        // centre produced flat green streaks with neither blades nor shadows.
        c *= 0.85 + 0.3 * smoothstep(0.3, 0.7, vnoise(wuv / 23.0 + 4.0));
        let smear = vnoise(vec2<f32>(uv.x * 70.0, uv.y * 0.6)) * 0.6 + vnoise(vec2<f32>(uv.x * 140.0, uv.y * 1.3)) * 0.4;
        let worn = clamp(rut * (0.65 + 0.35 * smear), 0.0, 1.0);
        let fines = tex(m.layer, wuv / 2.6, 6.0).rgb * vec3<f32>(0.66, 0.6, 0.52);
        var mud = mix(fines, lo * vec3<f32>(0.5, 0.44, 0.36), 0.35) * (0.82 + 0.3 * smear);
        mud *= mix(1.0, 0.7, road_wet);
        c = mix(c, mud, worn * 0.85);
        c *= mix(1.0, 0.62, road_wet);
        var earth_col = mix(g2 * vec3<f32>(0.55, 0.40, 0.26), lo * vec3<f32>(0.65, 0.50, 0.35), 0.4);
        earth_col *= (0.9 + 0.2 * smear) * mix(1.0, 0.65, road_wet);
        c = mix(c, earth_col, max(earth, muddy * 0.8));
        var asphalt_col = vec3<f32>(0.065, 0.068, 0.070) * (0.65 + 0.7 * dot(gr, vec3<f32>(0.333333)));
        asphalt_col *= mix(1.0, 0.88, worn) * mix(1.0, 0.7, light.fog.w);
        c = mix(c, asphalt_col, asphalt);
        base = c * m.tint;
        // Gravel relief, fading with distance; mud and asphalt are smoother.
        var rn = mix(nrm_xz(m.layer, wuv / aggregate, aggregate), nrm_xz(m.layer, wuv / 11.0, 11.0), 0.3);
        rn = mix(rn, nrm_xz(m.layer + 1, wuv / 2.2, 2.2), max(edge, shoulder));
        rn *= (1.0 - far_d * 0.6) * (1.0 - worn * 0.7) * (1.0 - asphalt * 0.6) * (1.0 - max(earth, muddy) * 0.4);
        nn = perturb_xz(n, rn);
        let join_noise = (vnoise(wuv / 1.1) - 0.5) * 0.08 * (1.0 - smoothstep(1.0, 1.2, a));
        let join = smoothstep(0.82, 1.22, a + join_noise);
        if join > 0.0 {
            let mapping = terrain_mapping(p);
            // m.tint boosts the driving lane, but must not boost only one
            // side of an otherwise identical terrain/shoulder junction.
            base = mix(base, shoulder_soil(m.layer + 1, p, mapping) * mix(1.0, 0.7, light.fog.w), join);
            nn = safe_normalize(mix(nn, perturb_xz(n, terrain_layer_normal(m.layer + 1, wuv, 3.0, mapping)), join));
        }
    } else if FEATURE_GRASS && (m.flags & 64u) != 0u {
        // grass_cards.gdshader: the photograph supplies blade colour, the
        // authored upward normal stays the same on both card sides.
        let height = clamp((p.y - inst.r1.w) / max(inst.scale * 0.5, 1e-4), 0.0, 1.0);
        let k = albedo_grad(m.layer, duv_x, duv_y);
        base = textureSampleGrad(albedo, plant_samp, uv, m.layer, duv_x * k, duv_y * k).rgb * m.tint;
        base *= mix(0.8, 1.0, height) * 1.25 * (1.0 - light.fog.w * 0.25);
        nn = vec3<f32>(0.0, 1.0, 0.0);
    } else if (m.flags & 16u) != 0u && m.layer >= 0 {
        var tw = pow(abs(n), vec3<f32>(4.0));
        tw /= tw.x + tw.y + tw.z;
        base = triplanar(m.layer, p, tw, m.scale, mat2x2<f32>(1.0, 0.0, 0.0, 1.0), 0.0) * m.tint;
        nn = triplanar_n(m.layer, p, n, tw, m.scale, mat2x2<f32>(1.0, 0.0, 0.0, 1.0), 0.0);
        orm = triplanar_orm(m.layer, p, tw, m.scale);
    } else if m.layer >= 0 && (m.flags & 16384u) != 0u {
        // LIVE_SCREEN writes mip 2 (512 square) and below. Never sample the
        // uninitialized higher-resolution levels or the unrelated normal map.
        let footprint = max(length(duv_x), length(duv_y)) * f32(textureDimensions(albedo).x);
        base = textureSampleLevel(albedo, samp, uv, m.layer, max(2.0, log2(max(footprint, 1.0)))).rgb * m.tint;
    } else if m.layer >= 0 && (m.flags & 2048u) == 0u {
        // PIXEL_ART samples the top mip: hard source pixels, no blur.
        let sharp = select(1.0, 0.0, (m.flags & 8192u) != 0u);
        let texel = tex_w(m.layer, uv, duv_x * sharp, duv_y * sharp);
        base = texel.rgb * m.tint;
        tex_alpha = texel.a;
        orm = orm_w(m.layer, uv, duv_x * sharp, duv_y * sharp);
        var frame = uv_frame(e1, e2, vi);
        if dot(geo_n, n) < 0.0 {
            // The far side of a two-sided card: its relief is the mirror image.
            frame[0] = -frame[0];
            frame[1] = -frame[1];
        }
        let relief = nrm_a(m.layer, uv, duv_x, duv_y);
        nn = perturb(n, relief.xy, frame[0], frame[1]);
        if FEATURE_SKIN_HAIR && (m.flags & 0x40000000u) != 0u { gloss = relief.z; }
    }
    n = nn;
    // Car parts get a procedural surface (car_material.wgsl): flags bits
    // 16.. hold 1 + their index in car_materials. Others keep the material.
    let car = (m.flags >> 16u) & 0x3fffu;
    var rough_m = m.roughness * orm.y;
    // Procedural bark (scene.rs BARK_LAYER): scale is the plate size in metres.
    // Not a game surface: their layer -2 - kind is -3 for kind 1.
    var bark_ao = 1.0;
    if m.layer == -3 && (m.flags & (4608u | 32768u)) == 0u {
        let b = bark_surface(p, n, m.scale);
        base = b.albedo;
        rough_m = b.rough;
        n = b.normal;
        bark_ao = b.ao;
    }
    if FEATURE_TRACKS && (m.flags & 6u)!=0u {
        let tracks=track_material(p);
        // Compacted mineral soil retains its texture and real illumination;
        // rubber appears only under actual asphalt slip. No final-color or
        // ambient darkening, and original normal-map relief is untouched.
        base*=vec3<f32>(1.0)-vec3<f32>(0.28,0.25,0.22)*tracks.x;
        base=mix(base,base*0.42,tracks.y*0.8);
        rough_m=mix(rough_m,max(rough_m-0.13,0.55),tracks.x);
        rough_m=mix(rough_m,0.72,tracks.y*0.65);
    }
    if FEATURE_PROCEDURAL && (m.flags & 2048u) != 0u {
        var rs: ProcRock;
        if m.layer >= 0 {
            rs = textured_rock(inst, vis.x, p, n, m.tint, m.scale, m.roughness, m.layer, fp.dx, fp.dy);
        } else {
            rs = proc_rock(inst, p, n, m.tint, m.scale, max(length(fp.dx), length(fp.dy)));
        }
        base = rs.base;
        rough_m = rs.rough;
        let face = n;
        n = rs.normal;
        if view_hf_dents() != 0u && max(length(fp.dx), length(fp.dy)) < 0.10 {
            let dent = rock_dents(p, face, n);
            base *= dent.w;
            n = dent.xyz;
        }
    }
    var metal = m.metal * orm.z;
    var coat = select(0.0, 1.0, (m.flags & 32u) != 0u);
    var coat_rough = 0.04;
    var coat_n = n;
    var glint = vec3<f32>(0.0);
    var car_debug = vec3<f32>(0.0);
    var surface_emit = vec3<f32>(0.0);
    if (m.flags & 32768u) != 0u {
        var local = vec3<f32>(0.0);
        for (var k = 0u; k < 3u; k++) { local += pos_of(vi[k]) * w[k]; }
        let corner = array<vec2<f32>, 3>(vec2<f32>(vertex_f32(vi[0] + 6u), vertex_f32(vi[0] + 7u)),
            vec2<f32>(vertex_f32(vi[1] + 6u), vertex_f32(vi[1] + 7u)), vec2<f32>(vertex_f32(vi[2] + 6u), vertex_f32(vi[2] + 7u)));
        let si = SurfaceIn(u32(max(-2 - source_layer(m.layer), 0)), vis.x, p, local, n, dir, vec3<f32>(b0, b1, b2),
            corner[0], corner[1], corner[2], fp.dx, fp.dy, m.tint, rough_m, metal, m.scale, light.anim.x);
        // TERRAIN + SURFACE: the engine's procedural terrain, not the game's.
        var so: SurfaceOut;
        if SHADE_CLASS != 2u && (m.flags & 2u) != 0u {
            var coarse: CoarseSurface;
            if TERRAIN_COARSE && max(length(fp.dx), length(fp.dy)) > COARSE_FOOTPRINT_M && !far_forest_full_res(si) { coarse = coarse_surface(px, depth); }
            if coarse.ok { so = coarse.so; field_sun = coarse.sun; } else { so = field_surface(si, m.layer); }
            if FAR_TREE_SHADOWS && near_sharp_cover(p) < 1.0 { far_tree_shadow = far_tree_cast(si); }
        } else { so = game_surface(si); }
        base = so.albedo;
        n = so.normal;
        coat_n = n;
        rough_m = so.rough;
        metal = so.metal;
        surface_emit = so.emission;
    }
    if FEATURE_CARS && car != 0u {
        let cs = car_surface(car_material(car - 1u), inst, p, n, dir, uv, base, m.roughness, m.metal);
        base = cs.base;
        n = cs.n;
        rough_m = cs.rough;
        metal = cs.metal;
        coat = cs.coat;
        coat_rough = cs.coat_rough;
        coat_n = cs.coat_n;
        glint = cs.glint;
        car_debug = cs.debug;
    }
    // Wounds (instance bits 27..31): wet blood stains in the body's own
    // space, spreading from a few blotches to most of it.
    let wounds = f32(inst.flags >> 27u) / 31.0;
    if wounds > 0.0 {
        let o = (pos_of(vi[0]) * b0 + pos_of(vi[1]) * b1 + pos_of(vi[2]) * b2) * inst.scale;
        let blot = 0.5 * vnoise(o.xy * 3.1 + o.z * 1.7) + 0.5 * vnoise(o.zy * 3.3 - o.x * 1.9 + 7.0);
        let fleck = vnoise(o.xz * 17.0 + o.y * 11.0);
        let cut = 1.0 - wounds * 0.55;
        let stain = max(smoothstep(cut, cut + 0.05, blot), smoothstep(0.98 - wounds * 0.25, 1.0 - wounds * 0.25, fleck));
        // Dried dark at the heart of a stain, fresher red at its edge.
        let blood = mix(vec3<f32>(0.075, 0.003, 0.003), vec3<f32>(0.03, 0.001, 0.001), smoothstep(cut + 0.05, cut + 0.2, blot));
        base = mix(base, blood, stain * 0.95);
        rough_m = mix(rough_m, 0.3, stain);
        metal = mix(metal, 0.0, stain);
    }
    // Weathering layers (material_maps.rs): moss always, wet and snow by
    // Lighting::wetness and Lighting::snow. Roads and verges keep their own wet look;
    // cars, skin, hair, lava and emissive surfaces take none.
    if (m.flags & (4u | 1024u | 4096u | 0xc0000000u)) == 0u && car == 0u {
        let wx = material_weather(cl.material);
        // The road-side terrain (TERRAIN without SURFACE) has its own wet look.
        let wet = select(light.fog.w * wx.x, 0.0, (m.flags & 32770u) == 2u);
        let snow = light.sh[0].w * wx.y;
        if wx.z > 0.0 || wet > 0.0 || snow > 0.0 {
            let up = surface_n.y;
            let grain = vnoise(p.xz * 2.3 + vec2<f32>(p.y * 1.7, 0.0)) * 0.65 + vnoise(p.xz * 9.1 - vec2<f32>(0.0, p.y * 5.3)) * 0.35;
            // Moss: upward and sheltered faces, broken up by the grain.
            let moss = wx.z * smoothstep(0.1, 0.6, up + (grain - 0.5) * 0.9) * smoothstep(0.0, 0.35, orm.x);
            if moss > 0.0 {
                let moss_col = mix(vec3<f32>(0.035, 0.06, 0.012), vec3<f32>(0.10, 0.13, 0.03), grain);
                base = mix(base, moss_col, moss);
                rough_m = mix(rough_m, 0.9, moss);
                metal *= 1.0 - moss;
                n = safe_normalize(mix(n, surface_n, moss * 0.4));
            }
            // Wet: porous surfaces soak up water and darken; every surface
            // gets a glossy film, and flat ground pools to a mirror.
            if wet > 0.0 {
                base *= mix(1.0, mix(0.82, 0.45, wx.w), wet);
                let pool = smoothstep(0.4, 1.0, wet) * smoothstep(0.85, 0.98, up) * smoothstep(0.58, 0.72, 1.0 - grain);
                rough_m = mix(rough_m, mix(0.28, 0.05, pool), wet * mix(0.55, 1.0, max(up, 0.0)));
                n = safe_normalize(mix(n, surface_n, pool));
            }
            // Snow settles on upward faces first, more of them as it grows.
            if snow > 0.0 {
                let reach = 0.95 - 0.75 * snow;
                let cover = smoothstep(reach, reach + 0.18, up + (grain - 0.5) * 0.3) * smoothstep(0.0, 0.3, snow);
                base = mix(base, vec3<f32>(0.82, 0.85, 0.9), cover);
                rough_m = mix(rough_m, 0.62, cover);
                metal *= 1.0 - cover;
                coat *= 1.0 - cover;
                n = safe_normalize(mix(n, surface_n, cover * 0.85));
            }
        }
    }
    // Skin and hair shares of this point (SKIN 1<<30, HAIR 1<<31).
    var skin = 0.0;
    var hair = 0.0;
    if FEATURE_SKIN_HAIR && (m.flags & 0xc0000000u) != 0u {
        let shares = skin_hair_shares(m.flags, tex_alpha);
        skin = shares.x;
        hair = shares.y;
        // Skin is a moist dielectric (IOR 1.4); hair a rough cuticle.
        rough_m = mix(rough_m, SKIN_ROUGHNESS, skin);
        rough_m = mix(rough_m, HAIR_ROUGHNESS, hair);
        metal *= 1.0 - skin - hair;
    }
    let l = light.sun_dir.xyz;
    let ndl = dot(n, l);
    var sun = max(ndl, 0.0);
    var back = vec3<f32>(0.0);
    var ao = bark_ao * orm.x;
    let grass = (m.flags & 64u) != 0u;
    let procedural_leaf = FEATURE_FOLIAGE && (m.flags & 512u) != 0u;
    let needles = procedural_leaf || (FEATURE_FOLIAGE && (m.flags & 9u) == 9u && (inst.flags & 1u) != 0u && !grass);
    if needles && !procedural_leaf {
        // foliage.gdshader: a little colour per tree, sunlight shining
        // through the needles from behind (BACKLIGHT), AO 0.85.
        // Procedural leaves carry stable authored colour. Hashing the moving
        // branch pivot would change their colour whenever the wind moves it.
        if !procedural_leaf {base *= 0.82 + 0.3 * hash2(vec2<f32>(inst.r0.w, inst.r2.w));}
        back = vec3<f32>(0.175, 0.21, 0.1) * max(-ndl, 0.0);
        ao = 0.85;
    } else if grass {
        let height = clamp((p.y - inst.r1.w) / max(inst.scale * 0.5, 1e-4), 0.0, 1.0);
        ao = mix(0.45, 1.0, height);
        back = vec3<f32>(0.12, 0.16, 0.05) * max(-ndl, 0.0);
    }
    // distant_sun_visibility reads it only past the far sun map, if any.
    if SHADE_CLASS == 1u && TERRAIN_COARSE && sun_coverage_fade(light.shadow_view_proj, p, 0.45) > 0.0
        && (far_sun.params.z <= 0.5 || sun_coverage_fade(far_sun.view_proj, p, far_sun.params.y) > 0.0) {
        coarse_sun_transmittance = coarse_sun_at(px, depth);
    }
    let terrain_error=select(0.0,world_visibility.params.z,(m.flags&6u)!=0u);
    var shadow = 1.0;
    // Sun visibility past the fine sun maps round p (skin_hair_direct).
    var far_shadow = 1.0;
    if FEATURE_SKIN_HAIR && skin > 0.0 && sun_visibility_required() {
        if light.near_levels.y < 0.5 {
            far_shadow = shadow_filter(shadow_tex, light.shadow_view_proj, light.sky.w, p + light.sun_dir.xyz * 3.0, surface_n, geo_n);
        }
        if FEATURE_WORLD_TERRAIN { far_shadow = min(far_shadow, world_terrain_sun_visibility(p, surface_n, terrain_error)); }
        far_shadow = mix(1.0, far_shadow, light.sun_color.w);
    }
    if sun > 0.0 || needles || (gi.vol_dims.w != 0u && (gi.vol_dims.w & 4u) == 0u) || coat > 0.0 || rough_m < 0.55 || skin + hair > 0.0 {
        var s_map = shadow_at_surface(p, surface_n * sign(dot(surface_n, l) + 1e-4), geo_n, terrain_error);
        // Forest ground past the sharp sun maps: each drawn tree's own
        // shadow (far_tree_cast) instead of the coarse maps' stand blobs.
        if FAR_TREE_SHADOWS && far_tree_shadow >= 0.0 {
            let sharp = near_sharp_cover(p);
            if sharp < 1.0 {
                var trees = 1.0 - far_tree_shadow;
                if FEATURE_WORLD_TERRAIN { trees = min(trees, world_terrain_sun_visibility(p, surface_n, terrain_error)); }
                s_map = mix(trees, s_map, sharp);
            }
        }
        shadow = mix(1.0, s_map, light.sun_color.w);
    }
    if FEATURE_GRASS_SHADE && shadow > 0.0 && (m.flags & 70u) != 0u {shadow*=mix(1.0,grass_sun_transmittance_receiver(p,l,(m.flags & 2u)!=0u),light.sun_color.w);}
    // Cloud shadows (sky_volume): the same weather field the clouds are drawn from.
    let cloud_shadow = cloud_sun_transmittance(p, l, light.clouds, light.cloud_motion);
    shadow *= cloud_shadow;
    far_shadow *= cloud_shadow;
    // Base layer: Lambert plus GGX, metals tint their reflection.
    let v = -dir;
    let ndv = max(dot(n, v), 1e-4);
    let rough = clamp(rough_m, 0.04, 1.0);
    var f0 = mix(vec3<f32>(0.04), base, metal);
    if needles {
        f0 = vec3<f32>(0.01); // foliage.gdshader: SPECULAR 0.125
    } else if grass {
        f0 = vec3<f32>(0.016); // grass_cards.gdshader: SPECULAR 0.2
    }
    f0 = mix(f0, vec3<f32>(0.028), skin);
    f0 = mix(f0, vec3<f32>(0.046), hair);
    let diffuse = base * (1.0 - metal) / PI;
    // Use the continuous surface hemisphere, unaffected by texture grain.
    // The triangle normal is only for intersection and texture derivatives.
    let visibility_n = surface_n;
    let sky_vis = sky_visibility(p, visibility_n);
    var meadow_edge=vec2<f32>(1.0);
    if FEATURE_MEADOW && (m.flags&6u)!=0u {meadow_edge=meadow_edge_visibility(p,surface_n,l);}
    if procedural_leaf {shadow*=NEEDLE_BRUSH_OWN;}
    let sun_e = light.sun_color.rgb * light.sun_dir.w * shadow * meadow_edge.y * field_sun;
    let refl = reflect(dir, n);
    let ssao = screen_space_ao(px, size, p, visibility_n, dist);
    let contact = contact_visibility(p + visibility_n * 0.025, visibility_n, refl, vis.x) * vec2<f32>(ssao, mix(1.0, ssao, 0.65));
    let bounce = gi.bounce.rgb * (gi.bounce.w * sky_vis + light.sun_dir.w * max(light.sun_dir.y, 0.0) * shadow);
    var reflection_vis = sky_vis;
    if coat > 0.0 || rough < 0.55 {
        reflection_vis = reflected_visibility(p, visibility_n, refl);
    }
    var direct:vec3<f32>;
    var indirect:vec3<f32>;
    var specular:vec3<f32>;
    if procedural_leaf {
        // Sky through the brush too, as voxels' crown_sky.
        let brush_sky=0.25+0.75*sqrt(NEEDLE_BRUSH_OWN);
        let sky_vis=sky_vis*brush_sky;
        let back_sky_vis=sky_visibility(p,-visibility_n)*brush_sky;
        let front_ambient=(sky_irradiance(n)*sky_vis+bounce*(1.0-sky_vis))*contact.x;
        let back_ambient=(sky_irradiance(-n)*back_sky_vis+bounce*(1.0-back_sky_vis))*contact.x;
        let spec_contact=mix(contact.y,contact.x,smoothstep(0.08,0.6,rough));
        let env=mix(bounce/PI,environment(refl,rough,bounce),reflection_vis)*spec_contact;
        let response=thin_foliage_response(base,n,v,rough,1.0,sun_e,front_ambient,back_ambient,env);
        direct=response.direct;indirect=response.indirect;specular=response.specular;
    } else if FEATURE_GRASS && grass {
        let height=clamp((p.y-inst.r1.w)/max(inst.scale*0.5,1e-4),0.0,1.0);
        let response=grass_leaf_response(base,height,v,sun_e,shadow,sky_vis,contact,rough);
        direct=response.direct;indirect=response.indirect;specular=response.specular;
    } else {
        // Both local contact models attenuate diffuse skylight. The accepted
        // edge field separately affects incident sun; headlights, bounce,
        // environment reflection, emission and fog are unchanged.
        var ambient=(sky_irradiance(n)*sky_vis*meadow_ground_sky*meadow_edge.x+bounce*(1.0-sky_vis))*ao*contact.x;
        let spec_contact=mix(contact.y,contact.x,smoothstep(0.08,0.6,rough));
        var env=mix(bounce/PI,environment(refl,rough,bounce),reflection_vis)*spec_contact;
        // Inside an indoor irradiance volume its probes replace sky and bounce.
        let cached=jittered_irradiance(p+visibility_n*0.05,px,visibility_n,n);
        let room=cached.value;
        if FEATURE_IRRADIANCE && room.w>0.0 {
          if aux_word(u32(gi.params.w)+7u) >= 1u {
            let access=clamp(room.x,0.0,1.0);
            // Attenuate only sky. Preserve the existing bounce term and add
            // the cached diffuse return from static surfaces under unit sky.
            let returned=sky_irradiance(vec3<f32>(0.0,1.0,0.0))*room.y
                +light.sun_color.rgb*light.sun_dir.w*max(room.z,0.0);
            let corrected=(sky_irradiance(n)*sky_vis*meadow_ground_sky*meadow_edge.x*access
                +bounce*(1.0-sky_vis)+returned+cached.emission*light.anim.y)*ao*contact.x;
            // Reflections of the sky are as occluded as the diffuse sky:
            // scale the environment by how much of the open-air ambient
            // reaches this point (Call of Duty normalises its reflection
            // probes by the baked lightmap the same way). A room then no
            // longer mirrors daylight; traced hits below still show the room.
            let open_air=dot(ambient,vec3<f32>(0.2126,0.7152,0.0722));
            let reached=dot(corrected,vec3<f32>(0.2126,0.7152,0.0722));
            env*=mix(1.0,clamp(reached/max(open_air,1e-4),0.0,1.0),room.w);
            ambient=mix(ambient,corrected,room.w);
          } else {
            ambient=mix(ambient,room.rgb*ao*contact.x,room.w);
            env=mix(env,irradiance_volume(p+visibility_n*0.05,refl).rgb/PI*spec_contact,room.w);
          }
        }
        // Traced reflections replace the environment where they hit. Under
        // a clear coat only the coat traces (below): it shows the reflection.
        let stochastic = sssr_on() && coat <= 0.0 && rough < gi.trace.y;
        // Reflection probes where a box holds the point: the room's live
        // capture, box-projected, at the roughness's mip, with the specular
        // occlusion; screen-space reflections replace it where they hit.
        let probe = probe_reflection(p, refl, rough);
        if probe.a > 0.0 {
            // At high roughness the reflected lobe approaches the hemisphere.
            // The room-centre capture can see a sunlit opening behind this
            // receiver. Anchor its broadest lobe to the local diffuse field.
            let broad = smoothstep(0.55, 1.0, rough) * max(room.w, 0.0);
            let local = mix(probe.rgb * spec_contact, ambient / PI, broad);
            env = mix(env, local, probe.a);
        }
        if coat <= 0.0 && rough < gi.trace.y && !stochastic {
            let traced = traced_reflection(p, visibility_n, refl, rough, px, size, dist);
            env = mix(env, traced.rgb, traced.a);
        }
        direct=diffuse*sun_e*sun+sun_e*back/PI+sun_e*ggx(n,v,l,rough,f0);
        if FEATURE_SKIN_HAIR && skin + hair > 0.0 {
            // Unshadowed sun: skin bleeds red into the shadow's edge and
            // light passes through ears; the shadow is applied inside.
            let sun_open=light.sun_color.rgb*light.sun_dir.w*meadow_edge.y*field_sun;
            let up=normalize(vec3<f32>(inst.r0.y,inst.r1.y,inst.r2.y));
            let ahead=normalize(vec3<f32>(inst.r0.z,inst.r1.z,inst.r2.z));
            let sh=skin_hair_direct(base*(1.0-metal),n,surface_n,geo_n,v,l,p,uv,up,ahead,shadow,far_shadow,sun_open,skin,hair,gloss);
            direct=direct*(1.0-skin-hair)+sh;
        }
        // Authored diffuse room return; retain crevice and contact occlusion.
        // z is irradiance in lux, independent of exposure and direct shadow bias.
        ambient += vec3<f32>(1.0,0.97,0.92)*light.fog_sampling.z*ao*contact.x;
        // The live first bounce of clustered lights (rsm.wgsl), irradiance.
        if FEATURE_CLUSTERED_LIGHTS { ambient += rsm_irradiance(px, p) * ao; }
        indirect=diffuse*ambient;
        specular=env*env_brdf(f0,rough,ndv)*ao*gloss;
        // Stochastic reflections: the resolve adds weight * reflection (the
        // environment where rays miss), so shade leaves the term out.
        if stochastic { sssr_write(px,size,p,visibility_n,v,rough,env_brdf(f0,rough,ndv)*ao*gloss,env,dist); specular=vec3<f32>(0.0); }
    }
    // Flake and swirl-mark glints sit under (or in) the coat. Non-car
    // materials retain a zero glint and their existing material coat.
    direct += sun_e * glint;
    if coat > 0.0 {
        // Clear coat: a thin glossy dielectric on top, with its own normal
        // and roughness (car paint); what it reflects the base does not get.
        let coat_v = max(dot(coat_n, v), 1e-4);
        let fc = (0.04 + 0.96 * pow(1.0 - coat_v, 5.0)) * coat;
        let coat_refl = reflect(dir, coat_n);
        let coat_contact = mix(contact.y, contact.x, smoothstep(0.08, 0.6, coat_rough));
        // The sun is a 0.5 degree disc, not a point: a mirror shows it no
        // smaller than roughness 0.05 does.
        direct = direct * (1.0 - fc) + sun_e * ggx(coat_n, v, l, max(coat_rough, 0.05), vec3<f32>(0.04)) * coat;
        indirect *= 1.0 - fc;
        var coat_env = mix(bounce / PI, environment(coat_refl, coat_rough, bounce), reflection_vis) * coat_contact;
        let traced = traced_reflection(p, visibility_n, coat_refl, coat_rough, px, size, dist);
        coat_env = mix(coat_env, traced.rgb, traced.a);
        specular = specular * (1.0 - fc) + coat_env * fc;
    }
    if FEATURE_LOCAL_LIGHTS && FEATURE_GRASS && grass {
        direct += headlight_grass_radiance(p,v,base,rough);
    } else if FEATURE_LOCAL_LIGHTS {
        direct += contact_headlight_radiance(p,n,visibility_n,v,diffuse,rough,f0,size,dist);
    }
    if FEATURE_CLUSTERED_LIGHTS && FEATURE_GRASS && grass {
        direct += clustered_grass_radiance(p,v,base,rough);
    } else if FEATURE_CLUSTERED_LIGHTS {
        direct += clustered_radiance(p,n,v,diffuse,rough,f0);
    }
    var rad = direct + indirect + specular;
    // Emission is scaled by Lighting::emissive_scale (exposure compensation).
    let emit = light.anim.y;
    if FEATURE_CARS { rad += car_lamp_emission(m.flags, base, inst, p) * emit; }
    // EMISSIVE: a glowing surface, `scale` is its radiance in nits for white;
    // with a texture layer the picture glows (screens), else the tint.
    // A light's face (negative scale) stays dark in a reflection probe's
    // capture: its light reaches glossy surfaces as its analytic specular.
    if (m.flags & 1024u) != 0u && !(gi.probes.x > 0.5 && m.scale < 0.0) { rad += select(m.tint, base, m.layer >= 0) * abs(m.scale) * emit; }
    rad += surface_emit * emit;
    // LAVA: molten emission plus sunlight reflected by the cooler surface.
    // MOLTEN_DEBRIS shares bit 7 with part_wind's history epoch, which wind
    // driven tree parts (PART_WIND, 32) toggle: only without that flag.
    if FEATURE_PROCEDURAL && (inst.flags & (128u | 32u)) == 128u {
        rad = proc_lava(p * 1200.0, light.anim.x * 6.0, max(length(fp.dx),length(fp.dy)) * 1200.0)
            * vec3<f32>(1.0,0.20,0.02) * 5000.0 * emit;
    }
    if FEATURE_PROCEDURAL && (m.flags & 4096u) != 0u {
        var lava_p = p; var lava_time = light.anim.x;
        var lava_footprint = max(length(fp.dx), length(fp.dy));
        if m.layer == -2 {
            // Small molten chambers: anchor the flowing field to the mesh UVs.
            lava_p = vec3<f32>(uv.x, 0.0, uv.y) * 3000.0;
            lava_footprint = max(length(duv_x), length(duv_y)) * 3000.0;
            lava_time *= 6.0;
        }
        let lava_color = proc_lava(lava_p, lava_time, lava_footprint);
        rad = lava_color * m.tint * m.scale * emit;
        if m.layer != -2 {
            // World pools retain their own glow in shade/night. The cooler
            // crust reflects more daylight than the bright molten veins;
            // reuse the stadium/dynamic sun visibility already sampled above.
            let hot = smoothstep(0.05, 0.7, lava_color.r);
            let lava_albedo = mix(vec3<f32>(0.20, 0.12, 0.06), vec3<f32>(0.08, 0.045, 0.02), hot);
            rad += lava_albedo * sun_e * sun / PI;
        }
    }
    let style = styled_light(material_kind(m), m.flags, p, n, base, rad, shadow);
    rad = style.radiance;
    // Hit flash (instance bits 16..23): the body itself lights up white,
    // nearly evenly; a slight facing term keeps its form readable.
    let flash = select(f32(instance_byte(inst.flags)) / 255.0, 0.0, lod_proxy(m.layer));
    if flash > 0.0 {
        let facing = clamp(dot(n, -dir), 0.0, 1.0);
        let white = vec3<f32>(1.0, 0.97, 0.92) * light.anim.z * (0.8 + 0.2 * facing);
        rad = mix(rad, white, clamp(flash * 0.9, 0.0, 1.0));
    }
    // A game's own glow on any surface (set_game_surface's game_glow).
    let glow = game_glow(p, vis.x, cl.material);
    rad = rad * (1.0 - glow.a) + glow.rgb;
    rad = apply_distance_fog(rad, dist, dir);
    // Froxel fog: the surface seen through the medium, plus what it scatters.
    if fog_on() {
        var fw: vec4<f32>;
        if gi.fog.w > 2.5 {
            // A probe capture: marched along the capture ray, to the grid's reach.
            fw = fog_capture(view.cam.xyz, view.cam.xyz + dir * min(dist, gi.fog.z), px);
        } else {
            // View depth from the depth buffer (reversed infinite Z: near / depth).
            fw = froxel_fog(px, size, view.params.y / max(textureLoad(depth_tex, px, 0), 1e-9));
        }
        rad = rad * fw.a + fw.rgb;
    }
    // Component diagnostics retain the exact material, pose and lighting.
    if !SHADE_DIAGNOSTICS {} else if light.sky.z > 0.5 && light.sky.z < 1.5 {
        rad = vec3<f32>(sky_vis) * 6000.0;
    } else if light.sky.z > 1.5 && light.sky.z < 2.5 {
        rad = base * 6000.0;
    } else if light.sky.z > 2.5 && light.sky.z < 3.5 {
        rad = direct;
    } else if light.sky.z > 3.5 && light.sky.z < 4.5 {
        rad = indirect;
    } else if light.sky.z > 4.5 && light.sky.z < 5.5 {
        rad = specular;
    } else if light.sky.z > 5.5 && light.sky.z < 6.5 {
        rad = vec3<f32>(contact.x) * 6000.0;
    } else if light.sky.z > 6.5 && light.sky.z < 7.5 {
        rad = vec3<f32>(reflection_vis) * 6000.0;
    } else if light.sky.z > 7.5 && light.sky.z < 8.5 {
        rad = vec3<f32>(shadow) * 6000.0;
    } else if light.sky.z > 8.5 && light.sky.z < 9.5 {
        rad = vec3<f32>(shadow_at_surface(p, surface_n * sign(dot(surface_n, l) + 1e-4), geo_n, terrain_error)) * 6000.0;
    } else if light.sky.z > 9.5 && light.sky.z < 10.5 {
        rad = vec3<f32>(grass_sun_transmittance_receiver(p,l,(m.flags & 2u)!=0u)) * 6000.0;
    } else if light.sky.z > 10.5 && light.sky.z < 11.5 {
        rad = (n * 0.5 + 0.5) * 6000.0;
    } else if light.sky.z > 11.5 && light.sky.z < 12.5 {
        rad = (surface_n * 0.5 + 0.5) * 6000.0;
    } else if light.sky.z > 12.5 && light.sky.z < 13.5 {
        rad = vec3<f32>(max(dot(n,l),0.0)) * 6000.0;
    } else if light.sky.z > 13.5 && light.sky.z < 14.5 {
        rad = vec3<f32>(max(dot(surface_n,l),0.0)) * 6000.0;
    } else if light.sky.z > 14.5 && light.sky.z < 15.5 {
        rad = vec3<f32>(shadow_filter_layer(near_shadow_tex,0,light.near_shadow_view_proj,light.near_shadow.x,p,surface_n*sign(dot(surface_n,l)+1e-4),geo_n)) * 6000.0;
    } else if light.sky.z > 15.5 && light.sky.z < 16.5 {
        rad = vec3<f32>(shadow_filter(shadow_tex,light.shadow_view_proj,light.sky.w,p,surface_n*sign(dot(surface_n,l)+1e-4),geo_n)) * 6000.0;
    } else if light.sky.z > 16.5 && light.sky.z < 17.5 {
        let clip=light.near_shadow_view_proj*vec4<f32>(p,1.0);
        let valid=light.near_shadow.z>0.5 && clip.z>=0.0 && clip.z<=1.0;
        rad=vec3<f32>(select(0.0,1.0-smoothstep(light.near_shadow.y,0.95,length(clip.xy)),valid))*6000.0;
    } else if light.sky.z > 19.5 && light.sky.z < 22.5 {
        rad = car_debug * 6000.0;
    } else if light.sky.z > 22.5 && light.sky.z < 24.5 {
        rad = world_terrain_debug(p,surface_n*sign(dot(surface_n,l)+1e-4),light.sky.z>23.5,terrain_error)*6000.0;
    } else if light.sky.z > 24.5 && light.sky.z < 25.5 {
        let page=foliage_page_visibility(p,surface_n*sign(dot(surface_n,l)+1e-4),geo_n);
        rad=select(vec3<f32>(1.0,0.0,1.0),vec3<f32>(page.x),page.y>0.5)*6000.0;
    } else if light.sky.z > 25.5 && light.sky.z < 26.5 {
        // The reflection ray's hit: red, green its screen position, blue the
        // march's confidence (black: no hit).
        let mh = march_hit(p, visibility_n, refl, px, size, dist);
        let ch = view.view_proj * vec4<f32>(mh.hit, 1.0);
        rad = select(vec3<f32>(0.0), vec3<f32>(ch.x / ch.w * 0.5 + 0.5, 0.5 - ch.y / ch.w * 0.5, mh.conf), mh.conf > 0.0) * 6000.0;
    } else if light.sky.z > 26.5 && light.sky.z < 28.5 {
        // 27: the reflection probes' reflection here (magenta: no box); 28:
        // the same as a mirror.
        let pr = probe_reflection(p, refl, select(rough, 0.0, light.sky.z > 27.5));
        rad = select(vec3<f32>(1.0, 0.0, 1.0), pr.rgb, pr.a > 0.0);
    }
    // fp16 target: a blast light next to a wall must saturate, not become inf.
    // Reflections exclude light faces (alpha 0). In the temporary finish
    // input the sign preserves that mask alongside the integer part tag.
    let face = (m.flags & 1024u) != 0u && m.scale < 0.0;
    let alpha = style_alpha(style.tag);
    let face_alpha = select(0.0, -alpha, WRITE_STYLE_TAGS);
    textureStore(out_hdr, px, vec4<f32>(min(rad, vec3<f32>(65504.0)), select(alpha, face_alpha, face)));
}

// Sky transmittance cache (canopy.rs): for one canopy level, the
// transmittance along the six axes through the same march sky_visibility
// would do (`transmittance`, 50 m), stored as L1 coefficients: a0 the mean,
// b half the opposing differences (T(w) = a0 + dot(b, w)). It runs only when
// the level's extinction was rebuilt, instead of five to ten marches per
// pixel. Sky visibility is smooth: one sample per 2x2x2 cells, written to
// all eight, keeps a moving level's rebake inside a frame.
@group(3) @binding(30) var sky_vis_out: texture_storage_3d<rgba16float, write>;
@group(3) @binding(31) var<uniform> sky_vis_level: vec4<u32>;

@compute @workgroup_size(4, 4, 4)
fn bake_sky_visibility(@builtin(global_invocation_id) gid: vec3<u32>) {
    let cell = gid * 2u;
    if any(cell >= gi.vol_dims.xyz) { return; }
    let origin = gi.vol_origins[sky_vis_level.x];
    let p = origin.xyz + (vec3<f32>(cell) + 1.0) * origin.w;
    let px = transmittance(p, vec3<f32>(1.0, 0.0, 0.0), 50.0);
    let nx = transmittance(p, vec3<f32>(-1.0, 0.0, 0.0), 50.0);
    let py = transmittance(p, vec3<f32>(0.0, 1.0, 0.0), 50.0);
    let ny = transmittance(p, vec3<f32>(0.0, -1.0, 0.0), 50.0);
    let pz = transmittance(p, vec3<f32>(0.0, 0.0, 1.0), 50.0);
    let nz = transmittance(p, vec3<f32>(0.0, 0.0, -1.0), 50.0);
    let value = vec4<f32>((px + nx + py + ny + pz + nz) / 6.0, (px - nx) * 0.5, (py - ny) * 0.5, (pz - nz) * 0.5);
    for (var k = 0u; k < 8u; k++) {
        let c = cell + vec3<u32>(k & 1u, (k >> 1u) & 1u, k >> 2u);
        if all(c < gi.vol_dims.xyz) { textureStore(sky_vis_out, vec3<i32>(c), value); }
    }
}

// Low-cost material lighting for the shared low graphics profile.
fn shade_simple_material(gid:vec3<u32>, terrain_lighting:bool) {
    if any(gid.xy>=vec2<u32>(view.size.xy)) {return;}
    let px=vec2<i32>(gid.xy);
    let ndc=(vec2<f32>(gid.xy)+0.5)/view.size.xy*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0);
    let far=view.inv_view_proj*vec4<f32>(ndc,1e-4,1.0);
    let dir=normalize(far.xyz/far.w-view.cam.xyz);
    let id=textureLoad(vis_tex,px,0).r;
    if textureLoad(depth_tex,px,0)<=0.0 || id==0xffffffffu {
        textureStore(out_hdr,px,vec4<f32>(visible_background_radiance(dir),1.0));return;
    }
    if SHADE_PROBE == 1u { textureStore(out_hdr,px,vec4<f32>(vec3<f32>(f32(id&255u)/255.0),1.0)); return; }
    let vis=visible[id>>7u];let cl=clusters[vis.y];let inst=instance_at(vis.x);
    irradiance_receiver_radius = select(0.0, 0.5, (inst.flags & 64u) != 0u);
    let packed=tri_word(cl.first_tri+(id&127u));
    var points:array<vec3<f32>,3>;var normals:array<vec3<f32>,3>;var uvs:array<vec2<f32>,3>;
    var local_points:array<vec3<f32>,3>;
    for(var k=0u;k<3u;k++) {
        let v=cluster_vert(cl.first_vert+((packed>>(8u*k))&255u))*8u;
        let pos=vec4<f32>(vertex_f32(v),vertex_f32(v+1u),vertex_f32(v+2u),1.0);
        local_points[k]=pos.xyz;
        points[k]=wind_position(member_slot(vis.x),inst.r1.w,vec3<f32>(dot(inst.r0,pos),dot(inst.r1,pos),dot(inst.r2,pos)));
        normals[k]=vec3<f32>(vertex_f32(v+3u),vertex_f32(v+4u),vertex_f32(v+5u));
        uvs[k]=vec2<f32>(vertex_f32(v+6u),vertex_f32(v+7u));
    }
    let e1=points[1]-points[0];let e2=points[2]-points[0];let pv=cross(dir,e2);
    let det=dot(e1,pv);let inv=select(1.0/det,0.0,abs(det)<1e-12);let tv=view.cam.xyz-points[0];
    let b1=clamp(dot(tv,pv)*inv,0.0,1.0);let b2=clamp(dot(dir,cross(tv,e1))*inv,0.0,1.0-b1);let b0=1.0-b1-b2;
    let p=points[0]*b0+points[1]*b1+points[2]*b2;
    let authored=normals[0]*b0+normals[1]*b1+normals[2]*b2;
    var n=normalize(wind_normal(member_slot(vis.x),xform_dir(inst,authored)));
    if dot(n,dir)>0.0 {n=-n;}
    if SHADE_PROBE == 2u { textureStore(out_hdr,px,vec4<f32>(n*0.5+0.5,1.0)); return; }
    let m=material(cl.material);var base=m.tint;
    var surface_emit = vec3<f32>(0.0);
    var sun_vis = 1.0;
    if terrain_lighting {
        let geometric_n = safe_normalize(cross(e1, e2));
        let plane_n = faceForward(geometric_n, dir, geometric_n);
        if (m.flags & 32768u) != 0u {
            set_pixel_footprint(ndc, p, plane_n, distance(p, view.cam.xyz));
            let local = local_points[0]*b0 + local_points[1]*b1 + local_points[2]*b2;
            let si = SurfaceIn(u32(max(-2 - source_layer(m.layer), 0)), vis.x, p, local, n, dir, vec3<f32>(b0,b1,b2), uvs[0],uvs[1],uvs[2],
                               fp.dx, fp.dy, m.tint, m.roughness, m.metal, m.scale, light.anim.x);
            var surface: SurfaceOut;
            if (m.flags & 2u) != 0u { surface = field_surface(si, m.layer); }
            else { surface = game_surface(si); }
            base = surface.albedo;
            n = surface.normal;
            surface_emit = surface.emission;
        } else if m.layer == -3 && (m.flags & (4608u | 32768u)) == 0u {
            let bark = bark_surface(p, n, m.scale);
            base = bark.albedo;
            n = bark.normal;
        }
        if sun_visibility_required() {
            sun_vis = shadow_detail_receiver(p, n, plane_n);
            if world_visibility.params.w > 0.5 { sun_vis = min(sun_vis, world_terrain_baked_sun(p)); }
        }
        sun_vis *= cloud_sun_transmittance(p, light.sun_dir.xyz, light.clouds, light.cloud_motion);
    }
    if SHADE_PROBE == 3u { textureStore(out_hdr,px,vec4<f32>(base*(0.2+sun_vis)+surface_emit,1.0)); return; }
    if m.layer>=0 && !(terrain_lighting && (m.flags&32770u)==32770u) {let uv=uvs[0]*b0+uvs[1]*b1+uvs[2]*b2;
        base*=textureSampleLevel(albedo,samp,uv,m.layer,max(log2(max(distance(p,view.cam.xyz)*0.03,1.0)),albedo_lod_floor(m.layer))).rgb;}
    var ambient=sky_irradiance(n);
    if FEATURE_IRRADIANCE && gi.params.w > 0.0 {
        let cached=irradiance_sample(p+n*0.05,n);
        let room=cached.value;
        if room.w>0.0 {
            var corrected=room.rgb;
            if aux_word(u32(gi.params.w)+7u)>=1u {
                corrected=ambient*clamp(room.x,0.0,1.0) + sky_irradiance(vec3<f32>(0.0,1.0,0.0))*room.y
                    + light.sun_color.rgb*light.sun_dir.w*max(room.z,0.0) + cached.emission*light.anim.y;
            }
            ambient=mix(ambient,corrected,room.w);
        }
    }
    var rad=base*(ambient+light.sun_color.rgb*light.sun_dir.w*sun_vis*max(dot(n,light.sun_dir.xyz),0.0))/3.14159265;
    if SHADE_PROBE == 4u { textureStore(out_hdr,px,vec4<f32>(min(rad,vec3<f32>(65504.0)),1.0)); return; }
    for(var index=0u;index<incident_light_count(p);index++) {
        if dot(n,incident_light_position(index)-p)<=0.0 {continue;}
        let sample=incident_light_sample(index,p,n);
        rad+=base*sample.irradiance*max(dot(n,sample.direction),0.0)/PI;
    }
    if SHADE_PROBE == 5u { textureStore(out_hdr,px,vec4<f32>(min(rad,vec3<f32>(65504.0)),1.0)); return; }
    rad += surface_emit * light.anim.y;
    if (m.flags&1024u)!=0u {rad+=base*abs(m.scale)*light.anim.y;}
    if (m.flags&4096u)!=0u {rad=base*m.scale*light.anim.y;}
    let style = styled_light(material_kind(m), m.flags, p, n, base, rad, 1.0);
    rad = style.radiance;
    if SHADE_PROBE == 6u { textureStore(out_hdr,px,vec4<f32>(min(rad,vec3<f32>(65504.0)),1.0)); return; }
    let flash=select(f32(instance_byte(inst.flags)) / 255.0, 0.0, lod_proxy(m.layer));
    rad=mix(rad,vec3<f32>(1.0,0.97,0.92)*light.anim.z*(0.8+0.2*max(dot(n,-dir),0.0)),flash*0.9);
    let glow=game_glow(p,vis.x,cl.material);rad=rad*(1.0-glow.a)+glow.rgb;
    rad=apply_distance_fog(rad,distance(p,view.cam.xyz),dir);
    if light.sky.z==2.0 {rad=base*6000.0;}
    if light.sky.z==11.0 {rad=(n*0.5+0.5)*6000.0;}
    textureStore(out_hdr,px,vec4<f32>(min(rad,vec3<f32>(65504.0)),style_alpha(style.tag)));
}

@compute @workgroup_size(8, 8)
fn shade_simple(@builtin(global_invocation_id) gid:vec3<u32>) { shade_simple_material(gid, false); }

@compute @workgroup_size(8, 8)
fn shade_compact(@builtin(global_invocation_id) gid:vec3<u32>) { shade_simple_material(gid, true); }
