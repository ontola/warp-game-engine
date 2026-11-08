//! Life stages of a grown plant as a side-view sheet, without a GPU:
//! cargo run --release -p render --example plant_growth_preview -- out.png
use render::plant_growth::{grow, Plant, Species};
use render::plant_growth_assembly::{assemble, Look, Parts};

const AGES: [u32; 10] = [1, 3, 6, 12, 25, 50, 80, 120, 200, 300];
const CELL: [usize; 2] = [300, 620];

fn line(px: &mut [u8], w: usize, h: usize, a: [f32; 2], b: [f32; 2], r: f32, c: [u8; 3]) {
    let n = ((b[0] - a[0]).abs().max((b[1] - a[1]).abs()).ceil() as usize).max(1);
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let (x, y) = (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t);
        let k = r.ceil() as i32;
        for dy in -k..=k {
            for dx in -k..=k {
                let (cx, cy) = (x as i32 + dx, y as i32 + dy);
                if cx >= 0 && cy >= 0 && (cx as usize) < w && (cy as usize) < h && ((dx * dx + dy * dy) as f32) <= r * r + 0.3 {
                    px[(cy as usize * w + cx as usize) * 4..][..3].copy_from_slice(&c);
                }
            }
        }
    }
}

fn draw(px: &mut [u8], w: usize, h: usize, cell: usize, plant: &Plant, scale: f32) {
    let ox = (cell * CELL[0] + CELL[0] / 2) as f32;
    let oy = h as f32 - 12.0;
    let at = |v: glam::Vec3| [ox + v.x * scale, oy - v.y * scale];
    for s in &plant.segments {
        let c = if s.alive { [120, 72, 44] } else { [150, 145, 135] };
        line(px, w, h, at(s.from), at(s.to), (s.radius[0] * scale).max(0.5), c);
    }
    for t in &plant.tufts {
        let g = (70.0 + 90.0 * t.fullness) as u8;
        line(px, w, h, at(t.at), at(t.at + t.axis * 0.05), (0.16 * t.fullness.sqrt() * scale).clamp(1.0, 6.0), [38, g, 40]);
    }
}

fn main() -> Result<(), String> {
    let out = std::env::args().nth(1).unwrap_or_else(|| "growth.png".into());
    let species = Species::jeffrey_pine();
    let parts = Parts::long_needle_pine();
    let (w, h) = (CELL[0] * AGES.len(), CELL[1]);
    let mut true_scale = vec![235u8; w * h * 4];
    let mut fitted = vec![235u8; w * h * 4];
    let plants: Vec<Plant> = AGES.iter().map(|&a| grow(&species, a, 1)).collect();
    let tallest = plants.iter().map(Plant::height).fold(0.0, f32::max);
    for (i, p) in plants.iter().enumerate() {
        let c = match assemble(p, &parts, Look::jeffrey_pine(), 1, render::plant_growth_assembly::limits()) {
            Ok((_, c)) => c,
            Err(e) => { println!("age {:3}: height {:5.1} m  radius {:4.1} m  segments {:6}  tufts {:5}  assembly failed: {e}", p.age, p.height(), p.crown_radius(), p.segments.len(), p.tufts.len()); continue; }
        };
        println!("age {:3}: height {:5.1} m  trunk r {:.2} m  crown base {:4.1} m  radius {:4.1} m  segments {:6}  tufts {:5}  bones {:2}  stems {:5}  placed tufts {:5} dropped {:5}", p.age, p.height(), p.trunk_radius(), p.crown_base(), p.crown_radius(), p.segments.len(), p.tufts.len(), c.bones, c.stems, c.tufts, c.tufts_dropped);
        draw(&mut true_scale, w, h, i, p, (h as f32 - 30.0) / tallest);
        draw(&mut fitted, w, h, i, p, ((h as f32 - 30.0) / p.height()).min((CELL[0] as f32 - 10.0) / (2.0 * p.crown_radius().max(0.01))).min(600.0));
    }
    let mut sheet = fitted;
    sheet.extend_from_slice(&true_scale);
    render::write_png(std::path::Path::new(&out), w as u32, 2 * h as u32, &sheet)
}
