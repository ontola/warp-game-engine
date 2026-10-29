//! Top-down maps of the terrain function's attributes, for tuning.
//! Run: field_maps OUT_DIR CX CZ SIZE_M [CUT]
use render::terrain_field;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let out = std::path::PathBuf::from(&a[1]);
    let (cx, cz, size): (f32, f32, f32) = (a[2].parse().unwrap(), a[3].parse().unwrap(), a[4].parse().unwrap());
    let cut: f32 = a.get(5).map(|s| s.parse().unwrap()).unwrap_or(0.0);
    let n = 512u32;
    let mut maps = vec![vec![0u8; (n * n * 3) as usize]; 2];
    for j in 0..n {
        for i in 0..n {
            let x = cx + (i as f32 / n as f32 - 0.5) * size;
            let z = cz + (j as f32 / n as f32 - 0.5) * size;
            let s = terrain_field::eval(x, z, cut, 1, &terrain_field::Style::default());
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
    image::save_buffer(out.join("attr.png"), &maps[0], n, n, image::ColorType::Rgb8).unwrap();
    image::save_buffer(out.join("height.png"), &maps[1], n, n, image::ColorType::Rgb8).unwrap();
}
