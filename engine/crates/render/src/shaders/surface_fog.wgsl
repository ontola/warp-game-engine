// Shared scene uniforms, sky sampling and distance fog for opaque and forward surfaces.
struct View {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    cam: vec4<f32>,
    planes: array<vec4<f32>, 6>,
    params: vec4<f32>,
    size: vec4<f32>,
    extra: vec4<f32>,
    grass: vec4<f32>,
    fog_cull: vec4<f32>,
    lod_cams: array<vec4<f32>, 4>, // shared sun map distance LOD (cull.wgsl)
    lod: vec4<f32>,
};

struct Light {
    sun_dir: vec4<f32>,      // towards the sun; w: illuminance in lux
    sun_color: vec4<f32>,    // linear colour; w: shadow strength
    shadow_view_proj: mat4x4<f32>,
    sky: vec4<f32>,          // x: sky luminance scale, y: sky rotation (rad), z: diagnostic mode, w: shadow texel (m)
    fog_color: vec4<f32>,    // legacy horizon radiance; w: legacy fog start (m)
    fog: vec4<f32>,          // x: legacy end (m), y: extinction/m, z: aerosol scale height (m; 0 legacy), w: wetness
    fog_scattering: vec4<f32>, // RGB single-scattering albedo; w: aerosol phase asymmetry
    sh: array<vec4<f32>, 9>, // sky irradiance, SH9, already in lux
    near_shadow_view_proj: mat4x4<f32>,
    near_shadow: vec4<f32>, // world texel size, blend-start radius, enabled
    night_stars: vec4<f32>, // peak radiance, zenith optical depth, zero-sun fast path, seed
    anim: vec4<f32>,        // x: seconds for animated procedural materials, y: emission scale, z: radiance shown as white
    fog_sampling: vec4<f32>, // x: sequence phase, y: temporal accumulation enabled, z: diffuse room return (lux), w: water optics (Lighting::water)
    near_levels_view_proj: array<mat4x4<f32>, 7>, // sun clipmap levels 1..7 (frame.rs near_levels)
    near_levels: vec4<f32>, // x: levels drawn, y: one clipmap (the shared map is its outermost level), z: first level (clipmap_base)
    clouds: vec4<f32>,       // cloud layer (cloud_weather.wgsl): coverage, base m, thickness m, extinction per m
    cloud_motion: vec4<f32>, // wind offset xz m, shadow strength, enabled
};

fn sky_dir_uv(d: vec3<f32>) -> vec2<f32> {
    let a = light.sky.y;
    let x = d.x * cos(a) - d.z * sin(a);
    let z = d.x * sin(a) + d.z * cos(a);
    let u = atan2(x, -z) / (2.0 * PI) + 0.5;
    let v = acos(clamp(d.y, -1.0, 1.0)) / PI;
    return vec2<f32>(u, v);
}

fn sky_radiance(d: vec3<f32>) -> vec3<f32> {
    let dims = vec2<i32>(textureDimensions(sky_tex));
    let pixel = sky_dir_uv(d) * vec2<f32>(dims) - 0.5;
    let base = vec2<i32>(floor(pixel));
    let weight = fract(pixel);
    var samples: array<vec3<f32>,4>;
    for (var corner=0u;corner<4u;corner++) {
        let offset=vec2<i32>(i32(corner&1u),i32(corner>>1u));
        let p=base+offset;
        // Equirectangular longitude wraps, latitude clamps at the poles.
        let coord=vec2<i32>((p.x%dims.x+dims.x)%dims.x,clamp(p.y,0,dims.y-1));
        samples[corner]=textureLoad(sky_tex,coord,0).rgb;
    }
    return mix(mix(samples[0],samples[1],weight.x),mix(samples[2],samples[3],weight.x),weight.y)*light.sky.x;
}

