// Blast effects after temporal reconstruction, ported from Growing Guns'
// violence.gd: billowing fire and smoke clouds (lumpy spheres with fresnel
// rims, a banded fire palette and soft depth fades), stretched flame tongues,
// additive ember streaks and a refracting heat shell. Every puff animates on
// the GPU from one clock; the CPU uploads births only when they change.
struct FxUniform {
    view_proj: mat4x4<f32>,
    inverse_view_proj: mat4x4<f32>,
    depth_vp: mat4x4<f32>,
    view: mat4x4<f32>,
    // camera position, near plane
    camera_near: vec4<f32>,
    // width, height, 1/width, 1/height
    size: vec4<f32>,
    // time, white luminance (nits), LDR exposure (zero for HDR), sRGB output
    clock: vec4<f32>,
    // Ray steps, soft-layer divisor, reciprocal soft-layer width/height.
    smoke: vec4<f32>,
    // Use exactly the composite guide mapping at reduced pixel centres.
    depth_from_output: mat4x4<f32>,
    ambient: vec4<f32>,
    sunlight: vec4<f32>,
    fog: vec4<f32>,
    horizon: vec4<f32>,
    sun_vp: mat4x4<f32>,
    fog_scattering: vec4<f32>,
    sun_direction: vec4<f32>,
    sky_mean: vec4<f32>,
    // Minimum camera-to-cloud distance in m (-1 when off), aerial far in km, aerial enabled.
    sky_volume: vec4<f32>,
};
struct Puff {
    // centre, kind (0 smoke, 1 fire, 2 ember, 3 heat, 4 column, 5 streak, 6 mote, 7 baked smoke)
    center_kind: vec4<f32>,
    // local axes with lengths (columns of the end transform) and per-kind data
    ax: vec4<f32>,
    ay: vec4<f32>,
    az: vec4<f32>,
    // body rgb (smoke) or tint (ember), peak opacity
    body: vec4<f32>,
    // birth, life, rise (m over life), age speed
    timing: vec4<f32>,
    // kind parameters: smoke/fire (smoke tint rgb, tail_power);
    // ember (size, elong01, grow_start, brightness); heat (strength, 0, 0, 0)
    extra: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: FxUniform;
@group(0) @binding(1) var<storage, read> puffs: array<Puff>;
@group(0) @binding(2) var<storage, read> order: array<u32>;
@group(0) @binding(3) var background: texture_2d<f32>;
@group(0) @binding(4) var scene_depth: texture_depth_2d;
@group(0) @binding(5) var samp: sampler;
@group(0) @binding(6) var smoke_animation: texture_3d<f32>;
@group(0) @binding(7) var sun_depth: texture_depth_2d;

@group(0) @binding(8) var cloud_light: texture_2d<f32>;
@group(0) @binding(9) var cloud_distance: texture_2d<f32>;
@group(0) @binding(10) var aerial_volume: texture_3d<f32>;

const CLEAR_AIR = vec4<f32>(0.0, 0.0, 0.0, 1.0);

// Same squared slice distribution and camera boundary as sky_volume.wgsl.
fn fx_aerial(uv: vec2<f32>, km: f32) -> vec4<f32> {
    let count = f32(textureDimensions(aerial_volume).z);
    let slice = sqrt(km / u.sky_volume.y) * count - 1.0;
    let w = (clamp(slice, 0.0, count - 1.0) + 0.5) / count;
    return mix(CLEAR_AIR, textureSampleLevel(aerial_volume, samp, vec3<f32>(uv, w), 0.0), clamp(slice + 1.0, 0.0, 1.0));
}

// Camera-to-fragment scattering (rgb) and transmission (a). Reuse the exact
// cloud history and bilinear footprint from the sky composite. For a fragment
// inside cloud, interpolate its accumulated opacity between the measured front,
// opacity-weighted mean and back of the cloud march.
fn fx_weather(world: vec3<f32>) -> vec4<f32> {
    if u.sky_volume.x < 0.0 && u.sky_volume.z == 0.0 { return CLEAR_AIR; }
    let d = distance(world, u.camera_near.xyz);
    let can_hit_cloud = u.sky_volume.x >= 0.0 && d >= u.sky_volume.x;
    if !can_hit_cloud && u.sky_volume.z == 0.0 { return CLEAR_AIR; }
    let clip = u.view_proj * vec4<f32>(world, 1.0);
    let uv = clamp(clip.xy / max(clip.w, 1e-6) * vec2<f32>(0.5, -0.5) + 0.5, vec2<f32>(0.0), vec2<f32>(1.0));
    let km = d * 0.001;
    var air = CLEAR_AIR;
    if u.sky_volume.z > 0.0 { air = fx_aerial(uv, km); }
    if !can_hit_cloud { return air; }
    let dims = vec2<i32>(textureDimensions(cloud_distance));
    let pixel = uv * vec2<f32>(dims) - 0.5;
    let base = vec2<i32>(floor(pixel));
    let f = fract(pixel);
    var cloud = vec4<f32>(0.0);
    var weighted_km = 0.0;
    var depth_weight = 0.0;
    for (var k=0; k<4; k++) {
        let o = vec2<i32>(k & 1, k >> 1u);
        let w = select(1.0-f.x, f.x, o.x==1) * select(1.0-f.y, f.y, o.y==1);
        let q = clamp(base + o, vec2<i32>(0), dims-1);
        let span = textureLoad(cloud_distance, q, 0).xyz;
        if km <= span.y || span.y >= span.z { continue; }
        var fraction = 1.0;
        if km < span.z {
            let middle = clamp(span.x, span.y, span.z);
            fraction = 0.5 * (clamp((km-span.y)/max(middle-span.y,1e-5),0.0,1.0)
                + clamp((km-middle)/max(span.z-middle,1e-5),0.0,1.0));
        }
        let sample = textureLoad(cloud_light, q, 0);
        cloud += w * fraction * vec4<f32>(sample.rgb, 1.0-sample.a);
        weighted_km += w * min(span.x,km);
        depth_weight += w;
    }
    if cloud.a < 1e-4 { return air; }
    let front_air = fx_aerial(uv, weighted_km/max(depth_weight,1e-5));
    let transmission = clamp(1.0-cloud.a, 0.0, 1.0);
    return vec4<f32>(air.rgb * transmission + cloud.rgb * front_air.a + front_air.rgb * cloud.a,
        air.a * transmission);
}

fn smoke_visibility(world: vec3<f32>) -> f32 {
    if u.sunlight.w <= 0.0 { return 1.0; }
    let p = u.sun_vp * vec4<f32>(world,1.0);
    let uv = p.xy * vec2<f32>(0.5,-0.5) + 0.5;
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || p.z<0.0 || p.z>1.0 { return 1.0; }
    let dims = vec2<i32>(textureDimensions(sun_depth));
    let pixel = uv * vec2<f32>(dims) - 0.5;
    let base = vec2<i32>(floor(pixel));
    let f = fract(pixel);
    var lit = 0.0;
    for(var y=0;y<2;y++) { for(var x=0;x<2;x++) {
        let q=clamp(base+vec2<i32>(x,y),vec2<i32>(0),dims-1);
        let weight=select(1.0-f.x,f.x,x==1)*select(1.0-f.y,f.y,y==1);
        lit += weight * select(0.0,1.0,p.z-0.0002 <= textureLoad(sun_depth,q,0));
    }}
    return mix(1.0,lit,u.sunlight.w);
}
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
        / (8.0 * 3.14159265 * (2.0 + g * g) * pow(1.0 + g * g - 2.0 * g * mu, 1.5));
}

