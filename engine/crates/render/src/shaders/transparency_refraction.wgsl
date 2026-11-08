// Screen-space refraction for transparency.wgsl, in its own optional bind
// group (no storage buffers): the HDR copied after the glass behind the
// nearest refracting surface, the opaque depth (read-only attachment) and
// the depth of that nearest refracting surface (opaque where there is none).
@group(3) @binding(0) var refraction_color: texture_2d<f32>;
@group(3) @binding(1) var refraction_depth: texture_depth_2d;
@group(3) @binding(2) var refraction_sampler: sampler;
@group(3) @binding(3) var refraction_front: texture_depth_2d;
const REFRACTION_ENABLED: bool = true;

struct RefractedBackground { bent: vec3<f32> };

// Phase 0: glass behind the nearest refracting surface (or anywhere without
// one). 1: that surface. 2: glass nearer than it. Same triangle, same
// vertex shader: its depth matches the front pass to rounding.
fn refraction_phase_keeps(phase: u32, pixel: vec2<f32>, z: f32) -> bool {
    let at = vec2<i32>(pixel);
    let front = textureLoad(refraction_front, at, 0);
    let opaque = textureLoad(refraction_depth, at, 0);
    let has = front > opaque;
    let tolerance = max(front, 1e-12) * 1e-5;
    if phase == 0u { return !has || z < front - tolerance; }
    if phase == 1u { return has && abs(z - front) <= tolerance; }
    return has && z > front + tolerance;
}

fn refraction_pixel_depth(uv: vec2<f32>) -> f32 {
    let size = vec2<i32>(textureDimensions(refraction_depth));
    let pixel = clamp(vec2<i32>(uv * vec2<f32>(size)), vec2<i32>(0), size - 1);
    return textureLoad(refraction_depth, pixel, 0);
}

fn refraction_world(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let hit = view.inv_view_proj * vec4<f32>(ndc, max(depth, 1e-7), 1.0);
    return hit.xyz / hit.w;
}

// `pixel` and `glass_depth` are the fragment's position; `thickness` < 0 is
// a solid reaching the background. Rough glass blurs over a small disc.
fn refraction_background(pixel: vec2<f32>, glass_depth: f32, p: vec3<f32>, v: vec3<f32>, n: vec3<f32>,
    ior: f32, thickness: f32, rough: f32) -> RefractedBackground {
    let size = vec2<f32>(textureDimensions(refraction_color));
    let uv = pixel / size;
    var out: RefractedBackground;
    let depth = refraction_pixel_depth(uv);
    // Reversed Z: 0 is the sky. Treat it as far away.
    let behind = select(1e3, min(distance(refraction_world(uv, depth), p), 1e3), depth > 1e-7);
    let incoming = -v;
    let bent = refract(incoming, n, 1.0 / ior);
    var target_point = p + bent * behind;
    if thickness >= 0.0 {
        // A pane: the ray crosses `thickness`, then leaves parallel to how it came in.
        let inside = min(thickness, behind) / max(dot(bent, -n), 0.2);
        let exit = p + bent * inside;
        target_point = exit + incoming * max(behind - inside, 0.0);
    }
    let clip = view.view_proj * vec4<f32>(target_point, 1.0);
    var bent_uv = uv;
    if clip.w > 1e-5 {
        bent_uv = clamp(clip.xy / clip.w * vec2<f32>(0.5, -0.5) + 0.5, vec2<f32>(0.0), vec2<f32>(1.0));
    }
    // Something in front of the glass is not what the glass shows.
    if refraction_pixel_depth(bent_uv) > glass_depth { bent_uv = uv; }
    let radius = rough * rough * 0.02 * size.y;
    if radius < 0.75 {
        out.bent = textureSampleLevel(refraction_color, refraction_sampler, bent_uv, 0.0).rgb;
        return out;
    }
    var sum = vec3<f32>(0.0);
    for (var i = 0u; i < 8u; i++) {
        // Vogel disc: even coverage with few taps.
        let r = sqrt((f32(i) + 0.5) / 8.0) * radius;
        let a = f32(i) * 2.39996;
        let tap = clamp(bent_uv + vec2<f32>(cos(a), sin(a)) * r / size, vec2<f32>(0.0), vec2<f32>(1.0));
        sum += textureSampleLevel(refraction_color, refraction_sampler, tap, 0.0).rgb;
    }
    out.bent = sum / 8.0;
    return out;
}
