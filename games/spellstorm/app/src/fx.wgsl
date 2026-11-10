// Spellstorm's effect looks, after the Godot game's src/fx shaders. This
// file reloads while the game runs (`spellstorm play --live`, or just save
// it): the next frame draws with it, and a file that does not compile keeps
// the last good one (the log says why).
//
// The engine calls game_sprite for every sprite with an effect:
//   s.uv      0..1 across the quad, y down (0 at the top)
//   s.size    the quad in metres
//   s.world   this pixel in the world
//   s.effect  which look (below)
//   s.params  the effect's numbers
//   s.texel   the atlas texel under the quad (premultiplied)
// It returns a colour (straight, linear) and the coverage in alpha. The
// sprite's own radiance (its colour and brightness from the game) scales the
// colour; additive sprites only add light.
//
// Effects 1..25 are the particle cells (effects.rs): 1 + the cell's index,
// params (seed, time, 0, 0). 64 is a wall of mist, 65 a field on the ground.

// ---- Noise (noise.gdshaderinc) ----

fn sn_hash(q: vec3<f32>) -> f32 {
    var p = fract(q * 0.3183099 + 0.1);
    p *= 17.0;
    return fract(p.x * p.y * p.z * (p.x + p.y + p.z));
}

fn sn_noise(x: vec3<f32>) -> f32 {
    let i = floor(x);
    var f = fract(x);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(mix(sn_hash(i), sn_hash(i + vec3(1.0, 0.0, 0.0)), f.x), mix(sn_hash(i + vec3(0.0, 1.0, 0.0)), sn_hash(i + vec3(1.0, 1.0, 0.0)), f.x), f.y),
        mix(mix(sn_hash(i + vec3(0.0, 0.0, 1.0)), sn_hash(i + vec3(1.0, 0.0, 1.0)), f.x), mix(sn_hash(i + vec3(0.0, 1.0, 1.0)), sn_hash(i + vec3(1.0, 1.0, 1.0)), f.x), f.y), f.z);
}

fn sn_fbm(q: vec3<f32>) -> f32 {
    var p = q;
    var v = 0.0;
    var a = 0.5;
    for (var i = 0; i < 4; i++) {
        v += a * sn_noise(p);
        p = p * 2.03 + vec3(1.7, 9.2, 3.1);
        a *= 0.5;
    }
    return v;
}

// Voronoi: x the distance to the nearest cell edge, y the cell's id.
fn el_cells(p: vec3<f32>) -> vec2<f32> {
    let i = floor(p);
    let f = fract(p);
    var d1 = 8.0;
    var d2 = 8.0;
    var id = 0.0;
    for (var z = -1; z <= 1; z++) {
        for (var y = -1; y <= 1; y++) {
            for (var x = -1; x <= 1; x++) {
                let g = vec3(f32(x), f32(y), f32(z));
                let o = vec3(sn_hash(i + g), sn_hash(i + g + 17.0), sn_hash(i + g + 31.0));
                let d = length(g + o - f);
                if d < d1 {
                    d2 = d1;
                    d1 = d;
                    id = sn_hash(i + g + 5.0);
                } else if d < d2 {
                    d2 = d;
                }
            }
        }
    }
    return vec2(d2 - d1, id);
}

// Godot's colours are sRGB; the sprite wants linear light.
fn lin(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3(0.0)), vec3(2.2));
}

// The atlas texel as it is, straight (the look before fx.wgsl).
fn atlas(s: GameSprite) -> vec4<f32> {
    return vec4(s.texel.rgb / max(s.texel.a, 1.0 / 255.0), s.texel.a);
}

// ---- The particle cells ----

