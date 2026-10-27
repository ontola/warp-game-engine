//! Decode WAV files with audio::sfx and print length and peak: a quick check
//! that a game's sound folder loads.  cargo run -p audio --example wavinfo -- DIR
fn main() {
    let dir = std::env::args().nth(1).expect("directory");
    let mut ok = 0;
    let mut paths: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("wav"))).collect();
    paths.sort();
    for p in paths {
        match audio::sfx::load_wav(&p) {
            Ok(c) => { ok += 1; println!("{:5.2} s  peak {:.2}  {}", c.seconds(), c.0.iter().fold(0f32, |m, s| m.max(s.abs())), p.file_name().unwrap().to_string_lossy()); }
            Err(e) => println!("FAILED {e}"),
        }
    }
    println!("{ok} decoded");
}
