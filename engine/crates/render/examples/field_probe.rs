//! Prints the terrain function's attributes along a line, for tuning.
//! Run: field_probe X0 Z0 X1 Z1 [STEPS]
use render::terrain_field;
fn main() {
    let a: Vec<f32> = std::env::args().skip(1).map(|s| s.parse().unwrap()).collect();
    let steps = a.get(4).copied().unwrap_or(40.0) as usize;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let (x, z) = (a[0] + (a[2] - a[0]) * t, a[1] + (a[3] - a[1]) * t);
        let s = terrain_field::eval(x, z, 0.0, 1);
        println!("{x:8.1} {z:8.1} y {:7.2} up {:.2} rock {:.2} scree {:.2} wet {:.2} water {:.2}", s.pos.y, s.up, s.rock, s.scree, s.wet, s.water);
    }
}