// Fog is evaluated once per plume pixel, outside the density raymarch.
// Four deterministic quadrature points avoid temporal shimmer in post-TAA FX.
// This shares the surface atmosphere, with a bounded shadow integration cost.
fn smoke_atmosphere(world: vec3<f32>) -> vec4<f32> {
    if u.sky_volume.z > 0.0 { return CLEAR_AIR; }
    let delta = world-u.camera_near.xyz;
    let distance = max(length(delta), 0.0001);
    let direction = delta/distance;
    if u.fog.w <= 0.0 {
        let linear = clamp((distance-u.fog.x)/max(u.fog.y-u.fog.x,1.0),0.0,1.0);
        let amount = max(linear,1.0-exp(-u.fog.z*distance));
        return vec4<f32>(u.horizon.rgb*amount,1.0-amount);
    }
    if u.fog.z <= 0.0 { return vec4<f32>(0.0,0.0,0.0,1.0); }
    let transmission = exp(-u.fog.z*fog_column(distance,u.camera_near.y,direction.y,u.fog.w));
    var visibility = 1.0;
    if u.sun_direction.w > 0.0 && u.sunlight.w > 0.0 && any(u.sunlight.rgb > vec3<f32>(0.0)) {
        var visible = 0.0;
        var weights = 0.0;
        for(var i=0u;i<4u;i++) {
            let t = distance*(f32(i)+0.5)/4.0;
            let point = u.camera_near.xyz+direction*t;
            let weight = exp(-max(point.y,0.0)/u.fog.w
                -u.fog.z*fog_column(t,u.camera_near.y,direction.y,u.fog.w));
            visible += smoke_visibility(point)*weight;
            weights += weight;
        }
        visibility = visible/max(weights,0.000001);
    }
    let solar = u.sunlight.rgb*3.14159265
        *fog_phase(clamp(dot(direction,u.sun_direction.xyz),-1.0,1.0),u.fog_scattering.w)*visibility*u.sun_direction.w;
    let source = u.fog_scattering.rgb*(u.sky_mean.rgb+solar);
    return vec4<f32>(source*(1.0-transmission),transmission);
}
fn smoke_illumination(world: vec3<f32>, self_light: f32, albedo: vec3<f32>) -> vec3<f32> {
    return albedo*(u.ambient.rgb*(0.35+0.65*self_light)
        +u.sunlight.rgb*(0.40*self_light*smoke_visibility(world)));
}
fn smoke_radiance(world: vec3<f32>, self_light: f32, albedo: vec3<f32>) -> vec3<f32> {
    let atmosphere = smoke_atmosphere(world);
    return smoke_illumination(world,self_light,albedo)*atmosphere.w+atmosphere.rgb;
}

// Billows use 32 x 18; tiny embers and heat shells retain 16 x 9.

struct V {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) age: f32,
    @location(4) shade: f32,
    @location(5) @interpolate(flat) id: u32,
};

// UV sphere with radius 0.5 after scaling, as six
// vertices per quad from the vertex index.
fn sphere(v: u32, segments: u32, rings: u32) -> vec3<f32> {
    let quad = v / 6u;
    let corner = array<vec2<u32>, 6>(vec2<u32>(0u, 0u), vec2<u32>(1u, 0u), vec2<u32>(1u, 1u), vec2<u32>(0u, 0u), vec2<u32>(1u, 1u), vec2<u32>(0u, 1u))[v % 6u];
    let s = quad % segments + corner.x;
    let r = quad / segments + corner.y;
    let theta = f32(s) / f32(segments) * 6.2831853;
    let phi = f32(r) / f32(rings) * 3.14159265;
    return vec3<f32>(cos(theta) * sin(phi), cos(phi), sin(theta) * sin(phi));
}

fn sphere_uv(v: u32, segments: u32, rings: u32) -> vec2<f32> {
    let quad = v / 6u;
    let corner = array<vec2<u32>, 6>(vec2<u32>(0u, 0u), vec2<u32>(1u, 0u), vec2<u32>(1u, 1u), vec2<u32>(0u, 0u), vec2<u32>(1u, 1u), vec2<u32>(0u, 1u))[v % 6u];
    return vec2<f32>(f32(quad % segments + corner.x) / f32(segments), f32(quad / segments + corner.y) / f32(rings));
}

// Godot's TRANS_EXPO / EASE_OUT.
fn expo_out(t: f32) -> f32 { return select(1.0 - pow(2.0, -10.0 * t), 1.0, t >= 1.0); }

