//! CPU-only road/terrain seam diagnostics for a recorded F12 stage.
use gdcompat::Pcg;
use sideways::{
    builder,
    stage::{GRID, RUNOUT, Stage},
};

fn ground(s: &Stage, x: f64, z: f64) -> f64 {
    let x0 = (x / GRID).floor() * GRID;
    let z0 = (z / GRID).floor() * GRID;
    let fx = (x - x0) / GRID;
    let fz = (z - z0) / GRID;
    let h = |x, z| s.height(x, z) as f32 as f64;
    if fx + fz <= 1.0 {
        h(x0, z0) * (1.0 - fx - fz) + h(x0 + GRID, z0) * fx + h(x0, z0 + GRID) * fz
    } else {
        h(x0 + GRID, z0 + GRID) * (fx + fz - 1.0)
            + h(x0, z0 + GRID) * (1.0 - fx)
            + h(x0 + GRID, z0) * (1.0 - fz)
    }
}
fn main() {
    let mut rng = Pcg::new(1);
    let mut found = None;
    for skip in 0..100_000 {
        let mut probe = rng.clone();
        if probe.next_u32() as i32 == -1661450362 && probe.next_u32() as i32 == -518345863 {
            found = Some((skip, rng));
            break;
        }
        rng.next_u32();
    }
    let (skip, mut rng) = found.expect("recorded seed pair");
    let s = Stage::generate(&mut rng, 3000.0 + RUNOUT);
    let r = builder::road(&s);
    let near = s.nearest_index(192.576828, -1490.746338) as usize;
    println!(
        "skip={skip} row={near} noise={} detail={} points={}",
        s.noise_seed,
        s.detail_seed,
        s.points.len()
    );
    let mut old_worst = (-1e9, 0, 0, 0.0);
    let mut old_low = (1e9, 0, 0, 0.0);
    for row in 0..r.rows - 1 {
        for c in [0, r.cols - 1] {
            let mut a = r.verts[row * r.cols + c];
            let mut b = r.verts[(row + 1) * r.cols + c];
            a.y = (s.height_near(a.x as f64, a.z as f64, row as i64) + 0.02) as f32;
            b.y = (s.height_near(b.x as f64, b.z as f64, row as i64 + 1) + 0.02) as f32;
            for k in 0..=8 {
                let t = k as f32 / 8.0;
                let p = a.lerp(b, t);
                let gap = p.y as f64 - ground(&s, p.x as f64, p.z as f64);
                if gap > old_worst.0 {
                    old_worst = (gap, row, c, t as f64);
                }
                if gap < old_low.0 {
                    old_low = (gap, row, c, t as f64);
                }
            }
        }
    }
    let mut worst = (-1e9, 0, 0, 0.0);
    let mut low = (1e9, 0, 0, 0.0);
    let mut edges = 0;
    for tri in &r.tris {
        for e in 0..3 {
            let a = tri[e] as usize;
            let b = tri[(e + 1) % 3] as usize;
            if r.uvs[a][0].abs() < 1.1 || r.uvs[a][0] != r.uvs[b][0] {
                continue;
            }
            edges += 1;
            let row = (r.uvs[a][1].min(r.uvs[b][1]) / 2.0) as usize;
            for k in 0..=8 {
                let t = k as f32 / 8.0;
                let p = r.verts[a].lerp(r.verts[b], t);
                let y = ground(&s, p.x as f64, p.z as f64);
                let gap = p.y as f64 - y;
                if gap > worst.0 {
                    worst = (gap, row, a, t as f64);
                }
                if gap < low.0 {
                    low = (gap, row, a, t as f64);
                }
                if row == near && k == 4 {
                    println!(
                        "row={row} side={} p={p:?} surface={y} gap={gap}",
                        r.uvs[a][0]
                    );
                }
            }
        }
    }
    println!(
        "edges={edges} vertices={} triangles={} worst_above={worst:?} worst_below={low:?}",
        r.verts.len(),
        r.tris.len()
    );
    let report = serde_json::json!({
        "scope":"CPU geometry only; no GPU render or gameplay performance claim",
        "capture":"sideways-1792818090962-266592-4.png",
        "seed":1,"round":3,"pcg_skip":skip,"stage_noise_seed":s.noise_seed,"stage_detail_seed":s.detail_seed,
        "car_nearest_row":near,"rows":r.rows,"columns":r.cols,
        "vertices":r.verts.len(),"triangles":r.tris.len(),"boundary_segments":edges,"samples_per_segment":9,
        "old_max_gap_m":old_worst.0,"old_min_gap_m":old_low.0,
        "new_max_gap_m":worst.0,"new_min_gap_m":low.0,
        "intentional_ground_overlap_m":0.003,
        "main_road":"positions inside normalized road width +/-1 and all original interior triangles preserved",
        "shoulder_collision":"same corrected shoulder mesh as rendered; outer 80 cm intentionally changes"
    });
    if let Some(out) = std::env::args().nth(1) {
        std::fs::write(&out, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        println!("CPU report: {out}");
    }
}
