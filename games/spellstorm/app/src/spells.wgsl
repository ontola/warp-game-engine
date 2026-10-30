// The signature spells' looks: the tidal wave, the black hole and the
// meteor (Godot's wave, black_hole and molten shaders). Like fx.wgsl this
// reloads while the game runs; the two are compiled as one, and this
// file's game_sprite hands every other effect to fx.wgsl's (fx_sprite).
// It can use fx.wgsl's noise (sn_hash, sn_noise, sn_fbm, el_cells, lin).

// ---- The tidal wave (wave.gdshader) ----
// params: time, sink (1 standing, 0 gone into a flood).
// A wall of water: deep blue at the foot, clear green-blue up the face
// where light comes through, white churning foam on the crest; the crest
// heaves and tears, never a straight edge.

fn wave(s: GameSprite) -> vec4<f32> {
    let t = s.params.x;
    let x = s.uv.x;
    // Lower towards both ends, like Godot's swept profile.
    let k = 1.0 - pow(abs(x * 2.0 - 1.0), 3.0);
    let crest_n = sn_noise(vec3(x * 6.0, t * 1.5, 0.0));
    let top = 0.15 + 0.85 * k - (crest_n - 0.5) * 0.25;
    let h = (1.0 - s.uv.y) / max(top, 0.05);
    if h > 1.0 {
        // Spray torn off the crest.
        let spray = smoothstep(0.55, 0.85, sn_fbm(vec3(x * 30.0, (1.0 - s.uv.y) * 8.0 - t * 4.0, t))) * (1.0 - smoothstep(1.0, 1.25, h));
        return vec4(lin(vec3(0.92, 0.97, 1.0)), spray * 0.8);
    }
    let n = sn_fbm(vec3(x * 18.0, h * 6.0 - t * 3.0, t * 0.5));
    var c = mix(vec3(0.02, 0.12, 0.32), vec3(0.1, 0.55, 0.7), smoothstep(0.1, 0.7, h + (n - 0.5) * 0.3));
    var foam = smoothstep(0.72, 0.9, h + (n - 0.5) * 0.35);
    foam = max(foam, smoothstep(0.62, 0.8, n) * smoothstep(0.3, 0.9, h) * 0.8);
    foam = max(foam, (1.0 - smoothstep(0.0, 0.12, h)) * smoothstep(0.45, 0.7, n));
    c = mix(c, vec3(0.92, 0.97, 1.0), foam);
    // A little light of its own, so it reads in the dark levels.
    c = c * 1.25 + vec3(0.15, 0.2, 0.25) * foam;
    let ends = smoothstep(0.0, 0.08, x) * smoothstep(0.0, 0.08, 1.0 - x);
    return vec4(lin(c), clamp((0.7 + 0.3 * foam) * ends, 0.0, 1.0));
}

// ---- The black hole (black_hole.gdshader) ----
// params: open (0..1, it grows in and collapses), time. The sprite's
// radiance is the void's colour. A soft black core, a thin ring of light
// hugging it, and spiral arms of dust flowing inward. (Godot also bent the
// scene behind it; sprites cannot read the screen.)

fn bh_hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

fn bh_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    var f = fract(p);
    f = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    return mix(mix(bh_hash(i), bh_hash(i + vec2(1.0, 0.0)), f.x), mix(bh_hash(i + vec2(0.0, 1.0)), bh_hash(i + vec2(1.0, 1.0)), f.x), f.y);
}

fn bh_rot(a: f32) -> mat2x2<f32> {
    return mat2x2<f32>(cos(a), sin(a), -sin(a), cos(a));
}

fn black_hole(s: GameSprite) -> vec4<f32> {
    let open = s.params.x;
    let t = s.params.y;
    let core: f32 = 0.26;
    let q = s.uv * 2.0 - 1.0;
    let r = length(q);
    let fall = smoothstep(1.0, 0.15, r) * open;
    // Spiral arms: noise wound tighter towards the middle, two copies
    // zooming in half a beat apart so the dust flows in forever.
    let p = bh_rot(2.4 / (r + 0.3) + t * 0.9) * q;
    var arms: f32 = 0.0;
    for (var i = 0; i < 2; i++) {
        let ph = fract(t * 0.22 + f32(i) * 0.5);
        let w = 1.0 - abs(2.0 * ph - 1.0);
        let z = p * 2.6 * exp2(ph) * vec2(1.0, 0.45) + f32(i) * 7.3;
        arms += w * (bh_noise(z) * 0.65 + bh_noise(z * 2.1 + 3.0) * 0.35);
    }
    let h = core * open;
    arms = smoothstep(0.42, 0.85, arms) * smoothstep(1.0, 0.45, r) * smoothstep(h, h + 0.2, r);
    let ring = exp(-pow((r - h * 1.12) / 0.035, 2.0)) * open;
    var col = vec3(1.0) * arms * 1.2 * open + vec3<f32>(1.24, 1.12, 1.24) * ring * 1.6;
    // Darkens what is behind it towards the middle, black in the core.
    let dark = max(0.55 * fall, 1.0 - smoothstep(h * 0.75, h * 1.05, r));
    let a = max(dark, clamp(arms * open + ring, 0.0, 1.0)) * smoothstep(1.0, 0.8, r);
    let lit = clamp(arms * open + ring, 0.0, 1.0);
    return vec4(col * (1.0 - smoothstep(h * 1.05, h * 0.75, r)) / max(a, 0.001) * lit, a);
}

