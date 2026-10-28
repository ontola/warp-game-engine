//! warp-audio: render and inspect procedural sounds without a game. The
//! command line lives in `audio::cli`, shared with `warp audio`.
fn main() {
    if let Err(e) = audio::cli::run(std::env::args().skip(1).collect()) {
        // The usage names no program: it is shared with `warp audio`.
        match e.strip_prefix("usage: ") { Some(u) => eprintln!("usage: warp-audio {u}"), None => eprintln!("{e}") }
        std::process::exit(1);
    }
}
