//! Top-down maps of the terrain function's attributes, for tuning.
//! Run: field_maps OUT_DIR CX CZ SIZE_M [CUT] [STYLE]
use render::terrain_field;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let out = std::path::PathBuf::from(&a[1]);
    let (cx, cz, size): (f32, f32, f32) = (a[2].parse().unwrap(), a[3].parse().unwrap(), a[4].parse().unwrap());
    let cut: f32 = a.get(5).map(|s| s.parse().unwrap()).unwrap_or(0.0);
    let style = a.get(6).and_then(|n| terrain_field::Style::named(n)).unwrap_or_default();
    let n = 512u32;
    let mut maps = vec![vec![0u8; (n * n * 3) as usize]; 3];
    let mut heights = vec![0.0f32; (n * n) as usize];
    let (mut rock, mut snow, mut grass) = (0u32, 0u32, 0u32);
    let mut ups = [0u32; 10];
    for j in 0..n {
        for i in 0..n {
            let x = cx + (i as f32 / n as f32 - 0.5) * size;
            let z = cz + (j as f32 / n as f32 - 0.5) * size;
            let s = terrain_field::eval(x, z, cut, 1, &style);
            heights[(j * n + i) as usize] = s.pos.y;
            ups[((s.up * 10.0) as usize).min(9)] += 1;
            if s.rock > 0.5 { rock += 1 } else if s.snow > 0.5 { snow += 1 } else if s.scree < 0.5 { grass += 1 }
            let k = ((j * n + i) * 3) as usize;
            let shade = (s.up * 255.0) as u8;
            maps[0][k] = (s.rock * 255.0) as u8;
            maps[0][k + 1] = (s.scree * 255.0) as u8;
            maps[0][k + 2] = if s.water > 0.0 { 255 } else { shade / 2 };
            let hy = ((s.pos.y + 200.0) / 1000.0 * 255.0).clamp(0.0, 255.0) as u8;
            maps[1][k] = hy;
            maps[1][k + 1] = (s.wet * 255.0) as u8;
            maps[1][k + 2] = (s.moor * 255.0) as u8;
        }
    }
    let pct = |c: u32| 100.0 * c as f32 / (n * n) as f32;
    println!("up deciles {:?}", ups.map(|c| (pct(c) + 0.5) as u32));
    println!("rock {:.0}%  snow {:.0}%  grass {:.0}%", pct(rock), pct(snow), pct(grass));
    // Shaded relief: green low, brown high, white snow, lit from the north-west.
    let px = size / n as f32;
    for j in 1..n - 1 {
        for i in 1..n - 1 {
            let h = |di: i32, dj: i32| heights[((j as i32 + dj) as u32 * n + (i as i32 + di) as u32) as usize];
            let (gx, gz) = ((h(1, 0) - h(-1, 0)) / (2.0 * px), (h(0, 1) - h(0, -1)) / (2.0 * px));
            let lit = ((-gx - gz) * 0.7 + 1.0) / (1.0 + gx * gx + gz * gz).sqrt();
            let t = ((h(0, 0) - 300.0) / 2200.0).clamp(0.0, 1.0);
            let c = [70.0 + 120.0 * t + 60.0 * t * t, 110.0 + 40.0 * t + 80.0 * t * t, 60.0 + 30.0 * t + 150.0 * t * t];
            let k = ((j * n + i) * 3) as usize;
            for q in 0..3 { maps[2][k + q] = (c[q] * lit.clamp(0.2, 1.4)).clamp(0.0, 255.0) as u8; }
        }
    }
    image::save_buffer(out.join("relief.png"), &maps[2], n, n, image::ColorType::Rgb8).unwrap();
    image::save_buffer(out.join("attr.png"), &maps[0], n, n, image::ColorType::Rgb8).unwrap();
    image::save_buffer(out.join("height.png"), &maps[1], n, n, image::ColorType::Rgb8).unwrap();
}
