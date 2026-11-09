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
    // Pixels too large for the first octave (the loop's first break) need
    // neither the warp nor the rotations.
    if clamp(16.0 / (4.0 * footprint) - 1.0, 0.0, 1.0) <= 0.0 { return vec2<f32>(0.0); }
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
    let a = material(at); let b = material(at + 1u); let c = material(at + 2u);
    let fs = ForestStyle(vec4<f32>(a.tint, bitcast<f32>(a.layer)), vec4<f32>(a.roughness, bitcast<f32>(a.flags), a.scale, a.metal),
        vec4<f32>(b.tint, bitcast<f32>(b.layer)), vec4<f32>(b.roughness, bitcast<f32>(b.flags), b.scale, b.metal));
    return FarForest(fs, c.tint.x, c.tint.y, c.scale, vec2<f32>(c.tint.z, c.roughness), (c.flags & 1u) != 0u);
}
// Whether this ground lies past the drawn trees where the painted ones
// stand: those are shaded per pixel, not by the 2x2 coarse material
// (shade.wgsl), which smeared each tree over 3-6 pixels.
fn far_forest_full_res(s: SurfaceIn) -> bool {
    if s.scale >= 0.0 { return false; }
    let params = far_forest_params(u32(-s.scale - 0.5));
    if params.density <= 0.0 { return false; }
    let origin = select(view.cam.xz, params.centre, params.centred);
    return distance(s.p.xz, origin) > params.start;
}
// The forest's stand field (forest_field ff_stand) for this ground, or -2
// where no forest is placed: > -0.05 inside a stand.
fn forest_ground(s: SurfaceIn, f: FieldAttr, footprint: f32) -> f32 {
    if s.scale >= 0.0 { return -2.0; }
    let params = far_forest_params(u32(-s.scale - 0.5));
    if params.density <= 0.0 { return -2.0; }
    return ff_stand(s.p.x, s.p.z, u32(s.metal + 0.5), ff_site(f.up, f.wet), footprint);
}
// How far the ground's lushness follows the stands (1: only them).
const FOREST_GROUND_LUSH: f32 = 0.7;

