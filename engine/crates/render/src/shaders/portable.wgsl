// The portable renderer (portable.rs): procedural terrain and sky for
// devices with fewer than 16 storage buffers per shader stage. Three here.
// Terrain patches are drawn straight from the pool procedural_terrain.wgsl
// writes: one instance per visible slot (a vertex buffer the CPU fills), vertex = corner of the shared 8x8-cell topology
// (procedural_terrain.rs reserve). Shading is a simplified field_surface.

struct U {
    vp: mat4x4<f32>,
    inv_vp: mat4x4<f32>,
    camera: vec4<f32>,      // position, exposure
    sun: vec4<f32>,         // direction (towards the sun), lux * exposure
    sun_color: vec4<f32>,   // rgb, sun disc on (1) or off
    horizon: vec4<f32>,     // sky horizon radiance * exposure, fog density
    sky: vec4<f32>,         // sky lux * exposure, fog start, fog end, debug
    pool: vec4<u32>,        // first instance, first vertex, terrain material, slots
};
struct Material { tint: vec3<f32>, layer: i32, roughness: f32, flags: u32, scale: f32, metal: f32 };

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var<storage, read> vertices: array<f32>;
@group(0) @binding(2) var<storage, read> instances: array<u32>;
@group(0) @binding(3) var<storage, read> materials: array<Material>;
@group(0) @binding(4) var albedo: texture_2d_array<f32>;
@group(0) @binding(5) var samp: sampler;

struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // rock, scree, wet, up; water, moor, snow (terrain_surface.wgsl field_unpack)
    @location(2) a: vec4<f32>,
    @location(3) b: vec3<f32>,
};

fn unpack(uv: vec2<f32>) -> array<f32, 7> {
    let a = u32(round(max(uv.x, 0.0)));
    let b = u32(round(max(uv.y, 0.0)));
    let w = f32((b >> 8u) & 63u) / 63.0;
    return array<f32, 7>(f32(a & 255u) / 255.0, f32((a >> 8u) & 255u) / 255.0, f32((a >> 16u) & 255u) / 255.0,
        f32(b & 255u) / 255.0, w * w * 2.55, f32((b >> 14u) & 31u) / 31.0, f32((b >> 19u) & 31u) / 31.0);
}

@vertex fn vs_terrain(@builtin(vertex_index) v: u32, @location(0) slot: u32) -> Out {
    var out: Out;
    // GpuInstance: 20 words; cluster_count (word 17) is 0 for a free slot.
    let inst = (u.pool.x + slot) * 20u;
    if instances[inst + 17u] == 0u {
        out.pos = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return out;
    }
    let tri = v / 3u;
    let k = v % 3u;
    let cell = tri / 2u;
    let i = cell % 8u;
    let j = cell / 8u;
    let a = j * 9u + i;
    let b = a + 1u;
    let c = a + 9u;
    let d = c + 1u;
    var corners = array<u32, 3>(a, c, d);
    let second = (tri & 1u) == 1u;
    if ((i + j) & 1u) == 0u {
        corners = array<u32, 3>(a, c, b);
        if second { corners = array<u32, 3>(b, c, d); }
    } else if second {
        corners = array<u32, 3>(a, d, b);
    }
    let at = (u.pool.y + slot * 81u + corners[k]) * 8u;
    let p = vec3<f32>(vertices[at], vertices[at + 1u], vertices[at + 2u]);
    let n = vec3<f32>(vertices[at + 3u], vertices[at + 4u], vertices[at + 5u]);
    let f = unpack(vec2<f32>(vertices[at + 6u], vertices[at + 7u]));
    // The patch transform (identity for terrain, kept for completeness).
    let r0 = vec4<f32>(bitcast<f32>(instances[inst]), bitcast<f32>(instances[inst + 1u]), bitcast<f32>(instances[inst + 2u]), bitcast<f32>(instances[inst + 3u]));
    let r1 = vec4<f32>(bitcast<f32>(instances[inst + 4u]), bitcast<f32>(instances[inst + 5u]), bitcast<f32>(instances[inst + 6u]), bitcast<f32>(instances[inst + 7u]));
    let r2 = vec4<f32>(bitcast<f32>(instances[inst + 8u]), bitcast<f32>(instances[inst + 9u]), bitcast<f32>(instances[inst + 10u]), bitcast<f32>(instances[inst + 11u]));
    let w = vec3<f32>(dot(r0, vec4<f32>(p, 1.0)), dot(r1, vec4<f32>(p, 1.0)), dot(r2, vec4<f32>(p, 1.0)));
    out.pos = u.vp * vec4<f32>(w, 1.0);
    out.world = w;
    out.normal = vec3<f32>(dot(r0.xyz, n), dot(r1.xyz, n), dot(r2.xyz, n));
    out.a = vec4<f32>(f[0], f[1], f[2], f[3]);
    out.b = vec3<f32>(f[4], f[5], f[6]);
    return out;
}

