//! Execute production deformation WGSL against an independent f64 surface oracle.
//! The GPU test is explicit/serial: ordinary unit tests never acquire an adapter.
use super::{Clock, Settings};
use bytemuck::{Pod, Zeroable};
use glam::{DVec3, Vec2};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Case {
    point: [f32; 4],
    shading: [f32; 4],
    geometric: [f32; 4],
    anchor: [f32; 4],
    root: [f32; 4],
    yaw: [f32; 4],
    branch: [f32; 4],
    settings: Settings,
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Output {
    world: [f32; 4],
    previous: [f32; 4],
    normal: [f32; 4],
    geometric: [f32; 4],
}
fn v(a: [f32; 4]) -> DVec3 {
    DVec3::new(a[0] as f64, a[1] as f64, a[2] as f64)
}
fn packed(v: DVec3, w: f32) -> [f32; 4] {
    [v.x as f32, v.y as f32, v.z as f32, w]
}

fn settings(time: f64, previous_time: f64, valid: bool) -> Settings {
    let current = crate::wind::WindState {
        time_seconds: time,
        strength: 0.83,
        direction: Vec2::new(0.8, 0.6),
        grass_shear: 0.48,
        ..Default::default()
    };
    let previous = crate::wind::WindState {
        time_seconds: previous_time,
        strength: 0.31,
        direction: Vec2::new(-0.6, 0.8),
        grass_shear: 0.27,
        ..Default::default()
    };
    let mut base = Settings::zeroed();
    base.shape = [1.0, 1.0, 1.0, 25.0];
    base.range[0] = 35.0;
    base.for_frame(
        Default::default(),
        &Clock {
            current,
            previous,
            valid,
        },
    )
    .unwrap()
}

#[test]
fn windswept_clock_keeps_independent_history_and_continuous_scrolls() {
    let s = settings(4096.25, 119.75, true);
    assert_eq!(s.animation[0], 0.25);
    assert_eq!(s.animation[2], 119.75);
    assert_eq!(s.previous[1..3], [0.8, 0.6]);
    assert_eq!(s.wind_history[..2], [-0.6, 0.8]);
    assert!((s.wind_history[2] - 0.31 * 0.27 / 0.4).abs() < 1e-6);
    let before = settings(119.999, 0.0, false);
    let after = settings(120.001, 0.0, false);
    assert!((after.animation[0] - before.animation[0] - 0.002).abs() < 1e-5);
    assert_eq!(after.animation[..2], after.animation[2..]);
    assert_eq!(after.wind_history[..2], after.previous[1..3]);
}

fn lean(anchor: DVec3) -> f64 {
    0.1 + 0.3 * anchor_random(anchor, 0x68bc21eb)
}
fn anchor_random(anchor: DVec3, salt: u32) -> f64 {
    let x = (anchor.x * 1024.0).round() as i32 as u32;
    let z = (anchor.z * 1024.0).round() as i32 as u32;
    let mut h = x.wrapping_mul(0x9e3779b1) ^ z.wrapping_mul(0x85ebca77) ^ salt;
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846ca68b);
    h ^= h >> 16;
    (h >> 8) as f64 / 16777216.0
}
fn lattice(x: i32, y: i32) -> f64 {
    let m = 0x5bd1e995u32;
    let mut h = 1190494759u32;
    for cell in [x, y] {
        let mut k = ((cell & 4095) as f32).to_bits().wrapping_mul(m);
        k ^= k >> 24;
        k = k.wrapping_mul(m);
        h = h.wrapping_mul(m) ^ k;
    }
    h ^= h >> 13;
    h = h.wrapping_mul(m);
    h ^= h >> 15;
    (h & 0x007fffff) as f64 / 8388608.0
}
fn noise(x: f64, y: f64) -> f64 {
    let ix = x.floor() as i32;
    let iy = y.floor() as i32;
    let smooth = |f: f64| f * f * (3.0 - 2.0 * f);
    let u = smooth(x - x.floor());
    let w = smooth(y - y.floor());
    let a = lattice(ix, iy) * (1.0 - u) + lattice(ix + 1, iy) * u;
    let b = lattice(ix, iy + 1) * (1.0 - u) + lattice(ix + 1, iy + 1) * u;
    2.0 * (a * (1.0 - w) + b * w) - 1.0
}
fn yaw(p: DVec3, c: &Case) -> DVec3 {
    let cs = c.yaw[0] as f64;
    let sn = c.yaw[1] as f64;
    DVec3::new(cs * p.x + sn * p.z, p.y, -sn * p.x + cs * p.z)
}
fn shear(p: DVec3, c: &Case) -> DVec3 {
    p + DVec3::Y * (p.x * c.yaw[2] as f64 + p.z * c.yaw[3] as f64)
}
fn rotate(p: DVec3, axis: DVec3, angle: f64) -> DVec3 {
    p * angle.cos() + axis.cross(p) * angle.sin() + axis * axis.dot(p) * (1.0 - angle.cos())
}
// Invert the rest centreline with bisection, deliberately not the shader's
// three-step Newton solve or its analytic cofactor implementation.
fn parameter(y: f64, height: f64, bend: f64) -> f64 {
    let target = y / height;
    let mut lo = 0.0;
    let mut hi = 1.0;
    for _ in 0..60 {
        let t = (lo + hi) * 0.5;
        if t * (bend * t * t).cos() < target {
            lo = t;
        } else {
            hi = t;
        }
    }
    (lo + hi) * 0.5
}
fn oracle(c: &Case, point: DVec3, prior: bool) -> DVec3 {
    let prior = prior && c.settings.previous[3] >= 0.5;
    let root = v(c.root);
    let pivot = yaw(v(c.anchor), c);
    let q = yaw((point - v(c.anchor)) * c.root[3] as f64, c);
    if c.anchor[3] == 0.0 {
        let dx = c.settings.previous[1] as f64;
        let dz = c.settings.previous[2] as f64;
        let axis = DVec3::new(-dz, 0.0, dx).normalize();
        let k = if prior { 2 } else { 0 };
        return root
            + shear(
                pivot + rotate(q, axis, (c.branch[k] as f64).atan2(c.branch[k + 1] as f64)),
                c,
            );
    }
    let t = parameter(
        point.y - c.anchor[1] as f64,
        c.anchor[3] as f64,
        lean(v(c.anchor)),
    );
    let s = &c.settings;
    let (scroll, dir, strength) = if prior {
        (
            [s.animation[2], s.animation[3]],
            [s.wind_history[0], s.wind_history[1]],
            s.wind_history[2],
        )
    } else {
        (
            [s.animation[0], s.animation[1]],
            [s.previous[1], s.previous[2]],
            s.range[1] * s.range[2],
        )
    };
    let a = root + pivot;
    let direction_offset = noise(a.x * 0.05 + scroll[1] as f64, a.z * 0.05 + scroll[1] as f64);
    let dx = direction_offset.cos() * dir[0] as f64 - direction_offset.sin() * dir[1] as f64;
    let dz = direction_offset.sin() * dir[0] as f64 + direction_offset.cos() * dir[1] as f64;
    let gust = 0.625 + 0.375 * noise(a.x * 0.25 + scroll[0] as f64, a.z * 0.25 + scroll[0] as f64);
    root + shear(
        pivot
            + rotate(
                q,
                DVec3::new(-dz, 0.0, dx).normalize(),
                1.25 * gust * gust * strength as f64 * t,
            ),
        c,
    )
}
fn surface(c: &Case, n: DVec3) -> DVec3 {
    let e1 = n
        .cross(if n.x.abs() < 0.8 { DVec3::X } else { DVec3::Y })
        .normalize();
    let e2 = n.cross(e1).normalize();
    let p = v(c.point);
    let h = 1e-5;
    let a = (oracle(c, p + e1 * h, false) - oracle(c, p - e1 * h, false)) / (2.0 * h);
    let b = (oracle(c, p + e2 * h, false) - oracle(c, p - e2 * h, false)) / (2.0 * h);
    a.cross(b)
}
fn case(t: f64, time: f64, history: bool) -> Case {
    let anchor = DVec3::new(0.125, 0.0, -0.23828125);
    let height = 0.67;
    let bend = lean(anchor);
    let a = bend * t * t;
    let point = anchor + DVec3::new(0.0, height * t * a.cos(), height * t * a.sin());
    let tangent = DVec3::new(
        0.0,
        a.cos() - 2.0 * a * a.sin(),
        a.sin() + 2.0 * a * a.cos(),
    );
    let normal = DVec3::X.cross(tangent).normalize();
    let (sn, cs) = 0.73f32.sin_cos();
    Case {
        point: packed(point, 0.0),
        shading: packed((normal + DVec3::X * 0.7).normalize(), 0.0),
        geometric: packed(normal, 0.0),
        anchor: packed(anchor, height as f32),
        root: [1.75, -0.4, -3.25, 1.4],
        yaw: [cs, sn, 0.17, -0.23],
        branch: [0.2f32.sin(), 0.2f32.cos(), -0.13f32.sin(), 0.13f32.cos()],
        settings: settings(time, time - 0.025, history),
    }
}
fn shader() -> String {
    [include_str!("shaders/meadow_field.wgsl"), include_str!("shaders/meadow_common.wgsl"), r#"
struct Case {
    point:vec4<f32>, shading:vec4<f32>, geometric:vec4<f32>, anchor:vec4<f32>,
    root:vec4<f32>, yaw:vec4<f32>, branch:vec4<f32>, settings:MeadowSettings,
};
struct Output { world:vec4<f32>, previous:vec4<f32>, normal:vec4<f32>, geometric:vec4<f32> };
@group(0) @binding(0) var<storage,read> cases:array<Case>;
@group(0) @binding(1) var<storage,read_write> results:array<Output>;
@compute @workgroup_size(64) fn verify(@builtin(global_invocation_id) id:vec3<u32>) {
    if id.x>=arrayLength(&cases) {return;}
    let c=cases[id.x];
    var r:MeadowDeformed;
    if c.geometric.w>0.5 {
        r=meadow_deform_vertex(c.point.xyz,c.shading,c.geometric.xyz,c.anchor,c.root,c.yaw,c.branch,c.settings);
    } else {
        r=meadow_deform_blade(c.point.xyz,c.shading.xyz,c.geometric.xyz,c.anchor,c.root,c.yaw,c.branch,c.settings);
    }
    results[id.x]=Output(vec4<f32>(r.world,r.area_scale),vec4<f32>(r.previous_world,0.0),
        vec4<f32>(r.normal,0.0),vec4<f32>(r.geometric_normal,0.0));
}
"#].join("\n")
}

#[test]
fn windswept_oracle_compute_shader_validates() {
    let source = shader();
    let module = naga::front::wgsl::parse_str(&source)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
}

#[test]
#[ignore = "serial GPU fixture; requires an idle adapter"]
fn windswept_gpu_matches_surface_oracle_and_history() {
    let mut cases = Vec::new();
    for time in [
        0.0, 1.37, 119.999, 120.001, 4095.999, 4096.001, 81919.999, 81920.001,
    ] {
        for t in [0.0, 0.17, 0.333, 0.5, 0.78, 0.97, 1.0] {
            cases.push(case(t, time, true));
        }
    }
    let history_invalid = cases.len();
    cases.push(case(0.5, 12.3, false));
    let zero_area = cases.len();
    let mut c = case(0.5, 12.3, true);
    c.geometric = [0.0; 4];
    cases.push(c);
    let legacy = cases.len();
    let mut c = case(0.5, 12.3, true);
    c.anchor[3] = 0.0;
    cases.push(c);
    let periodic = cases.len();
    let mut c = case(0.5, 0.25, true);
    c.settings.animation = [0.25, 0.5, 0.25, 0.5];
    cases.push(c);
    c.settings.animation = [4096.25, 4096.5, 4096.25, 4096.5];
    cases.push(c);
    let result = run_cases(&cases);
    for (i, (c, r)) in cases.iter().zip(&result).enumerate() {
        let error = v(r.world).distance(oracle(c, v(c.point), false));
        let previous_error = v(r.previous).distance(oracle(c, v(c.point), true));
        // Near a 4096-cell scroll wrap f32 addition loses at most ~0.0005
        // lattice units. This tolerance covers that quantified input rounding.
        assert!(
            error < 8e-4 && previous_error < 8e-4,
            "case {i}: current={error} previous={previous_error}"
        );
        assert!(
            v(r.normal).is_finite() && (v(r.normal).length() - 1.0).abs() < 2e-5,
            "normal {i}"
        );
        assert!(r.world[3].is_finite() && r.world[3] >= 0.0, "area {i}");
        let t = parameter(
            c.point[1] as f64 - c.anchor[1] as f64,
            c.anchor[3] as f64,
            lean(v(c.anchor)),
        );
        if (t > 0.01 && t < 0.99 || i == legacy) && i != zero_area {
            let area = surface(c, v(c.geometric).normalize());
            let shading = surface(c, v(c.shading).normalize()).normalize();
            assert!(
                v(r.geometric).distance(area.normalize()) < 3e-3,
                "geometric Jacobian {i}"
            );
            assert!(v(r.normal).distance(shading) < 3e-3, "shading cofactor {i}");
            assert!(
                (r.world[3] as f64 - area.length()).abs() < 4e-3,
                "surface area {i}: GPU={} oracle={}",
                r.world[3],
                area.length()
            );
        }
        if c.point[..3] == c.anchor[..3] {
            assert!(
                v(r.world).distance(v(c.root) + shear(yaw(v(c.anchor), c), c)) < 1e-6,
                "root moved {i}"
            );
        }
    }
    assert_eq!(
        result[history_invalid].world[..3],
        result[history_invalid].previous[..3]
    );
    assert_eq!(result[zero_area].world[3], 0.0);
    assert_eq!(result[zero_area].geometric, [0.0; 4]);
    assert!(v(result[periodic].world).distance(v(result[periodic + 1].world)) < 8e-4);
    // The two boundary samples are 2 ms apart, including the legacy 120 s
    // phase seam and both independently wrapped noise-scroll seams.
    for (before, after) in [(2, 3), (4, 5), (6, 7)] {
        assert!(v(result[before * 7 + 6].world).distance(v(result[after * 7 + 6].world)) < 0.01);
    }
    // A rigid blade has equal angular deflection at the middle and tip.
    // Divide displacement by rest radius; the nonlinear profile must differ.
    let displacement = |i: usize| {
        let mut still = cases[i];
        still.settings.range[1] = 0.0;
        v(result[i].world).distance(oracle(&still, v(still.point), false))
            / (v(still.point) - v(still.anchor)).length()
    };
    assert!(
        displacement(6) > displacement(3) * 1.6,
        "wind became rigid rotation"
    );
    assert!(
        v(result[3].world).distance(v(result[3].previous)) > 0.005,
        "history ignored independent direction/strength"
    );
    eprintln!("windswept production WGSL: {} cases verified against f64 position and finite-difference surface oracle", cases.len());
}

#[test]
#[ignore = "serial GPU fixture; requires an idle adapter"]
fn windswept_gpu_authored_coordinate_matches_reconstructed_surface() {
    use crate::{foliage_assembly::MeadowBladeBudget, meadow_data::MeadowGeometry};
    let source = MeadowGeometry::cook_windswept(73, 0.0, MeadowBladeBudget::Blades120).unwrap();
    let mut cases = Vec::new();
    let mut stations = std::collections::BTreeSet::new();
    for variant in 0..3 {
        for lod in 0..3 {
            let [first, count] = source.lod_ranges[variant][lod];
            // One entire blade from each cut plus varied anchors throughout
            // that cut exercises all authored stations, yaw and lean ranges.
            let indices = (first..first + count.min(33))
                .chain((first..first + count).step_by((count as usize / 31).max(1)));
            for index in indices {
                let vertex = source.vertices[index as usize];
                stations.insert(vertex.normal[3].to_bits());
                for time in [1.37, 120.001, 4096.001] {
                    let mut c = case(0.5, time, true);
                    c.point = vertex.position;
                    c.anchor = vertex.anchor;
                    c.shading = vertex.normal;
                    c.geometric = packed(v(vertex.normal).normalize(), 0.0);
                    cases.push(c);
                    c.geometric[3] = 1.0; // exercise the exact source helper
                    cases.push(c);
                }
            }
        }
    }
    // Legacy vertices ignore normal.w; the source helper must keep precisely
    // the same rigid pose/normal/area dispatch as the arbitrary sample helper.
    let mut legacy = case(0.5, 12.3, true);
    legacy.anchor[3] = 0.0;
    legacy.shading[3] = 0.91;
    cases.push(legacy);
    legacy.geometric[3] = 1.0;
    cases.push(legacy);
    assert_eq!(
        stations.len(),
        7,
        "six-segment source stations including endpoints"
    );
    let result = run_cases(&cases);
    for (pair, inputs) in result.chunks_exact(2).zip(cases.chunks_exact(2)) {
        let recovered = pair[0];
        let authored = pair[1];
        let c = &inputs[1];
        let t = c.shading[3];
        assert!(
            v(recovered.world).distance(v(authored.world)) < 2e-6,
            "source t={t}: current pose"
        );
        assert!(
            v(recovered.previous).distance(v(authored.previous)) < 2e-6,
            "source t={t}: previous pose"
        );
        assert!(
            v(recovered.normal).distance(v(authored.normal)) < 3e-6,
            "source t={t}: cofactor normal"
        );
        assert!(
            v(recovered.geometric).distance(v(authored.geometric)) < 3e-6,
            "source t={t}: geometric normal"
        );
        assert!(
            (recovered.world[3] - authored.world[3]).abs() < 6e-6,
            "source t={t}: area"
        );
        assert!(
            v(authored.world).distance(oracle(c, v(c.point), false)) < 8e-4,
            "source t={t}: independent position oracle"
        );
        if t > 0.01 && t < 0.99 && c.anchor[3] > 0.0 {
            let area = surface(c, v(c.geometric).normalize());
            let normal = surface(c, v(c.shading).normalize()).normalize();
            assert!(
                v(authored.normal).distance(normal) < 3e-3,
                "source t={t}: independent normal oracle"
            );
            assert!(
                (authored.world[3] as f64 - area.length()).abs() < 4e-3,
                "source t={t}: independent area oracle"
            );
        }
    }
    let last = result.len();
    assert_eq!(
        bytemuck::bytes_of(&result[last - 2]),
        bytemuck::bytes_of(&result[last - 1]),
        "legacy must remain bit-identical"
    );
    eprintln!(
        "windswept authored-t GPU path: {} paired cooked-source/legacy cases",
        result.len() / 2
    );
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MaterialCase {
    root_color: [f32; 4],
    anchor: [f32; 4],
    stations: [f32; 4], // lower/upper authored t, barycentric weight, unused
    edits: [f32; 4],
    flags: [u32; 4], // packed pigment, legacy marker, unused
    fallback: [f32; 4],
}
fn material_shader() -> String {
    // Use the actual pure material helpers from the production raster shader,
    // without pulling unrelated lighting/texture resources into this compute.
    let render = include_str!("shaders/meadow_render.wgsl");
    let start = render.find("fn meadow_windswept_tip(").unwrap();
    let end = render[start..].find("fn meadow_read4(").unwrap() + start;
    [include_str!("shaders/meadow_field.wgsl"), include_str!("shaders/meadow_common.wgsl"), &render[start..end], r#"
struct MaterialCase {
    root_color:vec4<f32>,anchor:vec4<f32>,stations:vec4<f32>,edits:vec4<f32>,flags:vec4<u32>,fallback:vec4<f32>,
};
@group(0) @binding(0) var<storage,read> cases:array<MaterialCase>;
@group(0) @binding(1) var<storage,read_write> results:array<vec4<f32>>;
@compute @workgroup_size(64) fn verify(@builtin(global_invocation_id) id:vec3<u32>) {
    if id.x>=arrayLength(&cases) {return;}
    let c=cases[id.x];let base=c.root_color.rgb*4.0;
    let tip=meadow_windswept_tip(base,c.anchor.xyz);
    var color=vec4<f32>(base,0.9);
    // Exactly the varying carried by source vertices and reconstructed by
    // visibility barycentrics, including the zero marker for legacy/atlas RGB.
    var blade=mix(vec4<f32>(tip,c.stations.x+1.0),vec4<f32>(tip,c.stations.y+1.0),c.stations.z);
    if c.flags.y!=0u {color=c.fallback;blade=vec4<f32>(0.0);}
    let albedo=meadow_fragment_albedo(color,blade,c.flags.x,c.edits.rgb);
    results[id.x]=vec4<f32>(albedo,blade.w-1.0);
}
"#].join("\n")
}

#[test]
fn windswept_material_compute_shader_validates() {
    let source = material_shader();
    let module = naga::front::wgsl::parse_str(&source)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
}

fn material_oracle(anchor: DVec3, dry: f64, t: f64, pigment: u32, edits: DVec3) -> DVec3 {
    let base = DVec3::new(0.02, 0.075, 0.01).lerp(
        DVec3::new(0.025, 0.1, 0.01),
        anchor_random(anchor, 0x9e3779b9),
    );
    let tip = DVec3::new(0.65, 0.8, 0.25)
        .lerp(DVec3::new(0.8, 0.9, 0.4), anchor_random(anchor, 0x7f4a7c15));
    let season = DVec3::ONE.lerp(DVec3::new(0.24 / 0.105, 0.19 / 0.18, 0.075 / 0.039), dry);
    let shade = 0.75 + 0.25 * anchor_random(anchor, 0x94d049bb);
    let tint = DVec3::new(
        (pigment & 1023) as f64,
        ((pigment >> 10) & 1023) as f64,
        ((pigment >> 20) & 1023) as f64,
    ) * (4.0 / 1023.0);
    let material = (base.lerp(tip, t.powi(4)) * season * shade * (0.25 + 0.75 * t * t))
        .clamp(DVec3::ZERO, DVec3::ONE);
    (material * tint * edits).clamp(DVec3::ZERO, DVec3::ONE)
}

#[test]
#[ignore = "serial GPU fixture; requires an idle adapter"]
fn windswept_gpu_material_is_lod_independent_and_season_correct() {
    use crate::{foliage_assembly::MeadowBladeBudget, meadow_data::MeadowGeometry};
    let mut cases = Vec::new();
    let mut expected = Vec::new();
    let pigment = 179u32 | (281 << 10) | (243 << 20);
    let edits = [0.8, 1.2, 0.65, 0.0];
    for dry in [0.0, 0.4, 1.0] {
        let source = MeadowGeometry::cook_windswept(73, dry, MeadowBladeBudget::Blades120).unwrap();
        for variant in 0..3 {
            for blade in [0u32, 7, 19, 39] {
                for t in [0.0f32, 0.07, 0.3, 0.5, 0.8, 0.93, 1.0] {
                    for (lod, stride) in [33u32, 15, 3].into_iter().enumerate() {
                        let first = source.lod_ranges[variant][lod][0] + blade * stride;
                        let vertices = &source.vertices[first as usize..(first + stride) as usize];
                        let mut lower = 0.0f32;
                        let mut upper = 1.0f32;
                        for vertex in vertices {
                            let station = vertex.normal[3];
                            if station <= t {
                                lower = lower.max(station);
                            }
                            if station >= t {
                                upper = upper.min(station);
                            }
                        }
                        let weight = if upper > lower {
                            (t - lower) / (upper - lower)
                        } else {
                            0.0
                        };
                        cases.push(MaterialCase {
                            root_color: vertices[0].color,
                            anchor: vertices[0].anchor,
                            stations: [lower, upper, weight, 0.0],
                            edits,
                            flags: [pigment, 0, 0, 0],
                            fallback: [0.0; 4],
                        });
                        expected.push((dry as f64, t as f64));
                    }
                }
            }
        }
    }
    let material_count = cases.len();
    for fallback in [[-0.1, 0.15, 1.7, 0.9], [0.27, 0.43, 0.09, 0.2]] {
        cases.push(MaterialCase {
            root_color: [0.0; 4],
            anchor: [0.0; 4],
            stations: [0.0; 4],
            edits,
            flags: [pigment, 1, 0, 0],
            fallback,
        });
    }
    let result: Vec<[f32; 4]> = run_compute(&cases, material_shader());
    for (i, ((c, out), &(dry, t))) in cases.iter().zip(&result).zip(&expected).enumerate() {
        assert!(
            (out[3] as f64 - t).abs() < 3e-7,
            "authored height reconstruction {i}"
        );
        let oracle = material_oracle(v(c.anchor), dry, out[3] as f64, pigment, v(edits));
        assert!(
            v(*out).distance(oracle) < 3e-6,
            "material {i}, season={dry}, t={t}: GPU={:?}, oracle={oracle:?}",
            out
        );
    }
    for (i, cuts) in result[..material_count].chunks_exact(3).enumerate() {
        assert!(
            v(cuts[0]).distance(v(cuts[1])) < 2e-6 && v(cuts[0]).distance(v(cuts[2])) < 2e-6,
            "LOD-dependent material at matched blade height, sample {i}: {cuts:?}"
        );
    }
    for i in material_count..cases.len() {
        assert_eq!(
            result[i][..3],
            cases[i].fallback[..3]
                .iter()
                .map(|x| x.clamp(0.0, 1.0))
                .collect::<Vec<_>>(),
            "legacy and atlas RGB must bypass analytical colour and extra multipliers"
        );
        assert_eq!(result[i][3], -1.0);
    }
    eprintln!("windswept fragment material: {} matched 6/3/1-cut samples, three seasons and two legacy/atlas cases", material_count);
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MorphCase {
    vertices: [crate::meadow_data::GpuMeadowVertex; 4], // fine, coarse left/right/tip
    root: [f32; 4],
    yaw: [f32; 4],
    branch: [f32; 4],
    camera: [f32; 4],
    settings: Settings,
    index: [u32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MorphOutput {
    world: [f32; 4],    // W: current factor
    previous: [f32; 4], // W: previous factor
    normal: [f32; 4],
    fine: [f32; 4],
    fine_normal: [f32; 4],
    endpoints: [[f32; 4]; 3],
    endpoint_normals: [[f32; 4]; 3],
    weights: [f32; 4],
}
fn morph_shader() -> String {
    [include_str!("shaders/meadow_field.wgsl"), include_str!("shaders/meadow_common.wgsl"),
        include_str!("shaders/meadow_transition.wgsl"), r#"
struct SourceVertex {point:vec4<f32>,normal:vec4<f32>,color:vec4<f32>,anchor:vec4<f32>};
struct MorphCase {
    vertices:array<SourceVertex,4>,root:vec4<f32>,yaw:vec4<f32>,branch:vec4<f32>,camera:vec4<f32>,
    settings:MeadowSettings,index:vec4<u32>,
};
struct MorphOutput {
    world:vec4<f32>,previous:vec4<f32>,normal:vec4<f32>,fine:vec4<f32>,fine_normal:vec4<f32>,
    endpoints:array<vec4<f32>,3>,endpoint_normals:array<vec4<f32>,3>,weights:vec4<f32>,
};
@group(0) @binding(0) var<storage,read> cases:array<MorphCase>;
@group(0) @binding(1) var<storage,read_write> results:array<MorphOutput>;
@compute @workgroup_size(64) fn verify(@builtin(global_invocation_id) id:vec3<u32>) {
    if id.x>=arrayLength(&cases) {return;}
    let c=cases[id.x];var poses:array<MeadowDeformed,4>;
    for(var i=0u;i<4u;i++) {
        let vertex=c.vertices[i];
        poses[i]=meadow_deform_vertex(vertex.point.xyz,vertex.normal,vertex.normal.xyz,vertex.anchor,
            c.root,c.yaw,c.branch,c.settings);
    }
    let weights=meadow_coarse_weights(c.index.x,33u,c.vertices[0].normal.w);
    let factors=vec2<f32>(meadow_lod_factor(c.root.xyz,c.root.w,c.camera.xyz),
        meadow_lod_factor(c.root.xyz,c.root.w,c.settings.previous_camera.xyz));
    let pose=meadow_morph_pose(poses[0],poses[1],poses[2],poses[3],weights,factors);
    var out:MorphOutput;
    out.world=vec4<f32>(pose.world,factors.x);out.previous=vec4<f32>(pose.previous_world,factors.y);
    out.normal=vec4<f32>(pose.normal,0.0);out.fine=vec4<f32>(poses[0].world,0.0);out.fine_normal=vec4<f32>(poses[0].normal,0.0);
    for(var i=0u;i<3u;i++) {
        out.endpoints[i]=vec4<f32>(poses[i+1u].world,0.0);
        out.endpoint_normals[i]=vec4<f32>(poses[i+1u].normal,0.0);
    }
    out.weights=vec4<f32>(weights,0.0);results[id.x]=out;
}
"#].join("\n")
}

#[test]
fn windswept_morph_compute_shader_validates() {
    let source = morph_shader();
    let module = naga::front::wgsl::parse_str(&source)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
}

fn morph_source_case(c: &MorphCase, vertex: usize) -> Case {
    let source = c.vertices[vertex];
    Case {
        point: source.position,
        shading: source.normal,
        geometric: source.normal,
        anchor: source.anchor,
        root: c.root,
        yaw: c.yaw,
        branch: c.branch,
        settings: c.settings,
    }
}
fn morph_weights(c: &MorphCase) -> DVec3 {
    // Infer the ribbon side from actual cooked positions, independently of
    // the production triangle-stream modulo/corner lookup.
    let across = (v(c.vertices[2].position) - v(c.vertices[1].position)).normalize();
    let p = v(c.vertices[0].position) - v(c.vertices[0].anchor);
    let right = if p.dot(across) > 0.0 { 1.0 } else { 0.0 };
    let t = c.vertices[0].normal[3] as f64;
    DVec3::new((1.0 - t) * (1.0 - right), (1.0 - t) * right, t)
}
fn morph_factor(distance: f64) -> f64 {
    let u = ((distance - 3.0) / 3.0).clamp(0.0, 1.0);
    u * u * (3.0 - 2.0 * u)
}

#[test]
#[ignore = "serial GPU fixture; requires an idle adapter"]
fn windswept_gpu_morph_meets_coarse_surface_and_tracks_previous_camera() {
    use crate::{foliage_assembly::MeadowBladeBudget, meadow_data::MeadowGeometry};
    let source = MeadowGeometry::cook_windswept(73, 0.0, MeadowBladeBudget::Blades120).unwrap();
    let distances = [
        (2.99, 2.99),
        (3.0, 3.0),
        (3.01, 3.0),
        (4.5, 4.2),
        (5.2, 4.0),
        (5.99, 5.98),
        (6.0, 5.99),
        (6.01, 6.0),
        (7.2, 6.01),
    ];
    let mut cases = Vec::new();
    for (variant, blade) in [(0, 0u32), (1, 11), (2, 31)] {
        let fine_first = source.lod_ranges[variant][0][0] + blade * 33;
        let coarse_first = source.lod_ranges[variant][2][0] + blade * 3;
        let coarse: [_; 3] = std::array::from_fn(|i| source.vertices[coarse_first as usize + i]);
        for (i, endpoint) in [0, 1, 32].into_iter().zip(coarse) {
            let fine = source.vertices[fine_first as usize + i];
            assert_eq!(fine.position, endpoint.position);
            assert_eq!(fine.normal, endpoint.normal);
        }
        for &(current_distance, previous_distance) in &distances {
            for local in 0..33 {
                let template = case(0.5, 2.0, true);
                let root = template.root;
                let camera =
                    |distance: f64| packed(v(root) + DVec3::Z * (distance * root[3] as f64), 1.0);
                let mut settings = settings(2.0, 2.0, true);
                // Wind is frozen; all nonzero motion here comes from the
                // camera-dependent representation, not changed gusts/strength.
                settings.animation[2] = settings.animation[0];
                settings.animation[3] = settings.animation[1];
                settings.wind_history[..3].copy_from_slice(&[
                    settings.previous[1],
                    settings.previous[2],
                    settings.range[1] * settings.range[2],
                ]);
                settings.previous_camera = camera(previous_distance);
                cases.push(MorphCase {
                    vertices: [
                        source.vertices[(fine_first + local) as usize],
                        coarse[0],
                        coarse[1],
                        coarse[2],
                    ],
                    root,
                    yaw: template.yaw,
                    branch: template.branch,
                    camera: camera(current_distance),
                    settings,
                    index: [local, 0, 0, 0],
                });
            }
        }
    }
    let result: Vec<MorphOutput> = run_compute(&cases, morph_shader());
    let mut nonunit_affine_normal = false;
    let mut camera_motion_observed = false;
    for (i, (c, out)) in cases.iter().zip(&result).enumerate() {
        let weights = morph_weights(c);
        assert!(
            v(out.weights).distance(weights) < 2e-7,
            "coarse corner mapping {i}"
        );
        let distance = v(c.camera).distance(v(c.root)) / c.root[3] as f64;
        let prior_distance = v(c.settings.previous_camera).distance(v(c.root)) / c.root[3] as f64;
        let factor = morph_factor(distance);
        let prior_factor = morph_factor(prior_distance);
        assert!(
            (out.world[3] as f64 - factor).abs() < 3e-7
                && (out.previous[3] as f64 - prior_factor).abs() < 3e-7,
            "camera factors {i}"
        );
        let fine = morph_source_case(c, 0);
        let coarse = (1..4)
            .zip(weights.to_array())
            .map(|(endpoint, w)| {
                let input = morph_source_case(c, endpoint);
                oracle(&input, v(input.point), false) * w
            })
            .sum::<DVec3>();
        let expected = oracle(&fine, v(fine.point), false).lerp(coarse, factor);
        let previous_expected = oracle(&fine, v(fine.point), true).lerp(coarse, prior_factor);
        assert!(
            v(out.world).distance(expected) < 3e-6,
            "morphed current surface {i}"
        );
        assert!(
            v(out.previous).distance(previous_expected) < 3e-6,
            "morphed previous camera surface {i}"
        );
        let affine = (0..3)
            .zip(weights.to_array())
            .map(|(j, w)| v(out.endpoint_normals[j]) * w)
            .sum::<DVec3>();
        let expected_normal = v(out.fine_normal).lerp(affine, factor);
        assert!(
            v(out.normal).distance(expected_normal) < 4e-7,
            "affine normal {i}"
        );
        if factor == 0.0 {
            assert_eq!(out.world[..3], out.fine[..3], "near field must not morph");
            assert_eq!(out.normal, out.fine_normal);
        }
        if [0, 1, 32].contains(&(c.index[0] as usize)) {
            assert!(
                v(out.world).distance(v(out.fine)) < 4e-7,
                "root/tip moved during morph {i}"
            );
        }
        if factor > 0.9999 && v(out.normal).length() < 0.97 {
            nonunit_affine_normal = true;
        }
        if (factor - prior_factor).abs() > 0.1 && v(out.world).distance(v(out.previous)) > 0.005 {
            camera_motion_observed = true;
        }
    }
    assert!(
        nonunit_affine_normal,
        "test must reject normalization before fragment interpolation"
    );
    assert!(
        camera_motion_observed,
        "frozen wind must still report camera-driven morph motion"
    );
    for (blade_index, blade_results) in result.chunks_exact(distances.len() * 33).enumerate() {
        // At/after the cut all eleven fine triangles tile the exact animated
        // coarse triangle, with matching affine normal field and total area.
        for distance_index in [6, 7, 8] {
            let vertices = &blade_results[distance_index * 33..(distance_index + 1) * 33];
            let corners = vertices[0].endpoints.map(v);
            let cross = (corners[1] - corners[0]).cross(corners[2] - corners[0]);
            let normal = cross.normalize();
            let area = cross.length() * 0.5;
            let mut fine_area = 0.0;
            for triangle in vertices.chunks_exact(3) {
                for vertex in triangle {
                    assert!(
                        (v(vertex.world) - corners[0]).dot(normal).abs() < 4e-7,
                        "cut plane {blade_index}"
                    );
                }
                let a = v(triangle[0].world);
                let b = v(triangle[1].world);
                let c = v(triangle[2].world);
                let face = (b - a).cross(c - a);
                assert!(
                    face.dot(normal) > 0.0,
                    "fine triangle inverted at coarse cut"
                );
                fine_area += face.length() * 0.5;
            }
            assert!(
                (fine_area - area).abs() < area * 1e-5 + 1e-7,
                "coarse/fine coverage area mismatch"
            );
        }
        // Topology history cannot reconstruct an old fine primitive after an
        // owner switches to the coarse mesh. At 6±.01m its geometric residual
        // is bounded here, rather than claimed pixel-identical across a cut.
        for local in 0..33 {
            let before = &blade_results[5 * 33 + local];
            let after = &blade_results[7 * 33 + local];
            assert!(
                v(before.world).distance(v(after.world)) < 3e-5,
                "cut-position residual"
            );
        }
    }
    eprintln!("windswept morph: {} cooked-source vertices; 11-triangle coarse-plane/area agreement, affine normals and independent camera history",cases.len());
}

fn run_cases(cases: &[Case]) -> Vec<Output> {
    run_compute(cases, shader())
}
fn run_compute<T: Pod, U: Pod>(cases: &[T], shader: String) -> Vec<U> {
    let gpu = crate::Gpu::new().expect("headless GPU");
    let input = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("wind oracle inputs"),
            contents: bytemuck::cast_slice(cases),
            usage: wgpu::BufferUsages::STORAGE,
        });
    let bytes = (cases.len() * std::mem::size_of::<U>()) as u64;
    let output = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("wind oracle output"),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("wind oracle readback"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let module = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("actual meadow deformation oracle"),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
    let pipeline = gpu
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("wind oracle"),
            layout: None,
            module: &module,
            entry_point: Some("verify"),
            compilation_options: Default::default(),
            cache: None,
        });
    let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("wind oracle"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: input.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: output.as_entire_binding(),
            },
        ],
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups((cases.len() as u32).div_ceil(64), 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &read, 0, bytes);
    gpu.queue.submit([encoder.finish()]);
    read.slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.expect("map wind oracle"));
    gpu.wait();
    let data = read
        .slice(..)
        .get_mapped_range()
        .expect("wind oracle readback");
    bytemuck::cast_slice(&data).to_vec()
}