// How much of this ground's sun the drawn trees standing towards the sun
// take, from their silhouettes (far_strip along the sun, as the painted
// ones shade their gaps): a tree or a gap of sunlight per pixel. shade.wgsl
// uses it where no sharp sun map reaches, instead of the coarse maps whose
// stand shadows merged into large dark blobs. -1 where no forest stands.
fn far_tree_cast(s: SurfaceIn) -> f32 {
    if s.scale >= 0.0 { return -1.0; }
    let params = far_forest_params(u32(-s.scale - 0.5));
    if params.density <= 0.0 { return -1.0; }
    let origin = select(view.cam.xz, params.centre, params.centred);
    // Past the drawn trees the painted ones shade their own ground.
    let drawn = 1.0 - smoothstep(params.start, params.end, distance(s.p.xz, origin));
    if drawn <= 0.0 { return -1.0; }
    let sun = light.sun_dir.xyz;
    let sun_xz = length(sun.xz);
    if sun.y <= 0.02 || sun_xz <= 1e-3 { return 0.0; }
    let f = field_attr(s);
    let seed = u32(s.metal + 0.5);
    let site = ff_site(f.up, f.wet);
    let footprint = sqrt(length(s.dx) * length(s.dy));
    let crowd = ff_share(s.p.x, s.p.z, seed, site, footprint) * params.density;
    if crowd <= 0.0 { return 0.0; }
    let r0 = params.style.crown.x * FAR_CROWN_SIZE;
    let len0 = max(params.style.far.w * min(sun_xz / sun.y, FAR_TREE_COT_MAX), 2.0 * r0);
    let fw = max(footprint * 0.5, 0.05);
    return (1.0 - far_strip(s.p.xz, sun.xz / sun_xz, len0, r0 * FAR_SHADOW_SIZE, fw, fw, crowd, seed, params.style, FAR_SHADOW_OPACITY).x) * drawn;
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
    let site = ff_site(f.up, f.wet);
    let sun = light.sun_dir.xyz;
    // The trees stand on this ground and rise towards the viewer (their
    // silhouettes, far_crowns).
    let eye = normalize(view.cam.xyz - s.p);
    let crowns = far_crowns(s.p.xz, footprint, s.dx, s.dy, eye, sun, seed, site, params.density, params.style);
    let cover = crowns.w * grows * fade;
    if cover <= 0.0 { return MeadowCanopy(ground, nn); }
    // Ground between crowns: the forest floor's shade where the stand is
    // dense, as forest_canopy's gaps.
    let crowd = ff_share(s.p.x, s.p.z, seed, site, footprint) * params.density;
    // Not forest_canopy's floor, nearly black under a closed canopy: the
    // reference's gaps between open pines show lit ground.
    let floor = mix(ground, params.style.floor.rgb, 0.25 * min(crowd * 1.25, 1.0));
    let gap = floor * (1.0 - FAR_GAP_SHADE * min(crowd * 1.25, 1.0) * grows * fade);
    // Drawn needles keep NEEDLE_BRUSH_OWN of the sun and 0.25 + 0.75 x its
    // root of the sky (shade.wgsl): their crowns shade themselves inside.
    // In a stand the neighbours take more: a crown's flanks lie in the
    // shadow of the crowns towards the sun, and the sky over a crown is
    // partly crowns (the canopy volume's job for drawn trees). Dense stands
    // read darker than their scattered edges, as in the reference.
    let sun_grow = ff_crown_growth(sun, params.style);
    let sun_shade = 1.0 - exp(-crowd * params.style.crown.y * sun_grow * sun_grow);
    // How closed the stand looks from here: crowns stacked behind crowns,
    // each in the others' shade.
    let packed = 1.0 - exp(-crowd * params.style.crown.y * far_view_area(eye, params.style));
    let sky_own = (0.25 + 0.75 * sqrt(FAR_CROWN_OWN)) * (1.0 - FAR_CROWN_SKY_LOSS * packed);
    let sun_own = FAR_CROWN_OWN * (1.0 - FAR_CROWN_SUN_LOSS * max(packed, sun_shade));
    let c = mix(gap, crowns.rgb * sky_own, cover);
    // Sun: of the ground between crowns, the share no crown shades from the
    // sun, except where the view sits behind the sun and the crowns hide
    // their own shadows.
    // Where the trees are drawn one by one their shadows are too: the
    // trees towards the sun from this ground, a crown or a gap of sunlight
    // per pixel like the reference's, not a mean. Averaged trees keep the
    // share, except where the view sits behind the sun and the crowns hide
    // their own shadows.
    let hidden = clamp(dot(eye, sun), 0.0, 1.0);
    var shadow = sun_shade * (1.0 - hidden);
    let sun_xz = length(sun.xz);
    if far_crown_far < 1.0 && sun.y > 0.02 && sun_xz > 1e-3 {
        let r0 = params.style.crown.x * FAR_CROWN_SIZE;
        let len0 = max(params.style.far.w * min(sun_xz / sun.y, FAR_TREE_COT_MAX), 2.0 * r0);
        let fw = max(footprint * 0.5, 0.05);
        let cast_sh = 1.0 - far_strip(s.p.xz, sun.xz / sun_xz, len0, r0 * FAR_SHADOW_SIZE, fw, fw, crowd, seed, params.style, FAR_SHADOW_OPACITY).x;
        shadow = mix(cast_sh, shadow, far_crown_far);
    }
    let sun_cover = shadow * grows * fade;
    let lum = vec3<f32>(0.3, 0.59, 0.11);
    let lit_c = dot(crowns.rgb, lum) * sky_own * cover;
    let lit_g = dot(gap, lum) * (1.0 - cover);
    field_sun = (lit_c * sun_own / sky_own + lit_g * (1.0 - sun_cover)) / max(lit_c + lit_g, 1e-4);
    let up = normalize(n + vec3<f32>(0.0, 1.0, 0.0));
    return MeadowCanopy(c, normalize(mix(nn, up, cover)));
}