// terrain_surface.wgsl FieldGround: colours (rgb) and a per-layer amount (w).
struct Ground { grass_dry: vec4<f32>, grass_lush: vec4<f32>, moor: vec4<f32>, soil: vec4<f32>, gravel: vec4<f32>, rock: vec4<f32>, snow: vec4<f32>, reserved: vec4<f32> };
const DEFAULT_GROUND = Ground(vec4<f32>(0.43, 0.55, 0.23, 0.3), vec4<f32>(0.32, 0.69, 0.28, 0.0),
    vec4<f32>(0.6, 0.72, 0.7, 0.8), vec4<f32>(0.85, 0.8, 0.75, 0.35), vec4<f32>(0.95, 0.97, 1.0, 0.5),
    vec4<f32>(1.0, 1.04, 1.1, 0.35), vec4<f32>(0.84, 0.87, 0.92, 0.0), vec4<f32>(0.0, 0.0, 0.0, 0.0));
fn pair(m: Material) -> array<vec4<f32>, 2> {
    return array<vec4<f32>, 2>(vec4<f32>(m.tint, bitcast<f32>(m.layer)), vec4<f32>(m.roughness, bitcast<f32>(m.flags), m.scale, m.metal));
}
fn ground(m: Material) -> Ground {
    if m.scale >= 0.0 { return DEFAULT_GROUND; }
    let at = u32(-m.scale - 0.5) + 3u;
    let a = pair(materials[at]);
    let b = pair(materials[at + 1u]);
    let c = pair(materials[at + 2u]);
    let d = pair(materials[at + 3u]);
    return Ground(a[0], a[1], b[0], b[1], c[0], c[1], d[0], d[1]);
}

// Past a few tens of metres a tiling texture shows its repeat as stripes:
// fade it to its mean colour (the last mip) with distance. Gradients come
// from the world position's, taken once in uniform control flow (Tint
// rejects implicit-derivative samples inside the per-pixel branches).
var<private> detail: f32 = 1.0;
var<private> dpx: vec3<f32>;
var<private> dpy: vec3<f32>;
fn layer(l: i32, uv: vec2<f32>, gx: vec2<f32>, gy: vec2<f32>) -> vec3<f32> {
    let mean = textureSampleLevel(albedo, samp, vec2<f32>(0.5), l, 16.0).rgb;
    return mix(mean, textureSampleGrad(albedo, samp, uv, l, gx, gy).rgb, detail);
}
fn flat_layer(l: i32, p: vec3<f32>, scale: f32) -> vec3<f32> { return layer(l, p.xz / scale, dpx.xz / scale, dpy.xz / scale); }
fn triplanar(l: i32, p: vec3<f32>, n: vec3<f32>, scale: f32) -> vec3<f32> {
    var w = abs(n);
    w = w * w * w * w;
    w /= w.x + w.y + w.z;
    return layer(l, p.zy / scale, dpx.zy / scale, dpy.zy / scale) * w.x + layer(l, p.xz / scale, dpx.xz / scale, dpy.xz / scale) * w.y
        + layer(l, p.xy / scale, dpx.xy / scale, dpy.xy / scale) * w.z;
}
// Two turned mappings blended by a slow noise, so the repeat shows no stripes.
fn turned(l: i32, p: vec3<f32>, scale: f32, mix_w: f32) -> vec3<f32> {
    let r = mat2x2<f32>(vec2<f32>(0.8, 0.6), vec2<f32>(-0.6, 0.8));
    let uv = p.xz / scale;
    let gx = dpx.xz / scale;
    let gy = dpy.xz / scale;
    return mix(layer(l, uv, gx, gy), layer(l, r * uv + 0.37, r * gx, r * gy), mix_w);
}
fn hash(p: vec2<f32>) -> f32 { return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453); }
fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let s = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + vec2<f32>(1.0, 0.0)), s.x), mix(hash(i + vec2<f32>(0.0, 1.0)), hash(i + vec2<f32>(1.0, 1.0)), s.x), s.y);
}

fn sky(dir: vec3<f32>) -> vec3<f32> {
    let up = clamp(dir.y, -1.0, 1.0);
    // The renderer's horizon radiance is warm (ground bounce); the zenith
    // is a blue of the same brightness.
    let lum = dot(u.horizon.rgb, vec3<f32>(0.3, 0.59, 0.11));
    let horizon = mix(u.horizon.rgb, lum * vec3<f32>(0.85, 0.93, 1.05), 0.5);
    let zenith = lum * vec3<f32>(0.3, 0.5, 0.95);
    var c = mix(horizon, zenith, sqrt(max(up, 0.0)));
    c = select(c, horizon * 0.6, up < 0.0);
    let mu = dot(dir, u.sun.xyz);
    c += u.sun_color.rgb * u.sun.w * 0.002 * pow(max(mu, 0.0), 64.0);
    if u.sun_color.w > 0.5 && mu > 0.99996 { c += u.sun_color.rgb * u.sun.w * 0.05; }
    return c;
}

fn encode_srgb(c: vec3<f32>) -> vec3<f32> { return select(c * 12.92, 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055, c > vec3<f32>(0.0031308)); }
fn tone(c: vec3<f32>) -> vec4<f32> {
    let x = max(c, vec3<f32>(0.0));
    return vec4<f32>(encode_srgb(x / (1.0 + x)), 1.0);
}

