//! CPU optical-selection oracle and validation of the actual shader variants.
//! These do not substitute for rendered OFF/ON depth, alpha and motion checks.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Texel { mip: u32, x: i32, y: i32 }

#[derive(Clone, Copy, Debug)]
struct Tap { texel: Texel, weight: f32 }

fn taps(uv: [f32; 2], mip_value: f32, coverage: impl Fn(Texel) -> f32) -> Vec<Tap> {
    let first_mip = mip_value.floor() as u32;
    let fraction = mip_value.fract();
    let mut taps = Vec::new();
    for level in 0..2 {
        let mip = (first_mip + level).min(5);
        let weight = if level == 0 { 1.0 - fraction } else { fraction };
        if weight <= 0.0 { continue; }
        let size = 32 >> mip;
        let q = uv.map(|v| v * size as f32 - 0.5);
        let first = q.map(|v| v.floor() as i32);
        // WGSL fract is x-floor(x), including negative edge coordinates.
        let f = q.map(|v| v - v.floor());
        for y in 0..2 {
            for x in 0..2 {
                let texel = Texel { mip, x: first[0] + x, y: first[1] + y };
                if texel.x < 0 || texel.y < 0 || texel.x >= size || texel.y >= size { continue; }
                let w = coverage(texel) * weight
                    * if x == 0 { 1.0 - f[0] } else { f[0] }
                    * if y == 0 { 1.0 - f[1] } else { f[1] };
                taps.push(Tap { texel, weight: w });
            }
        }
    }
    taps
}

fn payload(t: Texel) -> [[u32; 4]; 2] {
    // Different point/anchor and branch values make a changed discrete winner
    // observable even when neighboring coverage weights happen to be equal.
    [[t.x as u32, t.y as u32, t.mip, (t.x + t.y) as u32 % 4],
     [t.mip + 41, t.x as u32 + 83, t.y as u32 + 127, 0]]
}
const INCOMING: [[u32; 4]; 2] = [[101, 102, 103, 104], [201, 202, 203, 204]];

fn eager(taps: &[Tap]) -> ([[u32; 4]; 2], usize) {
    let mut result = INCOMING;
    let mut strongest = 0.0;
    let mut reads = 0;
    for tap in taps {
        if tap.weight > strongest {
            strongest = tap.weight;
            result = payload(tap.texel);
            reads += 2;
        }
    }
    (result, reads)
}

fn deferred(taps: &[Tap]) -> ([[u32; 4]; 2], usize) {
    let mut strongest = 0.0;
    let mut selected = None;
    for tap in taps {
        if tap.weight > strongest {
            strongest = tap.weight;
            selected = Some(tap.texel);
        }
    }
    selected.map_or((INCOMING, 0), |texel| (payload(texel), 2))
}

#[test]
fn representative_preserves_borders_mips_zero_coverage_and_ties() {
    let coords = [0.0, 0.5 / 32.0, 1.0 / 32.0, 0.25, 0.499, 0.5, 0.999, 1.0];
    let mips = [0.0, f32::EPSILON, 0.5, 0.999, 1.0, 1.5, 2.125, 4.999, 5.0];
    for u in coords { for v in coords { for mip in mips { for pattern in 0..5 {
        let samples = taps([u, v], mip, |t| match pattern {
            0 => 0.0,
            1 => 1.0,
            2 => 0.5,
            3 => if (t.x + t.y) & 1 == 0 { 1.0 } else { 0.0 },
            _ => ((t.x * 17 + t.y * 31 + t.mip as i32 * 13) % 65) as f32 / 64.0,
        });
        let old = eager(&samples);
        let new = deferred(&samples);
        assert_eq!(new.0, old.0, "uv={u},{v} mip={mip} pattern={pattern}");
        assert!(new.1 <= old.1 && new.1 <= 2);
        if pattern == 0 { assert_eq!(new, (INCOMING, 0)); }
    }}}}
    // At a symmetric pixel and integer mip, four equal positive taps keep the
    // first (top-left), not the last. UV=0 clips three of those four taps.
    let tied = taps([0.5, 0.5], 0.0, |_| 1.0);
    assert_eq!(tied.len(), 4);
    assert_eq!(deferred(&tied).0, payload(Texel { mip: 0, x: 15, y: 15 }));
    let corner = taps([0.0, 0.0], 0.0, |_| 1.0);
    assert_eq!(corner.len(), 1);
    assert_eq!(corner[0].weight, 0.25);
    assert_eq!(taps([1.0, 1.0], 5.0, |_| 1.0).len(), 1);
}