// One tongue of fire licking upwards (flame.gdshader). The sprite gives the
// colour; this gives the shape and the white-hot heart.
fn flame(s: GameSprite) -> vec4<f32> {
    let p = vec2(s.uv.x * 2.0 - 1.0, 1.0 - s.uv.y);
    let seed = s.params.x;
    let t = s.params.y;
    // Domain-warped noise rising through the quad: a first noise bends the
    // coordinates of a second, so the flame curls and tears instead of
    // sliding as a rigid pattern.
    // No little flame shape per sprite (many of those read as tiny cartoon
    // fires): each particle is a soft ball of burning gas whose edge the
    // noise eats away, so overlapping particles merge into one body of fire.
    let c2 = s.uv * 2.0 - 1.0;
    let q = vec2(c2.x * 1.4, c2.y * 1.4 + t * 1.6);
    let warp = vec2(sn_fbm(vec3(q, seed)), sn_fbm(vec3(q + vec2(5.2, 1.3), seed + 3.1)));
    let n = sn_fbm(vec3(q * 1.8 + (warp - 0.5) * 1.8, seed + 7.7 + t * 0.4));
    // params.z: the particle's age 0..1 (0 from older builds). Young gas at a
    // flamethrower's nozzle burns white-hot; it cools to orange and dark red.
    let age = s.params.z;
    let r = length(c2);
    // Tongues, not balls: the noise eats deep into the edge and leaves
    // holes, so overlapping particles read as torn flame.
    // Hot while young, cooling as it burns out: a flame is yellow at the
    // nozzle, orange in its body and red at its tips.
    let body = 1.0 - smoothstep(0.08, 1.0, r + (n - 0.5) * 1.15);
    let heat = clamp(body * (0.2 + 0.85 * n) * (1.15 - 1.0 * age), 0.0, 1.0);
    // Blackbody, with brightness tied to heat so the hue follows it: dim
    // gas is deep red, bright gas orange and yellow, only the heart near
    // white. (The stops are sRGB, made linear: a pale linear yellow shows
    // as cream, and a bright red saturates in the tone curve into pink.)
    // (AgX turns a deep linear orange salmon; these lean yellow for it.)
    var c = mix(lin(vec3(0.55, 0.1, 0.01)), lin(vec3(1.0, 0.56, 0.08)), smoothstep(0.1, 0.45, heat));
    c = mix(c, lin(vec3(1.0, 0.78, 0.25)), smoothstep(0.45, 0.78, heat));
    c = mix(c, lin(vec3(1.0, 0.93, 0.7)), smoothstep(0.88, 1.0, heat));
    let a = smoothstep(0.1, 0.32, heat);
    return vec4(c * (0.3 + 1.5 * heat), a);
}

// A soft, ragged puff of smoke that turns as it goes.
fn puff(s: GameSprite) -> vec4<f32> {
    let p = s.uv * 2.0 - 1.0;
    let seed = s.params.x;
    let t = s.params.y;
    let r = length(p);
    let n = sn_fbm(vec3(p * 1.6 + vec2(seed, 0.0), t * 0.35 + seed));
    let fine = sn_fbm(vec3(p * 4.0 - vec2(0.0, t * 0.3), seed + 4.0));
    let body = 1.0 - smoothstep(0.25, 1.0, r + (n - 0.5) * 0.7);
    let a = body * (0.45 + 0.55 * smoothstep(0.25, 0.75, n)) * (0.8 + 0.4 * fine);
    return vec4(vec3(0.75 + 0.4 * n), clamp(a, 0.0, 1.0));
}

// A wisp of mist: thin, drawn out, drifting.
fn wisp(s: GameSprite) -> vec4<f32> {
    let p = s.uv * 2.0 - 1.0;
    let seed = s.params.x;
    let t = s.params.y;
    let n = sn_fbm(vec3(p.x * 1.4 + seed, p.y * 2.2 - t * 0.3, t * 0.1));
    let r = length(p * vec2(0.8, 1.2));
    let a = (1.0 - smoothstep(0.2, 1.0, r + (n - 0.5) * 0.8)) * smoothstep(0.25, 0.7, n);
    return vec4(vec3(0.85 + 0.3 * n), clamp(a, 0.0, 1.0));
}

// A soft glow round anything bright (halo.gdshader): the wide faint part.
fn halo_out(s: GameSprite) -> vec4<f32> {
    let r = length(s.uv * 2.0 - 1.0);
    let k = pow(clamp(1.0 - r, 0.0, 1.0), 2.4);
    return vec4(vec3(1.0), k);
}

// The bright inner bloom of the same glow.
fn halo_in(s: GameSprite) -> vec4<f32> {
    let r = length(s.uv * 2.0 - 1.0);
    let k = pow(clamp(1.0 - r * 2.2, 0.0, 1.0), 2.0);
    return vec4(vec3(1.0), k);
}


