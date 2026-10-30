// Shading for procedural terrain (procedural_terrain.rs): materials with
// both TERRAIN and SURFACE set. Vertices pack attributes into 24 bits per
// uv channel (procedural_terrain.wgsl): uv.x = rock, scree, wet; uv.y = up,
// water depth (/2.55 m), moor, snow. Texture layers from Material::layer on:
// grass, moor (moss and heather), soil, rock, gravel, mossy rock.

struct FieldAttr {
    rock: f32,
    scree: f32,
    wet: f32,
    up: f32,
    water: f32,
    moor: f32,
    snow: f32,
};

fn field_unpack(uv: vec2<f32>) -> array<f32, 7> {
    // Exact integers below 2^24; adding 0.5 would round odd values up and
    // carry a full byte into the next one.
    let a = u32(round(max(uv.x, 0.0)));
    let b = u32(round(max(uv.y, 0.0)));
    return array<f32, 7>(f32(a & 255u) / 255.0, f32((a >> 8u) & 255u) / 255.0, f32((a >> 16u) & 255u) / 255.0,
        f32(b & 255u) / 255.0, f32((b >> 8u) & 63u) / 63.0 * 2.55, f32((b >> 14u) & 31u) / 31.0, f32((b >> 19u) & 31u) / 31.0);
}

fn field_attr(s: SurfaceIn) -> FieldAttr {
    let a = field_unpack(s.uv0);
    let b = field_unpack(s.uv1);
    let c = field_unpack(s.uv2);
    var v: array<f32, 7>;
    for (var k = 0; k < 7; k++) { v[k] = a[k] * s.bary.x + b[k] * s.bary.y + c[k] * s.bary.z; }
    return FieldAttr(v[0], v[1], v[2], v[3], v[4], v[5], v[6]);
}

// Integer-hash value noise: the shared sin hash breaks up into flat
// patches kilometres from the origin.
fn field_hash(i: vec2<f32>) -> f32 {
    let h0 = (bitcast<u32>(i32(i.x)) * 0x8da6b343u) ^ (bitcast<u32>(i32(i.y)) * 0xd8163841u);
    let s = h0 * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return f32(((w >> 22u) ^ w) >> 8u) / 16777215.0;
}

// Hex tiling (Mikkelsen 2022): three samples per point from the corners of a
// triangle grid, each with its own random turn and offset, blended so no
// tile repeat lines up. `uv` is in texture tiles.
struct HexTap { uv: array<vec2<f32>, 3>, r: array<mat2x2<f32>, 3>, w: vec3<f32> }

fn hex_taps(uv: vec2<f32>) -> HexTap {
    let g = uv * 2.0;
    let sk = vec2<f32>(g.x, -0.57735027 * g.x + 1.15470054 * g.y);
    let base = floor(sk);
    let fr = sk - base;
    let fz = 1.0 - fr.x - fr.y;
    let s = select(0.0, 1.0, fz < 0.0);
    let s2 = 2.0 * s - 1.0;
    var w = vec3<f32>(-fz * s2, s - fr.y * s2, s - fr.x * s2);
    let v = array<vec2<f32>, 3>(base + vec2<f32>(s, s), base + vec2<f32>(s, 1.0 - s), base + vec2<f32>(1.0 - s, s));
    var t: HexTap;
    for (var i = 0; i < 3; i++) {
        let a = field_hash(v[i] + 91.0) * 6.2831853;
        let c = cos(a);
        let sn = sin(a);
        t.r[i] = mat2x2<f32>(vec2<f32>(c, sn), vec2<f32>(-sn, c));
        t.uv[i] = t.r[i] * uv + vec2<f32>(field_hash(v[i] + 13.0), field_hash(v[i] + 57.0)) * 7.0;
    }
    w = w * w * w;
    t.w = w / (w.x + w.y + w.z);
    return t;
}

fn hex_tex(l: i32, uv: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>) -> vec3<f32> {
    let t = hex_taps(uv);
    var c = vec3<f32>(0.0);
    for (var i = 0; i < 3; i++) { c += tex_w(l, t.uv[i], t.r[i] * dx, t.r[i] * dy).rgb * t.w[i]; }
    return c;
}