// How much more ground a crown hides seen from `eye` than from above
// (ff_crown_growth squared), capped: the reference's pines, drawn, are open
// and narrow enough that gaps between them show kilometres out, where the
// solid-cone growth had closed every stand.
override FAR_VIEW_AREA_MAX: f32 = 2.5;
fn far_view_area(eye: vec3<f32>, fs: ForestStyle) -> f32 {
    let g = ff_crown_growth(eye, fs);
    return min(g * g, FAR_VIEW_AREA_MAX);
}

// A crown's shaded base relative to its mean (its lit top is 2 - this).
override FAR_CROWN_BASE: f32 = 0.25;
// A pine crown is open: the share of what is behind it one crown hides.
// Stacked crowns still leave gaps, so closed stands keep their grain
// (the reference's) instead of a solid cover.
override FAR_CROWN_OPACITY: f32 = 1.0;
// The share of a pixel a crown's edge is filtered over.
override FAR_CROWN_FILTER: f32 = 0.3;
// Ambient shade of the ground between crowns in a closed stand.
override FAR_GAP_SHADE: f32 = 0.0;
// Crown half-width relative to the style's (its top-down cover radius,
// which counts the open branch tips).
override FAR_CROWN_SIZE: f32 = 1.0;
// A pine's shadow on the ground: sun through its open crown, and narrower
// than its silhouette's full width (Mancave from 1200 m: full-strength,
// full-width shadows read as dark dashes 2-3x the crowns).
override FAR_SHADOW_OPACITY: f32 = 0.7;
override FAR_SHADOW_SIZE: f32 = 0.8;
// Steepest glancing view a silhouette stretches for (cot of elevation).
const FAR_TREE_COT_MAX: f32 = 8.0;
// KERN_FAR_CROWN_DEBUG: red the averaged share, green single-crown cover.
override FAR_CROWN_DEBUG: f32 = 0.0;
// The sun a drawn needle crown keeps (shade.wgsl NEEDLE_BRUSH_OWN).
const FAR_CROWN_OWN: f32 = 0.42;
// How much of that the neighbours in a closed stand take from a crown's
// sun and sky.
override FAR_CROWN_SUN_LOSS: f32 = 0.95;
override FAR_CROWN_SKY_LOSS: f32 = 0.7;