// Distance in metres from this pixel to the quad's spine: a capsule along
// its up, as wide as the quad. 0..1 along it from the head (top) is in .y.
fn spine(s: GameSprite) -> vec2<f32> {
    let half = s.size.x * 0.5;
    let x = (s.uv.x - 0.5) * s.size.x;
    let y = s.uv.y * s.size.y;
    let yc = clamp(y, half, max(s.size.y - half, half));
    return vec2(length(vec2(x, y - yc)) / max(half, 1e-4), clamp((y - half) / max(s.size.y - 2.0 * half, 1e-4), 0.0, 1.0));
}

// An ember or a spark: a hard hot point that flickers, or drawn stretched
// a streak whose head burns brightest and whose tail fades. params.z: age.
fn ember(s: GameSprite) -> vec4<f32> {
    let seed = s.params.x;
    let t = s.params.y;
    let age = s.params.z;
    let sp = spine(s);
    let d = sp.x;
    let tail = 1.0 - 0.75 * sp.y;
    let core = exp(-d * d * 9.0);
    // Faded to nothing at the quad's edge, or a slow one shows a square.
    let glow = exp(-d * d * 3.5) * 0.35 * (1.0 - smoothstep(0.7, 1.0, d));
    let flick = 0.65 + 0.35 * sn_noise(vec3(seed * 3.1, t * 14.0 + seed, 0.0));
    // Hot white at the heart, the sprite's colour round it.
    let c = mix(vec3(1.0), vec3(1.6, 1.25, 0.9), core * (1.0 - age));
    return vec4(c, clamp((core + glow) * tail * flick, 0.0, 1.0));
}

// A crystal of ice along the quad's up (tip at the top): a long diamond
// with a lit facet and a shadowed one, a bright ridge and rim, inner flaws,
// and a glint that runs along it as it turns.
fn shard(s: GameSprite) -> vec4<f32> {
    let seed = s.params.x;
    let t = s.params.y;
    let x = s.uv.x * 2.0 - 1.0;
    let y = 1.0 - s.uv.y * 2.0;
    let wide = 0.3;
    let w = select((y + 1.0) / (1.0 + wide) * 0.92, (1.0 - y) / (1.0 - wide) * 0.92, y > wide);
    let edge = abs(x) / max(w, 1e-3);
    let a = 1.0 - smoothstep(0.86, 1.0, edge);
    // Two facets: one catches the light, the other is the dark of deep ice.
    let lit = select(0.35, 1.0, x < 0.0);
    let ridge = 1.0 - smoothstep(0.0, 0.12, abs(x));
    let rim = smoothstep(0.6, 0.95, edge);
    let flaw = smoothstep(0.55, 0.8, sn_fbm(vec3(x * 3.0, y * 5.0, seed)));
    let sweep = fract(t * 0.9 + seed * 0.37) * 3.0 - 1.0;
    let glint = exp(-pow((y - sweep) * 4.0, 2.0));
    let lum = lit * (0.45 + 0.25 * flaw) + ridge * 0.6 + rim * 0.5 + glint * 1.4;
    let c = mix(vec3(0.55, 0.8, 1.0), vec3(1.0), clamp(ridge + glint + rim * 0.5, 0.0, 1.0));
    return vec4(c * lum, a * (0.55 + 0.45 * max(rim, ridge)));
}

// A four-pointed star that flashes for a moment as a crystal turns towards
// the light: mostly dark, now and then a sharp twinkle.
fn glint(s: GameSprite) -> vec4<f32> {
    let seed = s.params.x;
    let t = s.params.y;
    let p = s.uv * 2.0 - 1.0;
    let r = length(p);
    let rays = exp(-abs(p.x) * 26.0) * pow(max(1.0 - abs(p.y), 0.0), 3.0) + exp(-abs(p.y) * 26.0) * pow(max(1.0 - abs(p.x), 0.0), 3.0);
    let q = vec2(p.x + p.y, p.x - p.y) * 0.7071;
    let diag = (exp(-abs(q.x) * 30.0) * pow(max(1.0 - abs(q.y) * 1.6, 0.0), 3.0) + exp(-abs(q.y) * 30.0) * pow(max(1.0 - abs(q.x) * 1.6, 0.0), 3.0)) * 0.4;
    let core = exp(-r * r * 30.0);
    let tw = pow(0.5 + 0.5 * sin(t * (7.0 + fract(seed * 0.618) * 9.0) + seed * 5.0), 6.0);
    let k = 0.15 + 0.85 * tw;
    return vec4(vec3(1.0), clamp((core + (rays + diag) * k) * (0.4 + 0.6 * k), 0.0, 1.0));
}

