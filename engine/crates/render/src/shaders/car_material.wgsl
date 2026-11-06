// Procedural car materials, appended to shade.wgsl. Everything is made here
// from the object-space position and normal, a per-vertex curvature the
// loader bakes into the paint's uv, and noise: no texture assets. Parameters
// come from engine/data/car_materials.toml (cli/src/car_look.rs).
//
// Layers, bottom to top: bare metal, primer, paint (solid, metallic flake,
// pearl or candy) under a clear coat; then chips through the paint, rust,
// sun fade, scratches and polish swirls in the coat, a dust film and mud.
//
// Noise is filtered by the pixel footprint (fp, from ray differentials):
// detail smaller than a pixel fades to its mean instead of aliasing.

struct CarMaterial {
    colour: vec4<f32>,  // rgb paint or part colour (linear), w: kind
    second: vec4<f32>,  // rgb pearl shift / candy flake colour, w: finish
    surface: vec4<f32>, // base roughness, metalness, clear coat, coat roughness
    flake: vec4<f32>,   // flake amount, flake size (m), flake spread, orange peel
    wear: vec4<f32>,    // chips, scratches, swirls, sun fade
    grime: vec4<f32>,   // dust, mud, rust, seed
    dirt: vec4<f32>,    // rgb dust colour, w: wetness of the mud
    body: vec4<f32>,    // object-space bounds: min y, max y, min z, max z
    axles: vec4<f32>,   // front axle z, rear axle z, wheel radius, half track (< 0: a wheel)
};
fn car_material(i: u32) -> CarMaterial {
    let at = scene_tables[0].y + 9u * i;
    var v: array<vec4<f32>, 9>;
    for (var k = 0u; k < 9u; k++) { v[k] = bitcast<vec4<f32>>(scene_tables[at + k]); }
    return CarMaterial(v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7], v[8]);
}

const CAR_PAINT: u32 = 0u;
const CAR_RUBBER: u32 = 1u;
const CAR_METAL: u32 = 2u;
const CAR_CHROME: u32 = 3u;
const CAR_PLASTIC: u32 = 4u;
const CAR_GLASS: u32 = 5u;
const CAR_TEXTURED: u32 = 6u;

const FINISH_SOLID: u32 = 0u;
const FINISH_METALLIC: u32 = 1u;
const FINISH_PEARL: u32 = 2u;
const FINISH_CANDY: u32 = 3u;

const PRIMER: vec3<f32> = vec3<f32>(0.30, 0.31, 0.29);
const BARE_STEEL: vec3<f32> = vec3<f32>(0.52, 0.52, 0.50);

// What the lighting in shade() uses instead of the plain material.
struct CarSurface {
    base: vec3<f32>,
    n: vec3<f32>,
    rough: f32,
    metal: f32,
    coat: f32,          // clear coat weight 0..1
    coat_rough: f32,
    coat_n: vec3<f32>,  // the coat's own normal (orange peel)
    glint: vec3<f32>,   // extra sun BRDF * n.l: flakes and swirl marks
    debug: vec3<f32>,   // --set debug=20..22
};

// pcg3d (Jarzynski and Olano 2020): a well-mixed hash of an integer cell.
fn car_hash(c: vec3<i32>, seed: u32) -> vec3<f32> {
    var v = bitcast<vec3<u32>>(c) * 1664525u + vec3<u32>(1013904223u + seed, 12345u + seed * 7u, 777u);
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v ^= v >> vec3<u32>(16u);
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    return vec3<f32>(v >> vec3<u32>(8u)) / 16777216.0;
}

fn car_noise(p: vec3<f32>, seed: u32) -> f32 {
    let i = vec3<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(car_hash(i, seed).x, car_hash(i + vec3<i32>(1, 0, 0), seed).x, u.x);
    let b = mix(car_hash(i + vec3<i32>(0, 1, 0), seed).x, car_hash(i + vec3<i32>(1, 1, 0), seed).x, u.x);
    let c = mix(car_hash(i + vec3<i32>(0, 0, 1), seed).x, car_hash(i + vec3<i32>(1, 0, 1), seed).x, u.x);
    let d = mix(car_hash(i + vec3<i32>(0, 1, 1), seed).x, car_hash(i + vec3<i32>(1, 1, 1), seed).x, u.x);
    return mix(mix(a, b, u.y), mix(c, d, u.y), u.z);
}