// ---- A molten rock (the meteor, molten.gdshader) ----
// params: seed, time. Black crust drifting over glowing rock.

fn molten(s: GameSprite) -> vec4<f32> {
    let q = s.uv * 2.0 - 1.0;
    let r = length(q);
    if r > 1.0 {
        return vec4(0.0);
    }
    // A ball: the surface point under this pixel.
    let pz = sqrt(1.0 - r * r);
    let p = vec3(q, pz) * 2.6 + vec3(s.params.x, -s.params.y * 2.5, 0.0);
    let v = el_cells(p);
    let glow = 1.0 - smoothstep(0.0, 0.14, v.x);
    let churn = sn_fbm(p * 1.5 + vec3(s.params.y * 0.5));
    let light = pz;
    let crust = vec3(0.12, 0.05, 0.03) * (0.6 + 0.8 * light);
    let c = mix(crust * 0.15, mix(vec3(1.0, 0.45, 0.1), vec3(1.0, 0.9, 0.6), churn), max(glow, smoothstep(0.6, 0.85, churn)));
    return vec4(c, smoothstep(1.0, 0.94, r));
}

// ---- A pool of blood (blood.gdshader) ----
// params: where the nearest wizard is (x, z from the middle, metres), pull
// (0..1, fraction) + 1000 x suck (how far it has been drunk), time. The
// side facing the wizard runs out to them in a tongue; drinking draws the
// whole pool along it, thinning to nothing.

fn blood(s: GameSprite) -> vec4<f32> {
    let to = s.params.xy;
    let pull = fract(s.params.z);
    let suck = floor(s.params.z) / 1000.0;
    let t = s.params.w;
    // This pixel in metres from the pool's middle (x east, y south).
    let v = (s.uv - 0.5) * s.size;
    let dist = length(to);
    let d = select(vec2(0.0, 1.0), to / max(dist, 1e-4), dist > 1e-4);
    let side = vec2(-d.y, d.x);
    // Drunk: it runs along the tongue towards them, narrow and small.
    let shift = dist * 0.8 * suck * suck;
    let shrink = 1.0 - 0.6 * suck * suck;
    var along = (dot(v, d) - shift) / shrink;
    let across = dot(v, side) / (shrink * (1.0 - 0.85 * suck));
    // A ragged edge, different on every pool, slowly breathing.
    let ang = atan2(across, along);
    let rag = sn_fbm(vec3(cos(ang) * 1.6 + fract(t * 0.0103) * 50.0, sin(ang) * 1.6, t * 0.25));
    let radius = 0.35 * (0.75 + 0.55 * rag);
    let body = length(vec2(along, across)) / radius;
    // The tongue: a tapering strip out to the drinker.
    let reach = pull * 0.55 * radius / 0.3 * 2.2 + suck * dist;
    let k = clamp(along / max(reach, 1e-3), 0.0, 1.0);
    let half = radius * 0.55 * (1.0 - k * 0.7);
    let tongue = select(9.0, abs(across) / max(half, 1e-3) + step(reach, along) * 9.0, along > 0.0 && reach > 0.01);
    let f = min(body, tongue);
    let inside = 1.0 - smoothstep(0.85, 1.0, f);
    if inside <= 0.0 {
        return vec4(0.0);
    }
    // Dark, wet and glossy, lighter red where it is thin at the edge, a
    // sheen sliding over it.
    let rim = smoothstep(0.5, 1.0, f);
    let sheen = smoothstep(0.65, 0.9, sn_noise(vec3(v * 5.0, t * 0.6)));
    var c = mix(vec3(0.16, 0.0, 0.005), vec3(0.42, 0.02, 0.02), rim * 0.6);
    c += vec3(0.5, 0.15, 0.15) * sheen * (1.0 - rim) * 0.35;
    c += vec3(0.12, 0.0, 0.0) * (0.6 + 0.8 * pull);
    return vec4(lin(c), inside * (1.0 - 0.5 * suck));
}

