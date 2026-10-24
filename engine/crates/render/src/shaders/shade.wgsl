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
    sky: vec4<f32>,          // x: sky luminance scale, y: sky rotation (rad), z: exposure, w: shadow texel (m)
    fog_color: vec4<f32>,    // legacy horizon radiance; w: legacy fog start (m)
    fog: vec4<f32>,          // x: legacy end (m), y: extinction/m, z: aerosol scale height (m; 0 legacy), w: wetness
    fog_scattering: vec4<f32>, // RGB single-scattering albedo; w: aerosol phase asymmetry
    sh: array<vec4<f32>, 9>, // sky irradiance, SH9, already in lux
    near_shadow_view_proj: mat4x4<f32>,
    near_shadow: vec4<f32>, // world texel size, blend-start radius, enabled
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
@group(0) @binding(5) var<storage, read> materials: array<Material>;
@group(0) @binding(6) var albedo: texture_2d_array<f32>;
@group(0) @binding(7) var samp: sampler;
@group(0) @binding(8) var<storage, read> aux: array<u32>;
@group(0) @binding(9) var normal_tex: texture_2d_array<f32>;
@group(0) @binding(10) var plant_samp: sampler;

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

struct Gi {
    vol_origins: array<vec4<f32>, 3>,
    vol_anchors: array<vec4<f32>, 3>,
    vol_dims: vec4<u32>,
    hf: vec4<f32>,
    hf_dims: vec4<u32>,
    bounce: vec4<f32>,
    params: vec4<f32>, // contact on, reflection on, sun-free sky cap, unused
};

// Extinction density on three absolute world grids. Blend the densities
// before tracing, so overlapping levels never count the same forest twice.
// The continuous anchors control the handoff; snapped storage windows do
// not. An origin shift therefore cannot move a blend boundary in one frame.
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
        // Each level occupies one Y slab. Clamp inside its own texel centres
        // so trilinear filtering can never sample a different level's slab.
        let local = clamp(q, vec3<f32>(0.5), d - 0.5);
        let uv = vec3<f32>(local.x / d.x,
            (local.y + f32(level) * d.y) / (3.0 * d.y), local.z / d.z);
        sigma += remaining * weight * textureSampleLevel(canopy_tex, canopy_sampler, uv, 0.0).r;
        remaining *= 1.0 - weight;
        if remaining <= 0.0001 { break; }
    }
    return sigma;
}

// Smooth projected-area approximation for diffuse contact, and opaque ray
// intersection for glossy visibility. Proxies follow the live car transform.
// These small local bounds never share the coarse, translucent foliage grid.
fn contact_visibility(p: vec3<f32>, n: vec3<f32>, refl: vec3<f32>, owner: u32) -> vec2<f32> {
    if gi.vol_dims.w == 0u || gi.params.x < 0.5 { return vec2<f32>(1.0); }
    var diffuse = 1.0;
    var specular = 1.0;
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
        let q = p + dir * t;
        if q.y < ground_at(q.xz) - 0.3 {
            return 0.0;
        }
        let step = max(t * 0.45, 0.6);
        tr *= exp(-foliage(q) * 0.9 * step);
        t += step;
        if t > max_t || tr < 0.02 {
            break;
        }
    }
    return tr;
}

// Fraction of the sky a surface sees: five cones around its normal.
fn sky_visibility(p: vec3<f32>, n: vec3<f32>) -> f32 {
    if gi.vol_dims.w == 0u {
        return 1.0;
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
    if gi.vol_dims.w == 0u || gi.params.y < 0.5 { return 1.0; }
    let o = p + n * 0.06;
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
    let sky = mix(min(sky_radiance(r), vec3<f32>(gi.params.z)), sky_irradiance(r) / PI, smoothstep(0.05, 0.55, rough));
    let ground = bounce / PI;
    return mix(ground, sky, smoothstep(-0.08, 0.04, r.y));
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
    // This local single-scattering solution does not raymarch light occlusion
    // inside the medium or solve multiple scattering.
    let diffuse = max(light.sh[0].rgb * 0.282095, vec3<f32>(0.0));
    let solar = light.sun_color.rgb * light.sun_dir.w
        * fog_phase(clamp(dot(direction, light.sun_dir.xyz), -1.0, 1.0), light.fog_scattering.w);
    return light.fog_scattering.rgb * (diffuse + solar);
}

fn background_radiance(direction: vec3<f32>) -> vec3<f32> {
    let sky = sky_radiance(direction);
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
        return fog_segment(radiance, fog_scattering_source(direction), light.fog.y * column);
    }
    let amount = distance_fog_amount(distance);
    // Near surfaces do not need an additional sky texture lookup.
    if amount <= 0.0 { return radiance; }
    return fog_blend(radiance, background_radiance(direction), amount);
}