@fragment fn fs_terrain(in: Out) -> @location(0) vec4<f32> {
    let m = materials[u.pool.z];
    let g = ground(m);
    let l = m.layer;
    let n = normalize(in.normal);
    let p = in.world;
    let rock = in.a.x;
    let scree = in.a.y;
    let wet = in.a.z;
    let up = in.a.w;
    let water = in.b.x;
    let moor = in.b.y;
    let snow = in.b.z;
    dpx = dpdx(in.world);
    dpy = dpdy(in.world);
    detail = 1.0 - 0.75 * smoothstep(12.0, 90.0, distance(p, u.camera.xyz));
    let broad = noise(p.xz / 37.0 + 4.0);
    let mottle = noise(p.xz / 4.3 + 11.0);
    let lush = clamp(0.5 + (broad - 0.5) * 1.2 + 0.4 * wet, 0.0, 1.0);
    let turn = smoothstep(0.3, 0.7, noise(p.xz / 9.0 + 23.0));
    let grass = turned(l, p, 2.6, turn) * mix(g.grass_dry.rgb, g.grass_lush.rgb, lush);
    var c = grass;
    let m_moor = smoothstep(0.25, 0.8, clamp(moor + (broad - 0.5) * 0.7 + (mottle - 0.5) * 0.3, 0.0, 1.0));
    if m_moor > 0.0 { c = mix(c, mix(turned(l + 1, p, 3.2, turn) * g.moor.rgb, grass * g.moor.w, g.grass_dry.w), m_moor); }
    let m_soil = clamp(smoothstep(0.9, 0.75, up) * 0.35 + smoothstep(0.7, 0.85, mottle) * 0.25, 0.0, 1.0);
    if m_soil > 0.0 { c = mix(c, triplanar(l + 2, p, n, 3.0) * g.soil.rgb, m_soil); }
    c *= 0.82 + 0.3 * broad;
    let m_scree = smoothstep(0.2, 0.7, scree + (mottle - 0.5) * 0.4);
    if m_scree > 0.0 {
        let gravel = flat_layer(l + 4, p, 1.8);
        c = mix(c, mix(vec3<f32>(dot(gravel, vec3<f32>(0.3, 0.59, 0.11))), gravel, g.gravel.w) * g.gravel.rgb, m_scree);
    }
    let m_rock = smoothstep(0.25, 0.75, rock + (mottle - 0.5) * 0.35);
    if m_rock > 0.0 {
        let bare = triplanar(l + 3, p, n, 7.0);
        let stone = mix(vec3<f32>(dot(bare, vec3<f32>(0.3, 0.59, 0.11))), bare, g.rock.w) * g.rock.rgb * (0.85 + 0.3 * broad);
        let m_moss = smoothstep(0.55, 0.9, n.y) * (0.5 + 0.5 * moor) * smoothstep(0.3, 0.7, broad);
        c = mix(c, mix(stone, triplanar(l + 5, p, n, 3.5), m_moss), m_rock);
    }
    let m_snow = smoothstep(0.3, 0.7, snow + (mottle - 0.5) * 0.3);
    c = mix(c, g.snow.rgb, m_snow);
    let m_water = smoothstep(0.02, 0.2, water);
    c = mix(c, vec3<f32>(0.02, 0.035, 0.04), m_water);

    let sun = u.sun_color.rgb * u.sun.w * max(dot(n, u.sun.xyz), 0.0);
    let ambient = u.sky.x * (0.55 + 0.45 * n.y) * mix(vec3<f32>(1.0), u.horizon.rgb / max(max(u.horizon.r, u.horizon.g), max(u.horizon.b, 1e-6)), 0.5);
    var lit = c * (sun + ambient) / 3.14159265;
    // Water reflects the sky.
    let view = normalize(p - u.camera.xyz);
    lit = mix(lit, sky(reflect(view, vec3<f32>(0.0, 1.0, 0.0))) * 0.6, m_water * 0.8);
    // Haze towards the horizon colour.
    let d = distance(p, u.camera.xyz);
    let density = select(1.0 / 9000.0, u.horizon.w, u.horizon.w > 0.0);
    let fog = 1.0 - exp(-d * density);
    lit = mix(lit, sky(view), clamp(fog, 0.0, 1.0));
    if u.sky.w == 2.0 { lit = c; }
    if u.sky.w == 11.0 { lit = n * 0.5 + 0.5; }
    return tone(lit);
}

struct SkyOut { @builtin(position) pos: vec4<f32>, @location(0) ndc: vec2<f32> };
@vertex fn vs_sky(@builtin(vertex_index) v: u32) -> SkyOut {
    let xy = vec2<f32>(f32((v << 1u) & 2u), f32(v & 2u)) * 2.0 - 1.0;
    return SkyOut(vec4<f32>(xy, 0.0, 1.0), xy);
}
@fragment fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    let far = u.inv_vp * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(far.xyz / far.w - u.camera.xyz);
    return tone(sky(dir));
}