// Trees seen from `eye` past the drawn ones, standing on q's ground, for
// a pixel whose world derivatives are dx, dy: rgb their colour with
// self-shade, w the share of the pixel they cover.
//
// A tree seen at a glancing angle is a silhouette, not a disc on the
// ground: its crown, a crown wide across the view, stretches from its base
// away from the camera by its height x cot(elevation) (a 13 m pine at 3 km
// from 500 m up is 2 px wide and 7 px tall). The ground point q is covered
// by the trees standing on the strip between q and that far towards the
// camera, each a cone from 0.3 of its height up. Drawn trees give a crown
// or a gap per pixel; these do too, so the grain stays, until a pixel is
// wider than a cell, where they become their mean.
fn far_crowns(q: vec2<f32>, footprint: f32, dx: vec3<f32>, dy: vec3<f32>, eye: vec3<f32>, sun: vec3<f32>, seed: u32, site: f32, density: f32, fs: ForestStyle) -> vec4<f32> {
    let needles = mix(fs.needles.rgb, fs.far.rgb, ff_patch(q.x, q.y, seed, fs.crown.w));
    far_crown_far = 1.0;
    // KERN_FAR_CROWN_DEBUG=2: white noise in 2 m world cells, to see how
    // sharp the ground's shading reaches the screen out here.
    if FAR_CROWN_DEBUG == 2.0 {
        let h = fract(sin(dot(floor(q / 2.0), vec2<f32>(12.9898, 78.233))) * 43758.5453);
        return vec4<f32>(vec3<f32>(step(0.5, h)), 1.0);
    }
    let crowd = ff_share(q.x, q.y, seed, site, footprint) * density;
    if crowd <= 0.0 { return vec4<f32>(needles, 0.0); }
    let along = normalize(eye.xz + vec2<f32>(1e-6, 0.0));
    let across = vec2<f32>(-along.y, along.x);
    let cot = min(length(eye.xz) / max(eye.y, 1e-3), FAR_TREE_COT_MAX);
    let r0 = fs.crown.x * FAR_CROWN_SIZE;
    // Silhouette length on the ground (at least the crown seen from above).
    let len0 = max(fs.far.w * cot, 2.0 * r0);
    // The cone (0.3..1 of the height, half-width r at 0.3) covers 0.7 r len.
    let mean_cover = 1.0 - exp(-crowd * 0.7 * r0 * len0 / (FF_CELL * FF_CELL) * FAR_CROWN_OPACITY);
    // Sun from above lights a little more of each crown.
    let tops = 0.9 + 0.1 * max(sun.y, 0.0);
    let fa = max(max(abs(dot(dx.xz, across)), abs(dot(dy.xz, across))) * FAR_CROWN_FILTER, 0.02);
    let fl = max(max(abs(dot(dx.xz, along)), abs(dot(dy.xz, along))) * FAR_CROWN_FILTER, 0.02);
    let far = smoothstep(FF_CELL * 0.4, FF_CELL * 0.8, fa / FAR_CROWN_FILTER);
    far_crown_far = far;
    if far >= 1.0 {
        if FAR_CROWN_DEBUG == 1.0 { return vec4<f32>(1.0, 0.0, 0.0, 1.0); }
        return vec4<f32>(needles * tops, mean_cover);
    }
    let strip = far_strip(q, along, len0, r0, fa, fl, crowd, seed, fs, FAR_CROWN_OPACITY);
    let near_cover = 1.0 - strip.x;
    let near_tint = select(1.0, strip.y / strip.z, strip.z > 0.0);
    let cover = mix(near_cover, mean_cover, far);
    if FAR_CROWN_DEBUG == 1.0 { return vec4<f32>(far, near_cover * (1.0 - far), 0.0, 1.0); }
    return vec4<f32>(needles * tops * mix(near_tint, 1.0, far), cover);
}

// Whether far_crowns' last call averaged its trees (1) or drew them one
// by one (0).
var<private> far_crown_far: f32 = 1.0;

