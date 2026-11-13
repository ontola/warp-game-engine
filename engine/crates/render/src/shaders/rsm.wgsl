// Reflective shadow maps (Dachsbacher and Stamminger 2005): the first
// diffuse bounce of the clustered lights flagged `bounce` (source.w bit 16),
// gathered live from their shadow tiles, which also hold each texel's
// albedo and normal (raster.wgsl fs_rsm). See rsm.rs and
// engine/docs/interior-light.md.
//
// `rsm_gather` runs at a quarter of the view's resolution before shade: per
// pixel and light it picks texels of the light's tiles, half uniformly over
// the tile and half on a disc around the pixel's own projection (density
// falling as 1/r, where the 1/d^2 of near patches is), weighted by the
// mixture's pdf. Each texel is a patch the light reaches: the flux it
// reflects is albedo * (the light's irradiance there at normal incidence)
// * d^2 * (the texel's solid angle), the same light model shade uses
// directly, so no gain is tuned. Visibility between patch and pixel is an
// exact walk through a voxel grid of the scene's surfaces (`rsm.rs`
// `Voxels`). The result is blended with last frame's (reprojected, rejected
// where the depth changed) and shade reads it with a depth-aware 2x2
// upsample (`rsm_irradiance`), adding it to the diffuse ambient.
@group(1) @binding(23) var rsm_surface: texture_2d_array<u32>;
@group(1) @binding(24) var rsm_out: texture_storage_2d<rgba16float, write>;
@group(1) @binding(25) var rsm_prev: texture_2d<f32>;
@group(1) @binding(26) var rsm_in: texture_2d<f32>;

// Voxels per brick edge (rsm.rs BRICK).
const BRICK: u32 = 4u;
// The gather's resolution divisor (rsm.rs `scale`): 4, or 8 from 1600 lines.
fn rsm_scale(size: vec2<u32>) -> i32 { return select(4, 8, size.y > 1600u); }

fn rsm_on() -> bool { return gi.rsm.y > 0.5; }

fn rsm_world(q: vec2<i32>, size: vec2<u32>) -> vec4<f32> {
    let d = textureLoad(depth_tex, q, 0);
    if d <= 0.0001 { return vec4<f32>(0.0); }
    let ndc = (vec2<f32>(q) + 0.5) / vec2<f32>(size) * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);
    let h = view.inv_view_proj * vec4<f32>(ndc, d, 1.0);
    return vec4<f32>(h.xyz / h.w, 1.0);
}

// The surface normal at full-resolution pixel q from the depth buffer: per
// axis the neighbour nearer in depth (one side only at the view's edge),
// zero where the depth gives none.
fn rsm_normal(q: vec2<i32>, size: vec2<u32>, p: vec3<f32>) -> vec3<f32> {
    let max_px = vec2<i32>(size) - 1;
    var d = array<vec3<f32>, 2>(vec3<f32>(0.0), vec3<f32>(0.0));
    for (var a = 0; a < 2; a++) {
        let e = select(vec2<i32>(1, 0), vec2<i32>(0, 1), a == 1);
        var hi = vec4<f32>(0.0);
        var lo = vec4<f32>(0.0);
        if all(q + e <= max_px) { hi = rsm_world(q + e, size); }
        if all(q - e >= vec2<i32>(0)) { lo = rsm_world(q - e, size); }
        if hi.w > 0.0 && (lo.w <= 0.0 || length(hi.xyz - p) < length(p - lo.xyz)) { d[a] = hi.xyz - p; }
        else if lo.w > 0.0 { d[a] = p - lo.xyz; }
    }
    let c = cross(d[1], d[0]);
    if dot(c, c) < 1e-20 { return vec3<f32>(0.0); }
    return normalize(c);
}