// Ground dust packs horizontal travel in extra.rgb. All other billows keep
// their original tint layout; both quality paths share this smooth motion.
fn dust_drift(p:Puff, age:f32)->vec3<f32> {
    if p.center_kind.w==0.0 && p.az.w>=3.0 {return p.extra.xyz*(1.0-pow(1.0-age,4.0));}
    return vec3<f32>(0.0);
}
@vertex
fn fx_vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> V {
    let id = order[ii];
    let p = puffs[id];
    let kind = u32(p.center_kind.w + 0.5);
    let segments = select(16u, 48u, kind <= 1u);
    let rings = select(9u, 24u, kind <= 1u);
    let n = sphere(vi, segments, rings);
    var local = n * 0.5;
    let anim = clamp((u.clock.x - p.timing.x) / max(p.timing.y, 1e-4), 0.0, 1.0);
    var out: V;
    out.id = id;
    out.uv = sphere_uv(vi, segments, rings);
    out.shade = 0.0;
    var world = p.center_kind.xyz + dust_drift(p,anim);
    var normal = n;
    if kind <= 1u {
        // Billow: staggered age, lumps, linear growth, rising pivot.
        let seed = p.ax.w;
        let delay = p.ay.w;
        out.age = clamp((anim - delay) * (1.0 + p.timing.w) / max(1.0 - delay, 0.001), 0.0, 1.0);
        if anim <= delay || out.age >= 1.0 {
            out.world = world;
            out.normal = normal;
            out.position = vec4<f32>(0.0, 0.0, -2.0, 1.0);
            return out;
        }
        let s = seed * 100.0;
        let a = sin(local.x * 6.3 + s) * sin(local.y * 5.1 + s * 1.7) * sin(local.z * 7.2 + s * 0.9);
        let b = sin(local.x * 13.0 + s * 2.1) * sin(local.z * 11.0 + s * 1.3);
        let c = sin(local.x * 21.0 + s * 3.3) * sin(local.y * 18.0 + s * 2.1) * sin(local.z * 25.0 + s * 1.5);
        out.shade = (a * 0.7 + b * 0.3) * 0.55 + c * 0.45;
        local += n * (a * 0.7 + b * 0.3) * select(0.32, 0.14, p.az.w >= 2.0);
        local *= mix(0.35, 1.0, out.age);
        world += p.ax.xyz * local.x + p.ay.xyz * local.y + p.az.xyz * local.z + vec3<f32>(0.0, p.timing.z * anim, 0.0);
        // Normal through the inverse transpose of the (orthogonal) axes.
        normal = normalize(p.ax.xyz * n.x / dot(p.ax.xyz, p.ax.xyz) + p.ay.xyz * n.y / dot(p.ay.xyz, p.ay.xyz) + p.az.xyz * n.z / dot(p.az.xyz, p.az.xyz));
    } else if kind == 2u {
        // Ember: flies from the centre to `travel`, decelerating, stretching
        // into a streak along its path.
        let travel = p.ax.xyz;
        let delay = p.ax.w;
        let age = clamp((anim - delay) / max(1.0 - delay, 0.001), 0.0, 1.0);
        out.age = age;
        if anim <= delay || age >= 1.0 {
            out.world = world;
            out.normal = normal;
            out.position = vec4<f32>(0.0, 0.0, -2.0, 1.0);
            return out;
        }
        let prog = 1.0 - pow(1.0 - age, 2.0);
        let dist = length(travel);
        let dir = select(vec3<f32>(0.0, 1.0, 0.0), travel / max(dist, 1e-4), dist > 1e-4);
        let upv = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(dir.y) < 0.99);
        let ex = normalize(cross(upv, dir));
        let ez = cross(dir, ex);
        let size = p.extra.x;
        let elong = 1.0 + p.extra.y * 7.0;
        let grow = mix(p.extra.z, 1.0, prog);
        // Godot's ember mesh is a 0.06 m sphere.
        let v = n * 0.06;
        world += ex * v.x * size * grow + dir * v.y * size * grow * mix(1.0, elong, prog) + ez * v.z * size * grow + travel * prog;
    } else {
        // Heat shell: expands (expo out) to its end radius.
        let t = clamp((u.clock.x - p.timing.x) / max(p.timing.y, 1e-4), 0.0, 1.0);
        out.age = t;
        if t <= 0.0 || t >= 0.9 {
            out.world = world;
            out.normal = normal;
            out.position = vec4<f32>(0.0, 0.0, -2.0, 1.0);
            return out;
        }
        let r = mix(0.22, p.ax.x, expo_out(t));
        world += n * r;
    }
    out.world = world;
    out.normal = normal;
    out.position = u.view_proj * vec4<f32>(world, 1.0);
    return out;
}

// Open-ended unit cylinder: x/z are radius one, y spans 0..1. Compact
// topology is independent from the sphere billow path above.
fn cylinder_vertex(vi: u32, segments: u32) -> vec3<f32> {
    let corner = array<vec2<u32>, 6>(vec2<u32>(0u, 0u), vec2<u32>(1u, 0u), vec2<u32>(1u, 1u),
        vec2<u32>(0u, 0u), vec2<u32>(1u, 1u), vec2<u32>(0u, 1u))[vi % 6u];
    let theta = f32(vi / 6u + corner.x) / f32(segments) * 6.2831853;
    return vec3<f32>(cos(theta), f32(corner.y), sin(theta));
}

fn capped_cylinder_vertex(vi: u32, segments: u32) -> vec3<f32> {
    if vi < segments * 6u { return cylinder_vertex(vi, segments); }
    let cap_vertex = vi - segments * 6u;
    let y = f32(cap_vertex / (segments * 3u));
    let triangle = (cap_vertex / 3u) % segments;
    let corner = cap_vertex % 3u;
    if corner == 0u { return vec3<f32>(0.0, y, 0.0); }
    let theta = f32(triangle + corner - 1u) / f32(segments) * 6.2831853;
    return vec3<f32>(cos(theta), y, sin(theta));
}

fn additive_vertex(id: u32, world: vec3<f32>, age: f32) -> V {
    var out: V;
    out.id = id;
    out.world = world;
    out.normal = vec3<f32>(0.0, 1.0, 0.0);
    out.uv = vec2<f32>(0.0);
    out.age = age;
    out.shade = 0.0;
    out.position = u.view_proj * vec4<f32>(world, 1.0);
    return out;
}

@vertex
fn fx_column_vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> V {
    let id = order[ii];
    let p = puffs[id];
    let age = clamp((u.clock.x - p.timing.x) / p.timing.y, 0.0, 1.0);
    // Conservative projected cylinder box. Only near/inside volumes use the
    // whole viewport; distant narrow warning beams no longer shade every pixel.
    let radius=max(mix(p.ax.x,p.ax.y,pow(age,p.ax.w)),1e-5)*max(1.0,p.extra.z);
    let centre=p.center_kind.xyz+vec3<f32>(0.0,p.ax.z*0.5,0.0);
    let extent=vec3<f32>(radius,p.ax.z*0.5,radius);
    var lo=vec2<f32>(1.0);var hi=vec2<f32>(-1.0);var crosses=false;var front=false;
    for(var i=0u;i<8u;i++) {
        let sign=vec3<f32>(f32(i&1u),f32((i>>1u)&1u),f32((i>>2u)&1u))*2.0-1.0;
        let c=u.view_proj*vec4<f32>(centre+sign*extent,1.0);
        crosses=crosses||c.w<=u.camera_near.w;front=front||c.w>u.camera_near.w;
        let xy=c.xy/max(c.w,0.0001);lo=min(lo,xy);hi=max(hi,xy);
    }
    if crosses {lo=vec2<f32>(-1.0);hi=vec2<f32>(1.0);}
    if !front {lo=vec2<f32>(0.0);hi=vec2<f32>(0.0);}
    lo=clamp(lo,vec2<f32>(-1.0),vec2<f32>(1.0));hi=clamp(hi,vec2<f32>(-1.0),vec2<f32>(1.0));
    let corner=array<vec2<f32>,6>(vec2<f32>(0.0,0.0),vec2<f32>(1.0,0.0),vec2<f32>(1.0,1.0),vec2<f32>(0.0,0.0),vec2<f32>(1.0,1.0),vec2<f32>(0.0,1.0))[vi];
    let clip=mix(lo,hi,corner);
    let point = u.inverse_view_proj * vec4<f32>(clip, 1.0, 1.0);
    var out = additive_vertex(id, point.xyz / point.w, age);
    out.position = vec4<f32>(clip, 0.0, 1.0);
    return out;
}

