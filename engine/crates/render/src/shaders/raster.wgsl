// Visibility buffer raster: every visible cluster is one instance of a
// size-bucketed draw; vertices are pulled from storage buffers. The colour
// target stores which triangle covers each pixel; shading happens later,
// once per pixel.
override CLUSTER_BUCKETS: bool = true;
override DIRECT_BOUNDED: bool = false;
// See cull.wgsl.
override VOXEL_FAR_M: f32 = 0.0;

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
    lod_cams: array<vec4<f32>, 4>, // shared sun map distance LOD (cull.wgsl)
    lod: vec4<f32>,
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
    flags: u32,
};

struct Material {
    tint: vec3<f32>,
    layer: i32,
    roughness: f32,
    flags: u32,
    scale: f32,
    pad: f32,
};

// Cluster geometry in one buffer (topology.rs): vertices from word 0, then
// cluster vertex indices and triangles from scene table vec4 1.z and 1.w.
@group(0) @binding(0) var<storage, read> topology: array<u32>;
fn vertex_f32(i: u32) -> f32 { return bitcast<f32>(topology[i]); }
// Scene aux words (Scene::aux) live after the tables; vec4 1.x is their start,
// 1.y their count.
fn aux_word(i: u32) -> u32 {
    let v = scene_tables[scene_tables[1].x + (i >> 2u)];
    return v[i & 3u];
}
fn aux_len() -> u32 { return scene_tables[1].y; }

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
@group(0) @binding(10) var plant_samp: sampler;

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
// Voxel crown replacement words in the scene tables, from vec4 2.z.
fn voxel_replacement_at(i: u32) -> u32 {
    let v = scene_tables[scene_tables[2].z + (i >> 2u)];
    return v[i & 3u];
}

@group(1) @binding(0) var<uniform> view: View;
@group(1) @binding(1) var<storage, read> visible: array<vec2<u32>>;
// Cull counters, draw args and the bucket draw map, one buffer (cull.wgsl
// CullDraw): args from word 48, the map from word 128.
// cull.wgsl finalize: opaque records 0..15 and starts 16..19, masked
// records 20..35 (first_vertex (4+bucket)*384) and starts 36..39.
@group(1) @binding(2) var<storage, read> draw_words: array<u32>;
@group(1) @binding(4) var<storage,read> foliage_selection_words: array<u32>;
fn foliage_selection_at(i:u32)->vec4<u32> { return selection_unpack(foliage_selection_words[i]); }
// Packed group selection (aggregate_lod.wgsl writes it): one u32 per group,
// runtime-sized storage table. Bits 0-1 choice, 2-7
// voxel level (63: none), 8-9 sun mode, 16-31 the triangle error as f16.
fn selection_unpack(w:u32)->vec4<u32> {
    let y=(w>>2u)&63u;
    return vec4<u32>(w&3u,select(y,0xffffffffu,y==63u),bitcast<u32>(unpack2x16float(w>>16u).x),(w>>8u)&3u);
}