fn rsm_inverse(m: mat4x4<f32>) -> mat4x4<f32> {
    let a00 = m[0][0]; let a01 = m[0][1]; let a02 = m[0][2]; let a03 = m[0][3];
    let a10 = m[1][0]; let a11 = m[1][1]; let a12 = m[1][2]; let a13 = m[1][3];
    let a20 = m[2][0]; let a21 = m[2][1]; let a22 = m[2][2]; let a23 = m[2][3];
    let a30 = m[3][0]; let a31 = m[3][1]; let a32 = m[3][2]; let a33 = m[3][3];
    let b00 = a00 * a11 - a01 * a10; let b01 = a00 * a12 - a02 * a10;
    let b02 = a00 * a13 - a03 * a10; let b03 = a01 * a12 - a02 * a11;
    let b04 = a01 * a13 - a03 * a11; let b05 = a02 * a13 - a03 * a12;
    let b06 = a20 * a31 - a21 * a30; let b07 = a20 * a32 - a22 * a30;
    let b08 = a20 * a33 - a23 * a30; let b09 = a21 * a32 - a22 * a31;
    let b10 = a21 * a33 - a23 * a31; let b11 = a22 * a33 - a23 * a32;
    let det = b00 * b11 - b01 * b10 + b02 * b09 + b03 * b08 - b04 * b07 + b05 * b06;
    let k = 1.0 / det;
    return mat4x4<f32>(
        vec4<f32>(a11 * b11 - a12 * b10 + a13 * b09, a02 * b10 - a01 * b11 - a03 * b09, a31 * b05 - a32 * b04 + a33 * b03, a22 * b04 - a21 * b05 - a23 * b03) * k,
        vec4<f32>(a12 * b08 - a10 * b11 - a13 * b07, a00 * b11 - a02 * b08 + a03 * b07, a32 * b02 - a30 * b05 - a33 * b01, a20 * b05 - a22 * b02 + a23 * b01) * k,
        vec4<f32>(a10 * b10 - a11 * b08 + a13 * b06, a01 * b08 - a00 * b10 - a03 * b06, a30 * b04 - a31 * b02 + a33 * b00, a21 * b02 - a20 * b04 - a23 * b00) * k,
        vec4<f32>(a11 * b07 - a10 * b09 - a12 * b06, a00 * b09 - a01 * b07 + a02 * b06, a31 * b01 - a30 * b03 - a32 * b00, a20 * b03 - a21 * b01 + a22 * b00) * k);
}

