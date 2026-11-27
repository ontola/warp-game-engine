//! Audio output shared by games and standalone Warp sound tools.
pub use audio::device::{Speaker, open, start};
#[cfg(target_arch = "wasm32")]
pub use audio::device::worker;

static HUSHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A browser page's game waits paused behind the room: games with their
/// own speakers stay silent until it comes back (set by `warp::web::hush`,
/// read on the audio callback with [`hushed`]).
pub fn set_hushed(on: bool) { HUSHED.store(on, std::sync::atomic::Ordering::Relaxed); }

/// See [`set_hushed`].
pub fn hushed() -> bool { HUSHED.load(std::sync::atomic::Ordering::Relaxed) }