// Four octaves around 0.5; `fw` is the pixel footprint in units of p. An
// octave finer than about two pixels fades to its mean.
fn car_fbm(p: vec3<f32>, fw: f32, seed: u32) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var freq = 1.0;
    var q = p;
    for (var k = 0u; k < 4u; k++) {
        let keep = 1.0 - smoothstep(0.25, 0.7, fw * freq);
        sum += amp * mix(0.5, car_noise(q, seed + k * 31u), keep);
        amp *= 0.5;
        freq *= 2.03;
        q = q * 2.03 + vec3<f32>(17.1, 3.7, 9.2);
    }
    return sum / 0.9375;
}

// A noise gradient by central differences, for bumps (rust, mud, peel).
fn car_bump(p: vec3<f32>, fw: f32, seed: u32) -> vec3<f32> {
    let e = 0.35;
    let x = car_fbm(p + vec3<f32>(e, 0.0, 0.0), fw, seed) - car_fbm(p - vec3<f32>(e, 0.0, 0.0), fw, seed);
    let y = car_fbm(p + vec3<f32>(0.0, e, 0.0), fw, seed) - car_fbm(p - vec3<f32>(0.0, e, 0.0), fw, seed);
    let z = car_fbm(p + vec3<f32>(0.0, 0.0, e), fw, seed) - car_fbm(p - vec3<f32>(0.0, 0.0, e), fw, seed);
    return vec3<f32>(x, y, z) / (2.0 * e);
}

// Tilt n by a gradient, keeping only its tangential part.
fn car_tilt(n: vec3<f32>, g: vec3<f32>, k: f32) -> vec3<f32> {
    return safe_normalize(n - (g - n * dot(g, n)) * k);
}

// Instance space: the car's own frame (metres, forward -z, y up).
fn car_axes(inst: Instance) -> mat3x3<f32> {
    return mat3x3<f32>(vec3<f32>(inst.r0.x, inst.r1.x, inst.r2.x), vec3<f32>(inst.r0.y, inst.r1.y, inst.r2.y), vec3<f32>(inst.r0.z, inst.r1.z, inst.r2.z));
}

fn car_local(inst: Instance, p: vec3<f32>) -> vec3<f32> {
    let a = car_axes(inst);
    let o = p - vec3<f32>(inst.r0.w, inst.r1.w, inst.r2.w);
    return vec3<f32>(dot(a[0], o) / dot(a[0], a[0]), dot(a[1], o) / dot(a[1], a[1]), dot(a[2], o) / dot(a[2], a[2]));
}

fn car_local_dir(inst: Instance, d: vec3<f32>) -> vec3<f32> {
    let a = car_axes(inst);
    return safe_normalize(vec3<f32>(dot(a[0], d), dot(a[1], d), dot(a[2], d)));
}

fn car_world_dir(inst: Instance, d: vec3<f32>) -> vec3<f32> {
    return safe_normalize(car_axes(inst) * d);
}

// The 2D frame that best faces the surface: sides use (z, y), the roof and
// bonnet (x, z), nose and tail (x, y). The first axis runs lengthwise
// where it can: scratches from branches and washing run along the car.
fn car_plane(q: vec3<f32>, nl: vec3<f32>) -> vec2<f32> {
    let a = abs(nl);
    if a.x >= a.y && a.x >= a.z { return q.zy; }
    if a.y >= a.z { return q.zx; }
    return q.xy;
}

fn car_plane_axes(nl: vec3<f32>) -> mat2x3<f32> {
    let a = abs(nl);
    if a.x >= a.y && a.x >= a.z { return mat2x3<f32>(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, 1.0, 0.0)); }
    if a.y >= a.z { return mat2x3<f32>(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(1.0, 0.0, 0.0)); }
    return mat2x3<f32>(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0));
}

