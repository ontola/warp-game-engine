// Blast effects after temporal reconstruction, ported from Growing Guns'
// violence.gd: billowing fire and smoke clouds (lumpy spheres with fresnel
// rims, a banded fire palette and soft depth fades), stretched flame tongues,
// additive ember streaks and a refracting heat shell. Every puff animates on
// the GPU from one clock; the CPU uploads births only when they change.
struct FxUniform {
    view_proj: mat4x4<f32>,
    depth_vp: mat4x4<f32>,
    view: mat4x4<f32>,
    // camera position, near plane
    camera_near: vec4<f32>,
    // width, height, 1/width, 1/height
    size: vec4<f32>,
    // time, white luminance (nits), unused, unused
    clock: vec4<f32>,
};
struct Puff {
    // centre, kind (0 smoke, 1 fire, 2 ember, 3 heat)
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

const SEGMENTS: u32 = 16u;
const RINGS: u32 = 9u;

struct V {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) age: f32,
    @location(4) shade: f32,
    @location(5) @interpolate(flat) id: u32,
};

// Unit-diameter UV sphere (Godot SphereMesh radius 0.5, 16 x 9), as six
// vertices per quad from the vertex index.
fn sphere(v: u32) -> vec3<f32> {
    let quad = v / 6u;
    let corner = array<vec2<u32>, 6>(vec2<u32>(0u, 0u), vec2<u32>(1u, 0u), vec2<u32>(1u, 1u), vec2<u32>(0u, 0u), vec2<u32>(1u, 1u), vec2<u32>(0u, 1u))[v % 6u];
    let s = quad % SEGMENTS + corner.x;
    let r = quad / SEGMENTS + corner.y;
    let theta = f32(s) / f32(SEGMENTS) * 6.2831853;
    let phi = f32(r) / f32(RINGS) * 3.14159265;
    return vec3<f32>(cos(theta) * sin(phi), cos(phi), sin(theta) * sin(phi));
}

fn sphere_uv(v: u32) -> vec2<f32> {
    let quad = v / 6u;
    let corner = array<vec2<u32>, 6>(vec2<u32>(0u, 0u), vec2<u32>(1u, 0u), vec2<u32>(1u, 1u), vec2<u32>(0u, 0u), vec2<u32>(1u, 1u), vec2<u32>(0u, 1u))[v % 6u];
    return vec2<f32>(f32(quad % SEGMENTS + corner.x) / f32(SEGMENTS), f32(quad / SEGMENTS + corner.y) / f32(RINGS));
}

// Godot's TRANS_EXPO / EASE_OUT.
fn expo_out(t: f32) -> f32 { return select(1.0 - pow(2.0, -10.0 * t), 1.0, t >= 1.0); }

@vertex
fn fx_vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> V {
    let id = order[ii];
    let p = puffs[id];
    let kind = u32(p.center_kind.w + 0.5);
    let n = sphere(vi);
    var local = n * 0.5;
    let anim = clamp((u.clock.x - p.timing.x) / max(p.timing.y, 1e-4), 0.0, 1.0);
    var out: V;
    out.id = id;
    out.uv = sphere_uv(vi);
    out.shade = 0.0;
    var world = p.center_kind.xyz;
    var normal = n;
    if kind <= 1u {
        // Billow: staggered age, lumps, linear growth, rising pivot.
        let seed = p.ax.w;
        let delay = p.ay.w;
        out.age = clamp((anim - delay) * (1.0 + p.timing.w) / max(1.0 - delay, 0.001), 0.0, 1.0);
        let s = seed * 100.0;
        let a = sin(local.x * 6.3 + s) * sin(local.y * 5.1 + s * 1.7) * sin(local.z * 7.2 + s * 0.9);
        let b = sin(local.x * 13.0 + s * 2.1) * sin(local.z * 11.0 + s * 1.3);
        let c = sin(local.x * 21.0 + s * 3.3) * sin(local.y * 18.0 + s * 2.1) * sin(local.z * 25.0 + s * 1.5);
        out.shade = (a * 0.7 + b * 0.3) * 0.55 + c * 0.45;
        local += n * (a * 0.7 + b * 0.3) * 0.32;
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
        let r = mix(0.22, p.ax.x, expo_out(t));
        world += n * r;
    }
    out.world = world;
    out.normal = normal;
    out.position = u.view_proj * vec4<f32>(world, 1.0);
    return out;
}

fn depth_metres(uv: vec2<f32>) -> f32 {
    let dims = vec2<i32>(textureDimensions(scene_depth));
    let px = clamp(vec2<i32>(uv * vec2<f32>(dims)), vec2<i32>(0), dims - 1);
    return u.camera_near.w / max(textureLoad(scene_depth, px, 0), 1e-9);
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

@fragment
fn fx_billow(in: V) -> @location(0) vec4<f32> {
    let p = puffs[in.id];
    let gap = depth_gap(in.world);
    if gap <= 0.0 { discard; }
    let view = normalize(u.camera_near.xyz - in.world);
    let facing = abs(dot(normalize(in.normal), view));
    let fres = pow(1.0 - facing, 2.1);
    let age = in.age;
    let fade_in = smoothstep(0.0, 0.12, age);
    let t = clamp((age - 0.12) / 0.88, 0.0, 1.0);
    // Long tail with a soft landing.
    let fade_out = pow(1.0 - t, p.extra.w) * (1.0 - smoothstep(0.5, 1.0, t));
    var alpha = p.body.a * fade_in * fade_out * smoothstep(0.0, 0.85, facing);
    // Soft particles: dissolve where the puff meets solid geometry.
    alpha *= clamp(gap / 1.6, 0.0, 1.0);
    let cell = floor(in.uv * vec2<f32>(8.0, 5.0));
    let band = (cell.x + cell.y) % 3.0 - 1.0;
    var color: vec3<f32>;
    if u32(p.center_kind.w + 0.5) == 1u {
        let heat = clamp(p.az.w + band * 0.06, 0.0, 1.0);
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
        grey = floor(grey * 16.0 + 0.5) / 16.0;
        // Godot's arena haze lifts its unshaded smoke to a mid grey that
        // darkens the background by about a quarter; match that.
        color = vec3<f32>(grey * 3.0 + 0.12) * p.extra.rgb;
        alpha *= 0.75;
    }
    if alpha < 0.0005 { discard; }
    // Unlit: Godot's 0..1 albedo scale is white under the current light.
    return vec4<f32>(fp16_safe(color * u.clock.y) * alpha, alpha);
}

@fragment
fn fx_ember(in: V) -> @location(0) vec4<f32> {
    let p = puffs[in.id];
    if depth_gap(in.world) <= 0.0 { discard; }
    let b = (1.0 - smoothstep(0.35, 1.0, in.age)) * p.body.a;
    if b < 0.0005 { discard; }
    // Additive, HDR bright so it blooms.
    return vec4<f32>(fp16_safe(p.body.rgb * p.extra.w * u.clock.y) * b, 0.0);
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
    let alpha = weight * falloff;
    if alpha < 0.002 { discard; }
    return vec4<f32>(col * alpha, alpha);
}
