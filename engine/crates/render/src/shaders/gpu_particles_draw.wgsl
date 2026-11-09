// GPU particles: one camera-facing quad per live particle (gpu_particles.rs).
// Two storage buffers: the particles and this frame's live list.
struct View {
    output_vp: mat4x4<f32>,
    depth_vp: mat4x4<f32>,
    // camera position, near plane
    camera: vec4<f32>,
    // camera right, seconds since the last simulated step
    right: vec4<f32>,
    // camera up, smallest size per metre of distance
    up: vec4<f32>,
    // direction to the sun
    sun_dir: vec4<f32>,
    // sun radiance (colour * lux)
    sun: vec4<f32>,
    // sky irradiance
    ambient: vec4<f32>,
    // output size, 1 / size
    size: vec4<f32>,
};
struct Particle { pos_age: vec4<f32>, vel_life: vec4<f32>, look: vec4<u32>, phys: vec4<u32> };

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<storage, read> particles: array<Particle>;
@group(0) @binding(2) var<storage, read> alive: array<u32>;
@group(0) @binding(3) var scene_depth: texture_depth_2d;

struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) world: vec3<f32>,
    @location(3) @interpolate(flat) soft: f32,
    @location(4) @interpolate(flat) shape: u32,
    // Trail: the head's share of the quad's half length.
    @location(5) @interpolate(flat) cap: f32,
};

const PI: f32 = 3.14159265;

@vertex
fn particle_vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) instance: u32) -> Out {
    var out: Out;
    out.position = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    out.uv = vec2<f32>(0.0);
    out.color = vec4<f32>(0.0);
    out.world = vec3<f32>(0.0);
    out.soft = 1.0;
    out.shape = 0u;
    out.cap = 1.0;
    let p = particles[alive[instance]];
    let frac = view.right.w;
    let life = p.vel_life.w;
    let age = p.pos_age.w + frac;
    if life <= 0.0 || age < 0.0 || age >= life { return out; }
    let t = clamp(age / life, 0.0, 1.0);
    let vel = p.vel_life.xyz;
    let center = p.pos_age.xyz + vel * frac;
    let to = center - view.camera.xyz;
    let dist = length(to);
    if dist < view.camera.w * 2.0 { return out; }
    let eye = to / dist;
    let sizes = unpack2x16float(p.look.z);
    let sa = unpack2x16float(p.phys.z);
    let shape = p.phys.w >> 30u;
    let seed = f32(p.phys.w & 0xffffu) / 65535.0;
    // Never thinner than a pixel or so: distant sparks shimmer instead of vanishing.
    let raw = mix(sizes.x, sizes.y, t);
    var size = max(raw, dist * view.up.w);
    // A widened particle keeps about its light instead of growing brighter.
    let thin = clamp(raw / size, 0.0, 1.0);
    var side = view.right.xyz;
    var axis = view.up.xyz;
    var half_len = size * 0.5;
    if sa.x > 0.0 {
        let across = vel - dot(vel, eye) * eye;
        let l = length(across);
        if l > 1e-3 {
            axis = across / l;
            side = normalize(cross(axis, eye));
            half_len = size * 0.5 * (1.0 + sa.x * length(vel));
        }
    }
    var center_q = center;
    var cap = 1.0;
    if shape == 3u {
        // Trail: from where it was `stretch` seconds ago (in a straight line)
        // to where it is, a round head at the front.
        let across = vel - dot(vel, eye) * eye;
        // At most twelve widths: a fast chunk is a streak, not a line across the sky.
        let l = min(length(across) * sa.x, size * 12.0);
        side = view.right.xyz;
        axis = view.up.xyz;
        if l > size * 0.05 {
            axis = across / length(across);
            side = normalize(cross(axis, eye));
        }
        half_len = size * 0.5 + l * 0.5;
        center_q = center - axis * (l * 0.5);
        cap = size * 0.5 / half_len;
    }
    let q = array<vec2<f32>, 6>(vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0))[vi % 6u];
    let world = center_q + side * (q.x * size * 0.5) + axis * (q.y * half_len);
    let c = mix(unpack4x8unorm(p.look.x), unpack4x8unorm(p.look.y), t);
    let glow = bitcast<f32>(p.look.w);
    var rgb: vec3<f32>;
    if glow > 0.0 {
        // Embers flicker as they cool.
        let flicker = select(1.0, 0.65 + 0.35 * sin(age * 37.0 + seed * 40.0), shape == 2u);
        rgb = c.rgb * glow * flicker;
    } else {
        let wrap = 0.5 + 0.5 * max(view.sun_dir.y, 0.0);
        rgb = c.rgb * (view.ambient.rgb + view.sun.rgb * wrap * 0.5) / PI;
    }
    out.position = view.output_vp * vec4<f32>(world, 1.0);
    out.uv = q;
    out.color = vec4<f32>(rgb * c.a, c.a * (1.0 - clamp(sa.y, 0.0, 1.0))) * thin;
    out.world = world;
    out.soft = max(size * 0.5, 0.02);
    out.shape = shape;
    out.cap = cap;
    return out;
}

@fragment
fn particle_fs(input: Out) -> @location(0) vec4<f32> {
    let clip = view.depth_vp * vec4<f32>(input.world, 1.0);
    if clip.w <= 0.0 { discard; }
    let suv = clip.xy / clip.w * vec2<f32>(0.5, -0.5) + 0.5;
    if any(suv < vec2<f32>(0.0)) || any(suv > vec2<f32>(1.0)) { discard; }
    let dims = vec2<i32>(textureDimensions(scene_depth));
    let pixel = clamp(vec2<i32>(suv * vec2<f32>(dims)), vec2<i32>(0), dims - 1);
    let gap = view.camera.w / max(textureLoad(scene_depth, pixel, 0), 1e-9) - clip.w;
    if gap <= 0.0 { discard; }
    let q = input.uv;
    let r2 = dot(q, q);
    var a: f32;
    switch input.shape {
        // Spark: a streak, hot along its axis.
        case 1u: { a = (1.0 - q.x * q.x) * (1.0 - q.x * q.x) * smoothstep(1.0, 0.6, abs(q.y)); }
        // Trail: a hot round head fading back along its path.
        case 3u: {
            let head_y = 1.0 - input.cap;
            let dy = max(q.y - head_y, 0.0) / max(input.cap, 1e-4);
            let head = max(0.0, 1.0 - q.x * q.x - dy * dy);
            let along = clamp((q.y + 1.0) / max(2.0 - input.cap, 1e-4), 0.0, 1.0);
            let tail = (1.0 - q.x * q.x) * (1.0 - q.x * q.x) * along * along * select(1.0, 0.0, q.y > head_y);
            a = max(head * head, 0.55 * tail);
        }
        // Ember: a small hard dot.
        case 2u: { let k = max(0.0, 1.0 - r2); a = k * k * k; }
        // Soft: a round puff.
        default: { a = max(0.0, exp(-3.0 * r2) - 0.0498) / 0.9502; }
    }
    a *= clamp(gap / input.soft, 0.0, 1.0);
    if a < 0.0005 { discard; }
    return input.color * a;
}