// A bubble: a thin rim, a highlight, see-through inside. It swells and, in
// the last part of its life, pops: the rim tears open and flies apart.
fn bubble(s: GameSprite) -> vec4<f32> {
    let seed = s.params.x;
    let age = s.params.z;
    let p = s.uv * 2.0 - 1.0;
    let pop = smoothstep(0.8, 1.0, age);
    let rr = 0.62 + 0.3 * pop;
    let r = length(p);
    let ang = atan2(p.y, p.x);
    let torn = select(1.0, step(pop * 0.9, sn_noise(vec3(cos(ang) * 3.0 + seed, sin(ang) * 3.0, seed))), pop > 0.0);
    let rim = exp(-pow((r - rr) / (0.07 + 0.08 * pop), 2.0)) * torn;
    let hi = exp(-dot(p + vec2(0.28, 0.3), p + vec2(0.28, 0.3)) * 50.0) * (1.0 - pop);
    let fill = select(0.0, 0.12, r < rr) * (1.0 - pop);
    return vec4(vec3(1.0) + vec3(hi), clamp(rim * (1.0 - 0.6 * pop) + hi + fill, 0.0, 1.0));
}

fn cell_look(s: GameSprite) -> vec4<f32> {
    switch s.effect {
        case 2u: { return flame(s); }
        case 7u: { return puff(s); }
        case 8u: { return halo_out(s); }
        case 9u: { return halo_in(s); }
        case 15u: { return wisp(s); }
        case 19u: { return shard(s); }
        case 20u: { return glint(s); }
        case 21u: { return ember(s); }
        case 22u: { return bubble(s); }
        default: { return atlas(s); }
    }
}

// ---- A wall of mist (ward.gdshader) ----
// params: width, height (metres), open (1 shut, 0 gone), time.

fn ward(s: GameSprite) -> vec4<f32> {
    let size = s.params.xy;
    let open = s.params.z;
    let t = s.params.w;
    let w = vec2(s.uv.x * size.x, (1.0 - s.uv.y) * size.y);
    let rise = t * 0.35;
    let n = sn_fbm(vec3(w.x * 0.35, w.y * 0.6 - rise, t * 0.12));
    let wisp = sn_fbm(vec3(w.x * 0.9 + 7.0, w.y * 1.4 - rise * 1.6, t * 0.2));
    let h = w.y / size.y;
    // Thick below, thinning to nothing at the top, soft at both ends.
    let body = (1.0 - smoothstep(0.0, 1.0, h)) * smoothstep(0.0, 1.2, w.x) * smoothstep(0.0, 1.2, size.x - w.x);
    var a = body * (0.08 + 0.75 * smoothstep(0.3, 0.8, n) + 0.35 * smoothstep(0.4, 0.9, wisp) * (1.0 - h));
    // Runes along the foot: short bright marks that come and go.
    let cell = floor(w.x / 0.7);
    let lit = step(0.55, sn_hash(vec3(cell, floor(t * 0.8 + cell * 0.37), 3.0)));
    let mark = (1.0 - smoothstep(0.0, 0.06, abs(fract(w.x / 0.7) - 0.5) - 0.18)) * (1.0 - smoothstep(0.08, 0.3, w.y));
    var c = mix(vec3(0.35), vec3(0.8), n);
    // The foot melts into the floor instead of ending on a line (seen from
    // above the wall's lowest metre is only a few pixels high).
    a *= smoothstep(0.0, 1.0, w.y + (wisp - 0.5) * 0.6);
    // No rune marks along the foot: they read as flickering squares (Joep).
    // Opening: the haze thins from the top down and drifts apart.
    let gone = smoothstep(open - 0.15, open + 0.15, h * 0.6 + n * 0.4);
    return vec4(lin(c), clamp(a * (1.0 - gone) * open, 0.0, 0.7));
}

