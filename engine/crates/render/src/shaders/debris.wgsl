// GPU debris: spawn new shards into their pool slots, then integrate every
// shard for `counts.w` fixed steps against a one-bit occupancy grid and
// write its scene instance. After Growing Guns' gpu_debris.gdshader.
struct Shard { pos_age: vec4<f32>, vel_life: vec4<f32>, size_angle: vec4<f32>, axis_spin: vec4<f32>, mesh: vec4<u32> };
struct Instance { r0: vec4<f32>, r1: vec4<f32>, r2: vec4<f32>, sphere: vec4<f32>, first_cluster: u32, cluster_count: u32, scale: f32, flags: u32 };
struct Uniform {
    // dt, gravity, bounce, friction
    sim: vec4<f32>,
    // origin xyz, voxel size
    grid: vec4<f32>,
    // dims xyz, words
    dims: vec4<u32>,
    // spawns, capacity, first instance, steps
    counts: vec4<u32>,
    // kill y, settle speed, reset (1: kill all), unused
    extra: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: Uniform;
@group(0) @binding(1) var<storage, read_write> shards: array<Shard>;
@group(0) @binding(2) var<storage, read> spawns: array<Shard>;
@group(0) @binding(3) var<storage, read> occupancy: array<u32>;
@group(0) @binding(4) var<storage, read_write> instances: array<Instance>;

@compute @workgroup_size(64)
fn spawn(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.counts.x { return; }
    let s = spawns[id.x];
    if s.mesh.w < u.counts.y { shards[s.mesh.w] = s; }
}

fn solid(p: vec3<f32>) -> bool {
    let v = floor((p - u.grid.xyz) / u.grid.w);
    if any(v < vec3<f32>(0.0)) || any(v >= vec3<f32>(u.dims.xyz)) { return false; }
    let c = vec3<u32>(v);
    let i = c.x + c.y * u.dims.x + c.z * u.dims.x * u.dims.y;
    return ((occupancy[i >> 5u] >> (i & 31u)) & 1u) == 1u;
}

fn occ(p: vec3<f32>) -> f32 { return select(0.0, 1.0, solid(p)); }

// Outward normal from the occupancy gradient; straight up when flat.
fn normal_at(p: vec3<f32>) -> vec3<f32> {
    let e = u.grid.w;
    let g = vec3<f32>(
        occ(p + vec3<f32>(e, 0.0, 0.0)) - occ(p - vec3<f32>(e, 0.0, 0.0)),
        occ(p + vec3<f32>(0.0, e, 0.0)) - occ(p - vec3<f32>(0.0, e, 0.0)),
        occ(p + vec3<f32>(0.0, 0.0, e)) - occ(p - vec3<f32>(0.0, 0.0, e)));
    if dot(g, g) < 1e-6 { return vec3<f32>(0.0, 1.0, 0.0); }
    return normalize(-g);
}

fn rotation(a: vec3<f32>, angle: f32) -> mat3x3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    let t = 1.0 - c;
    return mat3x3<f32>(
        vec3<f32>(t * a.x * a.x + c, t * a.x * a.y + s * a.z, t * a.x * a.z - s * a.y),
        vec3<f32>(t * a.x * a.y - s * a.z, t * a.y * a.y + c, t * a.y * a.z + s * a.x),
        vec3<f32>(t * a.x * a.z + s * a.y, t * a.y * a.z - s * a.x, t * a.z * a.z + c));
}

@compute @workgroup_size(64)
fn simulate(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.counts.y { return; }
    var s = shards[id.x];
    if u.extra.z > 0.5 { s.vel_life.w = 0.0; }
    let dt = u.sim.x;
    var pos = s.pos_age.xyz;
    var vel = s.vel_life.xyz;
    var age = s.pos_age.w;
    var angle = s.size_angle.w;
    let life = s.vel_life.w;
    for (var k = 0u; k < u.counts.w; k++) {
        if age >= life { break; }
        age += dt;
        vel.y -= u.sim.y * dt;
        var next = pos + vel * dt;
        if solid(next) {
            let n = normal_at(next);
            let vn = dot(vel, n);
            // Tangential with friction, reflected normal with restitution.
            if vn < 0.0 { vel = (vel - n * vn) * u.sim.w - n * vn * u.sim.z; }
            next = pos + vel * dt;
            // Wedged in a corner: stay and bleed energy.
            if solid(next) { next = pos; vel *= 0.4; }
        }
        let sp = length(vel);
        // Settle: slow and supported stops dead instead of jittering.
        if sp < u.extra.y && solid(next - vec3<f32>(0.0, u.grid.w * 0.7, 0.0)) { vel = vec3<f32>(0.0); }
        angle += s.axis_spin.w * dt * clamp(sp * 0.3, 0.0, 1.0);
        pos = next;
        if pos.y < u.extra.x { age = life; }
    }
    s.pos_age = vec4<f32>(pos, age);
    s.vel_life = vec4<f32>(vel, life);
    s.size_angle.w = angle;
    shards[id.x] = s;

    var inst = instances[u.counts.z + id.x];
    // Shrink out over the last fifth of life.
    let fade = clamp((1.0 - age / max(life, 1e-4)) * 5.0, 0.0, 1.0);
    if life <= 0.0 || age >= life || fade <= 0.0 {
        inst.cluster_count = 0u;
        inst.sphere = vec4<f32>(0.0);
        instances[u.counts.z + id.x] = inst;
        return;
    }
    let size = s.size_angle.xyz * fade;
    let r = rotation(s.axis_spin.xyz, angle);
    let x = r[0] * size.x;
    let y = r[1] * size.y;
    let z = r[2] * size.z;
    inst.r0 = vec4<f32>(x.x, y.x, z.x, pos.x);
    inst.r1 = vec4<f32>(x.y, y.y, z.y, pos.y);
    inst.r2 = vec4<f32>(x.z, y.z, z.z, pos.z);
    let largest = max(size.x, max(size.y, size.z));
    inst.sphere = vec4<f32>(pos, bitcast<f32>(s.mesh.z) * largest);
    inst.scale = largest;
    inst.first_cluster = s.mesh.x;
    inst.cluster_count = s.mesh.y;
    instances[u.counts.z + id.x] = inst;
}
