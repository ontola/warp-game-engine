// Shading for procedural terrain (procedural_terrain.rs): materials with
// both TERRAIN and SURFACE set. Vertices pack attributes into 24 bits per
// uv channel (procedural_terrain.wgsl): uv.x = rock, scree, wet; uv.y = up,
// water depth (square root of /2.55 m), moor, snow. Texture layers from Material::layer on:
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

// Depth (metres) at which water covers the ground; procedural_terrain.wgsl
// writes thin water as this times its share of the cell.
const WATER_FILM: f32 = 0.04;

fn field_unpack(uv: vec2<f32>) -> array<f32, 7> {
    // Exact integers below 2^24; adding 0.5 would round odd values up and
    // carry a full byte into the next one.
    let a = u32(round(max(uv.x, 0.0)));
    let b = u32(round(max(uv.y, 0.0)));
    let w = f32((b >> 8u) & 63u) / 63.0;
    return array<f32, 7>(f32(a & 255u) / 255.0, f32((a >> 8u) & 255u) / 255.0, f32((a >> 16u) & 255u) / 255.0,
        f32(b & 255u) / 255.0, w * w * 2.55, f32((b >> 14u) & 31u) / 31.0, f32((b >> 19u) & 31u) / 31.0);
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

// The forest where single trees end, as forest_field.wgsl places them:
// its canopy (crowns from above, their mean once subpixel) in the game's
// ForestStyle. This reads the ground from the vertex attributes, so it needs
// no terrain evaluation. procedural_terrain's style block holds the style in
// data materials the terrain material's scale points at: -(first + 1); a
// scale >= 0 means no forest (and the default ground, field_ground).
// start..end: the handover band from drawn trees to the layer, measured
// from `centre` (where the trees were placed) when `centred`, else the camera.
struct FarForest { style: ForestStyle, start: f32, density: f32, end: f32, centre: vec2<f32>, centred: bool };
fn far_forest_params(at: u32) -> FarForest {
    let a = materials[at]; let b = materials[at + 1u]; let c = materials[at + 2u];
    let fs = ForestStyle(vec4<f32>(a.tint, bitcast<f32>(a.layer)), vec4<f32>(a.roughness, bitcast<f32>(a.flags), a.scale, a.metal),
        vec4<f32>(b.tint, bitcast<f32>(b.layer)), vec4<f32>(b.roughness, bitcast<f32>(b.flags), b.scale, b.metal));
    return FarForest(fs, c.tint.x, c.tint.y, c.scale, vec2<f32>(c.tint.z, c.roughness), (c.flags & 1u) != 0u);
}
fn far_forest(ground: vec3<f32>, nn: vec3<f32>, n: vec3<f32>, s: SurfaceIn, f: FieldAttr, footprint: f32) -> MeadowCanopy {
    if s.scale >= 0.0 { return MeadowCanopy(ground, nn); }
    let params = far_forest_params(u32(-s.scale - 0.5));
    if params.density <= 0.0 { return MeadowCanopy(ground, nn); }
    // Horizontal: the trees stand in a disc around where they were placed
    // (or the camera) at any height. In the band the layer takes exactly the
    // share of trees no longer drawn (forest_field::handover).
    let origin = select(view.cam.xz, params.centre, params.centred);
    let fade = smoothstep(params.start, params.end, distance(s.p.xz, origin));
    if fade <= 0.0 { return MeadowCanopy(ground, nn); }
    // forest_field ff_grows from the vertex attributes, with a pixel of
    // slack so stand edges stay crisp without aliasing.
    let e = clamp(footprint * 0.02, 0.005, 0.05);
    let grows = (1.0 - smoothstep(0.3 - e, 0.3 + e, f.rock)) * (1.0 - smoothstep(0.3 - e, 0.3 + e, f.scree))
        * (1.0 - smoothstep(0.01 - e * 0.2, 0.01 + e * 0.2, f.water)) * (1.0 - smoothstep(0.05 - e * 0.5, 0.05 + e * 0.5, f.snow))
        * smoothstep(0.82 - e, 0.82 + e, f.up);
    let seed = u32(s.metal + 0.5);
    let edge_w = max(0.04, footprint / 150.0);
    // forest_field's terrain-aware stands: forest on slopes, meadow on flat
    // and damp ground (the shadow taps assume this pixel's ground).
    let site = ff_site(f.up, f.wet);
    let stand = far_forest_stand(s.p.xz, seed, site, footprint, edge_w);
    // A stand is a wall of crowns: towards the sun from a clearing, any
    // stand closer than its height over the sun's slope shades this ground.
    let sun = light.sun_dir.xyz;
    let sun_xz = length(sun.xz);
    var shade = 0.0;
    if sun.y > 0.02 && sun_xz > 1e-3 && stand < 1.0 {
        let reach = min(params.style.far.w * 1.3 * sun_xz / sun.y, 160.0);
        let toward = sun.xz / sun_xz;
        // Crowns differ in height, so the shadow fades out over the last
        // part of its reach instead of ending in steps one tap apart.
        for (var k = 1; k <= 8; k++) {
            let t = f32(k) * 0.125;
            let tall = 1.0 - smoothstep(0.5, 1.0, t);
            shade = max(shade, tall * far_forest_stand(s.p.xz + toward * reach * t, seed, site, footprint, edge_w));
        }
    }
    let shaded = ground * mix(1.0, 0.45, shade * (1.0 - stand) * fade);
    if grows <= 0.0 { return MeadowCanopy(shaded, nn); }
    // The canopy stands a tree height above the ground: the view ray meets
    // the crown tops before this ground point, at q. From q on towards p it
    // runs under the crowns, so a stand it crosses on the way is the wall
    // of its edge (hiding the clearing behind it), and the tops shift with
    // their height (parallax). Looking down steeply q is p.
    let eye = normalize(view.cam.xyz - s.p);
    let rise = max(dot(eye, n), 0.0);
    let height = params.style.far.w;
    let reach = min(height * length(eye - n * dot(eye, n)) / max(rise, 0.05), 250.0);
    let back = normalize(eye.xz + vec2<f32>(1e-6, 0.0));
    let q = s.p.xz + back * reach;
    var seen = 1.0;
    var top = 0.0;
    var wall = 0.0;
    let taps = select(1, 6, reach > footprint);
    for (var k = 0; k < taps; k++) {
        let t = select(0.0, f32(k) / f32(taps - 1), taps > 1);
        let a = far_forest_stand(mix(q, s.p.xz, t), seed, site, footprint, edge_w);
        if k == 0 { top = seen * a; } else { wall += seen * a; }
        seen *= 1.0 - a;
    }
    let w_top = grows * fade * top;
    let w_wall = grows * fade * wall;
    if w_top + w_wall <= 0.0 { return MeadowCanopy(shaded, nn); }
    let canopy = forest_canopy(q.x, q.y, footprint, sun, eye, seed, params.density, site, params.style);
    let crowns = canopy.albedo * canopy.occlusion;
    // The wall: crowns' flanks over shaded trunks, facing the viewer.
    let wall_n = normalize(vec3<f32>(eye.x, 0.15, eye.z));
    let flank = mix(params.style.floor.rgb, canopy.albedo, 0.5) * 0.55;
    var c = mix(shaded, crowns, w_top);
    c = mix(c, flank, w_wall / max(1.0 - w_top, 1e-3));
    let nrm = normalize(mix(mix(nn, canopy.normal, w_top), wall_n, w_wall));
    let crowd = ff_share(q.x, q.y, seed, site, footprint) * params.density;
    let sun_grow = ff_crown_growth(sun, params.style);
    // Seen from the sun's side the crowns hide their own shadows (the hot
    // spot); against the sun every shadow shows.
    let hidden = clamp(dot(eye, sun), 0.0, 1.0);
    field_sun = mix(1.0, exp(-crowd * params.style.crown.y * sun_grow * sun_grow), min(w_top + w_wall, 1.0) * (1.0 - hidden));
    return MeadowCanopy(c, nrm);
}

// Whether (x, z) lies in a stand or hedgerow on ground of `site`, 0..1
// with a crisp edge `edge_w` wide; the clearings between stands keep their
// ground (their loose trees are too sparse to draw).
fn far_forest_stand(xz: vec2<f32>, seed: u32, site: f32, footprint: f32, edge_w: f32) -> f32 {
    return smoothstep(0.25 - edge_w, 0.25 + edge_w, ff_share(xz.x, xz.y, seed, site, footprint));
}

// The ground's palette (vegetation_style.rs GroundStyle): tint rgb, and per
// layer w: grass_dry.w moor's grass share, moor.w its grass darkening,
// soil.w damp darkening, gravel.w and rock.w the saturation kept.
struct FieldGround {
    grass_dry: vec4<f32>,
    grass_lush: vec4<f32>,
    moor: vec4<f32>,
    soil: vec4<f32>,
    gravel: vec4<f32>,
    rock: vec4<f32>,
    snow: vec4<f32>,
    reserved: vec4<f32>,
};
// Direct sun the painted forest keeps (shade.wgsl's sun term). Past the
// sun maps a drawn forest shades its ground through the GI canopy volume
// (distant_sun_visibility); the volume does not hold the painted forest, so
// it stood ~20 % brighter than the drawn one where it took over. Its stands
// keep the share of ground no crown shades from the sun (forest_field's
// overlap model, crowns seen from the sun) that the view can see. 1 off the
// forest.
var<private> field_sun: f32 = 1.0;
const FIELD_GROUND_DEFAULT = FieldGround(vec4<f32>(0.43, 0.55, 0.23, 0.3), vec4<f32>(0.32, 0.69, 0.28, 0.0),
    vec4<f32>(0.6, 0.72, 0.7, 0.8), vec4<f32>(0.85, 0.8, 0.75, 0.35), vec4<f32>(0.95, 0.97, 1.0, 0.5),
    vec4<f32>(1.0, 1.04, 1.1, 0.35), vec4<f32>(0.84, 0.87, 0.92, 0.0), vec4<f32>(0.0, 0.0, 0.0, 0.0));
fn field_material_pair(m: Material) -> array<vec4<f32>, 2> {
    return array<vec4<f32>, 2>(vec4<f32>(m.tint, bitcast<f32>(m.layer)), vec4<f32>(m.roughness, bitcast<f32>(m.flags), m.scale, m.metal));
}
fn field_ground_at(at: u32) -> FieldGround {
    let a = field_material_pair(materials[at]);
    let b = field_material_pair(materials[at + 1u]);
    let c = field_material_pair(materials[at + 2u]);
    let d = field_material_pair(materials[at + 3u]);
    return FieldGround(a[0], a[1], b[0], b[1], c[0], c[1], d[0], d[1]);
}

fn field_ground_add(acc: FieldGround, g: FieldGround, w: f32) -> FieldGround {
    return FieldGround(acc.grass_dry + g.grass_dry * w, acc.grass_lush + g.grass_lush * w, acc.moor + g.moor * w, acc.soil + g.soil * w,
        acc.gravel + g.gravel * w, acc.rock + g.rock * w, acc.snow + g.snow * w, acc.reserved + g.reserved * w);
}

// Ground colours per region (procedural_terrain.rs region_ground,
// regions.rs surface_words): material `base` holds the grid (origin, cell,
// nodes across, kinds), then each kind's GroundStyle (4 materials), then
// the nodes two to a material, each four slots of id * 256 + weight. The
// pixel mixes the kinds of its four surrounding nodes, as the terrain's
// shape does (regions.rs style_at), each kind read once.
fn region_ground(p: vec2<f32>, base: u32) -> FieldGround {
    let h = field_material_pair(materials[base]);
    let sx = u32(h[0].w);
    let sz = u32(h[1].x);
    let kinds = u32(h[1].y);
    let fx = clamp((p.x - h[0].x) / h[0].z, 0.0, f32(sx - 1u));
    let fz = clamp((p.y - h[0].y) / h[0].z, 0.0, f32(sz - 1u));
    let i0 = min(u32(floor(fx)), max(sx, 2u) - 2u);
    let j0 = min(u32(floor(fz)), max(sz, 2u) - 2u);
    let i1 = min(i0 + 1u, sx - 1u);
    let j1 = min(j0 + 1u, sz - 1u);
    let tx = fx - f32(i0);
    let tz = fz - f32(j0);
    let nodes = base + 1u + kinds * 4u;
    var at = array<u32, 4>(j0 * sx + i0, j0 * sx + i1, j1 * sx + i0, j1 * sx + i1);
    var f = array<f32, 4>((1.0 - tx) * (1.0 - tz), tx * (1.0 - tz), (1.0 - tx) * tz, tx * tz);
    var ids: array<u32, 16>;
    var ws: array<f32, 16>;
    var count = 0u;
    var sum = 0.0;
    for (var c = 0u; c < 4u; c++) {
        var pair = field_material_pair(materials[nodes + at[c] / 2u]);
        let slots = pair[at[c] % 2u];
        for (var k = 0u; k < 4u; k++) {
            let word = u32(slots[k]);
            let w = f32(word & 255u) / 255.0 * f[c];
            if w <= 0.0 { continue; }
            let id = word >> 8u;
            var found = false;
            for (var e = 0u; e < count; e++) {
                if ids[e] == id { ws[e] += w; found = true; }
            }
            if !found { ids[count] = id; ws[count] = w; count++; }
            sum += w;
        }
    }
    var g = FieldGround(vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0));
    for (var e = 0u; e < count; e++) {
        g = field_ground_add(g, field_ground_at(base + 1u + min(ids[e], kinds - 1u) * 4u), ws[e] / sum);
    }
    return g;
}