// ---- A field on the ground (ground_field.gdshader) ----
// params: the element (El: 0 poison, 1 void, 2 fire, 3 frost, 4 water,
// 5 lightning, 6 earth, 7 air, 8 steam, 9 ice, 10 mud, 11 magma, 12 storm),
// radius (metres), seed, time. The sprite's radiance is how lit it is.

fn ground(s: GameSprite) -> vec4<f32> {
    let kind = i32(s.params.x + 0.5);
    let radius = s.params.y;
    let seed = s.params.z;
    let t = s.params.w;
    let q = s.uv * 2.0 - 1.0;
    let r = length(q);
    let ang = atan2(q.y, q.x);
    // World-sized coordinates, so a big puddle is not a stretched small one.
    let w = q * radius;
    let rag = sn_fbm(vec3(cos(ang) * 1.7 + seed, sin(ang) * 1.7, t * 0.15));
    let edge_r = 0.72 + 0.26 * rag;
    let inside = 1.0 - smoothstep(edge_r - 0.08, edge_r, r);
    let rim = smoothstep(edge_r - 0.25, edge_r - 0.04, r) * inside;
    var c = vec3(0.5);
    var a = inside;
    if kind == 4 {
        // Water: dark and still, rings spreading from a few drip points,
        // lighter where it is shallow at the edge.
        var ripple = 0.0;
        for (var i = 0; i < 3; i++) {
            var at = vec2(sn_hash(vec3(f32(i), seed, 1.0)), sn_hash(vec3(f32(i), seed, 2.0))) * 2.0 - 1.0;
            at *= radius * 0.5;
            let ph = fract(t * 0.45 + f32(i) * 0.33);
            let d = length(w - at);
            ripple += (1.0 - smoothstep(0.0, 0.06, abs(d - ph * radius * 0.7))) * (1.0 - ph);
        }
        let sheen = sn_fbm(vec3(w * 0.9, t * 0.2));
        c = mix(vec3(0.03, 0.08, 0.16), vec3(0.12, 0.3, 0.45), sheen * 0.6 + rim * 0.5);
        c += vec3(0.5, 0.7, 0.85) * ripple * 0.35;
        a *= 0.82;
    } else if kind == 3 || kind == 9 {
        // Frost and ice: pale crystal broken into plates, rime at the rim.
        let wob = vec2(sn_fbm(vec3(w * 2.0, seed)), sn_fbm(vec3(w * 2.0, seed + 5.0))) * 0.35;
        let cell = el_cells(vec3((w + wob) * select(1.6, 1.1, kind == 9), seed));
        let fine = el_cells(vec3((w + wob) * 4.5, seed + 3.0));
        let broken = smoothstep(0.3, 0.6, sn_fbm(vec3(w * 1.5, seed + 9.0)));
        var crack = (1.0 - smoothstep(0.0, 0.018, cell.x)) * (0.4 + 0.6 * broken);
        crack = max(crack, (1.0 - smoothstep(0.0, 0.012, fine.x)) * broken * 0.45);
        let rime = sn_fbm(vec3(w * 3.0, seed));
        c = mix(vec3(0.07, 0.11, 0.16), vec3(0.3, 0.38, 0.46), cell.y * 0.35 + rime * 0.55);
        c = mix(c, vec3(0.55, 0.62, 0.7), max(rim * rime * 0.8, crack * 0.5));
        let glint = step(0.985, sn_hash(vec3(floor(w * 6.0), floor(t * 3.0))));
        c += vec3(0.8, 0.95, 1.0) * glint * crack * 3.0;
        a *= select(0.7, 0.85, kind == 9);
    } else if kind == 10 {
        // Mud: thick and brown, bubbles slowly swelling and popping.
        let cell = el_cells(vec3(w * 1.6, seed + t * 0.05));
        let bubble = smoothstep(0.25, 0.0, cell.x) * (0.5 + 0.5 * sin(t * 2.0 + cell.y * 30.0));
        let n = sn_fbm(vec3(w * 1.2, seed));
        c = mix(vec3(0.12, 0.08, 0.04), vec3(0.26, 0.18, 0.1), n);
        c = mix(c, vec3(0.32, 0.24, 0.14), bubble * 0.6);
        a *= 0.95;
    } else if kind == 8 {
        // Steam: wet, warm stones, a pale shimmer drifting over them.
        let n = sn_fbm(vec3(w * 1.4, t * 0.6));
        c = mix(vec3(0.25, 0.27, 0.3), vec3(0.8, 0.82, 0.86), n);
        a *= 0.25 + 0.2 * n;
    } else if kind == 2 || kind == 11 {
        // Fire (fire_ground.gdshader): scorched dark earth, a warm glow under
        // the flames and embers winking on and off. Magma: molten seams that
        // crawl and glow through a dark crust.
        let lava = select(0.0, 1.0, kind == 11);
        let tt = t * mix(1.0, 0.25, lava);
        let wp = s.world;
        let char_c = mix(vec3(0.05, 0.02, 0.015), vec3(0.3, 0.06, 0.0), lava);
        if kind == 2 {
            let warm = sn_fbm(vec3(wp.x * 0.9, tt * 0.5, wp.z * 0.9));
            let ember = smoothstep(0.72, 0.9, sn_noise(vec3(wp.x * 5.0, wp.z * 5.0, tt * 2.0)));
            c = mix(char_c, vec3(0.45, 0.1, 0.02), smoothstep(0.4, 0.75, warm) * 0.8);
            c += vec3(1.0, 0.55, 0.15) * ember * 1.2;
            a = clamp(inside * 0.6 + ember * inside, 0.0, 1.0);
        } else {
            let n = sn_fbm(vec3(wp.x, tt * 0.7, wp.z) * 0.8 + vec3(0.0, 0.0, -tt * 0.5));
            let seams = pow(1.0 - abs(n * 2.0 - 1.0), 2.5);
            let flick = 0.75 + 0.25 * sn_noise(vec3(wp.x * 2.0, tt * 4.0, wp.z * 2.0));
            let heat = clamp(seams * flick * (0.4 + 0.8 * inside), 0.0, 1.0);
            c = mix(char_c, vec3(0.9, 0.18, 0.02), smoothstep(0.05, 0.45, heat));
            c = mix(c, vec3(1.0, 0.7, 0.25), smoothstep(0.45, 0.85, heat));
            c = mix(c, vec3(1.0, 0.95, 0.7), smoothstep(0.92, 1.0, heat));
            c = mix(char_c * 0.5, c, smoothstep(0.1, 0.5, heat)) * (0.6 + heat * 0.6);
            a = clamp(inside * 0.95 + heat * inside, 0.0, 1.0);
        }
    } else if kind == 0 {
        // Poison: a sickly slick with bubbles of green rising through.
        let cell = el_cells(vec3(w * 1.3, seed + t * 0.1));
        let bubble = smoothstep(0.3, 0.0, cell.x) * (0.5 + 0.5 * sin(t * 3.0 + cell.y * 20.0));
        let n = sn_fbm(vec3(w * 0.8, t * 0.25));
        c = mix(vec3(0.08, 0.2, 0.04), vec3(0.3, 0.6, 0.1), n * 0.7 + bubble * 0.5);
        a *= 0.7;
    } else if kind == 1 {
        // Void: a pool of nothing, a violet swirl at its edge, specks of stars.
        let sw = atan2(q.y, q.x) + t * 0.8 + r * 3.0;
        let swirl = sn_fbm(vec3(cos(sw) * 2.0, sin(sw) * 2.0, t * 0.3));
        c = mix(vec3(0.01, 0.0, 0.03), vec3(0.45, 0.2, 0.8), smoothstep(0.35, 0.8, swirl) * (0.3 + rim * 1.2));
        c += vec3(0.9, 0.8, 1.0) * step(0.985, sn_hash(vec3(floor(w * 9.0), floor(t * 0.5))));
        a *= 0.8;
    } else {
        // Any other element: its colour from the sprite, churning.
        let n = sn_fbm(vec3(w * 0.8, t * 0.4));
        c = vec3(0.5 + 0.5 * n);
        a *= 0.7;
    }
    return vec4(lin(c), clamp(a, 0.0, 1.0));
}

fn game_sprite(s: GameSprite) -> vec4<f32> {
    if s.effect == 64u {
        return ward(s);
    }
    if s.effect == 65u {
        return ground(s);
    }
    return cell_look(s);
}
