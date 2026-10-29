// Neon Siege's own surfaces (warp: an entity with `surface = N` uses kind N).
//
// Kind 0, the backdrop: living tissue seen from inside a blood vessel. Near
// cells press against each other, so mostly their membranes show (a rounded
// Voronoi: the gap between cells widens where three meet). Far below, a
// second layer of bigger cells drifts the other way, heavily blurred.
// Everything moves slowly, so it never competes with the fight.
//
// Edit this file while the game runs: it recompiles on the next frame, and
// a version that does not compile keeps the last good one running.

struct SurfaceIn {
    // Which of the game's surfaces, from Material::layer.
    kind: u32,
    // The instance (index into the scene's instances).
    instance: u32,
    // World position, and object-space (rest pose) position.
    p: vec3<f32>,
    local: vec3<f32>,
    // Shading normal (world), and the direction from the eye to the point.
    n: vec3<f32>,
    dir: vec3<f32>,
    // Barycentrics and each corner's raw uv, so a game can pack per-vertex
    // data (such as vertex colours) into uvs and blend it itself.
    bary: vec3<f32>,
    uv0: vec2<f32>,
    uv1: vec2<f32>,
    uv2: vec2<f32>,
    // World-space footprint of one pixel (ray differentials).
    dx: vec3<f32>,
    dy: vec3<f32>,
    // Material tint, roughness, metalness and scale.
    tint: vec3<f32>,
    rough: f32,
    metal: f32,
    scale: f32,
    // Seconds (Lighting::time).
    time: f32,
};

struct SurfaceOut {
    albedo: vec3<f32>,
    normal: vec3<f32>,
    rough: f32,
    metal: f32,
    // Radiance in nits for white, scaled like EMISSIVE.
    emission: vec3<f32>,
};


// Tuning: cell sizes in metres, drift in metres per second, colours as
// linear rgb (multiplied by the entity's emissive in nits).
const BD_NEAR_CELL: f32 = 3.4;
const BD_FAR_CELL: f32 = 8.0;
const BD_NEAR_DRIFT: vec2<f32> = vec2<f32>(0.05, 0.025);
const BD_FAR_DRIFT: vec2<f32> = vec2<f32>(-0.03, 0.012);
const BD_MEMBRANE: vec3<f32> = vec3<f32>(0.9, 0.35, 0.45);
const BD_FAR: vec3<f32> = vec3<f32>(0.22, 0.07, 0.2);
const BD_PLASMA: vec3<f32> = vec3<f32>(0.03, 0.006, 0.018);

fn bd_hash(p: vec2<f32>) -> vec2<f32> {
    let q = vec2<f32>(dot(p, vec2<f32>(127.1, 311.7)), dot(p, vec2<f32>(269.5, 183.3)));
    return fract(sin(q) * 43758.5453);
}

// Rounded cells: x is 0 inside a cell and grows towards its membrane (and
// more where three cells meet), y is the distance to the nearest centre,
// z a random number per cell.
fn bd_cells(p: vec2<f32>, t: f32, k: f32) -> vec3<f32> {
    let cell = floor(p);
    let f = fract(p);
    var d1 = 9.0;
    var id = vec2<f32>(0.0);
    var ds: array<f32, 9>;
    var i = 0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let o = vec2<f32>(f32(x), f32(y));
            let h = bd_hash(cell + o);
            // Centres wander a little: cells breathe.
            let c = o + 0.5 + 0.3 * sin(t * (0.25 + 0.2 * h) + 6.2831 * h);
            let d = length(c - f);
            ds[i] = d;
            i++;
            if d < d1 { d1 = d; id = cell + o; }
        }
    }
    var sum = 0.0;
    for (var j = 0; j < 9; j++) { sum += exp(-k * (ds[j] - d1)); }
    return vec3<f32>(log(sum), d1, bd_hash(id + 7.0).x);
}

fn backdrop(s: SurfaceIn) -> vec3<f32> {
    let t = s.time;
    let xz = s.p.xz;
    // Near: thin bright membranes, a faint nucleus in some cells.
    let n = bd_cells(xz / BD_NEAR_CELL + BD_NEAR_DRIFT * t, t, 9.0);
    let membrane = smoothstep(0.45, 1.0, n.x) * (0.5 + 0.5 * n.z);
    let nucleus = (1.0 - smoothstep(0.08, 0.22, n.y)) * step(0.6, n.z) * 0.1;
    // Far: big, soft, out of focus (a low k blurs the membranes).
    let f = bd_cells(xz / BD_FAR_CELL + BD_FAR_DRIFT * t + 17.0, t * 0.6, 1.8);
    let far = smoothstep(0.5, 1.5, f.x) * (0.6 + 0.4 * f.z);
    // A slow pulse, as if a heart beats somewhere.
    let beat = 0.92 + 0.08 * sin(t * 1.3);
    return (BD_PLASMA + BD_FAR * far + BD_MEMBRANE * (membrane + nucleus) * beat) * s.scale;
}

fn game_surface(s: SurfaceIn) -> SurfaceOut {
    if s.kind == 0u {
        return SurfaceOut(vec3<f32>(0.0), s.n, 1.0, 0.0, backdrop(s));
    }
    return SurfaceOut(s.tint, s.n, s.rough, s.metal, vec3<f32>(0.0));
}
