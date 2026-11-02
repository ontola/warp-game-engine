// Deferred shading from the visibility buffer: per pixel, find the
// triangle, intersect the view ray with it for exact barycentrics, and
// light the surface in physical units (lux, cd/m2). Output is linear HDR.

struct View {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    cam: vec4<f32>,
    planes: array<vec4<f32>, 6>,
    params: vec4<f32>,
    size: vec4<f32>,
    extra: vec4<f32>,
    grass: vec4<f32>,
    fog_cull: vec4<f32>,
};

struct Light {
    sun_dir: vec4<f32>,      // towards the sun; w: illuminance in lux
    sun_color: vec4<f32>,    // linear colour; w: shadow strength
    shadow_view_proj: mat4x4<f32>,
    sky: vec4<f32>,          // x: sky luminance scale, y: sky rotation (rad), z: diagnostic mode, w: shadow texel (m)
    fog_color: vec4<f32>,    // legacy horizon radiance; w: legacy fog start (m)
    fog: vec4<f32>,          // x: legacy end (m), y: extinction/m, z: aerosol scale height (m; 0 legacy), w: wetness
    fog_scattering: vec4<f32>, // RGB single-scattering albedo; w: aerosol phase asymmetry
    sh: array<vec4<f32>, 9>, // sky irradiance, SH9, already in lux
    near_shadow_view_proj: mat4x4<f32>,
    near_shadow: vec4<f32>, // world texel size, blend-start radius, enabled
    night_stars: vec4<f32>, // peak radiance, zenith optical depth, zero-sun fast path, seed
    anim: vec4<f32>,        // x: seconds for animated procedural materials, y: emission scale, z: radiance shown as white
};

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
    pad: u32,
};

struct Material {
    tint: vec3<f32>,
    layer: i32,
    roughness: f32,
    flags: u32,
    scale: f32,
    metal: f32,
};

@group(0) @binding(0) var<storage, read> vertices: array<f32>;
@group(0) @binding(1) var<storage, read> cluster_verts: array<u32>;
@group(0) @binding(2) var<storage, read> tris: array<u32>;
@group(0) @binding(3) var<storage, read> clusters: array<Cluster>;
@group(0) @binding(4) var<storage, read> instances: array<Instance>;

// Virtual assembly members (vegetation_asset AttachLimits::virtual_members):
// bit 31, the branch owner (19 bits) and its template (12 bits). The owner's
// rows are its part_wind pose and its first_cluster packs the templates
// (start << 12 | count); a member is pose x template.
const VIRTUAL_MEMBER: u32 = 0x80000000u;
fn member_slot(id: u32) -> u32 { return select(id, (id >> 12u) & 0x7ffffu, (id & VIRTUAL_MEMBER) != 0u); }
fn compose_member(owner: Instance, t: Instance) -> Instance {
    var m = t;
    m.r0 = owner.r0.x * t.r0 + owner.r0.y * t.r1 + owner.r0.z * t.r2 + vec4<f32>(0.0, 0.0, 0.0, owner.r0.w);
    m.r1 = owner.r1.x * t.r0 + owner.r1.y * t.r1 + owner.r1.z * t.r2 + vec4<f32>(0.0, 0.0, 0.0, owner.r1.w);
    m.r2 = owner.r2.x * t.r0 + owner.r2.y * t.r1 + owner.r2.z * t.r2 + vec4<f32>(0.0, 0.0, 0.0, owner.r2.w);
    let c = vec4<f32>(t.sphere.xyz, 1.0);
    m.sphere = vec4<f32>(dot(owner.r0, c), dot(owner.r1, c), dot(owner.r2, c), -t.sphere.w * owner.scale);
    m.scale = t.scale * owner.scale;
    // The owner's history epoch (bit 7) marks a woken pose for motion.
    m.pad = (t.pad & ~0x80u) | (owner.pad & 0x80u);
    return m;
}
fn instance_at(id: u32) -> Instance {
    if (id & VIRTUAL_MEMBER) == 0u { return instances[id]; }
    let owner = instances[(id >> 12u) & 0x7ffffu];
    return compose_member(owner, instances[(owner.first_cluster >> 12u) + (id & 0xfffu)]);
}
@group(0) @binding(5) var<storage, read> materials: array<Material>;
@group(0) @binding(6) var albedo: texture_2d_array<f32>;
@group(0) @binding(7) var samp: sampler;
@group(0) @binding(8) var<storage, read> aux: array<u32>;
@group(0) @binding(9) var normal_tex: texture_2d_array<f32>;
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
@group(1) @binding(12) var<storage, read> solids: array<SolidOccluder>;
@group(1) @binding(13) var canopy_tex: texture_3d<f32>;
@group(1) @binding(14) var canopy_sampler: sampler;
@group(1) @binding(15) var near_shadow_tex: texture_depth_2d;
// The texture binding budget is already full. Copy the certified source IDs
// into this buffer; the opaque lighting pass reads one owner word per pixel.
struct HiddenMeadow {
    skipped:atomic<u32>, width:u32, height:u32, row_pairs:u32,
    padding:array<u32,60>, ids:array<vec2<u32>>,
};
@group(1) @binding(16) var<storage,read_write> hidden_meadow:HiddenMeadow;
// Scene features, specialised per pipeline. All default to true: every
// shader that includes this file behaves as before. The main shade pipeline
// sets a feature false when its scene cannot use it (Renderer::new), so a
// game without cars, grass, tracks or forests does not compile their code.
override FEATURE_CARS: bool = true;
// Procedural rock and lava surfaces (procedural_surface.wgsl).
override FEATURE_PROCEDURAL: bool = true;
override FEATURE_GRASS: bool = true;
override FEATURE_TRACKS: bool = true;
override FEATURE_WORLD_TERRAIN: bool = true;
override FEATURE_FOLIAGE_PAGES: bool = true;
override FEATURE_MEADOW: bool = true;
override FEATURE_TERRAIN: bool = true;
override FEATURE_FOLIAGE: bool = true;
override SKIP_HIDDEN_MEADOW:bool=false;
var<workgroup> hidden_meadow_count:atomic<u32>;
// Sky tiles: `classify` writes the sky of 8x8 tiles without geometry and
// lists the others; `shade` then runs (indirectly) over listed tiles only,
// so pure sky never occupies the heavy shading kernel.
override SKY_TILES: bool = false;
struct ShadeTiles { x: atomic<u32>, y: u32, z: u32, pad: u32, tiles: array<u32> };
@group(1) @binding(17) var<storage,read_write> shade_tiles: ShadeTiles;
// Rasterized voxel trees (voxel_forest.rs Stage::Splats): their depth,
// reversed Z, 0 where there is none. A pixel they cover in front of the
// opaque surface is theirs; its composite replaces whatever shade wrote.
override VOXEL_OCCLUSION: bool = false;
@group(1) @binding(18) var voxel_depth: texture_depth_2d;
// Rasterized voxel trees in the scene's own visibility buffer (bit 31 of the
// id, voxel_forest Stage::Splats): their surface, packed by
// voxel_splat_gbuffer, lit here like the leaves they stand for.
override VOXEL_SURFACE: bool = false;
const VOXEL_SKY_LIFT: f32 = 1.0;
@group(1) @binding(20) var voxel_surface: texture_2d<u32>;