@vertex
fn fx_streak_vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> V {
    let id = order[ii];
    let p = puffs[id];
    let age = clamp((u.clock.x - p.timing.x) / p.timing.y, 0.0, 1.0);
    let progress = 1.0 - pow(1.0 - age, p.ax.w);
    let travel = p.ax.xyz;
    let distance = length(travel);
    let direction = select(vec3<f32>(0.0, 1.0, 0.0), travel / max(distance, 1e-6), distance > 1e-6);
    let up = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(direction.y) < 0.99);
    let x = normalize(cross(up, direction));
    let z = cross(direction, x);
    let local = capped_cylinder_vertex(vi, 6u);
    let radius = mix(p.ay.x, p.ay.y, progress);
    let length = mix(p.ay.z, p.ay.w, progress);
    let world = p.center_kind.xyz + travel * progress + (x * local.x + z * local.z) * radius
        + direction * (local.y - 0.5) * length;
    return additive_vertex(id, world, age);
}

@vertex
fn fx_mote_vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> V {
    let id = order[ii];
    let p = puffs[id];
    let age = clamp((u.clock.x - p.timing.x) / p.timing.y, 0.0, 1.0);
    let motion_age = clamp((u.clock.x - p.timing.x - p.extra.x) / max(p.timing.y - p.extra.x, 1e-6), 0.0, 1.0);
    let center = p.center_kind.xyz + p.ax.xyz * pow(motion_age, p.ax.w);
    let q = array<vec2<f32>, 6>(vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0))[vi];
    let right = vec3<f32>(u.view[0].x, u.view[1].x, u.view[2].x);
    let up = vec3<f32>(u.view[0].y, u.view[1].y, u.view[2].y);
    // Give subpixel dots a soft footprint, conserving their energy as it grows.
    let clip = u.view_proj * vec4<f32>(center, 1.0);
    let focal_y = length(vec3<f32>(u.view_proj[0].y, u.view_proj[1].y, u.view_proj[2].y));
    let radius = max(p.ay.x, 1.5 * max(clip.w, 0.0) / (u.size.y * focal_y));
    var out = additive_vertex(id, center + (right * q.x + up * q.y) * radius, age);
    out.uv = q;
    out.shade = pow(p.ay.x / max(radius, 1e-6), 2.0);
    return out;
}

fn depth_metres(uv: vec2<f32>) -> f32 {
    let dims = vec2<i32>(textureDimensions(scene_depth));
    let px = clamp(vec2<i32>(uv * vec2<f32>(dims)), vec2<i32>(0), dims - 1);
    return u.camera_near.w / max(textureLoad(scene_depth, px, 0), 1e-9);
}

// A reduced texel centre can lie exactly on a full-depth texel boundary.
// Reprojecting its interpolated world position adds enough roundoff to select
// the adjacent texel. Share this pixel-based mapping and CPU reciprocals with
// the composite guide so background smoke never receives a foreground guide.
fn soft_depth_uv(pixel: vec2<f32>) -> vec2<f32> {
    let uv = pixel * u.smoke.zw;
    let clip = u.depth_from_output * vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 1.0, 1.0);
    return clip.xy / clip.w * vec2<f32>(0.5, -0.5) + 0.5;
}
fn billow_depth_gap(world: vec3<f32>, pixel: vec2<f32>) -> f32 {
    let clip = u.depth_vp * vec4<f32>(world, 1.0);
    var uv = clip.xy / clip.w * vec2<f32>(0.5, -0.5) + 0.5;
    if u.smoke.y > 1.0 { uv = soft_depth_uv(pixel); }
    return depth_metres(uv) - clip.w;
}

// Metres of free space in front of the opaque scene at this fragment
// (negative: hidden).
fn depth_gap(world: vec3<f32>) -> f32 {
    let clip = u.depth_vp * vec4<f32>(world, 1.0);
    let uv = clip.xy / clip.w * vec2<f32>(0.5, -0.5) + 0.5;
    return depth_metres(uv) - clip.w;
}

// The HDR target is fp16 in absolute luminance: scale down (keeping the
// hue) so the brightest channel stays finite.
fn fp16_safe(c: vec3<f32>) -> vec3<f32> {
    let m = max(c.r, max(c.g, c.b));
    return c * min(1.0, 60000.0 / max(m, 1e-6));
}

// Convert the same gameplay recipes for a minimum-profile colour attachment.
fn fx_output(world: vec3<f32>, source: vec4<f32>) -> vec4<f32> {
    let medium = fx_weather(world);
    // Premultiplied alpha retains in-scattered light in front of the puff.
    // Additive fire/sparks carry zero alpha and only lose transmitted energy.
    let value = vec4<f32>(source.rgb * medium.a + medium.rgb * source.a, source.a);
    if u.clock.z<=0.0 {return value;}
    let alpha=select(1.0,max(value.a,0.0001),value.a>0.0);
    let exposed=max(value.rgb/alpha*u.clock.z,vec3<f32>(0.0));
    var mapped=sqrt(exposed/(1.0+exposed));
    if u.clock.w>0.5 {mapped=select(mapped/12.92,pow((mapped+0.055)/1.055,vec3<f32>(2.4)),mapped>vec3<f32>(0.04045));}
    return vec4<f32>(mapped*alpha,value.a);
}

