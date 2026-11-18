//! Pop meter: finds one-frame content changes ("pops") in a frame sequence.
//!
//! usage: cargo run --release -p render --example pop_meter -- DIR [--radius R] [--threshold T] [--crops OUTDIR]
//! DIR holds 000.png, 001.png, ... (terrain_fly --dump). Per 16 px block and
//! frame, the residual is the smallest mean absolute luma difference to the
//! previous frame over shifts up to R px (what camera motion explains is
//! matched away). A pop is a block whose residual at t exceeds the larger of
//! the residuals at t-1 and t+1 in its 3x3 neighbourhood: a change that is
//! there for one frame transition only, unlike shimmer or motion. Writes
//! DIR/pops.json (per frame scores, top regions).
use std::path::{Path, PathBuf};

const B: usize = 16;

struct Luma { w: usize, h: usize, v: Vec<f32> }

fn load(path: &Path) -> Luma {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    let ch = match info.color_type { png::ColorType::Rgba => 4, png::ColorType::Rgb => 3, c => panic!("{c:?}") };
    let (w, h) = (info.width as usize, info.height as usize);
    let v = buf[..w * h * ch].chunks_exact(ch).map(|p| 0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32).collect();
    Luma { w, h, v }
}

fn rgb(path: &Path) -> (usize, usize, Vec<u8>) {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    let ch = if info.color_type == png::ColorType::Rgba { 4 } else { 3 };
    let (w, h) = (info.width as usize, info.height as usize);
    (w, h, buf[..w * h * ch].chunks_exact(ch).flat_map(|p| [p[0], p[1], p[2]]).collect())
}