// The trees whose silhouettes, cast from q's ground along `dir` (towards
// the viewer, or the sun for their shadows) `len0` long and `r0` wide,
// cover q: x the share they leave open, y their lit tint weighted by z
// (nearer ones in front). Walks the cells under the strip once each,
// column by column along its major grid axis. fa, fl: the pixel's
// filter across and along `dir`.
fn far_strip(q: vec2<f32>, dir: vec2<f32>, len0: f32, r0: f32, fa: f32, fl: f32, crowd: f32, seed: u32, fs: ForestStyle, opacity: f32) -> vec3<f32> {
    let side = vec2<f32>(-dir.y, dir.x);
    let reach = len0 * 1.25;
    let hw = r0 * 1.25;
    let q1 = q + dir * reach;
    let major_x = abs(dir.x) >= abs(dir.y);
    let a0 = select(q.y, q.x, major_x);
    let a1 = select(q1.y, q1.x, major_x);
    let b0 = select(q.x, q.y, major_x);
    let slope = select(dir.x / dir.y, dir.y / dir.x, major_x);
    let half = hw / max(select(abs(dir.y), abs(dir.x), major_x), 0.7);
    let c_lo = i32(floor((min(a0, a1) - hw) / FF_CELL));
    let c_hi = i32(floor((max(a0, a1) + hw) / FF_CELL));
    var open = 1.0;
    var tint = 0.0;
    var weight = 0.0;
    var visits = 0;
    for (var c = c_lo; c <= c_hi; c++) {
        // The strip's other coordinate over this column (clipped to it).
        let x_lo = max(f32(c) * FF_CELL, min(a0, a1) - hw);
        let x_hi = min(f32(c + 1) * FF_CELL, max(a0, a1) + hw);
        let y_a = b0 + (clamp(x_lo, min(a0, a1), max(a0, a1)) - a0) * slope;
        let y_b = b0 + (clamp(x_hi, min(a0, a1), max(a0, a1)) - a0) * slope;
        let r_lo = i32(floor((min(y_a, y_b) - half) / FF_CELL));
        let r_hi = i32(floor((max(y_a, y_b) + half) / FF_CELL));
        for (var rr = r_lo; rr <= r_hi; rr++) {
            visits++;
            if visits > 40 { break; }
            let ci = select(rr, c, major_x);
            let cj = select(c, rr, major_x);
            let sp = ff_spot(ci, cj, seed, crowd, fs.needles.w);
            if sp.w < 0.0 { continue; }
            let d = sp.xy - q;
            // How far along `dir` the tree stands (0 at q), and aside.
            let s = dot(d, dir);
            let x = abs(dot(d, side));
            let len = len0 * (0.85 + 0.3 * fract(sp.w * 13.7));
            let t = s / len;
            if t < 0.25 || t > 1.05 { continue; }
            // Half-width of the cone at this height, its edges filtered
            // over the pixel across, its ends along.
            let r = r0 * sp.z;
            let width = r * clamp((1.0 - t) / 0.7, 0.0, 1.0);
            let a_x = clamp(0.5 + (width - x) / fa, 0.0, 1.0);
            let a_t = clamp(0.5 + (s - 0.3 * len) / fl, 0.0, 1.0) * clamp(0.5 + (len - s) / fl, 0.0, 1.0);
            let a = a_x * a_t;
            if a <= 0.0 { continue; }
            open *= 1.0 - a * opacity;
            // A lit top over a shaded base (averaging 1 over the crown).
            let u = clamp((t - 0.3) / 0.7, 0.0, 1.0);
            let lit = (FAR_CROWN_BASE + (2.0 - 2.0 * FAR_CROWN_BASE) * u) * sp.w;
            let front = a * exp(min(s - reach, 0.0));
            tint += front * lit;
            weight += front;
        }
    }
    return vec3<f32>(open, tint, weight);
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
    let a = field_material_pair(material(at));
    let b = field_material_pair(material(at + 1u));
    let c = field_material_pair(material(at + 2u));
    let d = field_material_pair(material(at + 3u));
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
    let h = field_material_pair(material(base));
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
        var pair = field_material_pair(material(nodes + at[c] / 2u));
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
    if FEATURE_FIELD_REGIONS && g.reserved.x >= 0.5 { return region_ground(s.p.xz, u32(g.reserved.x - 0.5)); }
    return g;
}