// Needles thinner than a pixel speckle and shimmer: a needle base vertex
// (uv v below -2.5, plant_growth_assembly::needle_base_uv) moves out along
// its octahedron-encoded direction by NEEDLE_MIN_PX of a pixel at its depth
// (view.cam.w is pixels per metre at distance 1, or per metre for
// orthographic sun views, where clip w is 1). Shading clamps its barycentrics
// to the unwidened triangle.
override NEEDLE_MIN_PX: f32 = 0.6;
fn needle_widen(w: vec3<f32>, uv: vec2<f32>, inst: Instance) -> vec3<f32> {
    if uv.y > -2.5 || view.cam.w <= 0.0 || NEEDLE_MIN_PX <= 0.0 { return w; }
    let e = vec2<f32>(uv.x, uv.y + 4.0);
    var d = vec3<f32>(e.x, e.y, 1.0 - abs(e.x) - abs(e.y));
    if d.z < 0.0 { d = vec3<f32>((1.0 - abs(d.y)) * sign(d.x), (1.0 - abs(d.x)) * sign(d.y), d.z); }
    let out = vec3<f32>(dot(inst.r0.xyz, d), dot(inst.r1.xyz, d), dot(inst.r2.xyz, d));
    let clip_w = max((view.view_proj * vec4<f32>(w, 1.0)).w, 1e-4);
    return w + normalize(out) * NEEDLE_MIN_PX * clip_w / view.cam.w;
}

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) @interpolate(flat) id: u32,
    @location(1) @interpolate(flat) material: u32,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) grass_opacity: f32,
    @location(4) world: vec3<f32>,
    @location(5) @interpolate(flat) instance: u32,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VOut {
    var o: VOut;
    if DIRECT_BOUNDED && ii>=draw_words[48u + (vi/384u)*4u+1u] {
        o.pos=vec4<f32>(2.0,2.0,2.0,1.0);return o;
    }
    var slot = ii;
    if CLUSTER_BUCKETS {
        let record = vi / 384u;
        slot = draw_words[128u + draw_words[48u + select(36u + record - 4u, 16u + record, record < 4u)] + ii];
    }
    let vis = visible[slot];
    let cl = clusters[vis.y];
    let tri = (vi % 384u) / 3u;
    if tri >= cl.tri_count {
        o.pos = vec4<f32>(2.0, 2.0, 2.0, 1.0); // degenerate, outside the clip volume
        return o;
    }
    let inst = instance_at(vis.x);
    let packed = tri_word(cl.first_tri + tri);
    let local = (packed >> (8u * (vi % 3u))) & 0xffu;
    let v = cluster_vert(cl.first_vert + local) * 8u;
    let p = vec4<f32>(vertex_f32(v), vertex_f32(v + 1u), vertex_f32(v + 2u), 1.0);
    let rest = vec3<f32>(dot(inst.r0, p), dot(inst.r1, p), dot(inst.r2, p));
    var w = wind_position(member_slot(vis.x), inst.r1.w, rest);
    w = needle_widen(w, vec2<f32>(vertex_f32(v + 6u), vertex_f32(v + 7u)), inst);
    o.pos = view.view_proj * vec4<f32>(w, 1.0);
    o.world = w; o.instance = vis.x;
    o.id = (slot << 7u) | tri;
    o.material = cl.material;
    o.uv = vec2<f32>(vertex_f32(v + 6u), vertex_f32(v + 7u));
    // Only the explicitly packed crown material is replaced in color. Wood,
    // unrelated cutouts and source sun-shadow geometry remain unchanged.
    let replacement=voxel_replacement_at(member_slot(vis.x));
    var replaced=replacement==0xffffffffu || replacement==cl.material+1u;
    if replacement!=0xffffffffu && (replacement&0x80000000u)!=0u {
        replaced=foliage_selection_at(replacement&0x7fffffffu).x!=0u;
    }
    if VOXEL_FAR_M>0.0 && replacement!=0xffffffffu && (replacement&0xc0000000u)==0x40000000u {
        replaced=distance(vec3<f32>(inst.r0.w,inst.r1.w,inst.r2.w),view.cam.xyz)>VOXEL_FAR_M;
    }
    if view.params.z<0.5 && view.extra.y<1.5 && replaced {
        o.pos=vec4<f32>(2.0,2.0,2.0,1.0);
    }
    o.grass_opacity = 1.0;
    if (inst.flags & 8u) != 0u {
        o.grass_opacity = grass_card_opacity(vec3<f32>(inst.r0.w, inst.r1.w, inst.r2.w),
            view.cam.xyz, view.grass.xy, view.extra.w);
        if o.grass_opacity < GRASS_OPAQUE_THRESHOLD {
            o.pos = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        }
    }
    return o;
}