// Fine straight scratches: per cell one short segment, two offset grids so
// segments cross cell borders. Returns coverage of this pixel (0..1).
fn car_scratches(uv: vec2<f32>, fw: f32, density: f32, seed: u32) -> f32 {
    let cell = 0.07;
    var cover = 0.0;
    for (var layer = 0u; layer < 2u; layer++) {
        let shift = f32(layer) * 0.5;
        let g = uv / cell + shift;
        let id = floor(g);
        let r = car_hash(vec3<i32>(vec2<i32>(id), i32(layer)), seed + 911u);
        if r.x > density { continue; }
        let centre = vec2<f32>(0.3, 0.3) + r.yz * 0.4;
        // Mostly lengthwise, some at a steep angle.
        let r2 = car_hash(vec3<i32>(vec2<i32>(id), i32(layer) + 7), seed + 313u);
        let angle = (r2.x - 0.5) * select(0.6, 3.1, r2.y > 0.8);
        let d = vec2<f32>(cos(angle), sin(angle));
        let half_len = 0.15 + r2.z * 0.3;
        let rel = g - id - centre;
        let along = clamp(dot(rel, d), -half_len, half_len);
        let dist = length(rel - d * along) * cell;
        let width = 0.00025 + r2.y * 0.0004;
        // A line much thinner than the pixel covers width / footprint of it.
        let soft = max(fw, width);
        cover = max(cover, (1.0 - smoothstep(0.0, soft, dist)) * min(1.0, width * 2.0 / soft) * (0.5 + r.x / max(density, 1e-3) * 0.5));
    }
    return cover;
}

// Small stone chips: round dots on a 3D grid, denser where `p` is exposed.
fn car_dots(q: vec3<f32>, fw: f32, density: f32, seed: u32) -> f32 {
    let cell = 0.012;
    let g = q / cell;
    let id = vec3<i32>(floor(g));
    let r = car_hash(id, seed + 501u);
    let radius = 0.0008 + car_hash(id, seed + 502u).x * 0.0022;
    let centre = (vec3<f32>(id) + 0.25 + r * 0.5) * cell;
    let d = length(q - centre);
    let sharp = clamp((radius - d) / max(fw * 0.5, 1e-5) + 0.5, 0.0, 1.0) * select(0.0, 1.0, car_hash(id, seed + 503u).x < density);
    // Seen from far, the dots become their mean coverage.
    let mean = density * 3.14 * 0.002 * 0.002 * 0.66 / (cell * cell);
    return mix(sharp, mean, smoothstep(cell * 0.4, cell * 2.0, fw));
}

// Sun glints from metallic flakes under the coat. Each cell of size about
// a pixel (never below the flake size) holds one flake whose normal is the
// surface normal tilted at random; far away this becomes a smooth lobe.
fn car_flakes(q: vec3<f32>, n: vec3<f32>, nl: vec3<f32>, inst: Instance, v: vec3<f32>, l: vec3<f32>, fw: f32, cm: CarMaterial, f0: vec3<f32>, seed: u32) -> vec3<f32> {
    let size = max(cm.flake.y, 1e-5);
    let cell = max(size, fw * 0.7);
    let id = vec3<i32>(floor(q / cell));
    let r = car_hash(id, seed + 77u);
    let pa = car_plane_axes(nl);
    let t = car_world_dir(inst, pa[0]);
    let b = car_world_dir(inst, pa[1]);
    let spread = cm.flake.z;
    let flake_n = safe_normalize(n + (t * (r.x - 0.5) + b * (r.y - 0.5)) * 2.0 * spread);
    // Only a share of cells has a flake facing the light at all; their glint
    // is brighter, so the average matches a smooth lobe of the same spread.
    let share = 0.35;
    let spiky = ggx(flake_n, v, l, 0.07, f0) * select(0.0, 1.0 / share, r.z < share);
    let wide = ggx(n, v, l, clamp(spread * 0.9 + 0.07, 0.08, 1.0), f0);
    return mix(spiky, wide, smoothstep(size * 2.0, size * 14.0, fw)) * cm.flake.x;
}

