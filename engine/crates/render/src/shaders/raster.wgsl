// Visibility buffer raster: every visible cluster is one instance of a
// size-bucketed draw; vertices are pulled from storage buffers. The colour
// target stores which triangle covers each pixel; shading happens later,
// once per pixel.
override CLUSTER_BUCKETS: bool = true;

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
    pad: f32,
};

@group(0) @binding(0) var<storage, read> vertices: array<f32>;
@group(0) @binding(1) var<storage, read> cluster_verts: array<u32>;
@group(0) @binding(2) var<storage, read> tris: array<u32>;
@group(0) @binding(3) var<storage, read> clusters: array<Cluster>;
@group(0) @binding(4) var<storage, read> instances: array<Instance>;
@group(0) @binding(5) var<storage, read> materials: array<Material>;
@group(0) @binding(6) var albedo: texture_2d_array<f32>;
@group(0) @binding(7) var samp: sampler;
@group(0) @binding(10) var plant_samp: sampler;
@group(0) @binding(11) var<storage,read> voxel_replacement: array<u32>;

@group(1) @binding(0) var<uniform> view: View;
@group(1) @binding(1) var<storage, read> visible: array<vec2<u32>>;
@group(1) @binding(2) var<storage, read> draw_map: array<u32>;
@group(1) @binding(3) var<storage, read> draw_args: array<u32, 20>;
@group(1) @binding(4) var<uniform> foliage_selection: array<vec4<u32>,4096>;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) @interpolate(flat) id: u32,
    @location(1) @interpolate(flat) material: u32,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) grass_opacity: f32,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VOut {
    var o: VOut;
    var slot = ii;
    if CLUSTER_BUCKETS { slot = draw_map[draw_args[16u + vi / 384u] + ii]; }
    let vis = visible[slot];
    let cl = clusters[vis.y];
    let tri = (vi % 384u) / 3u;
    if tri >= cl.tri_count {
        o.pos = vec4<f32>(2.0, 2.0, 2.0, 1.0); // degenerate, outside the clip volume
        return o;
    }
    let inst = instances[vis.x];
    let packed = tris[cl.first_tri + tri];
    let local = (packed >> (8u * (vi % 3u))) & 0xffu;
    let v = cluster_verts[cl.first_vert + local] * 8u;
    let p = vec4<f32>(vertices[v], vertices[v + 1u], vertices[v + 2u], 1.0);
    let rest = vec3<f32>(dot(inst.r0, p), dot(inst.r1, p), dot(inst.r2, p));
    let w = wind_position(vis.x, inst.r1.w, rest);
    o.pos = view.view_proj * vec4<f32>(w, 1.0);
    o.id = (slot << 7u) | tri;
    o.material = cl.material;
    o.uv = vec2<f32>(vertices[v + 6u], vertices[v + 7u]);
    // Only the explicitly packed crown material is replaced in color. Wood,
    // unrelated cutouts and source sun-shadow geometry remain unchanged.
    let replacement=voxel_replacement[vis.x];
    var replaced=replacement==0xffffffffu || replacement==cl.material+1u;
    if replacement!=0xffffffffu && (replacement&0x80000000u)!=0u {
        replaced=foliage_selection[replacement&0x7fffffffu].x!=0u;
    }
    if view.params.z<0.5 && view.extra.y<1.5 && replaced {
        o.pos=vec4<f32>(2.0,2.0,2.0,1.0);
    }
    o.grass_opacity = 1.0;
    if (inst.pad & 8u) != 0u {
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
    let m = materials[in.material];
    if (m.flags & 1u) != 0u && m.layer >= 0 {
        // PIXEL_ART keeps the top mip's hard silhouette.
        let sharp = select(1.0, 0.0, (m.flags & 8192u) != 0u);
        let dx = dx * sharp;
        let dy = dy * sharp;
        var a: f32;
        if (m.flags & 64u) != 0u {
            a = textureSampleGrad(albedo, plant_samp, in.uv, m.layer, dx, dy).a;
        } else {
            a = textureSampleGrad(albedo, samp, in.uv, m.layer, dx, dy).a;
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

@fragment
fn fs_depth(in: VOut) {
    if alpha_cut(in) {
        discard;
    }
}