#[test]
fn representative_fetches_only_final_payload_after_record_breakers() {
    let samples: Vec<_> = (0..8).map(|i| Tap {
        texel: Texel { mip: i / 4, x: (i % 2) as i32, y: ((i / 2) % 2) as i32 },
        weight: (i + 1) as f32 / 32.0,
    }).collect();
    assert_eq!(eager(&samples).1, 16);
    assert_eq!(deferred(&samples).1, 2);
    assert_eq!(eager(&samples).0, deferred(&samples).0);
    let ties = vec![samples[7]; 8];
    assert_eq!(eager(&ties).1, 2);
    assert_eq!(deferred(&ties).1, 2);
    let mip_tie = [Tap { texel: samples[0].texel, weight: 0.125 },
                   Tap { texel: samples[4].texel, weight: 0.125 }];
    assert_eq!(deferred(&mip_tie).0, payload(samples[0].texel));
}

fn directional_accumulation(az: f32, el: f32, prune: bool) -> ([u32; 8], Option<usize>, usize) {
    let mut sums = [0.0f32; 8]; // RGB, four normal/roughness lanes, coverage.
    let mut strongest = 0.0;
    let mut representative = None;
    let mut samples = 0;
    for e in 0..2 { for a in 0..2 {
        let weight = (if a == 0 { 1.0 - az } else { az })
            * if e == 0 { 1.0 - el } else { el };
        if prune && weight == 0.0 { continue; }
        samples += 1;
        let i = e * 2 + a;
        let color = [[0.125f32, 0.8, 0.31, 0.7], [0.4, 0.2, 0.01, 0.0],
                     [0.1, 0.44, 0.9, 1.0], [0.33, 0.42, 0.73, 0.5]][i];
        let normal = [[-0.12f32, 0.8, 0.4, 0.7], [0.4, -0.2, 0.01, 0.0],
                      [-0.1, 0.44, -0.9, 1.0], [0.33, -0.42, 0.73, 0.5]][i];
        let covered = weight * color[3];
        for k in 0..3 { sums[k] += color[k] * covered; }
        for k in 0..4 { sums[k + 3] += normal[k] * covered; }
        sums[7] += covered;
        if covered > strongest { strongest = covered; representative = Some(i); }
    }}
    (sums.map(f32::to_bits), representative, samples)
}

#[test]
fn zero_angular_directions_preserve_accumulation_bits_and_selection() {
    for az in [0.0, f32::EPSILON, 0.125, 0.5, 0.999, 1.0] {
        for el in [0.0, f32::EPSILON, 0.25, 0.5, 0.999, 1.0] {
            let old = directional_accumulation(az, el, false);
            let new = directional_accumulation(az, el, true);
            assert_eq!((new.0, new.1), (old.0, old.1), "az={az},el={el}");
            assert!(new.2 <= old.2);
        }
    }
    assert_eq!(directional_accumulation(0.0, 0.0, true).2, 1);
    assert_eq!(directional_accumulation(0.125, 0.0, true).2, 2);
    assert_eq!(directional_accumulation(0.125, 0.25, true).2, 4);
}

#[test]
fn actual_shader_pruning_specializations_validate_default_off() {
    use naga::ShaderStage::{Fragment, Vertex};
    let source = super::shader_source();
    let module = naga::front::wgsl::parse_str(&source)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
    let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
    let option = module.overrides.iter().find(|(_, o)| o.name.as_deref() == Some("MEADOW_AGGREGATE_SAMPLE_PRUNE")).unwrap().1;
    assert!(matches!(module.global_expressions[option.init.unwrap()], naga::Expression::Literal(naga::Literal::Bool(false))));
    for prune in [0.0, 1.0] { for half in [0.0, 1.0] {
        let mut constants = naga::back::PipelineConstants::default();
        constants.insert("MEADOW_AGGREGATE_SAMPLE_PRUNE".into(), prune);
        constants.insert("MEADOW_AGGREGATE_HALF_RES".into(), half);
        for entry in [(Vertex, "meadow_aggregate_vs"), (Fragment, "meadow_aggregate_depth_fs"),
                      (Fragment, "meadow_aggregate_depth_motion_fs"), (Fragment, "meadow_aggregate_fs"),
                      (Vertex, "meadow_vs"), (Fragment, "meadow_fs")] {
            naga::back::pipeline_constants::process_overrides(&module, &info, Some(entry), &constants)
                .unwrap_or_else(|e| panic!("{entry:?} prune={prune} half={half}: {e}"));
        }
    }}
    // Bind the CPU oracles to the production strict tie/zero policies. They do
    // not certify texture filtering or GPU arithmetic; actual A/B still must.
    let shader = include_str!("shaders/meadow_aggregate.wgsl");
    assert!(shader.contains("if w>strongest"));
    assert!(shader.contains("if MEADOW_AGGREGATE_SAMPLE_PRUNE && strongest>0.0"));
    assert!(shader.contains("if MEADOW_AGGREGATE_SAMPLE_PRUNE && weight==0.0 {continue;}"));
    assert!(shader.find("weight==0.0").unwrap() < shader.find("let s=meadow_atlas_sample").unwrap());
}