@fragment
fn fx_column(in: V) -> @location(0) vec4<f32> {
    let p = puffs[in.id];
    let ray = normalize(in.world - u.camera_near.xyz);
    let origin = u.camera_near.xyz - p.center_kind.xyz;
    let radius = max(mix(p.ax.x, p.ax.y, pow(in.age, p.ax.w)), 1e-5);
    let bound = radius * max(1.0, p.extra.z);
    // Intersect the radial cylinder and finite height independently. Avoid
    // divisions by zero for axial rays and horizontal rays on either side.
    let a = dot(ray.xz, ray.xz);
    let b = dot(origin.xz, ray.xz);
    let c = dot(origin.xz, origin.xz) - bound * bound;
    var entry = 0.0;
    var exit = 1e9;
    if a > 1e-10 {
        let disc = b * b - a * c;
        if disc <= 0.0 { discard; }
        let root = sqrt(disc);
        entry = max(entry, (-b - root) / a);
        exit = min(exit, (-b + root) / a);
    } else if c >= 0.0 { discard; }
    if abs(ray.y) > 1e-7 {
        let bottom = -origin.y / ray.y;
        let top = (p.ax.z - origin.y) / ray.y;
        entry = max(entry, min(bottom, top));
        exit = min(exit, max(bottom, top));
    } else if origin.y < 0.0 || origin.y > p.ax.z { discard; }
    // Sample depth using its own (possibly jittered) projection. Convert
    // camera-forward depth to distance along this pixel's sightline.
    let probe = u.depth_vp * vec4<f32>(u.camera_near.xyz + ray, 1.0);
    let uv = probe.xy / probe.w * vec2<f32>(0.5, -0.5) + 0.5;
    let forward = max(-(u.view * vec4<f32>(ray, 0.0)).z, 1e-6);
    exit = min(exit, depth_metres(uv) / forward);
    if exit <= entry { discard; }
    let distance = exit - entry;
    let middle = origin + ray * (entry + distance * 0.5);
    var integral = 0.0;
    if abs(p.extra.z - 1.0) < 1e-5 {
        // Exact integral of 1 - r²/R². Density goes smoothly to zero at
        // the rim; no polygon faces, near-plane holes or double surfaces.
        integral = max(0.0, distance * (1.0 - (dot(middle.xz, middle.xz) + a * distance * distance / 12.0) / (radius * radius)));
    } else {
        for (var k = 0u; k < 8u; k++) {
            let point = origin + ray * (entry + distance * (f32(k) + 0.5) / 8.0);
            let local_radius = radius * mix(p.extra.z, 1.0, clamp(point.y / p.ax.z, 0.0, 1.0));
            integral += max(0.0, 1.0 - dot(point.xz, point.xz) / (local_radius * local_radius)) * distance / 8.0;
        }
    }
    let optical_depth = min(integral / radius, 32.0);
    let soft = 3.8 * (1.0 - exp(-optical_depth));
    let alpha = mix(p.ay.w, p.az.w, pow(in.age, p.body.z));
    if alpha <= 0.0 { discard; }
    let power = pow(in.age, p.body.w);
    let radiance = max(0.0, mix(p.body.x, p.body.y, power)
        + sin((u.clock.x - p.timing.x) * p.extra.x) * p.extra.y * power);
    let color = mix(p.ay.rgb, p.az.rgb, smoothstep(p.timing.z, p.timing.w, in.age));
    // Apply opacity before limiting HDR: a low-alpha high-radiance beam must
    // not lose energy merely because its unweighted radiance exceeds fp16.
    return fx_output(u.camera_near.xyz + ray * (entry + distance * 0.5), vec4<f32>(fp16_safe(color * (radiance * alpha * soft)), 0.0));
}

@fragment
fn fx_streak(in: V) -> @location(0) vec4<f32> {
    let p = puffs[in.id];
    if depth_gap(in.world) <= 0.0 { discard; }
    var fade = 1.0;
    if p.body.y > 0.0 { fade *= smoothstep(0.0, p.body.y, in.age); }
    if p.body.z > 0.0 { fade *= 1.0 - smoothstep(1.0 - p.body.z, 1.0, in.age); }
    return fx_output(in.world, vec4<f32>(fp16_safe(p.az.rgb * (p.az.w * p.body.x * fade)), 0.0));
}

@fragment
fn fx_mote(in: V) -> @location(0) vec4<f32> {
    let p = puffs[in.id];
    let r2 = dot(in.uv, in.uv);
    if r2 >= 1.0 { discard; }
    let gap = depth_gap(in.world);
    if gap <= 0.0 { discard; }
    var fade = 1.0;
    if p.body.y > 0.0 { fade *= smoothstep(0.0, p.body.y, in.age); }
    if p.body.z > 0.0 { fade *= 1.0 - smoothstep(1.0 - p.body.z, 1.0, in.age); }
    let dot = exp(-4.0 * r2) * (1.0 - smoothstep(0.6, 1.0, r2));
    let soft = smoothstep(0.0, max(p.ay.x * 2.0, 0.01), gap);
    return fx_output(in.world, vec4<f32>(fp16_safe(p.az.rgb * (p.az.w * p.body.x * fade * dot * soft * in.shade)), 0.0));
}