// Polish swirls: short circular arcs left by a buffer pad, a few per 8 cm
// cell. A groove reflects the sun only where the half vector lies in its
// cross-section, so near the sun's reflection only the arcs running across
// the direction to it light up: the familiar rings around the highlight.
fn car_swirls(q: vec3<f32>, nl: vec3<f32>, inst: Instance, n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, fw: f32, amount: f32, seed: u32) -> vec3<f32> {
    let h = safe_normalize(v + l);
    let near = smoothstep(0.9, 0.99, dot(h, n));
    if near <= 0.0 {
        return vec3<f32>(0.0);
    }
    let uv = car_plane(q, nl);
    let pa = car_plane_axes(nl);
    let cell = 0.08;
    var g = 0.0;
    for (var layer = 0u; layer < 3u; layer++) {
        let o = f32(layer) * 0.37;
        let id = floor(uv / cell + o);
        let r = car_hash(vec3<i32>(vec2<i32>(id), 40 + i32(layer)), seed + 4242u);
        if r.z > amount {
            continue;
        }
        // The pad's centre lies some way off; its arc crosses this cell.
        let radius = 0.04 + r.x * 0.14;
        let angle = r.y * 6.2832;
        let mid = (id - o + 0.5) * cell;
        let c = mid - vec2<f32>(cos(angle), sin(angle)) * radius;
        let rel = uv - c;
        let rad = length(rel);
        let width = 0.0002;
        let soft = max(fw, width);
        let cover = (1.0 - smoothstep(0.0, soft, abs(rad - radius))) * min(1.0, width * 3.0 / soft);
        let fade = 1.0 - smoothstep(cell * 0.6, cell * 1.3, length(uv - mid));
        let t2 = vec2<f32>(-rel.y, rel.x) / max(rad, 1e-4);
        let t = car_world_dir(inst, pa[0] * t2.x + pa[1] * t2.y);
        g += cover * fade * exp(-pow(dot(h, t) / 0.03, 2.0));
    }
    return vec3<f32>(g * near * 20.0 * max(dot(n, l), 0.0));
}

// Distance from the wheel arch line around an axle at height 0, in radii.
fn car_arch(q: vec3<f32>, axle: f32, radius: f32) -> f32 {
    return length(vec2<f32>(q.z - axle, q.y)) / max(radius, 0.05);
}

