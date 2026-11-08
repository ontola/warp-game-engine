// GPU particles: spawn and simulate (gpu_particles.rs). Four storage buffers.
struct Sim {
    // step seconds, floor height, floor friction per second, extra age (skip ahead)
    a: vec4<f32>,
    // wind (m/s), simulation time at the first step
    b: vec4<f32>,
    // steps, capacity, commands, births
    c: vec4<u32>,
    // ground heightfield: origin x, origin z, cell size, on (1)
    g: vec4<f32>,
    // ground cells along x and z
    gd: vec4<u32>,
};
struct Command {
    origin_radius: vec4<f32>,
    dir_cos: vec4<f32>,
    vel_duration: vec4<f32>,
    speed_life: vec4<f32>,
    color0: vec4<f32>,
    color1: vec4<f32>,
    // birth size, death size, glow nits, additive
    size_glow: vec4<f32>,
    // gravity, drag, bounce, turbulence
    phys: vec4<f32>,
    // stretch, shape, age at the first step, unused
    extra: vec4<f32>,
    // first slot, count, seed, first birth index
    slots: vec4<u32>,
};
struct Particle {
    pos_age: vec4<f32>,
    // velocity, lifetime (0: dead)
    vel_life: vec4<f32>,
    // colour at birth, colour at death (rgba8), sizes (2x f16), glow (f32 bits)
    look: vec4<u32>,
    // gravity|drag, bounce|turbulence, stretch|additive (2x f16), seed | shape << 30
    phys: vec4<u32>,
};
struct DrawArgs { vertex_count: u32, instance_count: atomic<u32>, first_vertex: u32, first_instance: u32 };

@group(0) @binding(0) var<uniform> sim: Sim;
@group(0) @binding(1) var<storage, read_write> particles: array<Particle>;
@group(0) @binding(2) var<storage, read> commands: array<Command>;
@group(0) @binding(3) var<storage, read_write> alive: array<u32>;
@group(0) @binding(4) var<storage, read_write> args: DrawArgs;
// Optional ground: the top of each column (a one-cell dummy when off).
@group(1) @binding(0) var<storage, read> heights: array<f32>;

const TAU: f32 = 6.2831853;

fn hash(x: u32) -> u32 {
    let s = x * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}
fn rand(s: ptr<function, u32>) -> f32 {
    *s = hash(*s);
    return f32(*s >> 8u) / 16777216.0;
}

@compute @workgroup_size(64)
fn spawn(@builtin(global_invocation_id) g: vec3<u32>) {
    let t = g.x;
    if t >= sim.c.w || sim.c.z == 0u { return; }
    // The last command whose first birth is at or before t.
    var lo = 0u;
    var hi = sim.c.z - 1u;
    loop {
        if lo >= hi { break; }
        let mid = (lo + hi + 1u) / 2u;
        if commands[mid].slots.w <= t { lo = mid; } else { hi = mid - 1u; }
    }
    let c = commands[lo];
    let i = t - c.slots.w;
    if i >= c.slots.y { return; }
    var s = hash(c.slots.z ^ hash(i + 0x9e3779b9u));
    // Uniform in the ball.
    let z = rand(&s) * 2.0 - 1.0;
    let a = rand(&s) * TAU;
    let ring = sqrt(max(0.0, 1.0 - z * z));
    let r = c.origin_radius.w * pow(rand(&s), 1.0 / 3.0);
    let pos = c.origin_radius.xyz + vec3<f32>(ring * cos(a), z, ring * sin(a)) * r;
    // Uniform over the cone's cap.
    let d = normalize(c.dir_cos.xyz);
    let ct = 1.0 - rand(&s) * (1.0 - c.dir_cos.w);
    let st = sqrt(max(0.0, 1.0 - ct * ct));
    let b = rand(&s) * TAU;
    let helper = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(d.y) > 0.99);
    let u = normalize(cross(helper, d));
    let v = cross(d, u);
    let dir = d * ct + (u * cos(b) + v * sin(b)) * st;
    let speed = mix(c.speed_life.x, c.speed_life.y, rand(&s));
    let life = max(0.001, mix(c.speed_life.z, c.speed_life.w, rand(&s)));
    // Births spread over the emitter's duration start later (negative age).
    let delay = c.vel_duration.w * (f32(i) + rand(&s)) / f32(max(c.slots.y, 1u));
    var p: Particle;
    p.pos_age = vec4<f32>(pos, c.extra.z - delay);
    p.vel_life = vec4<f32>(dir * speed + c.vel_duration.xyz, life);
    p.look = vec4<u32>(pack4x8unorm(clamp(c.color0, vec4<f32>(0.0), vec4<f32>(1.0))),
        pack4x8unorm(clamp(c.color1, vec4<f32>(0.0), vec4<f32>(1.0))),
        pack2x16float(c.size_glow.xy), bitcast<u32>(c.size_glow.z));
    p.phys = vec4<u32>(pack2x16float(c.phys.xy), pack2x16float(c.phys.zw),
        pack2x16float(vec2<f32>(c.extra.x, c.size_glow.w)),
        (hash(s) & 0x3fffffffu) | (min(u32(c.extra.y), 3u) << 30u));
    particles[(c.slots.x + i) % sim.c.y] = p;
}