// Grass proxy (style block material 7, procedural_terrain.rs grass_proxy):
// tint.y the drawn grass radius round the camera (0: off), roughness,
// scale, metal the colour the blades multiply the ground by. Fades in over
// the outer 30 % of the disc, where the cells thin out, on ground where
// grass_cells would plant them (rock, scree, water, snow excluded).
fn grass_proxy_hash(q: vec2<i32>) -> f32 {
    var h = u32(q.x) * 0x8da6b343u ^ u32(q.y) * 0xd8163841u;
    h = (h ^ (h >> 15u)) * 0x2c1b3c6du;
    h = (h ^ (h >> 12u)) * 0x297a2d39u;
    return f32((h ^ (h >> 15u)) & 0xffffu) / 65535.0;
}
fn grass_proxy(c: vec3<f32>, s: SurfaceIn, f: FieldAttr, meadow_w: f32, footprint: f32) -> vec3<f32> {
    if s.scale >= 0.0 { return c; }
    let m = material(u32(-s.scale - 0.5) + 7u);
    let radius = m.tint.y;
    if radius <= 0.0 { return c; }
    let d = distance(s.p.xz, view.cam.xz);
    // Only past the blades: the tint darkens (it is the blades' mean, lit
    // darker than bare ground), and inside the disc it made the ground
    // between them near black (Joep playtest 5).
    let w = smoothstep(0.7 * radius, radius, d) * meadow_w
        * (1.0 - smoothstep(0.3, 0.8, f.rock)) * (1.0 - smoothstep(0.6, 0.8, f.scree))
        * (1.0 - smoothstep(0.0, 0.02, f.water)) * (1.0 - smoothstep(0.15, 0.45, f.snow));
    // The blades read speckled down to a pixel, a flat tint smooth: tufts
    // of light and dark 0.3 m across, fading out as a pixel covers several
    // (Mancave: the edge is now a texture jump more than a colour one).
    let tuft = grass_proxy_hash(vec2<i32>(floor(s.p.xz / 0.3)));
    let speckle = 1.0 + 0.5 * (tuft - 0.5) * clamp(1.0 - footprint / 0.6, 0.0, 1.0);
    return mix(c, c * vec3<f32>(m.roughness, m.scale, m.metal) * speckle, w);
}

// Ditches drawn into the pixels: the terrain's ditch amount (style block
// material 7, procedural_terrain.rs reserve) on flat ground, and the
// distance (metres) to the nearest ditch line on terrain_field's grid.
fn field_ditch(xz: vec2<f32>, s: SurfaceIn, up: f32) -> vec2<f32> {
    if s.scale >= 0.0 { return vec2<f32>(0.0, 1000.0); }
    let amount = material(u32(-s.scale - 0.5) + 7u).tint.x * smoothstep(0.97, 0.995, up);
    if amount <= 0.0 { return vec2<f32>(0.0, 1000.0); }
    let a = f32(u32(s.metal + 0.5) % 97u) * 0.0647;
    let u = xz.x * cos(a) + xz.y * sin(a);
    let v = xz.y * cos(a) - xz.x * sin(a);
    let across = abs((u / 60.0 + 0.5) - floor(u / 60.0 + 0.5) - 0.5) * 60.0;
    let along = abs((v / 400.0 + 0.5) - floor(v / 400.0 + 0.5) - 0.5) * 400.0;
    return vec2<f32>(amount, min(across, along));
}

// Share of a pixel `fw` metres wide at distance `d` from a line that a
// strip |d| < w covers.
fn field_strip(d: f32, w: f32, fw: f32) -> f32 {
    let h = 0.5 * max(fw, 1e-3);
    return (clamp(d + h, -w, w) - clamp(d - h, -w, w)) / (2.0 * h);
}