fn alpha_cut(in: VOut) -> bool {
    let mip_scale = select(1.0, view.extra.z, view.extra.z > 0.0);
    let dx = dpdx(in.uv) * mip_scale;
    let dy = dpdy(in.uv) * mip_scale;
    if in.grass_opacity < GRASS_OPAQUE_THRESHOLD { return true; }
    if game_mask(in.world,in.instance,in.material) {return true;}
    let m = material(in.material);
    if (m.flags & 1u) != 0u && m.layer >= 0 {
        // PIXEL_ART keeps the top mip's hard silhouette.
        let sharp = select(1.0, 0.0, (m.flags & 8192u) != 0u);
        let dx = dx * sharp;
        let dy = dy * sharp;
        var a: f32;
        if (m.flags & 64u) != 0u {
            let k = albedo_grad(m.layer, dx, dy);
            a = textureSampleGrad(albedo, plant_samp, in.uv, m.layer, dx * k, dy * k).a;
        } else {
            let k = albedo_grad(m.layer, dx, dy);
            a = textureSampleGrad(albedo, samp, in.uv, m.layer, dx * k, dy * k).a;
        }
        // Grass density is a world-cell decision, not screen-space noise.
        // Keep its photo silhouette threshold fixed while TAA jitters geometry.
        let threshold = select(view.extra.x, 0.5, (m.flags & 64u) != 0u);
        return a < threshold;
    }
    return false;
}

@fragment
fn fs(in: VOut) -> @location(0) u32 {
    if alpha_cut(in) {
        discard;
    }
    return in.id;
}

// Low-resolution stadium broadcast. The existing raster vertices include live
// skinning, moving weapons and the current destructible geometry.
@fragment fn fs_broadcast(in: VOut) -> @location(0) vec4f {
    let n = normalize(cross(dpdy(in.world),dpdx(in.world)));
    if alpha_cut(in) { discard; }
    let m = material(in.material);
    var base = m.tint;
    if m.layer >= 0 && (m.flags & 16384u) == 0u {
        let uv = select(in.uv, in.world.xz*m.scale, (m.flags & 16u)!=0u);
        base *= textureSampleLevel(albedo,samp,uv,m.layer,max(2.,albedo_lod_floor(m.layer))).rgb;
    }
    let normal = select(-n,n,dot(n,view.cam.xyz-in.world)>=0.);
    let sky = view.lod_cams[0].w*(0.5+0.25*normal.y);
    let sun = view.lod_cams[1].rgb*max(dot(normal,view.lod_cams[0].xyz),0.);
    var lit = base*(0.25+(sky+sun)/3.14159);
    if (m.flags & 1024u)!=0u { lit = base*max(0.4,abs(m.scale)*view.lod_cams[1].w); }
    if (m.flags & 4096u)!=0u { lit=vec3f(1.8,0.42,0.055); }
    return vec4f(lit/(1.+lit),1.);
}

// Opaque clusters (cull.wgsl masked_cluster false): no discard, so the
// hardware can test and write depth before the fragment runs.
@fragment
fn fs_opaque(in: VOut) -> @location(0) u32 {
    return in.id;
}

@fragment
fn fs_depth(in: VOut) {
    if alpha_cut(in) {
        discard;
    }
}

// Cached sun maps (shared map and clipmap levels): 64x64 tiles of the map's
// own size; ordinary and headlamp entry points stay independent.
@group(2) @binding(0) var<uniform> shadow_damage_mask:array<vec4<u32>,32>;
@fragment fn fs_cached_depth(in:VOut) {
 let xy=vec2<u32>(in.pos.xy)/max(u32(view.size.x)/64u,1u);let tile=xy.y*64u+xy.x;
 let word=shadow_damage_mask[tile>>7u][(tile>>5u)&3u];
 if (word&(1u<<(tile&31u)))==0u {discard;}
 if alpha_cut(in) {discard;}
}