fn ground(p: vec3<f32>) -> f32 {
    if sim.g.w < 0.5 { return sim.a.y; }
    let c = vec2<i32>(floor((p.xz - sim.g.xy) / sim.g.z));
    if any(c < vec2<i32>(0)) || c.x >= i32(sim.gd.x) || c.y >= i32(sim.gd.y) { return sim.a.y; }
    return max(sim.a.y, heights[u32(c.y) * sim.gd.x + u32(c.x)]);
}

// Divergence free swirl: each axis depends only on the other two.
fn swirl(p: vec3<f32>, t: f32) -> vec3<f32> {
    let q = p * 0.8 + vec3<f32>(0.31 * t, -0.53 * t, 0.17 * t);
    return vec3<f32>(sin(q.y * 1.7 + cos(q.z * 1.3)), 0.7 * sin(q.z * 1.5 + cos(q.x * 1.9)), sin(q.x * 1.3 + cos(q.y * 1.1)));
}

@compute @workgroup_size(64)
fn simulate(@builtin(global_invocation_id) g: vec3<u32>) {
    let i = g.x;
    if i >= sim.c.y { return; }
    var p = particles[i];
    let life = p.vel_life.w;
    if life <= 0.0 { return; }
    let gd = unpack2x16float(p.phys.x);
    let bt = unpack2x16float(p.phys.y);
    let step = sim.a.x;
    var pos = p.pos_age.xyz;
    var vel = p.vel_life.xyz;
    var age = p.pos_age.w + sim.a.w;
    var dead = false;
    for (var k = 0u; k < sim.c.x && !dead; k++) {
        age += step;
        if age >= life { dead = true; }
        if age >= 0.0 && !dead {
            vel.y -= gd.x * step;
            if bt.y > 0.0 { vel += swirl(pos, sim.b.w + f32(k) * step) * (bt.y * step); }
            // Drag pulls towards the wind, so smoke drifts with it.
            vel += (sim.b.xyz - vel) * min(gd.y * step, 1.0);
            let prev = pos;
            pos += vel * step;
            let top = ground(pos);
            if pos.y < top && top - prev.y > 0.2 {
                // Into the side of a column: back out and bounce off the wall.
                dead = bt.x < 0.0;
                pos = vec3<f32>(prev.x, pos.y, prev.z);
                vel = vec3<f32>(-vel.x, vel.y, -vel.z) * vec3<f32>(max(bt.x, 0.0), 1.0, max(bt.x, 0.0));
            } else if pos.y < top {
                // Negative bounce: the particle dies where it lands.
                dead = bt.x < 0.0;
                pos.y = top;
                vel.y = max(-vel.y * bt.x, 0.0);
                if vel.y < 0.4 { vel.y = 0.0; }
                let slide = 1.0 - min(sim.a.z * step, 1.0);
                vel = vec3<f32>(vel.x * slide, vel.y, vel.z * slide);
            }
        }
    }
    if dead || age >= life {
        p.vel_life.w = 0.0;
        particles[i] = p;
        return;
    }
    p.pos_age = vec4<f32>(pos, age);
    p.vel_life = vec4<f32>(vel, life);
    particles[i] = p;
    if age >= 0.0 {
        let n = atomicAdd(&args.instance_count, 1u);
        alive[n] = i;
    }
}