// Whether open space joins a and b: a walk through every voxel the
// segment crosses (Amanatides and Woo 1987) in the surface grid (rsm.rs
// `Voxels::pack`: origin, voxel size, dims, one bit per voxel, one per
// 4x4x4 brick), brick by brick, voxel by voxel only in occupied bricks,
// between a point on the patch (normal np) and one on the pixel's surface
// (normal n). A surface fills the voxels it touches, which reach at most a
// voxel diagonal off it: the walk starts and ends that far out along the
// normals, clear of the two surfaces' own voxels. In a corner that point
// can still lie in the other wall's voxels: a solid run the walk starts or
// ends in, no longer than a voxel diagonal, is that wall. Any other solid
// voxel blocks, so a wall or door blocks the bounce unless an end lies
// within a voxel diagonal of it.
fn rsm_open(xp: vec3<f32>, np: vec3<f32>, p: vec3<f32>, n: vec3<f32>) -> bool {
    let o = u32(gi.rsm.x);
    if o == 0u { return true; }
    let origin = vec3<f32>(aux_f32(o), aux_f32(o + 1u), aux_f32(o + 2u));
    let voxel = aux_f32(o + 3u);
    // A surface's voxels have their centres within half a voxel of it along
    // its dominant axis (rsm.rs triangle_box): their corners reach half a
    // voxel plus half a diagonal off it.
    let skip = voxel * 0.5 * (1.0 + 1.7320508);
    let a = xp + np * skip;
    let b = p + n * skip;
    let dims = vec3<i32>(i32(aux_word(o + 4u)), i32(aux_word(o + 5u)), i32(aux_word(o + 6u)));
    let bricks_at = o + aux_word(o + 7u);
    let bdims = (dims + i32(BRICK) - 1) / i32(BRICK);
    let seg = b - a;
    let len = length(seg);
    if len < 1e-4 { return true; }
    let dir = seg / len;
    let inv = 1.0 / max(abs(dir), vec3<f32>(1e-6));
    let step = vec3<i32>(select(vec3<i32>(-1), vec3<i32>(1), dir >= vec3<f32>(0.0)));
    let pos = dir >= vec3<f32>(0.0);
    // Bricks first: one step across an empty one.
    let bsize = voxel * f32(BRICK);
    let fb = (a - origin) / bsize;
    var cb = vec3<i32>(floor(fb));
    var tb = select(fb - floor(fb), 1.0 - (fb - floor(fb)), pos) * inv * bsize;
    let tb_delta = inv * bsize;
    var t = 0.0;
    var lead = true;
    var tail = false;
    let span = dims.x * dims.y;
    let bspan = bdims.x * bdims.y;
    for (var kb = 0; kb < 96; kb++) {
        if t > len { break; }
        let t_out = min(min(tb.x, tb.y), tb.z);
        var occupied = false;
        if all(cb >= vec3<i32>(0)) && all(cb < bdims) {
            let j = u32(cb.x + cb.y * bdims.x + cb.z * bspan);
            occupied = (aux_word(bricks_at + (j >> 5u)) & (1u << (j & 31u))) != 0u;
        }
        if occupied {
            // Every voxel the segment crosses inside this brick.
            let f = (a + dir * (t + 1e-5) - origin) / voxel;
            var c = vec3<i32>(floor(f));
            var tm = t + select(f - floor(f), 1.0 - (f - floor(f)), pos) * inv * voxel;
            let td = inv * voxel;
            var tv = t;
            for (var k = 0; k < 3 * i32(BRICK); k++) {
                if tv >= t_out || tv > len { break; }
                var solid = false;
                if all(c >= vec3<i32>(0)) && all(c < dims) {
                    let bit = u32(c.x + c.y * dims.x + c.z * span);
                    solid = (aux_word(o + 8u + (bit >> 5u)) & (1u << (bit & 31u))) != 0u;
                }
                if lead && !(solid && tv <= skip) { lead = false; }
                if !lead {
                    if solid {
                        if len - tv > skip { return false; }
                        tail = true;
                    } else if tail { return false; }
                }
                if tm.x <= tm.y && tm.x <= tm.z { tv = tm.x; tm.x += td.x; c.x += step.x; }
                else if tm.y <= tm.z { tv = tm.y; tm.y += td.y; c.y += step.y; }
                else { tv = tm.z; tm.z += td.z; c.z += step.z; }
            }
        } else {
            // Open space: the start's surface is left, a surface at the end
            // was not the end's.
            lead = false;
            if tail { return false; }
        }
        t = t_out;
        if tb.x <= tb.y && tb.x <= tb.z { tb.x += tb_delta.x; cb.x += step.x; }
        else if tb.y <= tb.z { tb.y += tb_delta.y; cb.y += step.y; }
        else { tb.z += tb_delta.z; cb.z += step.z; }
    }
    return true;
}

fn rsm_rand(seed: ptr<function, u32>) -> f32 {
    *seed = pcg_hash(*seed);
    return f32(*seed >> 8u) / 16777216.0;
}