// Exact column integral of exp(-max(world_y,0)/height_scale) along a finite
// ray. Below the datum density stays constant. Evaluate the near-horizontal
// limit analytically to avoid cancellation and a horizon seam.
fn fog_column(distance_m: f32, height: f32, vertical: f32, height_scale: f32) -> f32 {
    let extent = max(distance_m, 0.0);
    let end = height + vertical * extent;
    if max(height, end) <= 0.0 { return extent; }
    if min(height, end) >= 0.0 {
        let q = vertical * extent / height_scale;
        if abs(q) < 0.001 {
            return extent * exp(-height / height_scale) * (1.0 - q * 0.5 + q * q / 6.0);
        }
        return height_scale * (exp(-height / height_scale) - exp(-end / height_scale)) / vertical;
    }
    let a = min(height, 0.0) + height_scale * (1.0 - exp(-max(height, 0.0) / height_scale));
    let b = min(end, 0.0) + height_scale * (1.0 - exp(-max(end, 0.0) / height_scale));
    return (b - a) / vertical;
}

// Normalised Cornette-Shanks phase, matching the atmospheric aerosol model.
// Direction points FROM camera TO surface, so forward scattering peaks
// when looking toward the sun. No baked planet horizon participates here.
fn fog_phase(mu: f32, g: f32) -> f32 {
    return 3.0 * (1.0 - g * g) * (1.0 + mu * mu)
        / (8.0 * PI * (2.0 + g * g) * pow(1.0 + g * g - 2.0 * g * mu, 1.5));
}

fn fog_segment(surface: vec3<f32>, scattering_source: vec3<f32>, optical_depth: f32) -> vec3<f32> {
    let transmission = exp(-max(optical_depth, 0.0));
    return surface * transmission + scattering_source * (1.0 - transmission);
}

fn fog_scattering_source(direction: vec3<f32>) -> vec3<f32> {
    // SH0 / sqrt(4*pi) is incident mean sky radiance. Diffuse incident
    // illumination is approximated as isotropic; direct sun keeps its phase.
    // The camera's terrain horizon blocks the directional solar glow. Keep
    // isotropic skylight: shaded air is not black. This bounded approximation
    // does not integrate separate sun visibility along the entire fog ray.
    let diffuse = max(light.sh[0].rgb * 0.282095, vec3<f32>(0.0));
    let solar = light.sun_color.rgb * light.sun_dir.w * clamp(view.fog_cull.z, 0.0, 1.0)
        * fog_phase(clamp(dot(direction, light.sun_dir.xyz), -1.0, 1.0), light.fog_scattering.w);
    return light.fog_scattering.rgb * (diffuse + solar);
}

fn attenuate_background(direction: vec3<f32>, sky: vec3<f32>) -> vec3<f32> {
    if light.fog.z <= 0.0 || light.fog.y <= 0.0 { return sky; }
    let source = fog_scattering_source(direction);
    // Upward rays integrate the remaining finite atmospheric column; rays
    // toward/below the horizon have infinite optical depth in this local
    // plane-parallel layer. Only background rays use the sky as boundary.
    if direction.y <= 0.0 { return source; }
    let column = (max(-view.cam.y, 0.0)
        + light.fog.z * exp(-max(view.cam.y, 0.0) / light.fog.z)) / direction.y;
    return fog_segment(sky, source, light.fog.y * column);
}

// The sky bake is seen from the ground, its horizon at elevation 0. From
// altitude h the true horizon dips by about sqrt(2h/R): between the two the
// bake would show its ground as a tan band above the far terrain. Rays down to
// the true horizon take the horizon sky; rays below it the bake's ground,
// hazed towards the horizon sky with the distance to where they meet the
// planet (hundreds of km from a few km up).
fn sky_at_elevation(direction: vec3<f32>, y: f32) -> vec3<f32> {
    let across = max(length(direction.xz), 1e-6);
    let side = sqrt(max(1.0 - y * y, 0.0));
    return sky_radiance(vec3<f32>(direction.x / across * side, y, direction.z / across * side));
}

fn background_sky(direction: vec3<f32>) -> vec3<f32> {
    if direction.y >= 0.0 { return sky_radiance(direction); }
    let r = 6360000.0;
    let h = max(view.cam.y, 0.0);
    let dip = sqrt(2.0 * h / r);
    // Just above the bake's horizon row: that row already blends in ground.
    let horizon = sky_at_elevation(direction, 0.02);
    let y = direction.y + dip;
    if y >= 0.0 { return horizon; }
    // h + t dy + t^2 / 2R = 0, nearer root.
    let dy = direction.y;
    let t = r * (-dy - sqrt(max(dy * dy - 2.0 * h / r, 0.0)));
    return mix(sky_at_elevation(direction, y), horizon, 1.0 - exp(-t / 30000.0));
}