fn car_surface(cm: CarMaterial, inst: Instance, p: vec3<f32>, n: vec3<f32>, dir: vec3<f32>, uv: vec2<f32>, base_in: vec3<f32>, rough_in: f32, metal_in: f32) -> CarSurface {
    let kind = u32(cm.colour.w + 0.5);
    let finish = u32(cm.second.w + 0.5);
    let seed = u32(cm.grime.w);
    let v = -dir;
    let l = light.sun_dir.xyz;
    let ndv = max(dot(n, v), 1e-4);
    let fw = max(max(length(fp.dx), length(fp.dy)), 1e-5);
    let q = car_local(inst, p);
    let nl = car_local_dir(inst, n);
    let wheel = cm.axles.w < 0.0;
    let radius = max(cm.axles.z, 0.05);
    var s: CarSurface;
    s.base = base_in;
    s.n = n;
    s.rough = rough_in;
    s.metal = metal_in;
    s.coat = 0.0;
    s.coat_rough = 0.04;
    s.coat_n = n;
    s.glint = vec3<f32>(0.0);
    s.debug = vec3<f32>(0.0);

    // Where grime and wear collect, from the body's shape.
    let h = clamp((q.y - cm.body.x) / max(cm.body.y - cm.body.x, 0.1), 0.0, 1.0);
    // On a turning wheel height means nothing; the tread is what gets dirty.
    let rad = length(q.yz) / radius;
    let low = select(1.0 - smoothstep(0.08, 0.55, h), smoothstep(0.55, 1.0, rad), wheel);
    let up = smoothstep(0.55, 0.9, nl.y);
    let front = smoothstep(0.25, 0.85, -nl.z) * (1.0 - h * 0.6);
    var spray = 0.0;
    var arch = 0.0;
    if !wheel {
        // Thrown up behind each wheel and onto the sills between them.
        for (var k = 0u; k < 2u; k++) {
            let axle = select(cm.axles.x, cm.axles.y, k == 1u);
            let behind = (q.z - axle) / radius;
            spray = max(spray, exp(-pow((behind - 1.1) / 1.3, 2.0)) * (1.0 - smoothstep(0.1, 0.6, h)));
            arch = max(arch, exp(-pow((car_arch(q, axle, radius) - 1.15) / 0.18, 2.0)));
        }
    }
    let convex = uv.x;  // baked curvature (1/m), positive on convex edges
    let broad = uv.y;   // the same, smoothed wider
    let paintable = kind == CAR_PAINT;
    let edge = select(0.0, max(smoothstep(6.0, 30.0, convex), smoothstep(3.0, 12.0, broad) * 0.6), paintable);
    let crease = select(0.0, smoothstep(4.0, 20.0, -broad), paintable);

    // ---- The material itself.
    let fw_peel = fw / 0.012;
    if kind == CAR_PAINT {
        var colour = cm.colour.rgb;
        s.rough = cm.surface.x;
        s.metal = cm.surface.y;
        s.coat = cm.surface.z;
        s.coat_rough = cm.surface.w;
        if finish == FINISH_METALLIC {
            // Flop: flakes lying in the surface send light back toward the
            // face, so a metallic is brighter head-on and darker edge-on.
            colour *= mix(0.45, 1.1, pow(ndv, 0.6));
        } else if finish == FINISH_PEARL {
            // Thin-film mica: the hue moves to `second` toward grazing angles.
            colour = mix(colour, cm.second.rgb, 0.7 * smoothstep(0.15, 0.9, 1.0 - ndv));
        } else if finish == FINISH_CANDY {
            // A tinted lacquer over bright flakes: light crosses it twice,
            // further at grazing angles, so the colour deepens there.
            let path = 1.0 / max(mix(1.0, ndv, 0.6), 0.15);
            colour = cm.second.rgb * pow(max(cm.colour.rgb, vec3<f32>(1e-3)), vec3<f32>(path));
        }
        // Sun fade: upward panels chalk, lose saturation and gloss.
        let fade = cm.wear.w * up * (0.7 + 0.6 * car_fbm(q * 3.0, fw * 3.0, seed + 5u));
        let grey = dot(colour, vec3<f32>(0.2126, 0.7152, 0.0722));
        colour = mix(colour, vec3<f32>(grey * 1.15 + 0.04, grey * 1.1 + 0.04, grey * 1.1 + 0.045), clamp(fade, 0.0, 1.0) * 0.75);
        s.coat *= 1.0 - clamp(fade, 0.0, 1.0) * 0.85;
        s.coat_rough = mix(s.coat_rough, 0.35, clamp(fade, 0.0, 1.0));
        s.rough = mix(s.rough, 0.75, clamp(fade, 0.0, 1.0) * 0.8);
        s.base = colour;
        // Orange peel: a gentle waviness of the coat only.
        if cm.flake.w > 0.0 {
            s.coat_n = car_tilt(n, car_world_dir(inst, car_bump(q / 0.012, fw_peel, seed + 9u)), cm.flake.w * 0.02);
        }
        if cm.flake.x > 0.0 && (finish == FINISH_METALLIC || finish == FINISH_PEARL || finish == FINISH_CANDY) {
            let flake_colour = select(colour, mix(colour, cm.second.rgb, 0.5), finish == FINISH_PEARL);
            s.glint = car_flakes(q, n, nl, inst, v, l, fw, cm, flake_colour, seed);
        }
    } else if kind == CAR_RUBBER {
        // A tyre texture also holds the brake disc, caliper and lettering:
        // dark is rubber, light grey inside the rim is the disc's steel.
        let lum = dot(base_in, vec3<f32>(0.2126, 0.7152, 0.0722));
        let sat = max(base_in.r, max(base_in.g, base_in.b)) - min(base_in.r, min(base_in.g, base_in.b));
        let rubber = 1.0 - smoothstep(0.05, 0.14, lum);
        let disc = (1.0 - rubber) * (1.0 - smoothstep(0.04, 0.12, sat)) * (1.0 - smoothstep(0.62, 0.75, rad));
        s.metal = disc;
        s.rough = mix(mix(0.45, cm.surface.x, rubber), 0.38, disc);
        // Aged rubber blooms brownish grey on the sidewall.
        let bloom = cm.wear.w * rubber * smoothstep(0.7, 0.85, rad);
        s.base = mix(base_in, vec3<f32>(0.05, 0.045, 0.04), bloom * 0.8);
        s.rough = mix(s.rough, 0.95, bloom);
    } else if kind == CAR_METAL {
        s.base = cm.colour.rgb * base_in;
        s.metal = 1.0;
        s.rough = cm.surface.x;
    } else if kind == CAR_CHROME {
        s.base = cm.colour.rgb;
        s.metal = 1.0;
        s.rough = cm.surface.x;
    } else if kind == CAR_PLASTIC {
        // Unpainted trim greys where the sun reaches it.
        let fade = cm.wear.w * up;
        s.base = mix(cm.colour.rgb * base_in, vec3<f32>(0.09, 0.09, 0.09), fade * 0.7);
        s.metal = 0.0;
        s.rough = mix(cm.surface.x, 0.8, fade);
    } else {
        s.rough = max(rough_in, cm.surface.x);
    }

    // ---- Wear through the paint: chips (primer, then steel), scratches.
    var primer = 0.0;
    var steel = 0.0;
    if kind == CAR_PAINT {
        let field = car_fbm(q * 13.0, fw * 13.0, seed + 11u);
        let t = cm.wear.x * (edge * 2.0 + front * 0.35 + low * 0.3 + arch * 0.5);
        primer = smoothstep(1.0 - t, 1.0 - t + 0.05, field);
        steel = smoothstep(1.0 - t * 0.7, 1.0 - t * 0.7 + 0.05, field);
        let dots = car_dots(q, fw, cm.wear.x * (front * 0.5 + low * 0.15 + arch * 0.25), seed);
        primer = max(primer, dots);
        steel = max(steel, dots * 0.6);
        let scratched_area = smoothstep(0.35, 0.75, car_fbm(q * 2.5, fw * 2.5, seed + 21u)) * (0.4 + 0.6 * (1.0 - up));
        let scratch = car_scratches(car_plane(q, nl), fw, clamp(cm.wear.y * scratched_area * 1.6, 0.0, 1.0), seed);
        // Most scratches only cut the coat: a lighter, rough line. Deep ones reach primer.
        s.base = mix(s.base, s.base * 0.6 + vec3<f32>(0.25), scratch * 0.5);
        s.coat *= 1.0 - scratch * 0.8;
        s.rough = mix(s.rough, 0.6, scratch);
        primer = max(primer, scratch * step(0.85, car_hash(vec3<i32>(floor(q * 40.0)), seed + 3u).x));
        if cm.wear.z > 0.0 && s.coat > 0.0 {
            s.glint += car_swirls(q, nl, inst, s.coat_n, v, l, fw, cm.wear.z * s.coat, seed);
        }
        s.base = mix(s.base, PRIMER, primer);
        s.metal = mix(s.metal, 0.0, primer);
        s.rough = mix(s.rough, 0.7, primer);
        s.coat *= 1.0 - primer;
        s.glint *= 1.0 - primer;
        s.base = mix(s.base, BARE_STEEL, steel);
        s.metal = mix(s.metal, 1.0, steel);
        s.rough = mix(s.rough, 0.45, steel);
    }

    // ---- Rust: from bare steel, low edges, arches and creases.
    var rust = 0.0;
    if cm.grime.z > 0.0 && (kind == CAR_PAINT || kind == CAR_METAL || kind == CAR_TEXTURED) {
        let patches = car_fbm(q * 3.5 + 3.0, fw * 3.5, seed + 41u);
        let grow = cm.grime.z * (steel * 1.6 + (edge * 0.6 + crease) * (0.3 + low) + arch * (0.4 + low) * 0.9 + low * low * 0.7);
        let reach = clamp(grow * (0.2 + patches * 1.2), 0.0, 1.5);
        rust = smoothstep(0.42, 0.55, reach + (car_fbm(q * 30.0, fw * 30.0, seed + 43u) - 0.5) * 0.6);
        // Paint lifts in blisters around the rust.
        let blister = smoothstep(0.2, 0.35, reach) * (1.0 - rust);
        if kind == CAR_PAINT && blister > 0.0 {
            s.n = car_tilt(s.n, car_world_dir(inst, car_bump(q / 0.006, fw / 0.006, seed + 47u)), blister * 0.015);
            s.coat_n = s.n;
        }
        // Brown run-off below it.
        let streak = car_fbm(vec3<f32>(q.x * 45.0, q.y * 2.0, q.z * 45.0), fw * 45.0, seed + 49u);
        let stain = cm.grime.z * smoothstep(0.5, 0.8, streak) * low * (1.0 - rust) * 0.5;
        s.base = mix(s.base, s.base * vec3<f32>(0.7, 0.5, 0.35), stain);
        let tone = car_fbm(q * 90.0, fw * 90.0, seed + 45u);
        let rust_colour = mix(vec3<f32>(0.09, 0.035, 0.015), vec3<f32>(0.40, 0.14, 0.04), smoothstep(0.3, 0.75, tone));
        s.base = mix(s.base, rust_colour, rust);
        s.metal = mix(s.metal, 0.0, rust);
        s.rough = mix(s.rough, 0.9, rust);
        s.coat *= 1.0 - rust;
        s.glint *= 1.0 - rust;
        s.n = car_tilt(s.n, car_world_dir(inst, car_bump(q / 0.004, fw / 0.004, seed + 51u)), rust * 0.05);
    }

    // ---- Dust film and mud on top of everything.
    let wet = clamp(max(cm.dirt.w, light.fog.w), 0.0, 1.0);
    let streaks = car_fbm(vec3<f32>(q.x * 30.0, q.y * 1.5, q.z * 30.0), fw * 30.0, seed + 61u);
    let patchy = car_fbm(q * 4.0, fw * 4.0, seed + 63u);
    let exposure = low * 1.1 + spray * 0.8 + up * 0.35 + crease * 0.4 + 0.15;
    let dust = clamp(cm.grime.x * exposure * (0.45 + patchy * 0.7 + (streaks - 0.5) * 0.6 * (1.0 - up)), 0.0, 1.0);
    let dust_w = smoothstep(0.05, 0.8, dust);
    s.base = mix(s.base, cm.dirt.rgb, dust_w * 0.85);
    s.rough = mix(s.rough, 0.85, dust_w);
    s.metal = mix(s.metal, 0.0, dust_w);
    s.coat *= 1.0 - dust_w * 0.9;
    s.glint *= 1.0 - dust_w;
    // Mud: splatter thrown off the tyres, thick on the sills.
    let splat = car_fbm(q * 28.0, fw * 28.0, seed + 71u) + car_fbm(q * 7.0, fw * 7.0, seed + 73u) * 0.6 - 0.3;
    let m_amount = cm.grime.y * (low * 0.85 + spray * 1.3 + select(0.0, 0.4, wheel));
    let mud = smoothstep(1.0 - m_amount, 1.0 - m_amount + 0.1, splat * 0.85);
    let mud_colour = cm.dirt.rgb * mix(0.55, 0.32, wet) * vec3<f32>(1.0, 0.92, 0.82);
    s.base = mix(s.base, mud_colour, mud);
    s.rough = mix(s.rough, mix(0.9, 0.35, wet), mud);
    s.metal = mix(s.metal, 0.0, mud);
    s.coat *= 1.0 - mud;
    s.glint *= 1.0 - mud;
    if mud > 0.0 {
        s.n = car_tilt(s.n, car_world_dir(inst, car_bump(q / 0.02, fw / 0.02, seed + 75u)), mud * 0.06);
    }

    s.rough = clamp(s.rough, 0.04, 1.0);
    s.metal = clamp(s.metal, 0.0, 1.0);
    s.coat = clamp(s.coat, 0.0, 1.0);
    // Diagnostics: 20 curvature (red convex, blue concave), 21 wear (red
    // chips, green scratches/primer, blue rust), 22 grime (red dust, green mud).
    let dbg = light.sky.z;
    if dbg > 19.5 && dbg < 20.5 {
        s.debug = vec3<f32>(edge, 0.0, crease);
    } else if dbg > 20.5 && dbg < 21.5 {
        s.debug = vec3<f32>(steel, primer, rust);
    } else if dbg > 21.5 && dbg < 22.5 {
        s.debug = vec3<f32>(dust_w, mud, low * 0.3);
    }
    return s;
}