// Irradiance at p (normal n) from light i's first bounce: `count` texels.
fn rsm_light(i: u32, p: vec3<f32>, n: vec3<f32>, count: u32, seed: ptr<function, u32>) -> vec3<f32> {
    let lamp = clustered_lamp(i);
    let packed = u32(lamp.source.w);
    let faces = packed & 7u;
    if faces == 0u || lamp.source.z < 0.5 { return vec3<f32>(0.0); }
    let first = u32(lamp.source.z) - 1u;
    let atlas = f32(textureDimensions(headlight_depth).x);
    // The face p itself projects into, for the local strategy.
    var own = 0u;
    if faces > 1u {
        let d = p - lamp.position_range.xyz;
        let a = abs(d);
        if a.x >= a.y && a.x >= a.z { own = select(1u, 0u, d.x > 0.0); }
        else if a.y >= a.z { own = select(3u, 2u, d.y > 0.0); }
        else { own = select(5u, 4u, d.z > 0.0); }
    }
    var sum = vec3<f32>(0.0);
    var seen = 0.0;
    var tried = 0.0;
    for (var s = 0u; s < count; s++) {
        let face = select(0u, min(u32(rsm_rand(seed) * f32(faces)), faces - 1u), faces > 1u);
        let fb = light_grid.offsets.x + (first + face) * 6u;
        let rect = bitcast<vec4<f32>>(scene_tables[fb + 4u]);
        if rect.w < 0.0 { continue; }
        let params = bitcast<vec4<f32>>(scene_tables[fb + 5u]);
        let m = mat4x4<f32>(bitcast<vec4<f32>>(scene_tables[fb]), bitcast<vec4<f32>>(scene_tables[fb + 1u]),
            bitcast<vec4<f32>>(scene_tables[fb + 2u]), bitcast<vec4<f32>>(scene_tables[fb + 3u]));
        let tile = rect.z * atlas;
        // Where p lands in this face, in tile texels (the local disc's centre).
        var centre = vec2<f32>(-1e9);
        if face == own {
            let clip = m * vec4<f32>(p, 1.0);
            if clip.w > 0.0 {
                let local = (clip.xy / clip.w) * vec2<f32>(0.5, -0.5) + 0.5;
                if all(local >= vec2<f32>(0.0)) && all(local <= vec2<f32>(1.0)) { centre = local * tile; }
            }
        }
        let radius = tile * 0.25;
        let has_local = centre.x > -1e8;
        var texel: vec2<f32>;
        if has_local && rsm_rand(seed) < 0.5 {
            let r = rsm_rand(seed) * radius;
            let a = rsm_rand(seed) * 6.2831853;
            texel = floor(centre + vec2<f32>(cos(a), sin(a)) * r);
        } else {
            texel = floor(vec2<f32>(rsm_rand(seed), rsm_rand(seed)) * tile);
        }
        if any(texel < vec2<f32>(0.0)) || any(texel >= vec2<f32>(tile)) { continue; }
        // The mixture's density at this texel (per texel), times the face's.
        let uniform_pdf = 1.0 / (tile * tile);
        var pdf = uniform_pdf;
        if has_local {
            let r = max(length(texel + 0.5 - centre), 0.5);
            let local_pdf = select(0.0, 1.0 / (6.2831853 * r * radius), r <= radius);
            pdf = 0.5 * uniform_pdf + 0.5 * local_pdf;
        }
        pdf /= f32(faces);
        let at = vec2<i32>(rect.xy * atlas + texel);
        let layer = i32(rect.w);
        let depth = textureLoad(headlight_depth, at, layer, 0);
        if depth >= 1.0 { continue; }
        let surf = textureLoad(rsm_surface, at, layer - 8, 0).x;
        if surf == 0u { continue; }
        let uv = (texel + 0.5) / tile;
        let inv_m = rsm_inverse(m);
        let h = inv_m * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
        let xp = h.xyz / h.w;
        let np = oct_decode(vec2<f32>(f32((surf >> 8u) & 255u), f32(surf & 255u)) / 255.0 * 2.0 - 1.0);
        let albedo = vec3<f32>(f32(surf >> 27u) / 31.0, f32((surf >> 21u) & 63u) / 63.0, f32((surf >> 16u) & 31u) / 31.0);
        // From the patch to p.
        let w = p - xp;
        let r2 = dot(w, w);
        if r2 < 1e-8 { continue; }
        let wd = w * inverseSqrt(r2);
        let cos_out = dot(np, wd);
        let cos_in = -dot(n, wd);
        if cos_out <= 0.0 || cos_in <= 0.0 { continue; }
        // The light at the patch, at normal incidence, as shade lights it.
        var e = local_light_at(lamp, xp);
        if all(e.irradiance == vec3<f32>(0.0)) { continue; }
        if packed >= 8u { e.irradiance *= light_shape(lamp, -e.direction); }
        let to = xp - lamp.position_range.xyz;
        let d2 = dot(to, to);
        // The texel's solid angle: params.x is its angular size at the
        // face's centre, cos^3 off axis (a pinhole face).
        let c0 = inv_m * vec4<f32>(0.0, 0.0, depth, 1.0);
        let axis = normalize(c0.xyz / c0.w - lamp.position_range.xyz);
        let cos_axis = max(dot(to * inverseSqrt(d2), axis), 0.05);
        let omega = params.x * params.x * cos_axis * cos_axis * cos_axis;
        // Reflected flux, and the patch's area for the near-field term
        // (a disc's: E = L A cos cos / (r^2 + A / pi)).
        let flux = albedo * e.irradiance * d2 * omega;
        let cos_light = max(dot(np, -to * inverseSqrt(d2)), 0.05);
        let area = d2 * omega / cos_light;
        let g = cos_out * cos_in / (3.14159265 * (r2 + area / 3.14159265));
        let c = flux * g / pdf;
        // Russian roulette on the visibility walk (the costly part): a texel
        // worth less than the mean of those before it is walked with
        // probability c / mean (at least 1 in 8) and then counts 1 / that,
        // which keeps the estimate unbiased.
        let lum = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
        var keep = 1.0;
        if tried > 0.0 { keep = clamp(lum * tried / max(seen, 1e-20), 0.125, 1.0); }
        seen += lum;
        tried += 1.0;
        if keep < 1.0 && rsm_rand(seed) >= keep { continue; }
        if !rsm_open(xp, np, p, n) { continue; }
        sum += c / keep;
    }
    return sum / f32(max(count, 1u));
}