@fragment
fn fx_billow(in: V) -> @location(0) vec4<f32> {
    let p = puffs[in.id];

    let gap = billow_depth_gap(in.world, in.position.xy);
    if gap <= 0.0 { discard; }

    let view = normalize(u.camera_near.xyz - in.world);
    let continuous = p.az.w >= 2.0;
    let signed_facing = dot(normalize(in.normal), view);
    // A single visible shell avoids unsorted back/front triangles printing
    // hard seams through the otherwise translucent billow.
    if continuous && signed_facing <= 0.0 { discard; }
    let facing = abs(signed_facing);
    let fres = pow(1.0 - facing, 2.1);
    let age = in.age;
    let fade_in = smoothstep(0.0, select(0.12,0.025,p.center_kind.w==0.0 && p.az.w>=3.0), age);
    let t = clamp((age - 0.12) / 0.88, 0.0, 1.0);
    // Long tail with a soft landing.
    let fade_out = pow(1.0 - t, p.extra.w) * (1.0 - smoothstep(0.5, 1.0, t));
    var alpha = p.body.a * fade_in * fade_out * smoothstep(0.0, 0.85, facing);
    if continuous {
        let q = (in.world-p.center_kind.xyz-dust_drift(p,age)) / max(length(p.ax.xyz), 0.01) * 16.0;
        let density = 0.75 + 0.25 * sin(q.x+q.y*0.7+p.ax.w*30.0) * sin(q.z-q.y);
        alpha *= facing * density;
    }
    // Soft particles: dissolve where the puff meets solid geometry.
    alpha *= clamp(gap / select(1.6,0.35,p.center_kind.w==0.0 && p.az.w>=3.0), 0.0, 1.0);
    let cell = floor(in.uv * vec2<f32>(8.0, 5.0));
    let band = select((cell.x + cell.y) % 3.0 - 1.0, in.shade, continuous);
    var color: vec3<f32>;
    if u32(p.center_kind.w + 0.5) == 1u {
        let heat = clamp(p.az.w - select(0.0, 2.0, continuous) + band * 0.06, 0.0, 1.0);
        let cool = 1.0 - smoothstep(0.0, 0.75, age);
        let fire = clamp(heat * 0.6 + facing * 0.25 + cool * 0.15, 0.0, 1.0);
        var c = vec3<f32>(0.03, 0.025, 0.02);
        c = mix(c, vec3<f32>(0.30, 0.09, 0.025), smoothstep(0.05, 0.34, fire));
        c = mix(c, vec3<f32>(1.00, 0.24, 0.04), smoothstep(0.30, 0.54, fire));
        c = mix(c, vec3<f32>(1.00, 0.62, 0.12), smoothstep(0.52, 0.74, fire));
        c = mix(c, vec3<f32>(1.00, 0.95, 0.72), smoothstep(0.74, 0.94, fire));
        color = c * mix(0.45, 16.0, fire);
        // Godot's tonemapper turns a 16x-white orange into cream; AgX keeps
        // more hue, so the hot end is pulled toward cream here.
        color = mix(color, vec3<f32>(1.0, 0.93, 0.78) * mix(0.45, 16.0, fire), smoothstep(0.45, 0.95, fire) * 0.7);
    } else {
        // Smoke: patchy darkness from the lump noise, darker rims, 16 levels.
        var shade = 0.45 + 0.85 * (in.shade * 0.5 + 0.5);
        shade = clamp(shade + band * 0.05, 0.0, 1.0);
        var grey = dot(p.body.rgb, vec3<f32>(0.3333)) * shade * 0.42;
        grey = mix(grey, grey * 0.22, fres);
        if !continuous { grey = floor(grey * 16.0 + 0.5) / 16.0; }
        // Godot's arena haze lifts its unshaded smoke to a mid grey that
        // darkens the background by about a quarter; match that.
        color = vec3<f32>(select(grey * 3.0 + 0.12, grey, continuous)) * select(p.extra.rgb,vec3<f32>(1.0),p.center_kind.w==0.0 && p.az.w>=3.0);
        alpha *= 0.75;
    }
    if alpha < 0.0005 { discard; }
    if u32(p.center_kind.w) == 0u {
        return fx_output(in.world, vec4<f32>(fp16_safe(smoke_radiance(in.world,clamp(in.shade*0.5+0.5,0.0,1.0),color))*alpha,alpha));
    }
    // Fire and sparks retain their emission.
    return fx_output(in.world, vec4<f32>(fp16_safe(color * u.clock.y) * alpha, alpha));
}

@fragment
fn fx_ember(in: V) -> @location(0) vec4<f32> {
    let p = puffs[in.id];
    if depth_gap(in.world) <= 0.0 { discard; }
    let b = (1.0 - smoothstep(0.35, 1.0, in.age)) * p.body.a;
    if b < 0.0005 { discard; }
    // Additive, HDR bright so it blooms.
    return fx_output(in.world, vec4<f32>(fp16_safe(p.body.rgb * p.extra.w * u.clock.y) * b, 0.0));
}

@fragment
fn fx_heat(in: V) -> @location(0) vec4<f32> {
    let p = puffs[in.id];
    if depth_gap(in.world) <= 0.0 { discard; }
    let falloff = 1.0 - expo_out(clamp(in.age / 0.9, 0.0, 1.0));
    let normal = normalize(in.world - p.center_kind.xyz);
    let view = normalize(u.camera_near.xyz - in.world);
    // Godot multiplies the (already view-space) NORMAL by VIEW_MATRIX once
    // more; kept for the same look.
    let nv = (u.view * vec4<f32>(normal, 0.0)).xyz;
    let n = normalize((u.view * vec4<f32>(nv, 0.0)).xyz);
    let fresnel = pow(1.0 - abs(dot(view, normal)), 1.1);
    let weight = 0.38 + 0.62 * fresnel;
    let strength = p.extra.x;
    let screen = in.position.xy * u.size.zw;
    // Godot's shell (strength x 5.2 UV) reads as a faint shimmer there;
    // here the same numbers draw a glass ball, so the offsets are scaled to
    // the look of the reference capture.
    let offset = n.xy * strength * 0.12 * weight * falloff;
    let zoom = (screen - vec2<f32>(0.5)) * strength * 0.05 * weight * falloff;
    let uv = clamp(screen - zoom + offset, u.size.zw * 0.5, vec2<f32>(1.0) - u.size.zw * 0.5);
    let col = textureSampleLevel(background, samp, uv, 0.0).rgb;
    let alpha = weight * falloff * fx_weather(in.world).a;
    if alpha < 0.002 { discard; }
    return vec4<f32>(col * alpha, alpha);
}

// One axis-aligned texture cube transformed by three orthogonal edge vectors.
// The birth point is the bottom centre, so the plume rises above an impact.
fn smoke_world(p: Puff, uvw: vec3<f32>) -> vec3<f32> {
    return p.center_kind.xyz + p.ax.xyz * (uvw.x - 0.5) + p.ay.xyz * uvw.y + p.az.xyz * (uvw.z - 0.5);
}
fn smoke_local_vector(p: Puff, v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(v, p.ax.xyz) / dot(p.ax.xyz, p.ax.xyz),
        dot(v, p.ay.xyz) / dot(p.ay.xyz, p.ay.xyz),
        dot(v, p.az.xyz) / dot(p.az.xyz, p.az.xyz));
}

struct SmokeBounds { low: vec3<f32>, high: vec3<f32>, peak: f32 };
fn smoke_bounds(frame: f32) -> SmokeBounds {
    let a = u32(clamp(floor(frame), 0.0, 31.0));
    let b = min(a + 1u, 31u);
    let lo_a = SMOKE_BOUNDS_MIN[a].xyz;
    let hi_a = SMOKE_BOUNDS_MAX[a];
    let lo_b = SMOKE_BOUNDS_MIN[b].xyz;
    let hi_b = SMOKE_BOUNDS_MAX[b];
    if hi_a.w == 0.0 { return SmokeBounds(lo_b, hi_b.xyz, hi_b.w); }
    if hi_b.w == 0.0 { return SmokeBounds(lo_a, hi_a.xyz, hi_a.w); }
    return SmokeBounds(min(lo_a, lo_b), max(hi_a.xyz, hi_b.xyz), max(hi_a.w, hi_b.w));
}
fn smoke_retirement(p: Puff) -> f32 {
    if p.extra.y <= 0.0 { return 1.0; }
    return 1.0 - smoothstep(p.extra.x, p.extra.x + p.extra.y, u.clock.x);
}