// A voxel pixel: the same sun, sky, contact and fog terms as a procedural
// leaf in `shade`, from the voxel's albedo, picked normal lobe and leaf
// fraction instead of a triangle's.
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
    let own = f32((g.w >> 16u) & 255u) / 255.0;
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
    if sun_coverage_fade(light.shadow_view_proj, p, 0.45) < 1.0 {
        others = shadow_at_surface(p, n * sign(dot(n, l) + 1e-4), vec3<f32>(0.0), 0.0);
    } else if FEATURE_WORLD_TERRAIN && sun_visibility_required() {
        others = world_terrain_sun_visibility(p, n, 0.0);
    }
    // Out there its neighbours' shade is folded into its own: the plant's
    // full transmittance instead of the square root the maps pair with.
    let outside = sun_coverage_fade(light.shadow_view_proj, p, 0.45);
    var shadow = mix(1.0, others * mix(sqrt(own), own * own, outside), light.sun_color.w);
    let sun_e = light.sun_color.rgb * light.sun_dir.w * shadow;
    let sky_vis = sky_visibility(p + n * VOXEL_SKY_LIFT, n);
    let back_sky_vis = sky_visibility(p - n * VOXEL_SKY_LIFT, -n);
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
    rad = apply_distance_fog(rad, dist, dir);
    if light.sky.z > 1.5 && light.sky.z < 2.5 { rad = base * 6000.0; }
    textureStore(out_hdr, px, vec4<f32>(min(rad, vec3<f32>(65504.0)), 1.0));
}
// Coarse terrain material (TERRAIN_COARSE): field_surface once per 2x2
// pixels, at each quad's centre, packed by terrain_coarse below. Where a
// pixel spans more than COARSE_FOOTPRINT_M of ground, shade interpolates it
// instead of evaluating the material itself; lighting stays per pixel.
// 8 cm: from about 100 m at 1440p, also straight down from 280 m (a 0.3 m
// threshold never triggered there). A/B at the playtest cameras: PSNR 44-46
// dB, under 0.01 % of pixels off by more than 3 %.
override TERRAIN_COARSE: bool = false;
const COARSE_FOOTPRINT_M: f32 = 0.08;
@group(1) @binding(19) var terrain_coarse_tex: texture_2d<u32>;
@group(2) @binding(0) var terrain_coarse_out: texture_storage_2d<rgba32uint, write>;
var<workgroup> tile_has_geometry: atomic<u32>;