@compute @workgroup_size(8, 8, 1)
fn rsm_gather(@builtin(global_invocation_id) gid: vec3<u32>) {
    let osize = textureDimensions(rsm_out);
    if any(gid.xy >= osize) { return; }
    let out_px = vec2<i32>(gid.xy);
    let size = vec2<u32>(view.size.xy);
    let max_px = vec2<i32>(size) - 1;
    let scale = rsm_scale(size);
    let q = min(out_px * scale + scale / 2, max_px);
    let pw = rsm_world(q, size);
    if pw.w <= 0.0 { textureStore(rsm_out, out_px, vec4<f32>(0.0)); return; }
    let p = pw.xyz;
    // The normal from the depth: per axis the neighbour nearer in depth.
    var n = rsm_normal(q, size, p);
    let view_w = (view.view_proj * vec4<f32>(p, 1.0)).w;
    if all(n == vec3<f32>(0.0)) { textureStore(rsm_out, out_px, vec4<f32>(0.0, 0.0, 0.0, view_w)); return; }
    if dot(n, view.cam.xyz - p) < 0.0 { n = -n; }
    var e = vec3<f32>(0.0);
    let cell = clustered_cell(p);
    if FEATURE_CLUSTERED_LIGHTS && cell != NO_CELL {
        var seed = pcg_hash(u32(out_px.x) ^ pcg_hash(u32(out_px.y) ^ pcg_hash(u32(gi.rsm.z))));
        let count = scene_tables[cell].x;
        for (var k = 0u; k < count; k++) {
            let i = clustered_index(cell, k);
            let w = u32(bitcast<vec4<f32>>(scene_tables[light_grid.offsets.w + i * 5u + 3u]).w);
            if (w & 65536u) == 0u { continue; }
            e += rsm_light(i, p, n, u32(gi.rsm.w), &seed);
        }
    }
    // Blend with last frame's where the same surface was (its view depth
    // then matches), else start over.
    var keep = 0.0;
    var history = vec3<f32>(0.0);
    if gi.rsm.y > 1.5 {
        let prev = gi.rsm_prev * vec4<f32>(p, 1.0);
        if prev.w > 1e-4 {
            let uv = vec2<f32>(prev.x / prev.w * 0.5 + 0.5, 0.5 - prev.y / prev.w * 0.5);
            if all(uv >= vec2<f32>(0.0)) && all(uv < vec2<f32>(1.0)) {
                let h = textureLoad(rsm_prev, vec2<i32>(uv * vec2<f32>(osize)), 0);
                if abs(h.a - prev.w) < 0.04 * prev.w { history = h.rgb; keep = 0.9; }
            }
        }
    }
    textureStore(rsm_out, out_px, vec4<f32>(mix(e, history, keep), view_w));
}

