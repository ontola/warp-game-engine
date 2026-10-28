// Crowd animation, after Growing Guns' crowd.gdshader: per spectator a body
// instance (torso + head) and an arms instance, posed every frame from the
// clock and the mood. Cheer waves raise the arms and hop the figure,
// excitement double-times the wave, panic crouches and trembles, blasts
// scatter nearby spectators along their row and they walk back after.
struct Seat {
    // position, yaw
    pos_yaw: vec4<f32>,
    // scale, phase, tempo, hop amplitude
    params: vec4<f32>,
    // body first cluster, count; arms first cluster, count
    meshes: vec4<u32>,
};
struct Instance { r0: vec4<f32>, r1: vec4<f32>, r2: vec4<f32>, sphere: vec4<f32>, first_cluster: u32, cluster_count: u32, scale: f32, flags: u32 };
struct Uniform {
    // time, excitement, panic, optional local shoulder pivot height
    mood: vec4<f32>,
    // ring centre xz, seats, first instance (as f32 bits: see counts)
    ring: vec4<f32>,
    counts: vec4<u32>,
    flee_blast: array<vec4<f32>, 4>,
    flee_age: vec4<f32>,
    flee_hold: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: Uniform;
@group(0) @binding(1) var<storage, read> seats: array<Seat>;
@group(0) @binding(2) var<storage, read> alive: array<u32>;
@group(0) @binding(3) var<storage, read_write> instances: array<Instance>;

const FLEE_OUT_END: f32 = 1.2;
const FLEE_RETURN_DUR: f32 = 4.5;

fn write(index: u32, x: vec3<f32>, y: vec3<f32>, z: vec3<f32>, t: vec3<f32>, first: u32, count: u32, radius: f32, scale: f32) {
    var inst = instances[index];
    inst.r0 = vec4<f32>(x.x, y.x, z.x, t.x);
    inst.r1 = vec4<f32>(x.y, y.y, z.y, t.y);
    inst.r2 = vec4<f32>(x.z, y.z, z.z, t.z);
    inst.sphere = vec4<f32>(t + y * 0.5, radius);
    inst.first_cluster = first;
    inst.cluster_count = count;
    inst.scale = scale;
    instances[index] = inst;
}

@compute @workgroup_size(64)
fn animate(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= u.counts.x { return; }
    let s = seats[i];
    let body_index = u.counts.y + i * 2u;
    if alive[i] == 0u {
        var a = instances[body_index]; a.cluster_count = 0u; a.sphere = vec4<f32>(0.0); instances[body_index] = a;
        var b = instances[body_index + 1u]; b.cluster_count = 0u; b.sphere = vec4<f32>(0.0); instances[body_index + 1u] = b;
        return;
    }
    let time = u.mood.x;
    let excitement = u.mood.y;
    let panic = u.mood.z;
    let scale = s.params.x;
    let phase = s.params.y;
    let t = time * (1.4 + s.params.z * 2.2) + phase * 6.2832;
    var up = max(sin(t), 0.0);
    up = max(up, max(sin(t * 1.93 + 1.7), 0.0) * excitement);
    let hop = (s.params.w + excitement * 0.22) * (1.0 + excitement * 0.8) * (1.0 - panic);
    var lift = up * hop * 0.4;
    // Shear x by height: idle sway plus the panic tremble.
    let shear = sin(t * 0.6) * 0.035 + panic * sin(time * 21.0 + phase * 47.0) * 0.045;
    let crouch = 1.0 - panic * 0.22;
    // Flee from recent blasts: tangential along the row, hold, walk back.
    var disp = vec2<f32>(0.0);
    var run = 0.0;
    let seat = s.pos_yaw.xyz;
    for (var k = 0u; k < 4u; k++) {
        let age = u.flee_age[k];
        let hold = u.flee_hold[k];
        if age >= hold + FLEE_RETURN_DUR { continue; }
        let blast = u.flee_blast[k];
        let to_seat = seat.xz - blast.xz;
        let reach = blast.w * 2.6 + 2.0;
        let d = length(to_seat);
        if d >= reach { continue; }
        let prox = 1.0 - d / reach;
        let radial = normalize(seat.xz - u.ring.xy);
        let tang = vec2<f32>(-radial.y, radial.x);
        let side = select(-1.0, 1.0, dot(tang, to_seat) >= 0.0);
        let outp = smoothstep(0.0, FLEE_OUT_END, age);
        let retp = smoothstep(hold, hold + FLEE_RETURN_DUR, age);
        disp += tang * side * prox * (3.0 + 5.0 * phase) * outp * (1.0 - retp);
        run = max(run, prox * clamp(outp * (1.0 - outp) * 4.0 + retp * (1.0 - retp) * 4.0, 0.0, 1.0));
    }
    lift += abs(sin(time * 10.5 + phase * 41.0)) * 0.17 * run;
    // Local frame: yaw about Y, uniform scale; figure +Z faces the arena.
    let c = cos(s.pos_yaw.w);
    let sn = sin(s.pos_yaw.w);
    let ex = vec3<f32>(c, 0.0, -sn) * scale;
    let ey = vec3<f32>(0.0, 1.0, 0.0) * scale;
    let ez = vec3<f32>(sn, 0.0, c) * scale;
    let origin = seat + vec3<f32>(disp.x, 0.0, disp.y) + ey * lift;
    // Body: crouch squashes Y, shear leans X with height.
    let by = ey * crouch + ex * shear * crouch;
    write(body_index, ex, by, ez, origin, s.meshes.x, s.meshes.y, scale * 0.9, scale);
    // Arms: raised with the cheer, spread outward (they sit at x = +-0.21).
    let arm = 1.0 - panic;
    if u.mood.w > 0.0 {
        let angle = -2.4 * arm * up;
        let ay = by * cos(angle) + ez * sin(angle);
        let az = ez * cos(angle) - by * sin(angle);
        let pivot = u.mood.w;
        write(body_index + 1u, ex, ay, az, origin + (by - ay) * pivot,
              s.meshes.z, s.meshes.w, scale * 0.9, scale);
        return;
    }
    let spread = 1.0 + arm * up * 0.12 / 0.21;
    let arms_origin = origin + by * 0.0 + ey * (arm * up * 0.34);
    write(body_index + 1u, ex * spread, by, ez, arms_origin, s.meshes.z, s.meshes.w, scale * 0.9, scale * spread);
}