// The style block's GroundStyle, after its ForestStyle and far forest
// params; per region when its reserved.x points at region colours.
fn field_ground(s: SurfaceIn) -> FieldGround {
    if s.scale >= 0.0 { return FIELD_GROUND_DEFAULT; }
    let g = field_ground_at(u32(-s.scale - 0.5) + 3u);
    if g.reserved.x >= 0.5 { return region_ground(s.p.xz, u32(g.reserved.x - 0.5)); }
    return g;
}

// Roads drawn into the pixels (procedural_terrain.rs road_surface,
// world_constraints.rs surface_words): data materials holding f32 words; the
// style block's far forest params' metal points at them (first + 1).
fn road_word(base: u32, i: u32) -> f32 {
    let m = materials[base + i / 8u];
    switch i % 8u {
        case 0u: { return m.tint.x; }
        case 1u: { return m.tint.y; }
        case 2u: { return m.tint.z; }
        case 3u: { return bitcast<f32>(m.layer); }
        case 4u: { return m.roughness; }
        case 5u: { return bitcast<f32>(m.flags); }
        case 6u: { return m.scale; }
        default: { return m.metal; }
    }
}

// The nearest road to a pixel: edge = metres past its edge (negative on
// it), u = across it, -1..1 edge to edge (right of travel positive).
struct RoadPixel { found: bool, edge: f32, u: f32, half: f32, along: f32 };
fn road_at(xz: vec2<f32>, scale: f32) -> RoadPixel {
    var r = RoadPixel(false, 1e9, 0.0, 1.0, 0.0);
    if scale >= 0.0 { return r; }
    let ptr = materials[u32(-scale - 0.5) + 2u].metal;
    if ptr < 0.5 { return r; }
    let base = u32(ptr - 0.5);
    let cell = road_word(base, 2u);
    let fi = floor((xz.x - road_word(base, 0u)) / cell);
    let fj = floor((xz.y - road_word(base, 1u)) / cell);
    let sx = road_word(base, 3u);
    if fi < 0.0 || fj < 0.0 || fi >= sx || fj >= road_word(base, 4u) { return r; }
    let c = u32(fj * sx + fi);
    let cells = u32(road_word(base, 5u));
    let items = u32(road_word(base, 6u));
    let first = u32(road_word(base, cells + c));
    let end = u32(road_word(base, cells + c + 1u));
    var best = 1e9;
    for (var k = first; k < end; k++) {
        let g = 8u + u32(road_word(base, items + k)) * 8u;
        let a = vec2<f32>(road_word(base, g), road_word(base, g + 1u));
        let ab = vec2<f32>(road_word(base, g + 2u), road_word(base, g + 3u)) - a;
        let t = clamp(dot(xz - a, ab) / max(dot(ab, ab), 1e-6), 0.0, 1.0);
        let d = distance(a + ab * t, xz);
        let half = mix(road_word(base, g + 4u), road_word(base, g + 5u), t);
        if d - half < best {
            best = d - half;
            let cross = ab.x * (xz.y - a.y) - ab.y * (xz.x - a.x);
            r = RoadPixel(true, d - half, select(d, -d, cross > 0.0) / half, half, mix(road_word(base, g + 6u), road_word(base, g + 7u), t));
        }
    }
    return r;
}