// The gathered bounce at a full-resolution pixel: the 2x2 nearest gather
// texels, bilinear, each only where its depth is this pixel's.
fn rsm_irradiance(px: vec2<i32>, p: vec3<f32>) -> vec3<f32> {
    if !rsm_on() { return vec3<f32>(0.0); }
    let osize = vec2<i32>(textureDimensions(rsm_in));
    let f = (vec2<f32>(px) + 0.5) / f32(rsm_scale(vec2<u32>(view.size.xy))) - 0.5;
    let b = vec2<i32>(floor(f));
    let t = f - floor(f);
    let w = (view.view_proj * vec4<f32>(p, 1.0)).w;
    var sum = vec3<f32>(0.0);
    var total = 0.0;
    // Where no tap shares the depth (a thin edge), the nearest in depth.
    var best = vec3<f32>(0.0);
    var best_d = 1e30;
    for (var k = 0; k < 4; k++) {
        let o = vec2<i32>(k & 1, k >> 1);
        let c = clamp(b + o, vec2<i32>(0), osize - 1);
        let s = textureLoad(rsm_in, c, 0);
        let bil = select(1.0 - t.x, t.x, o.x == 1) * select(1.0 - t.y, t.y, o.y == 1);
        let d = abs(s.a - w) / max(w, 1e-4);
        let near = 1.0 - smoothstep(0.02, 0.06, d);
        sum += s.rgb * bil * near;
        total += bil * near;
        if d < best_d { best_d = d; best = s.rgb; }
    }
    return select(best, sum / total, total > 1e-3);
}

// Edge-aware a-trous filter of the gather (Dammertz et al. 2010, as in
// SVGF): three passes of a 5x5 B3 spline kernel at steps 1, 2 and 4
// gather texels, after the temporal blend. A tap counts as far as it lies
// on the centre's plane (the depth buffer's surface at both), so the bounce
// does not smear across a corner or onto a nearer object.
fn rsm_filter(out_px: vec2<i32>, step: i32) {
    let osize = vec2<i32>(textureDimensions(rsm_out));
    if any(out_px >= osize) { return; }
    let centre = textureLoad(rsm_prev, out_px, 0);
    let size = vec2<u32>(view.size.xy);
    let max_px = vec2<i32>(size) - 1;
    let scale = rsm_scale(size);
    let q = min(out_px * scale + scale / 2, max_px);
    let pw = rsm_world(q, size);
    if pw.w <= 0.0 { textureStore(rsm_out, out_px, centre); return; }
    let p = pw.xyz;
    let n = rsm_normal(q, size, p);
    // Tolerance off the plane: a few millimetres per metre of distance.
    let tol = 0.01 * centre.a + 0.01;
    let h = array<f32, 5>(0.0625, 0.25, 0.375, 0.25, 0.0625);
    var sum = vec3<f32>(0.0);
    var total = 0.0;
    for (var j = -2; j <= 2; j++) {
        for (var i = -2; i <= 2; i++) {
            let c = out_px + vec2<i32>(i, j) * step;
            if any(c < vec2<i32>(0)) || any(c >= osize) { continue; }
            let s = textureLoad(rsm_prev, c, 0);
            let tw = rsm_world(min(c * scale + scale / 2, max_px), size);
            if tw.w <= 0.0 { continue; }
            let off = abs(dot(tw.xyz - p, n));
            let w = h[i + 2] * h[j + 2] * exp(-off / tol);
            sum += s.rgb * w;
            total += w;
        }
    }
    textureStore(rsm_out, out_px, vec4<f32>(sum / max(total, 1e-6), centre.a));
}

@compute @workgroup_size(8, 8, 1)
fn rsm_filter1(@builtin(global_invocation_id) gid: vec3<u32>) { rsm_filter(vec2<i32>(gid.xy), 1); }
@compute @workgroup_size(8, 8, 1)
fn rsm_filter2(@builtin(global_invocation_id) gid: vec3<u32>) { rsm_filter(vec2<i32>(gid.xy), 2); }
@compute @workgroup_size(8, 8, 1)
fn rsm_filter4(@builtin(global_invocation_id) gid: vec3<u32>) { rsm_filter(vec2<i32>(gid.xy), 4); }