// Irradiance (lux) from the sky for a surface facing n.
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

fn shadow_filter(map: texture_depth_2d, projection: mat4x4<f32>, world_texel: f32, p: vec3<f32>, n: vec3<f32>) -> f32 {
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

// Detail follows each camera but its texels remain fixed in world space.
// Blend the far-map/canopy result before adding the near detail, so a camera
// near the shared-map edge cannot overwrite a valid car shadow with foliage.
fn shadow_at(p: vec3<f32>, n: vec3<f32>) -> f32 {
    var weight = 0.0;
    if light.near_shadow.z > 0.5 {
        let c = light.near_shadow_view_proj * vec4<f32>(p, 1.0);
        if c.z >= 0.0 && c.z <= 1.0 {
            weight = 1.0 - smoothstep(light.near_shadow.y, 1.0, max(abs(c.x), abs(c.y)));
        }
    }
    if weight >= 1.0 {
        return shadow_filter(near_shadow_tex, light.near_shadow_view_proj, light.near_shadow.x, p, n);
    }
    var far = shadow_filter(shadow_tex, light.shadow_view_proj, light.sky.w, p, n);
    let c = light.shadow_view_proj * vec4<f32>(p, 1.0);
    let edge = max(abs(c.x), abs(c.y));
    // Fade to a covered coarse forest field at every shadow-frustum face,
    // including depth. A receiver outside Z must not silently become sunlit.
    let depth_edge = max(1.0 - c.z / 0.05, 1.0 - (1.0 - c.z) / 0.05);
    let coverage_fade = max(smoothstep(0.75, 1.0, edge), smoothstep(0.0, 1.0, depth_edge));
    if coverage_fade > 0.0 && gi.vol_dims.w != 0u {
        let tr = transmittance(p + n * 0.2, light.sun_dir.xyz, 50.0);
        far = mix(far, tr, coverage_fade);
    }
    if weight <= 0.0 { return far; }
    let detail = shadow_filter(near_shadow_tex, light.near_shadow_view_proj, light.near_shadow.x, p, n);
    return mix(far, detail, weight);
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

fn terrain_layer_normal(l: i32, uv: vec2<f32>, scale: f32, mapping: TerrainMapping) -> vec2<f32> {
    let a = mapping.a * nrm(l, mapping.a * uv / scale + mapping.offset_a, mapping.a * fp.dx.xz / scale, mapping.a * fp.dy.xz / scale);
    let b = mapping.b * nrm(l, mapping.b * uv / scale + mapping.offset_b, mapping.b * fp.dx.xz / scale, mapping.b * fp.dy.xz / scale);
    return mix(a, b, mapping.blend);
}

fn terrain_layer(l: i32, uv: vec2<f32>, scale: f32, mapping: TerrainMapping) -> vec3<f32> {
    let a = tex_w(l, mapping.a * uv / scale + mapping.offset_a, mapping.a * fp.dx.xz / scale, mapping.a * fp.dy.xz / scale).rgb;
    let b = tex_w(l, mapping.b * uv / scale + mapping.offset_b, mapping.b * fp.dx.xz / scale, mapping.b * fp.dy.xz / scale).rgb;
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

// One world-space rock material everywhere. The low-amplitude continuous
// warp changes grain alignment gradually; it never selects another texture
// rotation/scale. Analytic coordinate gradients drive BOTH normal mapping
// and anisotropic texture footprints, including oblique cliff views.
fn rock_surface(l: i32, p: vec3<f32>, n: vec3<f32>, tw: vec3<f32>) -> RockSurface {
    let phase = vec3<f32>(p.y * 0.071 + p.z * 0.043,
        p.x * 0.057 - p.z * 0.049, p.x * 0.053 + p.y * 0.037);
    let wave = sin(phase);
    let slope = cos(phase);
    let q = p + wave * vec3<f32>(0.28, 0.25, 0.31);
    let gx = vec3<f32>(1.0, 0.01988 * slope.x, 0.01204 * slope.x);
    let gy = vec3<f32>(0.01425 * slope.y, 1.0, -0.01225 * slope.y);
    let gz = vec3<f32>(0.01643 * slope.z, 0.01147 * slope.z, 1.0);
    let dx = vec3<f32>(dot(gx, fp.dx), dot(gy, fp.dx), dot(gz, fp.dx)) / 3.7;
    let dy = vec3<f32>(dot(gx, fp.dy), dot(gy, fp.dy), dot(gz, fp.dy)) / 3.7;
    let uv = q / 3.7;
    var result: RockSurface;
    result.color = tex_w(l, uv.zy, dx.zy, dy.zy).rgb * tw.x
        + tex_w(l, uv.xz, dx.xz, dy.xz).rgb * tw.y
        + tex_w(l, uv.xy, dx.xy, dy.xy).rgb * tw.z;
    let nx = perturb(n, nrm(l, uv.zy, dx.zy, dy.zy), gz, -gy);
    let ny = perturb(n, nrm(l, uv.xz, dx.xz, dy.xz), gx, -gz);
    let nz = perturb(n, nrm(l, uv.xy, dx.xy, dy.xy), gx, -gy);
    result.normal = safe_normalize(nx * tw.x + ny * tw.y + nz * tw.z);
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

@compute @workgroup_size(8, 8)
fn shade(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<u32>(view.size.xy);
    if gid.x >= size.x || gid.y >= size.y {
        return;
    }
    let px = vec2<i32>(gid.xy);
    let ndc = vec2<f32>((f32(gid.x) + 0.5) / view.size.x * 2.0 - 1.0, 1.0 - (f32(gid.y) + 0.5) / view.size.y * 2.0);
    let far = view.inv_view_proj * vec4<f32>(ndc, 1e-4, 1.0);
    let dir = normalize(far.xyz / far.w - view.cam.xyz);
    let depth = textureLoad(depth_tex, px, 0);
    if depth <= 0.0 {
        textureStore(out_hdr, px, vec4<f32>(background_radiance(dir), 1.0));
        return;
    }
    let id = textureLoad(vis_tex, px, 0).r;
    let vis = visible[id >> 7u];
    let tri = id & 127u;
    let cl = clusters[vis.y];
    let inst = instances[vis.x];
    let packed = tris[cl.first_tri + tri];
    var vi: array<u32, 3>;
    for (var k = 0u; k < 3u; k++) {
        vi[k] = vtx(cluster_verts[cl.first_vert + ((packed >> (8u * k)) & 0xffu)]);
    }
    // Match the actual deformed raster triangle before ray intersection and
    // texture derivatives. Rest-pose reconstruction would slide UVs and light.
    let p0 = wind_position(vis.x, inst.r1.w, xform(inst, pos_of(vi[0])));
    let p1 = wind_position(vis.x, inst.r1.w, xform(inst, pos_of(vi[1])));
    let p2 = wind_position(vis.x, inst.r1.w, xform(inst, pos_of(vi[2])));
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
    n = normalize(wind_normal(vis.x, xform_dir(inst, n)));
    let geo_n = normalize(cross(e1, e2));
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
    if (m.flags & 2u) != 0u {
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
    } else if (m.flags & 64u) != 0u {
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
    } else if m.layer >= 0 {
        base = tex_w(m.layer, uv, duv_x, duv_y).rgb * m.tint;
        var frame = uv_frame(e1, e2, vi);
        if dot(geo_n, n) < 0.0 {
            // The far side of a two-sided card: its relief is the mirror image.
            frame[0] = -frame[0];
            frame[1] = -frame[1];
        }
        nn = perturb(n, nrm(m.layer, uv, duv_x, duv_y), frame[0], frame[1]);
    }
    n = nn;
    let l = light.sun_dir.xyz;
    let ndl = dot(n, l);
    var sun = max(ndl, 0.0);
    var back = vec3<f32>(0.0);
    var ao = 1.0;
    let grass = (m.flags & 64u) != 0u;
    let procedural_leaf = (m.flags & 512u) != 0u;
    let needles = procedural_leaf || ((m.flags & 9u) == 9u && (inst.pad & 1u) != 0u && !grass);
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
    var shadow = 1.0;
    if sun > 0.0 || needles || gi.vol_dims.w != 0u || (m.flags & 32u) != 0u || m.roughness < 0.55 {
        let s_map = shadow_at(p, surface_n * sign(dot(surface_n, l) + 1e-4));
        shadow = mix(1.0, s_map, light.sun_color.w);
    }
    if (m.flags & 70u) != 0u {shadow*=mix(1.0,grass_sun_transmittance(p,l),light.sun_color.w);}
    // Base layer: Lambert plus GGX, metals tint their reflection.
    let v = -dir;
    let ndv = max(dot(n, v), 1e-4);
    let rough = clamp(m.roughness, 0.04, 1.0);
    var f0 = mix(vec3<f32>(0.04), base, m.metal);
    if needles {
        f0 = vec3<f32>(0.01); // foliage.gdshader: SPECULAR 0.125
    } else if grass {
        f0 = vec3<f32>(0.016); // grass_cards.gdshader: SPECULAR 0.2
    }
    let diffuse = base * (1.0 - m.metal) / PI;
    // Use the continuous surface hemisphere, unaffected by texture grain.
    // The triangle normal is only for intersection and texture derivatives.
    let visibility_n = surface_n;
    let sky_vis = sky_visibility(p, visibility_n);
    let sun_e = light.sun_color.rgb * light.sun_dir.w * shadow;
    let refl = reflect(dir, n);
    let contact = contact_visibility(p + visibility_n * 0.025, visibility_n, refl, vis.x);
    let bounce = gi.bounce.rgb * (gi.bounce.w * sky_vis + light.sun_dir.w * max(light.sun_dir.y, 0.0) * shadow);
    var reflection_vis = sky_vis;
    if (m.flags & 32u) != 0u || rough < 0.55 {
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
    } else if grass {
        let height=clamp((p.y-inst.r1.w)/max(inst.scale*0.5,1e-4),0.0,1.0);
        let response=grass_leaf_response(base,height,v,sun_e,shadow,sky_vis,contact,rough);
        direct=response.direct;indirect=response.indirect;specular=response.specular;
    } else {
        let ambient=(sky_irradiance(n)*sky_vis+bounce*(1.0-sky_vis))*ao*contact.x;
        let spec_contact=mix(contact.y,contact.x,smoothstep(0.08,0.6,rough));
        let env=mix(bounce/PI,environment(refl,rough,bounce),reflection_vis)*spec_contact;
        direct=diffuse*sun_e*sun+sun_e*back/PI+sun_e*ggx(n,v,l,rough,f0);
        indirect=diffuse*ambient;
        specular=env*env_brdf(f0,rough,ndv)*ao;
    }
    if (m.flags & 32u) != 0u {
        // Clear coat: a thin glossy dielectric on top; what it reflects the
        // base does not get.
        let fc = 0.04 + 0.96 * pow(1.0 - ndv, 5.0);
        direct = direct * (1.0 - fc) + sun_e * ggx(n, v, l, 0.04, vec3<f32>(0.04));
        indirect *= 1.0 - fc;
        specular = specular * (1.0 - fc) + mix(bounce / PI, environment(refl, 0.04, bounce), reflection_vis) * contact.y * fc;
    }
    direct += headlight_radiance(p,n,v,diffuse,rough,f0);
    var rad = direct + indirect + specular;
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
        rad = vec3<f32>(shadow_at(p, surface_n * sign(dot(surface_n, l) + 1e-4))) * 6000.0;
    } else if light.sky.z > 9.5 && light.sky.z < 10.5 {
        rad = vec3<f32>(grass_sun_transmittance(p,l)) * 6000.0;
    } else if light.sky.z > 10.5 && light.sky.z < 11.5 {
        rad = (n * 0.5 + 0.5) * 6000.0;
    } else if light.sky.z > 11.5 && light.sky.z < 12.5 {
        rad = (surface_n * 0.5 + 0.5) * 6000.0;
    } else if light.sky.z > 12.5 && light.sky.z < 13.5 {
        rad = vec3<f32>(max(dot(n,l),0.0)) * 6000.0;
    } else if light.sky.z > 13.5 && light.sky.z < 14.5 {
        rad = vec3<f32>(max(dot(surface_n,l),0.0)) * 6000.0;
    }
    textureStore(out_hdr, px, vec4<f32>(rad, 1.0));
}