@vertex
fn fx_smoke_vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> V {
    let id = order[ii];
    let p = puffs[id];
    let age = clamp((u.clock.x - p.timing.x) / max(p.timing.y, 0.001), 0.0, 1.0);
    let bounds = smoke_bounds(smoke_bake_time(age) / 2.6 * 31.0);
    var lo = vec2<f32>(1.0);
    var hi = vec2<f32>(-1.0);
    var front = false;
    var clipped = false;
    for (var corner = 0u; corner < 8u; corner++) {
        let uvw = vec3<f32>(f32(corner & 1u), f32((corner >> 1u) & 1u), f32((corner >> 2u) & 1u));
        let clip = u.view_proj * vec4<f32>(smoke_world(p, mix(bounds.low, bounds.high, uvw)), 1.0);
        if clip.w > u.camera_near.w {
            lo = min(lo, clip.xy / clip.w);
            hi = max(hi, clip.xy / clip.w);
            front = true;
        } else { clipped = true; }
    }
    // Crossing the near plane includes cameras inside the smoke. The slab
    // intersection in the fragment stage still rejects every missed ray.
    if front && clipped { lo = vec2<f32>(-1.0); hi = vec2<f32>(1.0); }
    lo = max(lo, vec2<f32>(-1.0));
    hi = min(hi, vec2<f32>(1.0));
    if !front || any(lo >= hi) || age <= 0.0 || age >= 1.0 || bounds.peak == 0.0 || smoke_retirement(p) <= 0.0 {
        lo = vec2<f32>(-2.0);
        hi = lo;
    }
    let corner = array<vec2<f32>, 6>(vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0))[vi];
    let clip = mix(lo, hi, corner);
    let point = u.inverse_view_proj * vec4<f32>(clip, 1.0, 1.0);
    var out = additive_vertex(id, point.xyz / point.w, age);
    out.position = vec4<f32>(clip, 0.0, 1.0);
    return out;
}

// 32 independent 48-cubed volumes share one Z-stacked texture. Explicit
// half-texel bounds prevent the trilinear filter crossing into another frame.
// Time interpolation is independent of the raster frame rate and view count.
fn smoke_frame(uvw: vec3<f32>, frame: f32) -> vec2<f32> {
    let voxel = clamp(uvw, vec3<f32>(0.0), vec3<f32>(1.0)) * 47.0 + 0.5;
    let coord = vec3<f32>(voxel.xy / 48.0, (voxel.z + frame * 48.0) / 1536.0);
    return textureSampleLevel(smoke_animation, samp, coord, 0.0).rg;
}
fn smoke_sample(uvw: vec3<f32>, frame: f32) -> vec2<f32> {
    let a = floor(frame);
    let b = min(a + 1.0, 31.0);
    return mix(smoke_frame(uvw, a), smoke_frame(uvw, b), frame - a);
}

// Preserve the first 1.2 seconds of the blast. The later motion eases down
// continuously, playing the 2.6-second bake over a 4.6-second lifetime.
// This monotone Hermite curve has speed 1 at the join and 0 at the endpoint.
fn smoke_bake_time(age: f32) -> f32 {
    let elapsed = age * 4.6;
    if elapsed <= 1.2 { return elapsed; }
    let t = clamp((elapsed - 1.2) / 3.4, 0.0, 1.0);
    return min(2.6, 1.2 + ((0.6 * t - 2.6) * t + 3.4) * t);
}

@fragment
fn fx_smoke(in: V) -> @location(0) vec4<f32> {
    let p = puffs[in.id];
    let ray = normalize(in.world - u.camera_near.xyz);
    let origin = smoke_local_vector(p, u.camera_near.xyz - p.center_kind.xyz) + vec3<f32>(0.5, 0.0, 0.5);
    let direction = smoke_local_vector(p, ray);
    let bake_time = smoke_bake_time(in.age);
    let frame = bake_time / 2.6 * 31.0;
    let bounds = smoke_bounds(frame);
    let retirement = smoke_retirement(p);
    if bounds.peak == 0.0 || retirement <= 0.0 { discard; }
    var full_entry = 0.0;
    var full_exit = 1e9;
    var entry = 0.0;
    var exit = 1e9;
    for (var axis = 0u; axis < 3u; axis++) {
        if abs(direction[axis]) < 1e-8 {
            if origin[axis] < bounds.low[axis] || origin[axis] > bounds.high[axis] { discard; }
        } else {
            let a = -origin[axis] / direction[axis];
            let b = (1.0 - origin[axis]) / direction[axis];
            full_entry = max(full_entry, min(a, b));
            full_exit = min(full_exit, max(a, b));
            let c = (bounds.low[axis] - origin[axis]) / direction[axis];
            let d = (bounds.high[axis] - origin[axis]) / direction[axis];
            entry = max(entry, min(c, d));
            exit = min(exit, max(c, d));
        }
    }
    let probe = u.depth_vp * vec4<f32>(u.camera_near.xyz + ray, 1.0);
    var uv = probe.xy / probe.w * vec2<f32>(0.5, -0.5) + 0.5;
    if u.smoke.y > 1.0 { uv = soft_depth_uv(in.position.xy); }
    let forward = max(-(u.view * vec4<f32>(ray, 0.0)).z, 1e-6);
    let scene_exit = depth_metres(uv) / forward;
    full_exit = min(full_exit, scene_exit);
    exit = min(exit, scene_exit);
    if exit <= entry { discard; }

    let samples = select(u32(clamp(u.smoke.x, 8.0, 48.0)), 24u, u.clock.z > 0.0);
    // Retain the original cube-aligned quadrature positions. Tight supports
    // skip empty samples without introducing frame-boundary sampling jumps.
    let step = (full_exit - full_entry) / f32(samples);
    let first = u32(clamp(ceil((entry - full_entry) / step - 0.5), 0.0, f32(samples)));
    let end = u32(clamp(floor((exit - full_entry) / step - 0.5) + 1.0, 0.0, f32(samples)));
    let optical_step = step * length(direction) * p.body.a;
    // The source bake applies this density envelope. Scale the shape trim with
    // it so thinning wisps are not erased by a fixed threshold near the end.
    let density_trim = 0.025 * (1.0 - smoothstep(1.7, 2.6, bake_time));
    var transmission = 1.0;
    var color = vec3<f32>(0.0);
    var weighted_dist = 0.0;
    // One world-shadow query per plume pixel, rather than per volume step.
    let world = smoke_world(p, vec3<f32>(0.5));
    let atmosphere = smoke_atmosphere(world);
    let unattenuated_ambient = p.body.rgb*u.ambient.rgb*0.35;
    let ambient_light = unattenuated_ambient*atmosphere.w+atmosphere.rgb;
    let direct_light = (smoke_illumination(world,1.0,p.body.rgb)-unattenuated_ambient)*atmosphere.w;
    for (var k = first; k < end; k++) {
        let point = origin + direction * (full_entry + (f32(k) + 0.5) * step);
        let field = smoke_sample(point, frame);
        let density = max(0.0, (field.r - density_trim) / 0.975) * retirement;
        let alpha = 1.0 - exp(-density * optical_step);
        // The G channel is baked light transmittance through this same frame.
        // It provides self-shadowed form; it does not sample world shadows.
        let radiance = ambient_light + direct_light * pow(field.g, 1.3 * retirement);
        weighted_dist += transmission * alpha * (full_entry + (f32(k) + 0.5) * step);
        color += transmission * alpha * radiance;
        transmission *= 1.0 - alpha;
        if transmission < 0.015 { break; }
    }
    if transmission > 0.9995 { discard; }
    return fx_output(u.camera_near.xyz + ray * (weighted_dist / (1.0 - transmission)), vec4<f32>(fp16_safe(color), 1.0 - transmission));
}

