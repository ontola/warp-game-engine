//! CPU contract oracles for the meadow Hi-Z query. These test the conservative
//! reduction/footprint mathematics; GPU image equality remains a separate gate.

#[derive(Clone)]
struct Level {
    width: usize,
    height: usize,
    depth: Vec<f32>,
}

fn pyramid(width: usize, height: usize, depth: Vec<f32>) -> Vec<Level> {
    assert_eq!(depth.len(), width * height);
    let mut levels = vec![Level {
        width,
        height,
        depth,
    }];
    while levels.last().unwrap().width > 1 || levels.last().unwrap().height > 1 {
        let source = levels.last().unwrap();
        let width = (source.width / 2).max(1);
        let height = (source.height / 2).max(1);
        let mut depth = Vec::with_capacity(width * height);
        for y in 0..height {
            for x in 0..width {
                let end_x = if x + 1 == width {
                    source.width
                } else {
                    2 * x + 2
                };
                let end_y = if y + 1 == height {
                    source.height
                } else {
                    2 * y + 2
                };
                let minimum = (2 * y..end_y)
                    .flat_map(|sy| {
                        (2 * x..end_x).map(move |sx| source.depth[sy * source.width + sx])
                    })
                    .fold(1.0f32, f32::min);
                depth.push(minimum);
            }
        }
        levels.push(Level {
            width,
            height,
            depth,
        });
    }
    levels
}

/// Inclusive BASE pixel rectangle, after the shader's outward pixel expansion.
fn query(levels: &[Level], lo: [usize; 2], hi: [usize; 2]) -> f32 {
    for (mip, level) in levels.iter().enumerate() {
        let first = [
            (lo[0] >> mip).min(level.width - 1),
            (lo[1] >> mip).min(level.height - 1),
        ];
        let last = [
            (hi[0] >> mip).min(level.width - 1),
            (hi[1] >> mip).min(level.height - 1),
        ];
        if last[0] - first[0] > 1 || last[1] - first[1] > 1 {
            continue;
        }
        return (first[1]..=last[1])
            .flat_map(|y| (first[0]..=last[0]).map(move |x| level.depth[y * level.width + x]))
            .fold(1.0f32, f32::min);
    }
    panic!("a complete pyramid must cover every rectangle");
}

#[test]
fn odd_hiz_footprints_never_claim_more_occlusion_than_base_pixels() {
    for (width, height) in [(1, 9), (9, 1), (5, 3), (9, 7), (16, 8), (17, 13)] {
        let base: Vec<_> = (0..width * height)
            .map(|i| ((i * 37 + 11) % 101) as f32 / 101.0)
            .collect();
        let levels = pyramid(width, height, base.clone());
        // Exhaustive rectangles, independently compare against every original
        // base pixel. This catches odd-edge coverage lost by UV*mipSize.
        for y0 in 0..height {
            for y1 in y0..height {
                for x0 in 0..width {
                    for x1 in x0..width {
                        let mut exact = 1.0f32;
                        for y in y0..=y1 {
                            for x in x0..=x1 {
                                exact = exact.min(base[y * width + x]);
                            }
                        }
                        let conservative = query(&levels, [x0, y0], [x1, y1]);
                        assert!(
                            conservative <= exact,
                            "{width}x{height} [{x0},{y0}]..[{x1},{y1}]: {conservative}>{exact}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn a_single_sky_opening_prevents_complete_box_rejection() {
    let (width, height) = (9, 7);
    let clear = pyramid(width, height, vec![0.8; width * height]);
    assert!(query(&clear, [1, 1], [8, 6]) > 0.5 + 2e-5);
    // Every covered base pixel can independently be the sole opening. Include
    // the odd final row/column, rather than testing only a convenient centre.
    for y in 1..height {
        for x in 1..width {
            let mut depths = vec![0.8; width * height];
            depths[y * width + x] = 0.0;
            let levels = pyramid(width, height, depths);
            assert_eq!(query(&levels, [1, 1], [8, 6]), 0.0, "lost sky at {x},{y}");
        }
    }
}

#[test]
fn reversed_depth_requires_all_occluders_in_front_with_margin() {
    let levels = pyramid(5, 3, vec![0.6; 15]);
    let depth = query(&levels, [0, 0], [4, 2]);
    assert!(depth > 0.4 + 2e-5, "fully hidden box should be rejected");
    assert!(!(depth > 0.7 + 2e-5), "foreground box must remain visible");
    assert!(
        !(depth > 0.59999 + 2e-5),
        "near-equal depth must remain visible"
    );
    assert!(!(depth > 0.6 + 2e-5), "coplanar depth must remain visible");
}
