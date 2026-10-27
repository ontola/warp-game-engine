fn main() {
    let (mut max, mut lights) = (0, 0);
    for s in 0..2000u64 { let a = growing_guns::Arena::generate(s); max = max.max(a.blocks.len()); lights = lights.max(a.lights.len()); }
    println!("max blocks {max}, max lights {lights}");
}