// ---- A jet of water (the water spray) ----
// params: time, seed. A shaft along the jet, its top (uv.y 0) the far end.
// Dense white water, tight at the staff, streaks rushing outward, the edge
// ragged and widening until it tears into spray at the end.

fn jet(s: GameSprite) -> vec4<f32> {
    let t = s.params.x;
    let a = 1.0 - s.uv.y;
    let m = a * s.size.y;
    let x = s.uv.x * 2.0 - 1.0;
    let n = sn_fbm(vec3(x * 2.5, m * 2.2 - t * 13.0, s.params.y));
    let w = 0.22 + 0.78 * smoothstep(0.0, 1.0, a);
    let edge = abs(x) / w + (n - 0.5) * (0.25 + 0.6 * a);
    let core = 1.0 - smoothstep(0.3, 0.95, edge);
    let streak = sn_noise(vec3(x * 9.0 / w, m * 1.4 - t * 19.0, s.params.y + 3.1));
    let tear = 1.0 - smoothstep(0.7, 1.0, a + 0.25 * (n - 0.5));
    let alpha = core * tear * smoothstep(0.0, 0.05, a) * (0.6 + 0.4 * streak);
    let c = mix(lin(vec3(0.62, 0.8, 0.95)), vec3(1.0), smoothstep(0.2, 0.8, core) * (0.7 + 0.3 * streak));
    return vec4(c, clamp(alpha, 0.0, 1.0));
}

// ---- The water a tidal wave pushes (surge) ----
// params: time, sink, seed. Flat on the floor, its top (uv.y 0) the front.
// Deep churning water, a white rolling front edge, foam streaks dragged
// back along the flow, thinning out behind and fraying at the ends.

fn surge(s: GameSprite) -> vec4<f32> {
    let t = s.params.x;
    let sink = s.params.y;
    let x = s.uv.x;
    let back = s.uv.y;
    let m = vec2(x * s.size.x, back * s.size.y);
    let n = sn_fbm(vec3(m.x * 0.9, m.y * 1.6 + t * 6.0, s.params.z));
    let ends = smoothstep(0.0, 0.07 + 0.05 * n, x) * smoothstep(0.0, 0.07 + 0.05 * n, 1.0 - x);
    let tail = 1.0 - smoothstep(0.45, 1.0, back + (n - 0.5) * 0.3);
    var c = mix(vec3(0.04, 0.2, 0.42), vec3(0.12, 0.5, 0.68), n);
    let front = 1.0 - smoothstep(0.0, 0.16 + 0.1 * n, back);
    let streak = smoothstep(0.6, 0.85, sn_noise(vec3(m.x * 4.0, m.y * 0.8 + t * 5.0, s.params.z + 7.0)));
    let foam = max(front, streak * 0.7);
    c = mix(c, vec3(0.92, 0.97, 1.0), foam);
    c = c * 1.2 + vec3(0.1, 0.15, 0.2) * foam;
    return vec4(lin(c), clamp(ends * tail * (0.75 + 0.25 * foam) * (0.3 + 0.7 * sink), 0.0, 1.0));
}

// ---- The lane a rolling spell will sweep (cast_mark.gdshader mode 1) ----
// params: fill (0..1 as it charges), time, length in metres. Flat on the
// floor, its top (uv.y 0) the far end: a bright rim, a faint fill and
// chevrons running the way it will go.

fn lane(s: GameSprite) -> vec4<f32> {
    let fill = s.params.x;
    let t = s.params.y;
    let along = (1.0 - s.uv.y) * s.params.z;
    let x = abs(s.uv.x - 0.5) * 2.0;
    let m = vec2(x * s.size.x * 0.5, min(s.uv.y, 1.0 - s.uv.y) * s.size.y);
    let rim = 1.0 - smoothstep(0.06, 0.16, min(s.size.x * 0.5 - m.x, m.y));
    let chev = fract(along * 0.4 - x * 0.9 - t * 1.6);
    let arrow = smoothstep(0.0, 0.08, chev) * (1.0 - smoothstep(0.12, 0.3, chev)) * (1.0 - x * 0.6);
    let ends = smoothstep(0.0, 0.08, s.uv.y) * smoothstep(0.0, 0.25, 1.0 - s.uv.y);
    let a = (rim * 0.8 + arrow * 0.3 + 0.035) * fill * mix(0.5, 1.0, ends);
    return vec4(vec3(1.0), clamp(a, 0.0, 1.0));
}

fn game_sprite(s: GameSprite) -> vec4<f32> {
    switch s.effect {
        case 66u: { return wave(s); }
        case 67u: { return black_hole(s); }
        case 68u: { return molten(s); }
        case 69u: { return blood(s); }
        case 70u: { return jet(s); }
        case 71u: { return surge(s); }
        case 72u: { return lane(s); }
        default: { return fx_sprite(s); }
    }
}