fn field_surface(s: SurfaceIn, layer: i32) -> SurfaceOut {
    let f = field_attr(s);
    let g = field_ground(s);
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
    let grass = terrain_layer(layer, wuv, 2.6, mapping) * mix(g.grass_dry.rgb, g.grass_lush.rgb, lush);
    // Layers a pixel holds none of are not sampled (moor, soil, gravel):
    // their weights come first.
    let m_moor = smoothstep(0.25, 0.8, clamp(f.moor + (broad - 0.5) * 0.7 + (mottle - 0.5) * 0.3, 0.0, 1.0));
    // Worn soil only on steep banks and the odd bare patch.
    let m_soil = clamp(smoothstep(0.9, 0.75, f.up) * 0.35 + smoothstep(0.7, 0.85, mottle) * 0.25, 0.0, 1.0);
    let road = road_at(p.xz, s.scale);
    let on_road = road.found && road.edge < 4.0;
    // Moor: dark olive heather and bracken with grass between.
    var moor = grass;
    var nt = vec2<f32>(0.0);
    if m_moor < 1.0 { nt = terrain_layer_normal(layer, wuv, 2.6, mapping); }
    if m_moor > 0.0 {
        moor = mix(terrain_layer(layer + 1, wuv, 3.2, mapping) * g.moor.rgb, grass * g.moor.w, g.grass_dry.w);
        nt = mix(nt, terrain_layer_normal(layer + 1, wuv, 3.2, mapping), m_moor);
    }
    var soil = vec3<f32>(0.0);
    if m_soil > 0.0 || on_road { soil = terrain_layer(layer + 2, wuv, 3.0, mapping) * g.soil.rgb; }
    var c = mix(grass, moor, m_moor);
    c = mix(c, soil, m_soil);
    if m_soil > 0.0 { nt = mix(nt, terrain_layer_normal(layer + 2, wuv, 3.0, mapping), m_soil); }
    var nn = perturb_xz(n, nt);
    c *= 0.82 + 0.3 * broad;

    // Past the meadow's blades the ground reads as a grass canopy.
    let canopy = meadow_canopy(c, nn, n, s.dir, wuv, footprint, s.time, u32(s.metal + 0.5),
        (1.0 - m_soil) * (1.0 - 0.5 * m_moor) * smoothstep(0.75, 0.9, f.up));
    c = canopy.albedo;
    nn = canopy.normal;

    // Past the trees the forest is part of the ground (forest_field).
    // At a glancing view a pixel is long and narrow: the forest filters by
    // its mean width, so clumps and gaps survive towards the horizon.
    let forest = far_forest(c, nn, n, s, f, sqrt(length(s.dx) * length(s.dy)));
    c = forest.albedo;
    nn = forest.normal;

    // Loose stone.
    let m_scree = smoothstep(0.2, 0.7, f.scree + (mottle - 0.5) * 0.4);
    // Mountain stone is grey limestone and gneiss: the textures are warm, so
    // most of their colour goes.
    if m_scree > 0.0 {
        let gravel0 = terrain_layer(layer + 4, wuv, 1.8, mapping);
        let gravel = mix(vec3<f32>(dot(gravel0, vec3<f32>(0.3, 0.59, 0.11))), gravel0, g.gravel.w) * g.gravel.rgb;
        c = mix(c, gravel, m_scree);
        nn = normalize(mix(nn, perturb_xz(n, terrain_layer_normal(layer + 4, wuv, 1.8, mapping)), m_scree));
    }

    // Bedrock: triplanar, mossy where it faces up.
    let m_rock = smoothstep(0.25, 0.75, f.rock + (mottle - 0.5) * 0.35);
    if m_rock > 0.0 {
        var tw = abs(n);
        tw = tw * tw * tw * tw;
        tw /= tw.x + tw.y + tw.z;
        let rot = mat2x2<f32>(vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0));
        // Hex tiling: every few metres a new random turn, so no repeat shows.
        let bare0 = hex_triplanar(layer + 3, p, tw, 7.0);
        let bare = mix(vec3<f32>(dot(bare0, vec3<f32>(0.3, 0.59, 0.11))), bare0, g.rock.w) * g.rock.rgb * (0.85 + 0.3 * broad);
        let mossy = triplanar(layer + 5, p, tw, 3.5, rot, 0.37);
        let m_moss = smoothstep(0.55, 0.9, n.y) * (0.5 + 0.5 * f.moor) * smoothstep(0.3, 0.7, broad);
        let rc = mix(bare, mossy, m_moss);
        let bare_n = hex_triplanar_n(layer + 3, p, n, tw, 7.0);
        let rn = normalize(mix(bare_n, triplanar_n(layer + 5, p, n, tw, 3.5, rot, 0.37), m_moss));
        c = mix(c, rc, m_rock);
        nn = normalize(mix(nn, rn, m_rock));
    }

    // A road: compacted gravel with two darker, smoother wheel lines and
    // loose stone between and beside them, a ragged edge, then a shoulder
    // where loose stone thins out into the grass. Exact per pixel, so the
    // edge stays sharp on any mesh detail.
    var road_rough = 0.0;
    var road_cover = 0.0;
    if on_road {
        let fine = field_fbm(wuv * 2.3 + 17.0, footprint * 2.3);
        // The edge wanders half a metre to a metre over 5-15 m, with small
        // raggedness on top.
        // fbm stays near 0.5, so its swing is scaled up to about a metre.
        let wander = clamp((field_fbm(wuv / 9.0 + 41.0, footprint / 9.0) - 0.5) * 5.0, -1.2, 1.2);
        let ragged = wander + (fine - 0.5) * 1.2 + (mottle - 0.5) * 0.5;
        let soft = 0.12 + footprint;
        let m_road = 1.0 - smoothstep(-soft, soft, road.edge + ragged);
        let m_shoulder = (1.0 - smoothstep(0.0, 2.5, road.edge + ragged)) * smoothstep(0.35, 0.6, fine + 0.25 * (1.0 - smoothstep(0.0, 1.0, road.edge + ragged)));
        // Gravel at two scales, so it has grain from a metre to a hand.
        let big = terrain_layer(layer + 4, wuv, 1.3, mapping);
        let small = terrain_layer(layer + 4, wuv + vec2<f32>(0.37, 0.71), 0.45, mapping);
        let stone0 = mix(big, small, 0.5 * (1.0 - smoothstep(0.02, 0.08, footprint)));
        let lum = dot(stone0, vec3<f32>(0.3, 0.59, 0.11));
        // The road takes its colour from the land's soil: brown gravel in
        // the forest, sand-coloured in the desert.
        let land = 0.5 * (g.soil.rgb + g.grass_dry.rgb);
        let soil_tint = land / max(dot(land, vec3<f32>(0.3, 0.59, 0.11)), 1e-3);
        let stone = mix(vec3<f32>(lum), stone0, 0.4) * mix(vec3<f32>(0.5, 0.45, 0.37), 0.5 * soil_tint, 0.8);
        // Two packed wheel lines 1.8 m apart, 0.4 m wide, wandering a
        // little; loose gravel heaped between and outside them.
        let side_m = road.u * road.half + 0.08 * (fine - 0.5) * 2.0;
        let rut_w = 0.45 + footprint;
        let rut = (1.0 - smoothstep(0.1, rut_w, abs(abs(side_m) - 0.9))) * (1.0 - smoothstep(-0.8, 0.0, road.edge));
        let pebble = field_fbm(wuv * 9.0 + 3.0, footprint * 9.0);
        let grit = field_fbm(wuv * 31.0 + 9.0, footprint * 31.0);
        let loose = (1.0 - rut) * (0.6 + 0.4 * smoothstep(0.4, 0.75, pebble));
        var rc = stone * (0.88 + 0.45 * (grit - 0.5) * loose + 0.25 * (pebble - 0.5)) * mix(1.08, 0.82, rut);
        // Single stones a few centimetres across, where a pixel can hold
        // them: lighter and darker pebbles over the loose gravel.
        if footprint < 0.05 {
            let q = wuv / 0.1;
            let cell = floor(q);
            var stones = 0.0;
            var shade = 0.0;
            for (var dy = -1; dy <= 1; dy++) {
                for (var dx = -1; dx <= 1; dx++) {
                    let k = cell + vec2<f32>(f32(dx), f32(dy));
                    let h = field_hash(k + 913.0);
                    if h < 0.45 { continue; }
                    let centre = k + vec2<f32>(field_hash(k + 17.0), field_hash(k + 53.0));
                    let r = 0.25 + 0.35 * field_hash(k + 101.0);
                    let d = length(q - centre) / r;
                    let m = 1.0 - smoothstep(0.7, 1.0, d);
                    if m > stones { stones = m; shade = field_hash(k + 211.0); }
                }
            }
            let near = 1.0 - smoothstep(0.025, 0.05, footprint);
            rc = mix(rc, stone * (0.45 + 1.1 * shade), stones * loose * near * 0.9);
        }
        // Worn soil where the edge meets the grass.
        let worn = (1.0 - smoothstep(0.0, 0.9, abs(road.edge + 0.1 + ragged))) * 0.5;
        c = mix(c, soil * 0.8, worn * (1.0 - m_road));
        c = mix(c, stone * (0.95 + 0.3 * (pebble - 0.5)), m_shoulder * (1.0 - m_road) * 0.8);
        // A little of the land's own colour: dust from the verges.
        rc = mix(rc, c * 0.85, 0.2);
        c = mix(c, rc, m_road);
        let rn = perturb_xz(n, mix(terrain_layer_normal(layer + 4, wuv, 1.3, mapping), terrain_layer_normal(layer + 4, wuv + vec2<f32>(0.37, 0.71), 0.45, mapping), 0.5) * (1.0 - 0.75 * rut));
        nn = normalize(mix(nn, rn, max(m_road, m_shoulder * 0.6)));
        road_rough = mix(0.95, 0.7, rut);
        road_cover = m_road;
    }

    // Roughness finer than the mesh, from the terrain function itself (the
    // material's metal carries the seed: terrain is never metal).
    let ds = field_detail_slope(p.xz, u32(s.metal + 0.5), footprint, f.rock, f.up, f.moor);
    nn = normalize(nn + vec3<f32>(-ds.x, 0.0, -ds.y));

    // Damp ground darkens near water.
    c *= 1.0 - g.soil.w * f.wet * (1.0 - m_rock * 0.5);
    var rough = mix(0.95, 0.55, f.wet * 0.6);
    rough = mix(rough, road_rough, road_cover);

    // Snow over everything but water: bright, fairly smooth, thinning into
    // patches at its edge.
    var m_snow = smoothstep(0.3, 0.7, f.snow + (mottle - 0.5) * 0.5 + (broad - 0.5) * 0.3);
    // Seasonal snow (biome.rs Environment, GroundStyle snow.w) on open
    // ground: thinning on slopes and in patches as the cover falls.
    if g.snow.w > 0.0 {
        let cover = g.snow.w * smoothstep(0.6, 0.85, f.up) * (1.0 - f.water * 4.0);
        m_snow = max(m_snow, smoothstep(0.3, 0.7, cover + (mottle - 0.5) * 0.4 + (broad - 0.5) * 0.3));
    }
    if m_snow > 0.0 {
        c = mix(c, g.snow.rgb * (0.93 + 0.1 * mottle), m_snow);
        nn = normalize(mix(nn, n, m_snow * 0.8));
        rough = mix(rough, 0.6, m_snow);
    }

    // Water: a dark, smooth, level surface, clearer where shallow. Water
    // thinner than the mesh comes as a film of WATER_FILM metres times its
    // share of the ground (terrain_field.rs Sample::film): the cover is
    // linear in depth up to there, so a far ditch keeps its share of water
    // as its strip widens to the cell.
    if f.water > 0.0005 {
        let depth = clamp(f.water / 0.8, 0.0, 1.0);
        let w = min(f.water / WATER_FILM, 1.0);
        let ripple = vec2<f32>(field_fbm(wuv * 1.7 + vec2<f32>(s.time * 0.31, 0.0), footprint * 1.7), field_fbm(wuv * 1.7 + vec2<f32>(5.0, s.time * 0.27), footprint * 1.7)) - 0.5;
        let wn = normalize(vec3<f32>(ripple.x * 0.12, 1.0, ripple.y * 0.12));
        c = mix(c, mix(c * 0.45, vec3<f32>(0.018, 0.03, 0.028), depth), w);
        nn = normalize(mix(nn, wn, w));
        rough = mix(rough, 0.04, w);
    }
    return SurfaceOut(c * s.tint, nn, rough, 0.0, vec3<f32>(0.0));
}