// Roads drawn into the pixels (procedural_terrain.rs road_surface,
// world_constraints.rs surface_words): data materials holding f32 words; the
// style block's far forest params' metal points at them (first + 1).
fn road_word(base: u32, i: u32) -> f32 {
    let m = material(base + i / 8u);
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
    if !FEATURE_FIELD_ROADS || scale >= 0.0 { return r; }
    let ptr = material(u32(-scale - 0.5) + 2u).metal;
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

// A ground layer mapped from above smears down a steep face, such as the cut
// round a levelled plot: there it blends to triplanar. Gentle ground pays nothing.
fn steep_layer(l: i32, p: vec3<f32>, n: vec3<f32>, scale: f32, mapping: TerrainMapping) -> vec3<f32> {
    let flat_map = terrain_layer(l, p.xz, scale, mapping);
    let steep = smoothstep(0.7, 0.45, n.y);
    if steep <= 0.0 { return flat_map; }
    var tw = abs(n);
    tw = tw * tw * tw * tw;
    tw /= tw.x + tw.y + tw.z;
    let rot = mat2x2<f32>(vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0));
    return mix(flat_map, triplanar(l, p, tw, scale, rot, 0.0), steep);
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
    var lush = clamp(0.5 + (broad - 0.5) * 1.2 + 0.4 * f.wet, 0.0, 1.0);
    // Where a forest grows the ground is green, and it dries out away from
    // the stands: the trees and the ground's colour read one field.
    let stands = forest_ground(s, f, footprint);
    if stands > -1.0 { lush = clamp(mix(lush, smoothstep(-0.45, 0.15, stands), FOREST_GROUND_LUSH), 0.0, 1.0); }
    // Darker than a lawn: seen from afar a meadow is mostly blade shadow,
    // so the ground matches the grass where the blades fade out.
    let grass = steep_layer(layer, p, n, 2.6, mapping) * mix(g.grass_dry.rgb, g.grass_lush.rgb, lush);
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
    if m_soil > 0.0 || on_road { soil = steep_layer(layer + 2, p, n, 3.0, mapping) * g.soil.rgb; }
    var c = mix(grass, moor, m_moor);
    c = mix(c, soil, m_soil);
    if m_soil > 0.0 { nt = mix(nt, terrain_layer_normal(layer + 2, wuv, 3.0, mapping), m_soil); }
    // The bump map is mapped from above too: it streaks a steep face the same way.
    var nn = perturb_xz(n, nt * smoothstep(0.45, 0.7, n.y));
    c *= 0.82 + 0.3 * broad;

    // Past the meadow's blades the ground reads as a grass canopy.
    let meadow_w = (1.0 - m_soil) * (1.0 - 0.5 * m_moor) * smoothstep(0.75, 0.9, f.up);
    let canopy = meadow_canopy(c, nn, n, s.dir, wuv, footprint, s.time, u32(s.metal + 0.5), meadow_w);
    c = canopy.albedo;
    // Past the drawn grass cells (a disc round the camera) the ground takes
    // the blades' mean colour, so no green circle shows from the air.
    c = grass_proxy(c, s, f, meadow_w, footprint);
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
        let gravel0 = steep_layer(layer + 4, p, n, 1.8, mapping);
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
    // Near a ditch the vertices' water (its film widened to the mesh cell,
    // or the carved bed's edge along triangle edges) gives way to the
    // ditch's own line, sharp and antialiased at any distance. The band is
    // as wide as the film can spread on the coarsest cell for this pixel
    // size (procedural_terrain::Settings::cell_pixels).
    // Its colour must not depend on the mesh either: a film is water spread
    // thin over the cell, not shallow water, so it takes the depth of the
    // bed it stands for (half the darkening, as a stream's or a ditch's
    // average). Carved beds darken further with their real depth.
    var w = min(f.water / WATER_FILM, 1.0);
    var depth = max(clamp(f.water / 0.8, 0.0, 1.0), 0.5 * w);
    let ditch = field_ditch(p.xz, s, f.up);
    if ditch.x > 0.0 {
        let cell = 24.0 * footprint;
        let band = max(1.2, cell) + 0.5 * cell + 0.5;
        let near = ditch.x * (1.0 - smoothstep(band, band + 1.0, ditch.y));
        w = mix(w, ditch.x * field_strip(ditch.y, 1.2, footprint), near);
        // The ditch's full-detail depth (terrain_field's profile, 0.8 m in
        // the middle), its mean across the strip once a pixel spans it.
        let full = max(1.3 * (1.0 - smoothstep(0.6, 1.8, ditch.y)) - 0.5, 0.0) / 0.8;
        depth = mix(depth, mix(full, 0.6, clamp(footprint / 2.4, 0.0, 1.0)), near);
    }
    if w > 0.0 {
        let ripple = vec2<f32>(field_fbm(wuv * 1.7 + vec2<f32>(s.time * 0.31, 0.0), footprint * 1.7), field_fbm(wuv * 1.7 + vec2<f32>(5.0, s.time * 0.27), footprint * 1.7)) - 0.5;
        let wn = normalize(vec3<f32>(ripple.x * 0.12, 1.0, ripple.y * 0.12));
        if water_optics_on() {
            // Depth in metres: the carved depth, or what the film stands for.
            let body = water_body(c, max(f.water, depth * 0.8), s.dir, wn, wuv, s.time, footprint);
            c = mix(c, body, w);
        } else {
            c = mix(c, mix(c * 0.45, vec3<f32>(0.018, 0.03, 0.028), depth), w);
        }
        nn = normalize(mix(nn, wn, w));
        rough = mix(rough, 0.04, w);
    }
    return SurfaceOut(c * s.tint, nn, rough, 0.0, vec3<f32>(0.0));
}