// ---- Reflection probes (reflection_probes.rs) ----
@group(1) @binding(27) var probe_cube: texture_cube_array<f32>;

// A room (reflection_probes.rs ProbeBox, the boxes rooms declare for their
// probes): how far p lies outside box i (0 inside), and how much p belongs
// to any room, 1 within 2 cm of a box (a wall's face lies on its room's
// bounds) and fading to 0 at reflection_probes::FADE_M outside every box.
// Shared by the probes' reflections and the froxel fog's bounds, so both
// have one idea of where a room is.
const ROOM_FADE_M: f32 = 0.5;
fn room_box_out(i: u32, p: vec3<f32>) -> f32 {
    let lo = gi.probe_boxes[3u * i + 1u].xyz;
    let hi = gi.probe_boxes[3u * i + 2u].xyz;
    return length(max(lo - p, vec3<f32>(0.0)) + max(p - hi, vec3<f32>(0.0)));
}
fn room_presence(p: vec3<f32>) -> f32 {
    var w = 0.0;
    for (var i = 0u; i < min(u32(gi.probes.y), 8u); i++) {
        let out = room_box_out(i, p);
        w = max(w, select(1.0 - smoothstep(0.0, ROOM_FADE_M, out), 1.0, out <= 0.02));
    }
    return w;
}

// The probes' reflection of direction r at p: the ray box-projected (where
// it leaves the room, seen from the capture point), read at the roughness's
// mip. Boxes that hold p (within 2 cm: a wall's face lies on its room's
// bounds) count alone; a point outside every box (in a doorway, in the
// wall between two rooms) blends the boxes within reflection_probes::FADE_M
// that the reflected ray heads into, each fading to 0 at that distance. rgb the mean, a the weight (at most 1).
fn probe_reflection(p: vec3<f32>, r: vec3<f32>, rough: f32) -> vec4<f32> {
    let count = u32(gi.probes.y);
    if count == 0u || gi.probes.x > 0.5 || gi.probes.z < 0.5 { return vec4<f32>(0.0); }
    let levels = f32(textureNumLevels(probe_cube));
    var inside = vec4<f32>(0.0);
    var near = vec4<f32>(0.0);
    for (var i = 0u; i < min(count, 8u); i++) {
        let cap = gi.probe_boxes[3u * i];
        if cap.w < 0.5 { continue; }
        let lo = gi.probe_boxes[3u * i + 1u].xyz;
        let hi = gi.probe_boxes[3u * i + 2u].xyz;
        let out = room_box_out(i, p);
        if out >= ROOM_FADE_M { continue; }
        // Outside the box, only where the reflected ray heads into it: a
        // door's face sees its own room, not the room behind the door.
        if out > 0.02 {
            let inv = 1.0 / select(r, vec3<f32>(1e-6), abs(r) < vec3<f32>(1e-6));
            let ta = (lo - p) * inv;
            let tb = (hi - p) * inv;
            let enter = max(max(min(ta.x, tb.x), min(ta.y, tb.y)), min(ta.z, tb.z));
            let exit = min(min(max(ta.x, tb.x), max(ta.y, tb.y)), max(ta.z, tb.z));
            if exit <= max(enter, 0.0) { continue; }
        }
        let q = clamp(p, lo, hi);
        let far = select(lo, hi, r > vec3<f32>(0.0));
        let ts = (far - q) / select(r, vec3<f32>(1e-6), abs(r) < vec3<f32>(1e-6));
        let t = min(ts.x, min(ts.y, ts.z));
        let c = textureSampleLevel(probe_cube, canopy_sampler, q + r * max(t, 0.0) - cap.xyz, i32(i), rough * (levels - 1.0)).rgb;
        if out <= 0.02 { inside += vec4<f32>(c, 1.0); }
        else { let w = 1.0 - smoothstep(0.0, ROOM_FADE_M, out); near += vec4<f32>(c * w, w); }
    }
    if inside.a > 0.0 { return vec4<f32>(inside.rgb / inside.a, 1.0); }
    if near.a <= 0.0 { return vec4<f32>(0.0); }
    return vec4<f32>(near.rgb / near.a, min(near.a, 1.0));
}