struct Gi {
    vol_origins: array<vec4<f32>, 3>,
    vol_anchors: array<vec4<f32>, 3>,
    vol_dims: vec4<u32>,
    hf: vec4<f32>,
    hf_dims: vec4<u32>,
    bounce: vec4<f32>,
    params: vec4<f32>, // contact on, reflection on, sun-free sky cap, irradiance volume aux offset (0: none)
};

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
    return f32((aux[o + 8u + (idx >> 2u)] >> ((idx & 3u) * 8u)) & 0xffu);
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
    let dims = vec3<u32>(aux[o + 4u], aux[o + 5u], aux[o + 6u]);
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
fn screen_space_ao(px: vec2<i32>, size: vec2<u32>, p: vec3<f32>, n: vec3<f32>, dist: f32) -> f32 {
    if gi.params.x < 0.5 || dist >= 48.0 { return 1.0; }
    let d0 = textureLoad(depth_tex, px, 0);
    if d0 <= 0.0001 { return 1.0; }
    let fade = 1.0 - smoothstep(30.0, 48.0, dist);
    let r = i32(clamp(0.75 * view.cam.w / max(dist, 0.8), 2.0, 14.0));
    let max_px = vec2<i32>(size) - vec2<i32>(1);
    let d_r = textureLoad(depth_tex, clamp(px + vec2<i32>(r, 0), vec2<i32>(0), max_px), 0) - d0;
    let d_l = textureLoad(depth_tex, clamp(px + vec2<i32>(-r, 0), vec2<i32>(0), max_px), 0) - d0;
    let d_u = textureLoad(depth_tex, clamp(px + vec2<i32>(0, -r), vec2<i32>(0), max_px), 0) - d0;
    let d_d = textureLoad(depth_tex, clamp(px + vec2<i32>(0, r), vec2<i32>(0), max_px), 0) - d0;
    let scale = dist * dist * 8.5;
    let ox = clamp((d_r + d_l) * scale, 0.0, 1.0) * step(abs(d_r - d_l) * scale, 1.6);
    let oy = clamp((d_u + d_d) * scale, 0.0, 1.0) * step(abs(d_u - d_d) * scale, 1.6);
    return 1.0 - clamp((ox + oy) * 0.22 * fade, 0.0, 0.48);
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
        let bounds = solids[i];
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

fn vtx(i: u32) -> u32 {
    return i * 8u;
}

fn pos_of(v: u32) -> vec3<f32> {
    return vec3<f32>(vertices[v], vertices[v + 1u], vertices[v + 2u]);
}

fn xform(inst: Instance, p: vec3<f32>) -> vec3<f32> {
    let q = vec4<f32>(p, 1.0);
    return vec3<f32>(dot(inst.r0, q), dot(inst.r1, q), dot(inst.r2, q));
}

fn xform_dir(inst: Instance, d: vec3<f32>) -> vec3<f32> {
    if (inst.pad & 32u) != 0u {
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

fn sky_dir_uv(d: vec3<f32>) -> vec2<f32> {
    let a = light.sky.y;
    let x = d.x * cos(a) - d.z * sin(a);
    let z = d.x * sin(a) + d.z * cos(a);
    let u = atan2(x, -z) / (2.0 * PI) + 0.5;
    let v = acos(clamp(d.y, -1.0, 1.0)) / PI;
    return vec2<f32>(u, v);
}

fn sky_radiance(d: vec3<f32>) -> vec3<f32> {
    let dims = vec2<i32>(textureDimensions(sky_tex));
    let pixel = sky_dir_uv(d) * vec2<f32>(dims) - 0.5;
    let base = vec2<i32>(floor(pixel));
    let weight = fract(pixel);
    var samples: array<vec3<f32>,4>;
    for (var corner=0u;corner<4u;corner++) {
        let offset=vec2<i32>(i32(corner&1u),i32(corner>>1u));
        let p=base+offset;
        // Equirectangular longitude wraps, latitude clamps at the poles.
        let coord=vec2<i32>((p.x%dims.x+dims.x)%dims.x,clamp(p.y,0,dims.y-1));
        samples[corner]=textureLoad(sky_tex,coord,0).rgb;
    }
    return mix(mix(samples[0],samples[1],weight.x),mix(samples[2],samples[3],weight.x),weight.y)*light.sky.x;
}

// Exact column integral of exp(-max(world_y,0)/height_scale) along a finite
// ray. Below the datum density stays constant. Evaluate the near-horizontal
// limit analytically to avoid cancellation and a horizon seam.
fn fog_column(distance_m: f32, height: f32, vertical: f32, height_scale: f32) -> f32 {
    let extent = max(distance_m, 0.0);
    let end = height + vertical * extent;
    if max(height, end) <= 0.0 { return extent; }
    if min(height, end) >= 0.0 {
        let q = vertical * extent / height_scale;
        if abs(q) < 0.001 {
            return extent * exp(-height / height_scale) * (1.0 - q * 0.5 + q * q / 6.0);
        }
        return height_scale * (exp(-height / height_scale) - exp(-end / height_scale)) / vertical;
    }
    let a = min(height, 0.0) + height_scale * (1.0 - exp(-max(height, 0.0) / height_scale));
    let b = min(end, 0.0) + height_scale * (1.0 - exp(-max(end, 0.0) / height_scale));
    return (b - a) / vertical;
}

// Normalised Cornette-Shanks phase, matching the atmospheric aerosol model.
// Direction points FROM camera TO surface, so forward scattering peaks
// when looking toward the sun. No baked planet horizon participates here.
fn fog_phase(mu: f32, g: f32) -> f32 {
    return 3.0 * (1.0 - g * g) * (1.0 + mu * mu)
        / (8.0 * PI * (2.0 + g * g) * pow(1.0 + g * g - 2.0 * g * mu, 1.5));
}

fn fog_segment(surface: vec3<f32>, scattering_source: vec3<f32>, optical_depth: f32) -> vec3<f32> {
    let transmission = exp(-max(optical_depth, 0.0));
    return surface * transmission + scattering_source * (1.0 - transmission);
}

fn fog_scattering_source(direction: vec3<f32>) -> vec3<f32> {
    // SH0 / sqrt(4*pi) is incident mean sky radiance. Diffuse incident
    // illumination is approximated as isotropic; direct sun keeps its phase.
    // The camera's terrain horizon blocks the directional solar glow. Keep
    // isotropic skylight: shaded air is not black. This bounded approximation
    // does not integrate separate sun visibility along the entire fog ray.
    let diffuse = max(light.sh[0].rgb * 0.282095, vec3<f32>(0.0));
    let solar = light.sun_color.rgb * light.sun_dir.w * clamp(view.fog_cull.z, 0.0, 1.0)
        * fog_phase(clamp(dot(direction, light.sun_dir.xyz), -1.0, 1.0), light.fog_scattering.w);
    return light.fog_scattering.rgb * (diffuse + solar);
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

fn attenuate_background(direction: vec3<f32>, sky: vec3<f32>) -> vec3<f32> {
    if light.fog.z <= 0.0 || light.fog.y <= 0.0 { return sky; }
    let source = fog_scattering_source(direction);
    // Upward rays integrate the remaining finite atmospheric column; rays
    // toward/below the horizon have infinite optical depth in this local
    // plane-parallel layer. Only background rays use the sky as boundary.
    if direction.y <= 0.0 { return source; }
    let column = (max(-view.cam.y, 0.0)
        + light.fog.z * exp(-max(view.cam.y, 0.0) / light.fog.z)) / direction.y;
    return fog_segment(sky, source, light.fog.y * column);
}

fn background_radiance(direction: vec3<f32>) -> vec3<f32> {
    // Surface fog must remain star-free: stars are behind opaque geometry,
    // not bright flecks painted onto fully fogged distant mountains.
    return attenuate_background(direction, sky_radiance(direction));
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
    return attenuate_background(direction, min(sky_radiance(direction) + night_star_radiance(direction) + solar_disc(direction), vec3<f32>(65504.0)));
}

fn distance_fog_amount(distance: f32) -> f32 {
    let linear = clamp((distance - light.fog_color.w) / max(light.fog.x - light.fog_color.w, 1.0), 0.0, 1.0);
    let exponential = 1.0 - exp(-light.fog.y * distance);
    return max(linear, exponential);
}

fn fog_blend(source: vec3<f32>, background: vec3<f32>, amount: f32) -> vec3<f32> {
    if amount <= 0.0 { return source; }
    if amount >= 1.0 { return background; }
    return mix(source, background, amount);
}

fn apply_distance_fog(radiance: vec3<f32>, distance: f32, direction: vec3<f32>) -> vec3<f32> {
    if light.fog.z > 0.0 {
        if light.fog.y <= 0.0 { return radiance; }
        let column = fog_column(distance, view.cam.y, direction.y, light.fog.z);
        // Direct sunlight in the local air is occluded by scene geometry too.
        // Four samples cover the segment; diffuse skylight remains in shade.
        var visible = 0.0;
        for (var i = 0u; i < 4u; i++) {
            let p = view.cam.xyz + direction * distance * (f32(i) + 0.5) / 4.0;
            let q = light.shadow_view_proj * vec4<f32>(p,1.0);
            let uv = q.xy * vec2<f32>(0.5,-0.5) + 0.5;
            var sample = 1.0;
            if all(uv>=vec2<f32>(0.0)) && all(uv<=vec2<f32>(1.0)) && q.z>=0.0 && q.z<=1.0 {
                sample = textureSampleCompareLevel(shadow_tex,shadow_samp,uv,q.z-0.0002);
            }
            visible += sample;
        }
        visible = mix(1.0, visible * 0.25, light.sun_color.w);
        let diffuse = light.fog_scattering.rgb * max(light.sh[0].rgb * 0.282095, vec3<f32>(0.0));
        let source = diffuse + (fog_scattering_source(direction) - diffuse) * visible;
        return fog_segment(radiance, source, light.fog.y * column);
    }
    let amount = distance_fog_amount(distance);
    // Near surfaces do not need an additional sky texture lookup.
    if amount <= 0.0 { return radiance; }
    return fog_blend(radiance, background_radiance(direction), amount);
}

// Irradiance (lux) from the sky for a surface facing n.
// Indoor irradiance probes (irradiance_volume.rs): an ambient cube per grid
// point in `aux`. Returns irradiance and a weight, 1 inside the grid, fading
// to 0 over the outer half cell, 0 without a volume.
fn aux_f32(i: u32) -> f32 { return bitcast<f32>(aux[i]); }

// Bullet dents (dents.rs): the aux offset of their grid, 0 for none.
fn view_hf_dents() -> u32 { return gi.hf_dims.w; }

// The craters around p on a ROCK face with geometric normal `face`: a
// ragged paraboloid bowl per dent bends `normal` towards its centre and
// darkens the inside (occlusion), with a paler rim of fresh stone.
// Returns (normal, albedo multiplier).
fn rock_dents(p: vec3<f32>, face: vec3<f32>, normal: vec3<f32>) -> vec4<f32> {
    let o = view_hf_dents();
    let buckets = aux[o];
    let index_cap = aux[o + 2u];
    let cell_m = aux_f32(o + 3u);
    let c = vec3<i32>(floor(p / cell_m));
    let h = (u32(c.x) * 73856093u) ^ (u32(c.y) * 19349663u) ^ (u32(c.z) * 83492791u);
    let entry = aux[o + 4u + (h & (buckets - 1u))];
    let count = min(entry & 255u, 15u);
    if count == 0u { return vec4<f32>(normal, 1.0); }
    var nn = normal;
    var dim = 1.0;
    let list = o + 4u + buckets + (entry >> 8u);
    let dent_base = o + 4u + buckets + index_cap;
    for (var k = 0u; k < count; k++) {
        let i = aux[list + k];
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
    return vec3<f32>(f32(word&1023u),f32((word>>10u)&1023u),f32((word>>20u)&1023u))/1023.0;
}
struct IrradianceSample { value: vec4<f32>, emission: vec3<f32> }
fn unpack_emission(word: u32) -> vec3<f32> {
    return vec3<f32>(f32(word&511u),f32((word>>9u)&511u),f32((word>>18u)&511u))*exp2(f32(word>>27u)-24.0);
}
fn irradiance_sample(p: vec3<f32>, n: vec3<f32>) -> IrradianceSample {
    let o = u32(gi.params.w);
    if o == 0u { return IrradianceSample(vec4<f32>(0.0),vec3<f32>(0.0)); }
    let transport = aux[o + 7u] >= 2u;
    let sun_transport = aux[o + 7u] == 3u;
    let visibility = aux[o + 7u] >= 1u;
    let origin = vec3<f32>(aux_f32(o), aux_f32(o + 1u), aux_f32(o + 2u));
    let cell = aux_f32(o + 3u);
    let dims = vec3<u32>(aux[o + 4u], aux[o + 5u], aux[o + 6u]);
    let f = (p - origin) / cell;
    let last = vec3<f32>(dims - vec3<u32>(1u));
    let outside = max(max(-f, f - last), vec3<f32>(0.0));
    let weight = clamp(1.0 - max(outside.x, max(outside.y, outside.z)) * 2.0, 0.0, 1.0);
    if weight <= 0.0 { return IrradianceSample(vec4<f32>(0.0),vec3<f32>(0.0)); }
    let fc = clamp(f, vec3<f32>(0.0), last);
    let cell0 = min(floor(fc), last - 1.0);
    let t = fc - cell0;
    let n2 = n * n;
    let axis = vec3<u32>(select(1u, 0u, n.x >= 0.0), select(3u, 2u, n.y >= 0.0), select(5u, 4u, n.z >= 0.0)) * 3u;
    if visibility {
        // Four-probe tetrahedral interpolation: continuous across cell faces,
        // half the fetches of trilinear, scalar visibility only.
        var high=0u; var low=0u;
        if t.y>t.x { high=1u; } if t.z>t[high] { high=2u; }
        if t.y<t.x { low=1u; } if t.z<t[low] { low=2u; }
        // Resolve equal fractions deterministically to distinct axes.
        if high==low { high=0u; low=2u; }
        let middle=3u-high-low;
        var corner=vec3<u32>(0u);
        let weights=vec4<f32>(1.0-t[high],t[high]-t[middle],t[middle]-t[low],t[low]);
        var value=vec3<f32>(0.0);
        var emission=vec3<f32>(0.0);
        var valid_weight=0.0;
        let trilinear=aux[o+7u]==6u;
        for(var c=0u;c<select(4u,8u,trilinear);c++) {
            if c==1u { corner[high]=1u; }
            if c==2u { corner[middle]=1u; }
            if c==3u { corner[low]=1u; }
            var blend_weight=weights[min(c,3u)];
            if trilinear {
                corner=vec3<u32>(c&1u,(c>>1u)&1u,(c>>2u)&1u);
                let w3=select(1.0-t,t,corner==vec3<u32>(1u));
                blend_weight=w3.x*w3.y*w3.z;
            }
            let g=vec3<u32>(cell0)+corner;
            let base=o+8u+(g.x+g.y*dims.x+g.z*dims.x*dims.y)*18u;
            if aux[o+7u]>=4u {
                var valid=select(1.0,0.0,(aux[base]&0x80000000u)!=0u);
                if aux[o+7u]==5u {
                    let plane=vec4<f32>(aux_f32(base+1u),aux_f32(base+2u),aux_f32(base+4u),aux_f32(base+5u));
                    // Zero plane passes through. Blend over 15 cm to avoid a hard edge.
                    valid*=smoothstep(-0.15,0.0,dot(plane.xyz,p)+plane.w);
                }
                if aux[o+7u]==7u {
                    let plane=vec4<f32>(aux_f32(base+2u),aux_f32(base+5u),aux_f32(base+8u),aux_f32(base+11u));
                    valid*=smoothstep(-0.15,0.0,dot(plane.xyz,p)+plane.w);
                    let e=unpack_emission(aux[base+axis.x+1u])*n2.x
                        +unpack_emission(aux[base+axis.y+1u])*n2.y
                        +unpack_emission(aux[base+axis.z+1u])*n2.z;
                    emission+=e*blend_weight*valid;
                }
                let v=unpack_sky_transport(aux[base+axis.x])*n2.x
                    +unpack_sky_transport(aux[base+axis.y])*n2.y
                    +unpack_sky_transport(aux[base+axis.z])*n2.z;
                value+=v*blend_weight*valid;
                valid_weight+=blend_weight*valid;
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
        if valid_weight<1e-5 { return IrradianceSample(vec4<f32>(0.0),vec3<f32>(0.0)); }
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
            let bias = min(0.06, far_sun.params.x * 0.1);
            let c = far_sun.view_proj * vec4<f32>(p + (n + light.sun_dir.xyz) * bias, 1.0);
            let uv = c.xy * vec2<f32>(0.5,-0.5) + 0.5;
            if far_sun.params.w>0.5 {mapped=stage_sun_visibility(p,plane_n);}
            else {mapped = textureSampleCompareLevel(far_sun_tex, shadow_samp, uv, 0, c.z);}
        }
        if fade <= 0.0 { return mapped; }
    }
    var fallback = 1.0;
    // vol_dims.w bit 4: no foliage and no heightfield, the march returns 1.
    if gi.vol_dims.w != 0u && (gi.vol_dims.w & 4u) == 0u {
        fallback = transmittance(p + n * 0.2, light.sun_dir.xyz, 50.0);
    }
    return mix(mapped, fallback, fade);
}

// Detail follows each camera but its texels remain fixed in world space.
// Blend the shared maps before adding the near detail, so a camera
// near the shared-map edge cannot overwrite a valid car shadow with foliage.
fn shadow_detail_receiver(p: vec3<f32>, n: vec3<f32>, plane_n: vec3<f32>) -> f32 {
    var weight = 0.0;
    if light.near_shadow.z > 0.5 {
        let c = light.near_shadow_view_proj * vec4<f32>(p, 1.0);
        if c.z >= 0.0 && c.z <= 1.0 {
            weight = 1.0 - smoothstep(light.near_shadow.y, 1.0, max(abs(c.x), abs(c.y)));
        }
    }
    if weight >= 1.0 {
        return shadow_filter(near_shadow_tex, light.near_shadow_view_proj, light.near_shadow.x, p, n, plane_n);
    }
    var far = shadow_filter(shadow_tex, light.shadow_view_proj, light.sky.w, p, n, plane_n);
    // The outer map uses actual alpha-tested tree geometry, not the coarse
    // density field that wrongly buried sunlit crowns in their own shadow.
    // Over the outer 55 % of the map: sharp tree shadows thin out into the
    // coarse distant field instead of meeting it at a line.
    let coverage_fade = sun_coverage_fade(light.shadow_view_proj, p, 0.45);
    if coverage_fade > 0.0 && (far_sun.params.z > 0.5 || foliage_sun_pages.counts.z != 0u || gi.vol_dims.w != 0u) {
        far = mix(far, distant_sun_visibility(p,n,plane_n), coverage_fade);
    }
    if weight <= 0.0 { return far; }
    let detail = shadow_filter(near_shadow_tex, light.near_shadow_view_proj, light.near_shadow.x, p, n, plane_n);
    return mix(far, detail, weight);
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
    return textureSampleLevel(albedo, samp, uv, layer, lod);
}

// The surface's footprint across one pixel, from ray differentials: the
// rays through the next pixel right and down, hit on the triangle's plane.
struct Footprint {
    dx: vec3<f32>,
    dy: vec3<f32>,
};

var<private> fp: Footprint;

fn track_tile(key:vec2<i32>)->vec4<i32> {
    var h=bitcast<u32>(key.x)*0x9e3779b9u ^ bitcast<u32>(key.y)*0x85ebca6bu;
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
    return textureSampleGrad(albedo, samp, uv, layer, gx, gy);
}

fn tex_xz(layer: i32, uv: vec2<f32>, scale: f32) -> vec4<f32> {
    return tex_w(layer, uv, fp.dx.xz / scale, fp.dy.xz / scale);
}

// Tangent-space normal (xy in -1..1) of layer l, sampled like the albedo.
fn nrm(l: i32, uv: vec2<f32>, gx: vec2<f32>, gy: vec2<f32>) -> vec2<f32> {
    return textureSampleGrad(normal_tex, samp, uv, l, gx, gy).xy * 2.0 - 1.0;
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
    let uv0 = vec2<f32>(vertices[vi[0] + 6u], vertices[vi[0] + 7u]);
    let d1 = vec2<f32>(vertices[vi[1] + 6u], vertices[vi[1] + 7u]) - uv0;
    let d2 = vec2<f32>(vertices[vi[2] + 6u], vertices[vi[2] + 7u]) - uv0;
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
    let uv0 = vec2<f32>(vertices[vi[0] + 6u], vertices[vi[0] + 7u]);
    let uv1 = vec2<f32>(vertices[vi[1] + 6u], vertices[vi[1] + 7u]);
    let uv2 = vec2<f32>(vertices[vi[2] + 6u], vertices[vi[2] + 7u]);
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
    var h = bitcast<u32>(cell.x) * 0x9e3779b9u ^ bitcast<u32>(cell.y) * 0x85ebca6bu ^ seed;
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
    result.direct=budget*sun_e*(reflection*max(ndl,0.0)+transmission*max(-ndl,0.0))
        +sun_e*ggx(n,v,light.sun_dir.xyz,rough,f0);
    result.indirect=budget*(reflection*front_irradiance+transmission*back_irradiance);
    result.specular=env*specular_weight;
    return result;
}

fn coarse_pack(so: SurfaceOut, sun: f32) -> vec4<u32> {
    return vec4<u32>(pack2x16float(so.albedo.rg), pack2x16float(vec2<f32>(so.albedo.b, so.rough)),
        pack2x16float(so.normal.xy), pack2x16float(vec2<f32>(so.normal.z, sun)));
}

@compute @workgroup_size(8, 8)
fn terrain_coarse(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<u32>(view.size.xy);
    let half = (size + 1u) / 2u;
    if gid.x >= half.x || gid.y >= half.y { return; }
    let cx = vec2<i32>(gid.xy);
    let invalid = vec4<u32>(0u, pack2x16float(vec2<f32>(0.0, -1.0)), 0u, 0u);
    let px = min(cx * 2, vec2<i32>(size) - 1);
    if textureLoad(depth_tex, px, 0) <= 0.0 { textureStore(terrain_coarse_out, cx, invalid); return; }
    let id = textureLoad(vis_tex, px, 0).r;
    let vis = visible[id >> 7u];
    let cl = clusters[vis.y];
    let m = materials[cl.material];
    if (m.flags & 32770u) != 32770u { textureStore(terrain_coarse_out, cx, invalid); return; }
    let inst = instance_at(vis.x);
    let packed = tris[cl.first_tri + (id & 127u)];
    var vi: array<u32, 3>;
    for (var k = 0u; k < 3u; k++) { vi[k] = vtx(cluster_verts[cl.first_vert + ((packed >> (8u * k)) & 0xffu)]); }
    // The ray through the quad's centre, on the triangle of its first pixel.
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
        n += vec3<f32>(vertices[v + 3u], vertices[v + 4u], vertices[v + 5u]) * w[k];
        uv += vec2<f32>(vertices[v + 6u], vertices[v + 7u]) * w[k];
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
    // As `shade`: the material's own layer relief first (TERRAIN|SURFACE
    // takes its generic textured branch).
    if m.layer >= 0 {
        var frame = uv_frame(e1, e2, vi);
        if dot(geo_n, n) < 0.0 { frame[0] = -frame[0]; frame[1] = -frame[1]; }
        n = perturb(n, nrm(m.layer, uv, uv_at(fp.dx, e1, e2, vi), uv_at(fp.dy, e1, e2, vi)), frame[0], frame[1]);
    }
    var local = vec3<f32>(0.0);
    for (var k = 0u; k < 3u; k++) { local += pos_of(vi[k]) * w[k]; }
    let corner = array<vec2<f32>, 3>(vec2<f32>(vertices[vi[0] + 6u], vertices[vi[0] + 7u]),
        vec2<f32>(vertices[vi[1] + 6u], vertices[vi[1] + 7u]), vec2<f32>(vertices[vi[2] + 6u], vertices[vi[2] + 7u]));
    let si = SurfaceIn(u32(max(-2 - m.layer, 0)), vis.x, p, local, n, dir, vec3<f32>(b0, b1, b2),
        corner[0], corner[1], corner[2], fp.dx, fp.dy, m.tint, m.roughness, m.metal, m.scale, light.anim.x);
    field_sun = 1.0;
    let so = field_surface(si, m.layer);
    textureStore(terrain_coarse_out, cx, coarse_pack(so, field_sun));
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
    let full = vec2<i32>(view.size.xy) - 1;
    for (var k = 0; k < 4; k++) {
        let o = vec2<i32>(k & 1, k >> 1);
        let c = clamp(base + o, vec2<i32>(0), dims - 1);
        let t = textureLoad(terrain_coarse_tex, c, 0);
        let ab = unpack2x16float(t.y);
        if ab.y < 0.0 { return out; }
        let d = textureLoad(depth_tex, min(c * 2, full), 0);
        if abs(d - depth) > 0.02 * depth { return out; }
        let wgt = select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1);
        let nz = unpack2x16float(t.w);
        albedo += vec3<f32>(unpack2x16float(t.x), ab.x) * wgt;
        rough += ab.y * wgt;
        normal += vec3<f32>(unpack2x16float(t.z), nz.x) * wgt;
        sun += nz.y * wgt;
    }
    out.ok = true;
    out.so = SurfaceOut(albedo, normalize(normal), rough, 0.0, vec3<f32>(0.0));
    out.sun = sun;
    return out;
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
        if local == 0u { shade_tiles.tiles[atomicAdd(&shade_tiles.x, 1u)] = wid.y * groups.x + wid.x; }
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
        let t = shade_tiles.tiles[wid.x];
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
        if light.sky.z==0.0 && all(gid.xy<size) && hidden_meadow.width==size.x && hidden_meadow.height==size.y {
            let owner=hidden_meadow.ids[gid.y*hidden_meadow.row_pairs+gid.x].x;
            hidden=(owner&0x80000000u)!=0u && textureLoad(depth_tex,vec2<i32>(gid.xy),0)>0.0;
        }
        if hidden {atomicAdd(&hidden_meadow_count,1u);}
        workgroupBarrier();
        if local==0u {let count=atomicLoad(&hidden_meadow_count);if count!=0u {atomicAdd(&hidden_meadow.skipped,count);}}
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
        textureStore(out_hdr, px, vec4<f32>(visible_background_radiance(dir), 1.0));
        return;
    }
    if VOXEL_OCCLUSION && textureLoad(voxel_depth, px, 0) > depth { return; }
    let id = textureLoad(vis_tex, px, 0).r;
    if VOXEL_SURFACE && (id & 0x80000000u) != 0u { shade_voxel(px, size, dir, depth); return; }
    let vis = visible[id >> 7u];
    let tri = id & 127u;
    let cl = clusters[vis.y];
    let inst = instance_at(vis.x);
    let packed = tris[cl.first_tri + tri];
    var vi: array<u32, 3>;
    for (var k = 0u; k < 3u; k++) {
        vi[k] = vtx(cluster_verts[cl.first_vert + ((packed >> (8u * k)) & 0xffu)]);
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
        n += vec3<f32>(vertices[v + 3u], vertices[v + 4u], vertices[v + 5u]) * w[k];
        uv += vec2<f32>(vertices[v + 6u], vertices[v + 7u]) * w[k];
    }
    n = normalize(wind_normal(member_slot(vis.x), xform_dir(inst, n)));
    let geo_n = normalize(cross(e1, e2));
    sun_receiver_tri[0]=p0;sun_receiver_tri[1]=p1;sun_receiver_tri[2]=p2;
    sun_receiver_tri_set=true;
    sun_receiver_error=cl.own_error*inst.scale;
    let m = materials[cl.material];
    // Two-sided cards and backfaces face the viewer.
    if dot(n, dir) > 0.0 && (m.flags & 8u) != 0u && (m.flags & 64u) == 0u {
        n = -n;
    }
    // Keep the smooth authored normal before texture relief. Visibility
    // traced around a flat triangle normal changes abruptly at every edge.
    let surface_n = n;
    let dist = distance(p, view.cam.xyz);
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
    // Reconstruct output-resolution detail rather than selecting coarse
    // input-resolution mip levels. Zero retains native and shadow behavior.
    let mip_scale = select(1.0, view.extra.z, view.extra.z > 0.0);
    fp.dx *= mip_scale;
    fp.dy *= mip_scale;
    let duv_x = uv_at(fp.dx, e1, e2, vi) ;
    let duv_y = uv_at(fp.dy, e1, e2, vi);
    var base = m.tint;
    var nn = n; // the normal after normal maps
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
        let rows = arrayLength(&aux);
        let row = u32(clamp(floor(uv.y / 2.0), 0.0, f32(max(rows, 1u) - 1u)));
        let surf = select(0u, aux[row], rows > 0u);
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
        base = textureSampleGrad(albedo, plant_samp, uv, m.layer, duv_x, duv_y).rgb * m.tint;
        base *= mix(0.8, 1.0, height) * 1.25 * (1.0 - light.fog.w * 0.25);
        nn = vec3<f32>(0.0, 1.0, 0.0);
    } else if (m.flags & 16u) != 0u && m.layer >= 0 {
        var tw = pow(abs(n), vec3<f32>(4.0));
        tw /= tw.x + tw.y + tw.z;
        base = triplanar(m.layer, p, tw, m.scale, mat2x2<f32>(1.0, 0.0, 0.0, 1.0), 0.0) * m.tint;
        nn = triplanar_n(m.layer, p, n, tw, m.scale, mat2x2<f32>(1.0, 0.0, 0.0, 1.0), 0.0);
    } else if m.layer >= 0 && (m.flags & 16384u) != 0u {
        // LIVE_SCREEN writes mip 2 (512 square) and below. Never sample the
        // uninitialized higher-resolution levels or the unrelated normal map.
        let footprint = max(length(duv_x), length(duv_y)) * f32(textureDimensions(albedo).x);
        base = textureSampleLevel(albedo, samp, uv, m.layer, max(2.0, log2(max(footprint, 1.0)))).rgb * m.tint;
    } else if m.layer >= 0 && (m.flags & 2048u) == 0u {
        // PIXEL_ART samples the top mip: hard source pixels, no blur.
        let sharp = select(1.0, 0.0, (m.flags & 8192u) != 0u);
        base = tex_w(m.layer, uv, duv_x * sharp, duv_y * sharp).rgb * m.tint;
        var frame = uv_frame(e1, e2, vi);
        if dot(geo_n, n) < 0.0 {
            // The far side of a two-sided card: its relief is the mirror image.
            frame[0] = -frame[0];
            frame[1] = -frame[1];
        }
        nn = perturb(n, nrm(m.layer, uv, duv_x, duv_y), frame[0], frame[1]);
    }
    n = nn;
    // Car parts get a procedural surface (car_material.wgsl): flags bits
    // 16.. hold 1 + their index in car_materials. Others keep the material.
    let car = m.flags >> 16u;
    var rough_m = m.roughness;
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
    var metal = m.metal;
    var coat = select(0.0, 1.0, (m.flags & 32u) != 0u);
    var coat_rough = 0.04;
    var coat_n = n;
    var glint = vec3<f32>(0.0);
    var car_debug = vec3<f32>(0.0);
    var surface_emit = vec3<f32>(0.0);
    if (m.flags & 32768u) != 0u {
        var local = vec3<f32>(0.0);
        for (var k = 0u; k < 3u; k++) { local += pos_of(vi[k]) * w[k]; }
        let corner = array<vec2<f32>, 3>(vec2<f32>(vertices[vi[0] + 6u], vertices[vi[0] + 7u]),
            vec2<f32>(vertices[vi[1] + 6u], vertices[vi[1] + 7u]), vec2<f32>(vertices[vi[2] + 6u], vertices[vi[2] + 7u]));
        let si = SurfaceIn(u32(max(-2 - m.layer, 0)), vis.x, p, local, n, dir, vec3<f32>(b0, b1, b2),
            corner[0], corner[1], corner[2], fp.dx, fp.dy, m.tint, rough_m, metal, m.scale, light.anim.x);
        // TERRAIN + SURFACE: the engine's procedural terrain, not the game's.
        var so: SurfaceOut;
        if (m.flags & 2u) != 0u {
            var coarse: CoarseSurface;
            if TERRAIN_COARSE && max(length(fp.dx), length(fp.dy)) > COARSE_FOOTPRINT_M { coarse = coarse_surface(px, depth); }
            if coarse.ok { so = coarse.so; field_sun = coarse.sun; } else { so = field_surface(si, m.layer); }
        } else { so = game_surface(si); }
        base = so.albedo;
        n = so.normal;
        coat_n = n;
        rough_m = so.rough;
        metal = so.metal;
        surface_emit = so.emission;
    }
    if FEATURE_CARS && car != 0u {
        let cs = car_surface(car_materials[car - 1u], inst, p, n, dir, uv, base, m.roughness, m.metal);
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
    let wounds = f32(inst.pad >> 27u) / 31.0;
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
    let l = light.sun_dir.xyz;
    let ndl = dot(n, l);
    var sun = max(ndl, 0.0);
    var back = vec3<f32>(0.0);
    var ao = 1.0;
    let grass = (m.flags & 64u) != 0u;
    let procedural_leaf = FEATURE_FOLIAGE && (m.flags & 512u) != 0u;
    let needles = procedural_leaf || (FEATURE_FOLIAGE && (m.flags & 9u) == 9u && (inst.pad & 1u) != 0u && !grass);
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
    let terrain_error=select(0.0,world_visibility.params.z,(m.flags&6u)!=0u);
    var shadow = 1.0;
    if sun > 0.0 || needles || (gi.vol_dims.w != 0u && (gi.vol_dims.w & 4u) == 0u) || coat > 0.0 || rough_m < 0.55 {
        let s_map = shadow_at_surface(p, surface_n * sign(dot(surface_n, l) + 1e-4), geo_n, terrain_error);
        shadow = mix(1.0, s_map, light.sun_color.w);
    }
    if FEATURE_GRASS && shadow > 0.0 && (m.flags & 70u) != 0u {shadow*=mix(1.0,grass_sun_transmittance_receiver(p,l,(m.flags & 2u)!=0u),light.sun_color.w);}
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
    let diffuse = base * (1.0 - metal) / PI;
    // Use the continuous surface hemisphere, unaffected by texture grain.
    // The triangle normal is only for intersection and texture derivatives.
    let visibility_n = surface_n;
    let sky_vis = sky_visibility(p, visibility_n);
    var meadow_edge=vec2<f32>(1.0);
    if FEATURE_MEADOW && (m.flags&6u)!=0u {meadow_edge=meadow_edge_visibility(p,surface_n,l);}
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
        let back_sky_vis=sky_visibility(p,-visibility_n);
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
        let cached=irradiance_sample(p+visibility_n*0.05,n);
        let room=cached.value;
        if room.w>0.0 {
          if aux[u32(gi.params.w)+7u] >= 1u {
            let access=clamp(room.x,0.0,1.0);
            // Attenuate only sky. Preserve the existing bounce term and add
            // the cached diffuse return from static surfaces under unit sky.
            let returned=sky_irradiance(vec3<f32>(0.0,1.0,0.0))*room.y
                +light.sun_color.rgb*light.sun_dir.w*max(room.z,0.0);
            let corrected=(sky_irradiance(n)*sky_vis*meadow_ground_sky*meadow_edge.x*access
                +bounce*(1.0-sky_vis)+returned+cached.emission*light.anim.y)*ao*contact.x;
            ambient=mix(ambient,corrected,room.w);
            // No blanket suppression of glossy reflections with diffuse AO.
          } else {
            ambient=mix(ambient,room.rgb*ao*contact.x,room.w);
            env=mix(env,irradiance_volume(p+visibility_n*0.05,refl).rgb/PI*spec_contact,room.w);
          }
        }
        direct=diffuse*sun_e*sun+sun_e*back/PI+sun_e*ggx(n,v,l,rough,f0);
        indirect=diffuse*ambient;
        specular=env*env_brdf(f0,rough,ndv)*ao;
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
        specular = specular * (1.0 - fc) + mix(bounce / PI, environment(coat_refl, coat_rough, bounce), reflection_vis) * coat_contact * fc;
    }
    if FEATURE_GRASS && grass {
        direct += headlight_grass_radiance(p,v,base,rough);
    } else {
        direct += headlight_radiance(p,n,v,diffuse,rough,f0);
    }
    var rad = direct + indirect + specular;
    // Emission is scaled by Lighting::emissive_scale (exposure compensation).
    let emit = light.anim.y;
    if FEATURE_CARS { rad += car_lamp_emission(m.flags, base, inst, p) * emit; }
    // EMISSIVE: a glowing surface, `scale` is its radiance in nits for white;
    // with a texture layer the picture glows (screens), else the tint.
    if (m.flags & 1024u) != 0u { rad += select(m.tint, base, m.layer >= 0) * m.scale * emit; }
    rad += surface_emit * emit;
    // LAVA: unlit, flowing molten rock; `tint * scale` is the radiance of white.
    if FEATURE_PROCEDURAL && (m.flags & 4096u) != 0u { rad = proc_lava(p, light.anim.x, max(length(fp.dx), length(fp.dy))) * m.tint * m.scale * emit; }
    // Hit flash (instance bits 16..23): the body itself lights up white,
    // nearly evenly; a slight facing term keeps its form readable.
    let flash = f32((inst.pad >> 16u) & 0xffu) / 255.0;
    if flash > 0.0 {
        let facing = clamp(dot(n, -dir), 0.0, 1.0);
        let white = vec3<f32>(1.0, 0.97, 0.92) * light.anim.z * (0.8 + 0.2 * facing);
        rad = mix(rad, white, clamp(flash * 0.9, 0.0, 1.0));
    }
    rad = apply_distance_fog(rad, dist, dir);
    // Component diagnostics retain the exact material, pose and lighting.
    if light.sky.z > 0.5 && light.sky.z < 1.5 {
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
        rad = vec3<f32>(shadow_filter(near_shadow_tex,light.near_shadow_view_proj,light.near_shadow.x,p,surface_n*sign(dot(surface_n,l)+1e-4),geo_n)) * 6000.0;
    } else if light.sky.z > 15.5 && light.sky.z < 16.5 {
        rad = vec3<f32>(shadow_filter(shadow_tex,light.shadow_view_proj,light.sky.w,p,surface_n*sign(dot(surface_n,l)+1e-4),geo_n)) * 6000.0;
    } else if light.sky.z > 16.5 && light.sky.z < 17.5 {
        let clip=light.near_shadow_view_proj*vec4<f32>(p,1.0);
        let valid=light.near_shadow.z>0.5 && clip.z>=0.0 && clip.z<=1.0;
        rad=vec3<f32>(select(0.0,1.0-smoothstep(light.near_shadow.y,1.0,max(abs(clip.x),abs(clip.y))),valid))*6000.0;
    } else if light.sky.z > 19.5 && light.sky.z < 22.5 {
        rad = car_debug * 6000.0;
    } else if light.sky.z > 22.5 && light.sky.z < 24.5 {
        rad = world_terrain_debug(p,surface_n*sign(dot(surface_n,l)+1e-4),light.sky.z>23.5,terrain_error)*6000.0;
    } else if light.sky.z > 24.5 && light.sky.z < 25.5 {
        let page=foliage_page_visibility(p,surface_n*sign(dot(surface_n,l)+1e-4),geo_n);
        rad=select(vec3<f32>(1.0,0.0,1.0),vec3<f32>(page.x),page.y>0.5)*6000.0;
    }
    // fp16 target: a blast light next to a wall must saturate, not become inf.
    textureStore(out_hdr, px, vec4<f32>(min(rad, vec3<f32>(65504.0)), 1.0));
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
@compute @workgroup_size(8, 8)
fn shade_simple(@builtin(global_invocation_id) gid:vec3<u32>) {
    if any(gid.xy>=vec2<u32>(view.size.xy)) {return;}
    let px=vec2<i32>(gid.xy);
    let ndc=(vec2<f32>(gid.xy)+0.5)/view.size.xy*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0);
    let far=view.inv_view_proj*vec4<f32>(ndc,1e-4,1.0);
    let dir=normalize(far.xyz/far.w-view.cam.xyz);
    let id=textureLoad(vis_tex,px,0).r;
    if textureLoad(depth_tex,px,0)<=0.0 || id==0xffffffffu {
        textureStore(out_hdr,px,vec4<f32>(visible_background_radiance(dir),1.0));return;
    }
    let vis=visible[id>>7u];let cl=clusters[vis.y];let inst=instance_at(vis.x);
    let packed=tris[cl.first_tri+(id&127u)];
    var points:array<vec3<f32>,3>;var normals:array<vec3<f32>,3>;var uvs:array<vec2<f32>,3>;
    for(var k=0u;k<3u;k++) {
        let v=cluster_verts[cl.first_vert+((packed>>(8u*k))&255u)]*8u;
        let pos=vec4<f32>(vertices[v],vertices[v+1u],vertices[v+2u],1.0);
        points[k]=wind_position(member_slot(vis.x),inst.r1.w,vec3<f32>(dot(inst.r0,pos),dot(inst.r1,pos),dot(inst.r2,pos)));
        normals[k]=vec3<f32>(vertices[v+3u],vertices[v+4u],vertices[v+5u]);
        uvs[k]=vec2<f32>(vertices[v+6u],vertices[v+7u]);
    }
    let e1=points[1]-points[0];let e2=points[2]-points[0];let pv=cross(dir,e2);
    let det=dot(e1,pv);let inv=select(1.0/det,0.0,abs(det)<1e-12);let tv=view.cam.xyz-points[0];
    let b1=clamp(dot(tv,pv)*inv,0.0,1.0);let b2=clamp(dot(dir,cross(tv,e1))*inv,0.0,1.0-b1);let b0=1.0-b1-b2;
    let p=points[0]*b0+points[1]*b1+points[2]*b2;
    let authored=normals[0]*b0+normals[1]*b1+normals[2]*b2;
    var n=normalize(wind_normal(member_slot(vis.x),vec3<f32>(dot(inst.r0.xyz,authored),dot(inst.r1.xyz,authored),dot(inst.r2.xyz,authored))));
    if dot(n,dir)>0.0 {n=-n;}
    let m=materials[cl.material];var base=m.tint;
    if m.layer>=0 {let uv=uvs[0]*b0+uvs[1]*b1+uvs[2]*b2;
        base*=textureSampleLevel(albedo,samp,uv,m.layer,max(log2(max(distance(p,view.cam.xyz)*0.03,1.0)),0.0)).rgb;}
    let ambient=sky_irradiance(n);
    var rad=base*(ambient+light.sun_color.rgb*light.sun_dir.w*max(dot(n,light.sun_dir.xyz),0.0))/3.14159265;
    for(var index=0u;index<incident_light_count(p);index++) {
        if dot(n,incident_light_position(index)-p)<=0.0 {continue;}
        let sample=incident_light_sample(index,p,n);
        rad+=base*sample.irradiance*max(dot(n,sample.direction),0.0)/PI;
    }
    if (m.flags&1024u)!=0u {rad+=base*m.scale*light.anim.y;}
    if (m.flags&4096u)!=0u {rad=base*m.scale*light.anim.y;}
    let flash=f32((inst.pad>>16u)&255u)/255.0;
    rad=mix(rad,vec3<f32>(1.0,0.97,0.92)*light.anim.z*(0.8+0.2*max(dot(n,-dir),0.0)),flash*0.9);
    rad=apply_distance_fog(rad,distance(p,view.cam.xyz),dir);
    if light.sky.z==2.0 {rad=base*6000.0;}
    if light.sky.z==11.0 {rad=(n*0.5+0.5)*6000.0;}
    textureStore(out_hdr,px,vec4<f32>(min(rad,vec3<f32>(65504.0)),1.0));
}