// Minimum billows use a bounded projected sphere and an analytic soft density.
// Six vertices replace the lumpy 864-vertex sphere; near/inside views still work.
fn compact_billow(p:Puff)->vec4<f32> {
 let anim=clamp((u.clock.x-p.timing.x)/max(p.timing.y,0.0001),0.0,1.0);
 let age=clamp((anim-p.ay.w)*(1.0+p.timing.w)/max(1.0-p.ay.w,0.001),0.0,1.0);
 let centre=p.center_kind.xyz+dust_drift(p,anim)+vec3<f32>(0.0,p.timing.z*anim,0.0);
 let radius=max(length(p.ax.xyz),max(length(p.ay.xyz),length(p.az.xyz)))*0.5*mix(0.35,1.0,age);
 return vec4<f32>(centre,select(radius,0.0,anim<=p.ay.w||age>=1.0));
}
@vertex fn fx_min_billow_vs(@builtin(vertex_index) vi:u32,@builtin(instance_index) ii:u32)->V {
 let id=order[ii];let p=puffs[id];let ball=compact_billow(p);
 let age=clamp((u.clock.x-p.timing.x)/max(p.timing.y,0.0001),0.0,1.0);
 var lo=vec2<f32>(1.0);var hi=vec2<f32>(-1.0);var crosses=false;var front=false;
 for(var i=0u;i<8u;i++) {
  let sign=vec3<f32>(f32(i&1u),f32((i>>1u)&1u),f32((i>>2u)&1u))*2.0-1.0;
  let clip=u.view_proj*vec4<f32>(ball.xyz+sign*ball.w,1.0);
  if clip.w<=u.camera_near.w {crosses=true;}else{front=true;}
  let xy=clip.xy/max(clip.w,0.0001);lo=min(lo,xy);hi=max(hi,xy);
 }
 if crosses {lo=vec2<f32>(-1.0);hi=vec2<f32>(1.0);}
 if !front {lo=vec2<f32>(0.0);hi=vec2<f32>(0.0);}
 lo=clamp(lo,vec2<f32>(-1.0),vec2<f32>(1.0));hi=clamp(hi,vec2<f32>(-1.0),vec2<f32>(1.0));
 let uv=array<vec2<f32>,6>(vec2<f32>(0.0,0.0),vec2<f32>(1.0,0.0),vec2<f32>(1.0,1.0),vec2<f32>(0.0,0.0),vec2<f32>(1.0,1.0),vec2<f32>(0.0,1.0))[vi];
 let xy=mix(lo,hi,uv);let point=u.inverse_view_proj*vec4<f32>(xy,1.0,1.0);
 var out=additive_vertex(id,point.xyz/point.w,age);out.position=vec4<f32>(xy,0.0,1.0);return out;
}
@fragment fn fx_min_billow(in:V)->@location(0) vec4<f32> {
 let p=puffs[in.id];let ball=compact_billow(p);if ball.w<=0.001 {discard;}
 let ray=normalize(in.world-u.camera_near.xyz);let offset=u.camera_near.xyz-ball.xyz;
 let b=dot(offset,ray);let d=b*b-dot(offset,offset)+ball.w*ball.w;if d<=0.0 {discard;}
 let root=sqrt(d);let entry=max(-b-root,u.camera_near.w);
 let screen=select(in.position.xy*u.size.zw,soft_depth_uv(in.position.xy),u.smoke.y>1.0);let forward=max(-(u.view*vec4<f32>(ray,0.0)).z,0.0001);
 let exit=min(-b+root,depth_metres(screen)/forward);if exit<=entry {discard;}
 if p.center_kind.w==0.0 && p.az.w>=3.0 {
  // Tiny dust volumes: six proxy vertices and six cheap density samples.
  // Same atmosphere and lighting as smoke, without thousands of shell vertices.
  let age=in.age;let step=(exit-entry)/6.0;var optical=0.0;
  for(var i=0u;i<6u;i++) {
   let point=u.camera_near.xyz+ray*(entry+(f32(i)+0.5)*step);
   let q=(point-ball.xyz)/ball.w;
   let shape=1.0-smoothstep(0.15,1.0,length(q*vec3<f32>(1.0,1.8,1.0)));
   let seed=p.ax.w*61.0;
   let noise=0.65+0.35*sin(q.x*8.0+seed+age)*sin(q.z*7.0-q.y*9.0+seed);
   optical+=shape*noise*step/ball.w;
  }
  let fade=smoothstep(0.0,0.025,age)*(1.0-smoothstep(0.35,1.0,age));
  let alpha=(1.0-exp(-optical*4.0))*fade;
  let color=smoke_radiance(ball.xyz,0.55,p.body.rgb);
  return fx_output(u.camera_near.xyz + ray * ((entry + exit) * 0.5), vec4<f32>(fp16_safe(color)*alpha,alpha));
 }
 let density=clamp((exit-entry)/ball.w,0.0,1.0);
 let age=clamp((in.age-p.ay.w)*(1.0+p.timing.w)/max(1.0-p.ay.w,0.001),0.0,1.0);
 let fade=smoothstep(0.0,select(0.12,0.025,p.center_kind.w==0.0 && p.az.w>=3.0),age)*(1.0-age)*(1.0-age);
 let alpha=p.body.a*fade*density;
 var color=p.body.rgb*select(p.extra.rgb,vec3<f32>(1.0),p.center_kind.w==0.0 && p.az.w>=3.0)*0.6;
 if p.center_kind.w>0.5 {
  color=mix(vec3<f32>(3.0,0.12,0.005),vec3<f32>(9.0,2.0,0.12),density*(1.0-age));
  return fx_output(u.camera_near.xyz + ray * ((entry + exit) * 0.5), vec4<f32>(color*u.clock.y*fade*density,0.0));
 }
 return fx_output(u.camera_near.xyz + ray * ((entry + exit) * 0.5), vec4<f32>(color*u.clock.y*alpha,alpha));
}