fn hex_nrm(l: i32, uv: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>) -> vec2<f32> {
    let t = hex_taps(uv);
    var g = vec2<f32>(0.0);
    for (var i = 0; i < 3; i++) { g += transpose(t.r[i]) * nrm(l, t.uv[i], t.r[i] * dx, t.r[i] * dy) * t.w[i]; }
    return g;
}

// Triplanar with hex tiling on each plane; planes with little weight are skipped.
fn hex_triplanar(l: i32, p: vec3<f32>, tw: vec3<f32>, scale: f32) -> vec3<f32> {
    let k = 1.0 / scale;
    var c = vec3<f32>(0.0);
    if tw.x > 0.02 { c += hex_tex(l, p.zy * k, fp.dx.zy * k, fp.dy.zy * k) * tw.x; }
    if tw.y > 0.02 { c += hex_tex(l, p.xz * k, fp.dx.xz * k, fp.dy.xz * k) * tw.y; }
    if tw.z > 0.02 { c += hex_tex(l, p.xy * k, fp.dx.xy * k, fp.dy.xy * k) * tw.z; }
    return c / max(dot(tw, vec3<f32>(f32(tw.x > 0.02), f32(tw.y > 0.02), f32(tw.z > 0.02))), 1e-4);
}

fn hex_triplanar_n(l: i32, p: vec3<f32>, n: vec3<f32>, tw: vec3<f32>, scale: f32) -> vec3<f32> {
    let k = 1.0 / scale;
    var acc = vec3<f32>(0.0);
    if tw.x > 0.02 { acc += perturb(n, hex_nrm(l, p.zy * k, fp.dx.zy * k, fp.dy.zy * k), vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, -1.0, 0.0)) * tw.x; }
    if tw.y > 0.02 { acc += perturb(n, hex_nrm(l, p.xz * k, fp.dx.xz * k, fp.dy.xz * k), vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, -1.0)) * tw.y; }
    if tw.z > 0.02 { acc += perturb(n, hex_nrm(l, p.xy * k, fp.dx.xy * k, fp.dy.xy * k), vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, -1.0, 0.0)) * tw.z; }
    return safe_normalize(acc);
}

fn field_vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(field_hash(i), field_hash(i + vec2<f32>(1.0, 0.0)), u.x), mix(field_hash(i + vec2<f32>(0.0, 1.0)), field_hash(i + vec2<f32>(1.0, 1.0)), u.x), u.y);
}

// Three octaves, each faded to its mean once it is smaller than a pixel.
fn field_fbm(p: vec2<f32>, footprint: f32) -> f32 {
    var v = 0.0;
    var a = 0.55;
    var q = p;
    var fw = footprint;
    for (var k = 0; k < 3; k++) {
        let w = clamp(1.5 - 2.0 * fw, 0.0, 1.0);
        v += a * mix(0.5, field_vnoise(q), w);
        q = q * 2.1 + vec2<f32>(17.3, 5.9);
        fw *= 2.1;
        a = select(0.15, 0.3, k == 0);
    }
    return v;
}

// The terrain function's gradient noise, as terrain_field.wgsl (same hash,
// so detail lines up with the geometry it stands in for).
fn fs_hash(ix: i32, iz: i32, seed: u32) -> u32 {
    let h = (bitcast<u32>(ix) * 0x8da6b343u) ^ (bitcast<u32>(iz) * 0xd8163841u) ^ (seed * 0xcb1ab31fu);
    let s = h * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}

fn fs_grad(ix: i32, iz: i32, seed: u32) -> vec2<f32> {
    let h = fs_hash(ix, iz, seed);
    return vec2<f32>(f32(h & 0xffffu) / 32767.5 - 1.0, f32(h >> 16u) / 32767.5 - 1.0);
}