/// Motion-matched residual per block of `cur` against `prev`.
fn residuals(prev: &Luma, cur: &Luma, radius: i32) -> Vec<f32> {
    let (bw, bh) = (cur.w / B, cur.h / B);
    let mut out = vec![0f32; bw * bh];
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8);
    let rows_per = bh.div_ceil(threads);
    std::thread::scope(|s| {
        for (chunk, rows) in out.chunks_mut(rows_per * bw).enumerate() {
            s.spawn(move || {
                for (i, slot) in rows.iter_mut().enumerate() {
                    let (bx, by) = (i % bw, chunk * rows_per + i / bw);
                    let (x0, y0) = (bx * B, by * B);
                    let sad = |dx: i32, dy: i32| -> f32 {
                        let mut s = 0f32; let mut n = 0;
                        for y in (0..B).step_by(2) { for x in (0..B).step_by(2) {
                            let (px, py) = (x0 + x, y0 + y);
                            let (qx, qy) = (px as i32 + dx, py as i32 + dy);
                            if qx < 0 || qy < 0 || qx >= prev.w as i32 || qy >= prev.h as i32 { continue; }
                            s += (cur.v[py * cur.w + px] - prev.v[qy as usize * prev.w + qx as usize]).abs(); n += 1;
                        } }
                        if n == 0 { f32::MAX } else { s / n as f32 }
                    };
                    // Coarse search on a 2 px grid, then refine around the best.
                    let mut best = (sad(0, 0), 0, 0);
                    for dy in (-radius..=radius).step_by(2) { for dx in (-radius..=radius).step_by(2) {
                        let v = sad(dx, dy); if v < best.0 { best = (v, dx, dy); }
                    } }
                    let (cx, cy) = (best.1, best.2);
                    for dy in -1..=1 { for dx in -1..=1 { let v = sad(cx + dx, cy + dy); if v < best.0 { best = (v, cx + dx, cy + dy); } } }
                    *slot = best.0;
                }
            });
        }
    });
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = PathBuf::from(&args[1]);
    let opt = |name: &str| args.windows(2).find(|w| w[0] == name).map(|w| w[1].clone());
    let radius: i32 = opt("--radius").and_then(|v| v.parse().ok()).unwrap_or(12);
    let threshold: f32 = opt("--threshold").and_then(|v| v.parse().ok()).unwrap_or(4.0);
    let crops = opt("--crops").map(PathBuf::from);
    let mut frames: Vec<PathBuf> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "png") && p.file_stem().is_some_and(|s| s.to_string_lossy().chars().all(|c| c.is_ascii_digit()))).collect();
    frames.sort();
    let n = frames.len();
    assert!(n >= 3, "need at least 3 frames");
    // r[t] compares frame t with t-1 (t >= 1).
    let mut r: Vec<Vec<f32>> = vec![Vec::new()];
    let mut prev = load(&frames[0]);
    let (bw, bh) = (prev.w / B, prev.h / B);
    for (t, path) in frames.iter().enumerate().skip(1) {
        let cur = load(path);
        r.push(residuals(&prev, &cur, radius));
        prev = cur;
        if t % 50 == 0 { eprintln!("frame {t}/{n}"); }
    }
    let neighbourhood_max = |v: &[f32], bx: usize, by: usize| -> f32 {
        let mut m = 0f32;
        for y in by.saturating_sub(1)..(by + 2).min(bh) { for x in bx.saturating_sub(1)..(bx + 2).min(bw) { m = m.max(v[y * bw + x]); } }
        m
    };
    // Per frame residual and spike grids (f32 LE, bw x bh each, frames 1..n), for lookups.
    let mut grids = std::io::BufWriter::new(std::fs::File::create(dir.join("grids.bin")).unwrap());
    let mut per_frame = Vec::new();
    let mut regions = Vec::new();
    for t in 1..n {
        let mut spike = vec![0f32; bw * bh];
        for by in 0..bh { for bx in 0..bw {
            let i = by * bw + bx;
            let before = if t >= 2 { neighbourhood_max(&r[t - 1], bx, by) } else { 0. };
            let after = if t + 1 < n { neighbourhood_max(&r[t + 1], bx, by) } else { 0. };
            spike[i] = (r[t][i] - before.max(after)).max(0.);
        } }
        { use std::io::Write; for v in r[t].iter().chain(&spike) { grids.write_all(&v.to_le_bytes()).unwrap(); } }
        let mean_residual = r[t].iter().sum::<f32>() / r[t].len() as f32;
        // Connected regions of spiking blocks.
        let mut seen = vec![false; bw * bh];
        let mut frame_score = 0f32; let mut frame_blocks = 0;
        for start in 0..bw * bh {
            if seen[start] || spike[start] < threshold { continue; }
            let mut stack = vec![start]; seen[start] = true;
            let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
            let (mut sum, mut peak, mut count) = (0f32, 0f32, 0);
            while let Some(i) = stack.pop() {
                let (bx, by) = (i % bw, i / bw);
                x0 = x0.min(bx); y0 = y0.min(by); x1 = x1.max(bx); y1 = y1.max(by);
                sum += spike[i]; peak = peak.max(spike[i]); count += 1;
                for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let (nx, ny) = (bx as i32 + dx, by as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= bw as i32 || ny >= bh as i32 { continue; }
                    let j = ny as usize * bw + nx as usize;
                    if !seen[j] && spike[j] >= threshold { seen[j] = true; stack.push(j); }
                }
            }
            frame_score += sum; frame_blocks += count;
            regions.push(serde_json::json!({"frame": t, "x": x0 * B, "y": y0 * B, "w": (x1 - x0 + 1) * B, "h": (y1 - y0 + 1) * B,
                "blocks": count, "score": sum, "peak": peak}));
        }
        per_frame.push(serde_json::json!({"frame": t, "pop_score": frame_score, "pop_blocks": frame_blocks, "mean_residual": mean_residual}));
    }
    regions.sort_by(|a, b| b["score"].as_f64().unwrap().total_cmp(&a["score"].as_f64().unwrap()));
    let mut top: Vec<_> = regions.iter().take(20).cloned().collect();
    // Mean luma inside each top region before and after: brighter, darker or a
    // silhouette change (large mean absolute difference, small mean shift).
    for reg in &mut top {
        let t = reg["frame"].as_u64().unwrap() as usize;
        let (a, b) = (load(&frames[t - 1]), load(&frames[t]));
        let (rx, ry, rw, rh) = (reg["x"].as_u64().unwrap() as usize, reg["y"].as_u64().unwrap() as usize, reg["w"].as_u64().unwrap() as usize, reg["h"].as_u64().unwrap() as usize);
        let (mut sa, mut sb, mut sd, mut n) = (0f64, 0f64, 0f64, 0f64);
        for y in ry..(ry + rh).min(a.h) { for x in rx..(rx + rw).min(a.w) {
            let (va, vb) = (a.v[y * a.w + x] as f64, b.v[y * b.w + x] as f64);
            sa += va; sb += vb; sd += (va - vb).abs(); n += 1.;
        } }
        reg["luma_before"] = serde_json::json!(sa / n); reg["luma_after"] = serde_json::json!(sb / n); reg["mean_abs_diff"] = serde_json::json!(sd / n);
    }
    if let Some(out) = &crops {
        std::fs::create_dir_all(out).unwrap();
        for (k, reg) in top.iter().enumerate() {
            let t = reg["frame"].as_u64().unwrap() as usize;
            let (rx, ry, rw, rh) = ["x", "y", "w", "h"].map(|k| reg[k].as_u64().unwrap() as usize).into();
            // A square crop around the region, at least 192 px, before | after.
            let side = rw.max(rh).max(192) + 64;
            let (w, h, a) = rgb(&frames[t - 1]); let (_, _, b) = rgb(&frames[t]);
            let cx = (rx + rw / 2).clamp(side / 2, w - side / 2); let cy = (ry + rh / 2).clamp(side / 2, h - side / 2);
            let (ox, oy) = (cx - side / 2, cy - side / 2);
            let mut img = vec![0u8; side * 2 * side * 3 + 0];
            for y in 0..side { for x in 0..side {
                for c in 0..3 {
                    img[(y * side * 2 + x) * 3 + c] = a[((oy + y) * w + ox + x) * 3 + c];
                    img[(y * side * 2 + side + x) * 3 + c] = b[((oy + y) * w + ox + x) * 3 + c];
                }
            } }
            let file = std::fs::File::create(out.join(format!("{k:02}-f{t:03}.png"))).unwrap();
            let mut enc = png::Encoder::new(std::io::BufWriter::new(file), (side * 2) as u32, side as u32);
            enc.set_color(png::ColorType::Rgb); enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(&img).unwrap();
        }
    }
    let report = serde_json::json!({"frames": n, "block": B, "grid": [bw, bh], "radius": radius, "threshold": threshold, "per_frame": per_frame, "top": top, "regions": regions.len()});
    std::fs::write(dir.join("pops.json"), serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    let total: f64 = per_frame.iter().map(|f| f["pop_score"].as_f64().unwrap()).sum();
    println!("{} frames, {} pop regions, total score {:.0}", n, regions.len(), total);
    for reg in &top { println!("{reg}"); }
}