// Water optics (Lighting::water, shade.wgsl WaterView): what the eye sees
// through the surface, as the albedo the lighting then treats like ground.
// The surface is the terrain at water level, so the bed is not drawn: it is
// the ground colour `bed` lying `depth` metres down. The view ray refracts
// at the rippled surface (Snell, n = 1.333) and travels to the bed, the sun
// refracts down to it; along both paths the water absorbs, red first, and
// scatters its own dark green back (Beer-Lambert, single scattering). What
// the surface reflects (Fresnel) it does not let through: the reflection
// itself is shade.wgsl's specular (sky, or the screen-space reflection).
// Sunlight focused by the ripples draws moving caustics on shallow beds,
// where the refracted ray lands, so they shift with the view like the bed.
fn water_body(bed: vec3<f32>, depth: f32, dir: vec3<f32>, wn: vec3<f32>, wuv: vec2<f32>, time: f32, footprint: f32) -> vec3<f32> {
    let eta = 1.0 / 1.333;
    let cos_i = clamp(dot(wn, -dir), 0.02, 1.0);
    let t = refract(dir, wn, eta);
    let cos_t = max(-t.y, 0.05);
    let l = light.sun_dir.xyz;
    let cos_l = sqrt(max(1.0 - eta * eta * (1.0 - clamp(l.y, 0.0, 1.0) * clamp(l.y, 0.0, 1.0)), 0.0));
    // Path down along the view, and the light's path down to the bed.
    let path = depth * (1.0 / cos_t + 1.0 / max(cos_l, 0.2));
    // Extinction per metre: lowland fresh water with algae, tannin and
    // silt (red goes first, then blue), scattering back an olive green.
    let sigma = vec3<f32>(0.95, 0.5, 0.62);
    let tr = exp(-sigma * path);
    let deep = vec3<f32>(0.024, 0.036, 0.024);
    // Caustics at the refracted landing point, fading with depth and once
    // a pixel covers several ripple cells (they would only alias).
    let land = wuv + t.xz / cos_t * depth;
    let near = (1.0 - smoothstep(0.015, 0.06, footprint)) * (1.0 - smoothstep(0.3, 1.5, depth)) * smoothstep(0.0, 0.08, l.y);
    var caustic = 1.0;
    if near > 0.0 {
        let a = field_fbm(land * 2.3 + vec2<f32>(time * 0.41, time * 0.23), footprint * 2.3);
        let b = field_fbm(land * 2.9 - vec2<f32>(time * 0.19, time * 0.37) + 13.0, footprint * 2.9);
        // Bright lines where the two moving fields cross (a - b = 0).
        let ridge = 1.0 - smoothstep(0.0, 0.07, abs(a - b));
        caustic = 1.0 + near * (1.0 * ridge - 0.3);
    }
    // Grass does not grow under standing water: silt settles on the bed.
    let settled = mix(bed, vec3<f32>(0.15, 0.125, 0.09), smoothstep(0.1, 0.6, depth) * 0.7);
    let fresnel = 0.02 + 0.98 * pow(1.0 - cos_i, 5.0);
    return (settled * caustic * tr + deep * (1.0 - tr)) * (1.0 - fresnel);
}