fn fs_noised(p: vec2<f32>, seed: u32) -> vec3<f32> {
    let fl = floor(p);
    let ix = i32(fl.x);
    let iz = i32(fl.y);
    let f = p - fl;
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let du = 30.0 * f * f * (f * (f - 2.0) + 1.0);
    let ga = fs_grad(ix, iz, seed);
    let gb = fs_grad(ix + 1, iz, seed);
    let gc = fs_grad(ix, iz + 1, seed);
    let gd = fs_grad(ix + 1, iz + 1, seed);
    let va = ga.x * f.x + ga.y * f.y;
    let vb = gb.x * (f.x - 1.0) + gb.y * f.y;
    let vc = gc.x * f.x + gc.y * (f.y - 1.0);
    let vd = gd.x * (f.x - 1.0) + gd.y * (f.y - 1.0);
    let k = va - vb - vc + vd;
    let v = va + u.x * (vb - va) + u.y * (vc - va) + u.x * u.y * k;
    let dx = ga.x + u.x * (gb.x - ga.x) + u.y * (gc.x - ga.x) + u.x * u.y * (ga.x - gb.x - gc.x + gd.x) + du.x * (u.y * k + vb - va);
    let dz = ga.y + u.x * (gb.y - ga.y) + u.y * (gc.y - ga.y) + u.x * u.y * (ga.y - gb.y - gc.y + gd.y) + du.y * (u.x * k + vc - va);
    return vec3<f32>(v, dx, dz);
}

// Slope (d/dx, d/dz) of the terrain function's fine roughness octaves that
// the mesh is too coarse to carry but a pixel can still show: the same
// octaves terrain_field.wgsl adds as geometry closer in. The mesh cell is
// taken as ~24 pixels (procedural_terrain::Settings::cell_pixels).
fn field_detail_slope(xz: vec2<f32>, seed: u32, footprint: f32, rock: f32, up: f32, moor: f32) -> vec2<f32> {
    let p = xz / 2048.0;
    let warp = vec2<f32>(fs_noised(p * 1.5 + vec2<f32>(5.2, 1.3), seed + 3u).x, fs_noised(p * 1.5 + vec2<f32>(1.7, 9.2), seed + 3u).x);
    var q = p + warp * 0.12;
    let r = mat2x2<f32>(vec2<f32>(1.6, 1.2), vec2<f32>(-1.2, 1.6));
    var j = mat2x2<f32>(vec2<f32>(1.0 / 2048.0, 0.0), vec2<f32>(0.0, 1.0 / 2048.0));
    for (var i = 0; i < 7; i++) { q = r * q; j = r * j; }
    let cut = 24.0 * footprint;
    var wl = 16.0;
    var da = 0.8 * mix(0.6 + 0.8 * smoothstep(0.97, 0.8, up) + 0.3 * moor, 2.5, rock);
    var g = vec2<f32>(0.0);
    for (var i = 7u; i < 17u; i++) {
        let w_pix = clamp(wl / (4.0 * footprint) - 1.0, 0.0, 1.0);
        if w_pix <= 0.0 { break; }
        let w = w_pix * (1.0 - clamp(wl / (2.0 * cut) - 1.0, 0.0, 1.0));
        if w > 0.0 {
            let n = fs_noised(q, seed + 10u + i);
            let dq = mix(n.yz, -sign(n.x) * n.yz, rock);
            g += transpose(j) * dq * (da * 0.6 * w);
        }
        da *= 0.6597539;
        wl *= 0.5;
        q = r * q;
        j = r * j;
    }
    return g;
}

// The forest where single trees end, as forest_field.wgsl places them
// (same stand noise, chance and ground rules; this one reads the ground from
// the vertex attributes, so it needs no terrain evaluation). The material's
// scale carries it (procedural_terrain::far_forest): whole metres from the
// camera where it starts, the fraction the tree density; 4.0 means none.
// A forest seen from afar: dark crowns over the ground between them, more
// of them towards a glancing view, lumpy and shadowed on the side away from
// the sun.
fn far_forest(ground: vec3<f32>, nn: vec3<f32>, n: vec3<f32>, s: SurfaceIn, f: FieldAttr, footprint: f32) -> MeadowCanopy {
    let density = fract(s.scale);
    if density <= 0.0 { return MeadowCanopy(ground, nn); }
    let start = floor(s.scale);
    // Horizontal: the trees stand in a disc around the camera at any height.
    let fade = smoothstep(start - 150.0, start, distance(s.p.xz, view.cam.xz));
    if fade <= 0.0 { return MeadowCanopy(ground, nn); }
    let seed = u32(s.metal + 0.5);
    let stand = fs_noised(s.p.xz / 300.0, seed + 77u).x;
    let grows = (1.0 - smoothstep(0.25, 0.35, f.rock)) * (1.0 - smoothstep(0.25, 0.35, f.scree))
        * (1.0 - smoothstep(0.005, 0.015, f.water)) * (1.0 - smoothstep(0.03, 0.07, f.snow)) * smoothstep(0.8, 0.84, f.up);
    let chance = clamp((stand + 0.15) * 2.0, 0.0, 1.0) * density * grows;
    if chance <= 0.0 { return MeadowCanopy(ground, nn); }
    let tone = clamp(stand * 0.5 + 0.5, 0.0, 1.0);
    // A crown covers about 0.4 of its 8 m cell from above; between crowns
    // the ground lies mostly in their shade.
    let ndv = clamp(dot(n, -s.dir), 0.05, 1.0);
    let cover = 1.0 - exp(-chance * 0.5 / ndv);
    let floor = ground * mix(1.0, 0.35, smoothstep(0.0, 0.3, chance));
    // Crowns a few metres across while a pixel can still show them.
    let lumps_w = 1.0 - smoothstep(1.5, 6.0, footprint);
    let lump = fs_noised(s.p.xz / 3.5, seed + 91u);
    let crown_n = normalize(vec3<f32>(-lump.y * 0.6 * lumps_w, 1.0, -lump.z * 0.6 * lumps_w));
    let crown = vec3<f32>(0.045, 0.085, 0.035) * (0.8 + 0.4 * tone) * (0.85 + 0.3 * lump.x * lumps_w);
    let forest = mix(floor, crown, cover);
    return MeadowCanopy(mix(ground, forest, fade), normalize(mix(nn, crown_n, cover * fade)));
}

