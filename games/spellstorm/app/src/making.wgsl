// ---- Monsters being made (view.rs MAKING) ----
// A table in `aux` at MK_AT: MK_N records of 8 words, a monster each while
// it comes into the world: its instances [lo, hi), the floor under it, the
// height it is built up to (world y), its height, and the glow's colour.
// Its body exists below that height and not above (game_mask), the edge
// ragged like something growing, glowing hot just under it (game_glow):
// built up from its toes to the top of its head (Joep).

const MK_AT: u32 = 1u;
const MK_N: u32 = 16u;

fn mk_record(instance: u32) -> u32 {
    if aux_len() < MK_AT + MK_N * 8u { return 0u; }
    for (var i = 0u; i < MK_N; i++) {
        let b = MK_AT + i * 8u;
        if instance >= aux_word(b) && instance < aux_word(b + 1u) { return b; }
    }
    return 0u;
}

fn mk_hash(p: vec3<f32>) -> f32 {
    let q = fract(p * vec3<f32>(0.1031, 0.1030, 0.0973));
    let r = q + dot(q, q.yxz + 33.33);
    return fract((r.x + r.y) * r.z);
}

fn mk_noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(mix(mk_hash(i), mk_hash(i + vec3<f32>(1.0, 0.0, 0.0)), u.x),
                mix(mk_hash(i + vec3<f32>(0.0, 1.0, 0.0)), mk_hash(i + vec3<f32>(1.0, 1.0, 0.0)), u.x), u.y);
    let b = mix(mix(mk_hash(i + vec3<f32>(0.0, 0.0, 1.0)), mk_hash(i + vec3<f32>(1.0, 0.0, 1.0)), u.x),
                mix(mk_hash(i + vec3<f32>(0.0, 1.0, 1.0)), mk_hash(i + vec3<f32>(1.0, 1.0, 1.0)), u.x), u.y);
    return mix(a, b, u.z);
}

// How far below the edge `p` is (metres; negative: not made yet).
fn mk_depth(p: vec3<f32>, b: u32) -> f32 {
    let front = bitcast<f32>(aux_word(b + 3u));
    let h = max(bitcast<f32>(aux_word(b + 4u)), 0.3);
    // Ragged, a little ahead in some places and behind in others.
    let n = mk_noise(p * 7.0) * 0.65 + mk_noise(p * 19.0) * 0.35;
    return front + (n - 0.5) * 0.16 * h - p.y;
}