fn background_radiance(direction: vec3<f32>) -> vec3<f32> {
    // Surface fog must remain star-free: stars are behind opaque geometry,
    // not bright flecks painted onto fully fogged distant mountains.
    return attenuate_background(direction, background_sky(direction));
}

fn distance_fog_amount(distance: f32) -> f32 {
    let linear = clamp((distance - light.fog_color.w) / max(light.fog.x - light.fog_color.w, 1.0), 0.0, 1.0);
    let exponential = 1.0 - exp(-light.fog.y * distance);
    return max(linear, exponential);
}

fn fog_blend(source: vec3<f32>, background: vec3<f32>, amount: f32) -> vec3<f32> {
    if amount <= 0.0 { return source; }
    if amount >= 1.0 { return background; }
    return mix(source, background, amount);
}

fn apply_distance_fog(radiance: vec3<f32>, distance: f32, direction: vec3<f32>) -> vec3<f32> {
    if light.fog.z > 0.0 {
        if light.fog.y <= 0.0 { return radiance; }
        let column = fog_column(distance, view.cam.y, direction.y, light.fog.z);
        // Night/occluded sun contributes no directional scattering. Avoid
        // marching solar shadows when only diffuse skylight remains. Thin air
        // (a near wall: optical depth under 0.004) blends at most 0.4% of the
        // fog source in, so its shadowing is not worth eight shadow samples.
        if light.sun_dir.w <= 0.0 || view.fog_cull.z <= 0.0 || light.sun_color.w <= 0.0 || light.fog.y * column < 0.004 {
            return fog_segment(radiance, fog_scattering_source(direction), light.fog.y * column);
        }
        // Stratified integration avoids coherent four-slice light curtains.
        // Offset the strata per pixel; TAA/spatial filtering can resolve the
        // fine sampling error instead of preserving large bright bands.
        let clip = view.view_proj * vec4<f32>(view.cam.xyz + direction, 1.0);
        let pixel = floor((clip.xy / clip.w * 0.5 + 0.5) * view.size.xy);
        let temporal = light.fog_sampling.y > 0.5;
        let jitter = select(0.5, fract(52.9829189 * fract(dot(pixel, vec2<f32>(0.06711056, 0.00583715)))
            + light.fog_sampling.x), temporal);
        // Paused/no-TAA captures cannot resolve stochastic error over time.
        let samples = select(32u, 8u, temporal);
        var visible = 0.0;
        var weight_sum = 0.0;
        for (var i = 0u; i < samples; i++) {
            let t = distance * (f32(i) + jitter) / f32(samples);
            let p = view.cam.xyz + direction * t;
            let q = light.shadow_view_proj * vec4<f32>(p,1.0);
            let uv = q.xy * vec2<f32>(0.5,-0.5) + 0.5;
            var visibility = 1.0;
            if all(uv>=vec2<f32>(0.0)) && all(uv<=vec2<f32>(1.0)) && q.z>=0.0 && q.z<=1.0 {
                visibility = textureSampleCompareLevel(shadow_tex,shadow_samp,uv,q.z-0.0002);
            }
            // Scattering near the camera contributes more than distant air:
            // density times transmission along the camera-to-sample segment.
            let weight = exp(-max(p.y, 0.0) / light.fog.z
                - light.fog.y * fog_column(t, view.cam.y, direction.y, light.fog.z));
            visible += visibility * weight;
            weight_sum += weight;
        }
        visible = mix(1.0, visible / max(weight_sum, 0.000001), light.sun_color.w);
        let diffuse = light.fog_scattering.rgb * max(light.sh[0].rgb * 0.282095, vec3<f32>(0.0));
        let source = diffuse + (fog_scattering_source(direction) - diffuse) * visible;
        return fog_segment(radiance, source, light.fog.y * column);
    }
    let amount = distance_fog_amount(distance);
    // Near surfaces do not need an additional sky texture lookup.
    if amount <= 0.0 { return radiance; }
    return fog_blend(radiance, background_radiance(direction), amount);
}