fn field_surface(s: SurfaceIn, layer: i32) -> SurfaceOut {
    let f = field_attr(s);
    let p = s.p;
    let n = s.n;
    let wuv = p.xz;
    let mapping = terrain_mapping(p);
    // Breakup noise, metres: large patches and small mottling.
    let footprint = max(length(s.dx), length(s.dy));
    let broad = field_fbm(wuv / 37.0 + 4.0, footprint / 37.0);
    let mottle = field_fbm(wuv / 4.3 + 11.0, footprint / 4.3);

    // Ground cover on gentle soil: grass, with moss and heather on moor.
    // The grass texture is dry; tint it to wet upland pasture, lusher in
    // hollows and near water, paler on dry patches.
    let lush = clamp(0.5 + (broad - 0.5) * 1.2 + 0.4 * f.wet, 0.0, 1.0);
    // Darker than a lawn: seen from afar a meadow is mostly blade shadow,
    // so the ground matches the grass where the blades fade out.
    let grass = terrain_layer(layer, wuv, 2.6, mapping) * mix(vec3<f32>(0.43, 0.55, 0.23), vec3<f32>(0.32, 0.69, 0.28), lush);
    // Moor: dark olive heather and bracken with grass between.
    let moor = mix(terrain_layer(layer + 1, wuv, 3.2, mapping) * vec3<f32>(0.6, 0.72, 0.7), grass * 0.8, 0.3);
    let soil = terrain_layer(layer + 2, wuv, 3.0, mapping) * vec3<f32>(0.85, 0.8, 0.75);
    let m_moor = smoothstep(0.25, 0.8, clamp(f.moor + (broad - 0.5) * 0.7 + (mottle - 0.5) * 0.3, 0.0, 1.0));
    var c = mix(grass, moor, m_moor);
    var nt = mix(terrain_layer_normal(layer, wuv, 2.6, mapping), terrain_layer_normal(layer + 1, wuv, 3.2, mapping), m_moor);
    // Worn soil only on steep banks and the odd bare patch.
    let m_soil = clamp(smoothstep(0.9, 0.75, f.up) * 0.35 + smoothstep(0.7, 0.85, mottle) * 0.25, 0.0, 1.0);
    c = mix(c, soil, m_soil);
    nt = mix(nt, terrain_layer_normal(layer + 2, wuv, 3.0, mapping), m_soil);
    var nn = perturb_xz(n, nt);
    c *= 0.82 + 0.3 * broad;

    // Past the meadow's blades the ground reads as a grass canopy.
    let canopy = meadow_canopy(c, nn, n, s.dir, wuv, footprint, s.time, u32(s.metal + 0.5),
        (1.0 - m_soil) * (1.0 - 0.5 * m_moor) * smoothstep(0.75, 0.9, f.up));
    c = canopy.albedo;
    nn = canopy.normal;

    // Past the trees the forest is part of the ground (forest_field).
    let forest = far_forest(c, nn, n, s, f, footprint);
    c = forest.albedo;
    nn = forest.normal;

    // Loose stone.
    let m_scree = smoothstep(0.2, 0.7, f.scree + (mottle - 0.5) * 0.4);
    // Mountain stone is grey limestone and gneiss: the textures are warm, so
    // most of their colour goes.
    let gravel0 = terrain_layer(layer + 4, wuv, 1.8, mapping);
    let gravel = mix(vec3<f32>(dot(gravel0, vec3<f32>(0.3, 0.59, 0.11))), gravel0, 0.5) * vec3<f32>(0.95, 0.97, 1.0);
    c = mix(c, gravel, m_scree);
    nn = normalize(mix(nn, perturb_xz(n, terrain_layer_normal(layer + 4, wuv, 1.8, mapping)), m_scree));

    // Bedrock: triplanar, mossy where it faces up.
    let m_rock = smoothstep(0.25, 0.75, f.rock + (mottle - 0.5) * 0.35);
    if m_rock > 0.0 {
        var tw = abs(n);
        tw = tw * tw * tw * tw;
        tw /= tw.x + tw.y + tw.z;
        let rot = mat2x2<f32>(vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0));
        // Hex tiling: every few metres a new random turn, so no repeat shows.
        let bare0 = hex_triplanar(layer + 3, p, tw, 7.0);
        let bare = mix(vec3<f32>(dot(bare0, vec3<f32>(0.3, 0.59, 0.11))), bare0, 0.35) * vec3<f32>(1.0, 1.04, 1.1) * (0.85 + 0.3 * broad);
        let mossy = triplanar(layer + 5, p, tw, 3.5, rot, 0.37);
        let m_moss = smoothstep(0.55, 0.9, n.y) * (0.5 + 0.5 * f.moor) * smoothstep(0.3, 0.7, broad);
        let rc = mix(bare, mossy, m_moss);
        let bare_n = hex_triplanar_n(layer + 3, p, n, tw, 7.0);
        let rn = normalize(mix(bare_n, triplanar_n(layer + 5, p, n, tw, 3.5, rot, 0.37), m_moss));
        c = mix(c, rc, m_rock);
        nn = normalize(mix(nn, rn, m_rock));
    }

    // Roughness finer than the mesh, from the terrain function itself (the
    // material's metal carries the seed: terrain is never metal).
    let ds = field_detail_slope(p.xz, u32(s.metal + 0.5), footprint, f.rock, f.up, f.moor);
    nn = normalize(nn + vec3<f32>(-ds.x, 0.0, -ds.y));

    // Damp ground darkens near water.
    c *= 1.0 - 0.35 * f.wet * (1.0 - m_rock * 0.5);
    var rough = mix(0.95, 0.55, f.wet * 0.6);

    // Snow over everything but water: bright, fairly smooth, thinning into
    // patches at its edge.
    let m_snow = smoothstep(0.3, 0.7, f.snow + (mottle - 0.5) * 0.5 + (broad - 0.5) * 0.3);
    if m_snow > 0.0 {
        c = mix(c, vec3<f32>(0.84, 0.87, 0.92) * (0.93 + 0.1 * mottle), m_snow);
        nn = normalize(mix(nn, n, m_snow * 0.8));
        rough = mix(rough, 0.6, m_snow);
    }

    // Water: a dark, smooth, level surface, clearer where shallow.
    if f.water > 0.01 {
        let depth = clamp(f.water / 0.8, 0.0, 1.0);
        let w = smoothstep(0.0, 0.06, f.water);
        let ripple = vec2<f32>(field_fbm(wuv * 1.7 + vec2<f32>(s.time * 0.31, 0.0), footprint * 1.7), field_fbm(wuv * 1.7 + vec2<f32>(5.0, s.time * 0.27), footprint * 1.7)) - 0.5;
        let wn = normalize(vec3<f32>(ripple.x * 0.12, 1.0, ripple.y * 0.12));
        c = mix(c, mix(c * 0.45, vec3<f32>(0.018, 0.03, 0.028), depth), w);
        nn = normalize(mix(nn, wn, w));
        rough = mix(rough, 0.04, w);
    }
    return SurfaceOut(c * s.tint, nn, rough, 0.0, vec3<f32>(0.0));
}
