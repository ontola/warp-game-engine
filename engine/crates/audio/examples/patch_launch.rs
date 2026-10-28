//! CPU-only voice launch probe; no device or mixer is opened.
//! cargo run --release -p audio --features luau --example patch_launch -- [patch.luau]
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct CountAlloc;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(size, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) { unsafe { System.dealloc(ptr, layout) } }
}

#[global_allocator]
static ALLOCATOR: CountAlloc = CountAlloc;

#[cfg(feature = "luau")]
fn main() {
    use std::hint::black_box;
    let path = std::env::args().nth(1).unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/../../../games/growing-guns/sounds/shot.luau").into());
    let patch = audio::synth::lua::load_file(std::path::Path::new(&path)).expect("load patch").remove(0);
    let prepared = patch.compile().expect("compile patch");
    const N: usize = 4096;
    let bench = |label: &str, mut launch: Box<dyn FnMut(u32) -> audio::synth::Voice>| {
        drop(black_box(launch(0)));
        ALLOCS.store(0, Ordering::Relaxed);
        BYTES.store(0, Ordering::Relaxed);
        let started = std::time::Instant::now();
        for seed in 0..N { drop(black_box(launch(black_box(seed as u32)))); }
        let elapsed = started.elapsed();
        let allocations = ALLOCS.load(Ordering::Relaxed);
        let bytes = BYTES.load(Ordering::Relaxed);
        println!("{label}: {:.3} us/voice; {:.1} allocations/voice; {:.0} allocated bytes/voice", elapsed.as_secs_f64() * 1e6 / N as f64, allocations as f64 / N as f64, bytes as f64 / N as f64);
    };
    println!("{}: {} nodes, {N} launches, 48000 Hz", patch.name, patch.nodes.len());
    bench("source", Box::new(|seed| patch.voice(&[], seed, 48_000.0)));
    bench("prepared", Box::new(|seed| prepared.voice(&[], seed, 48_000.0)));
    let mut voice = prepared.voice(&[], 1, 48_000.0);
    let mut block = [0.0; audio::synth::BLOCK];
    ALLOCS.store(0, Ordering::Relaxed);
    for _ in 0..N { black_box(voice.render(black_box(&mut block))); }
    let allocations = ALLOCS.load(Ordering::Relaxed);
    println!("prepared render: {allocations} allocations across {N} blocks");
}

#[cfg(not(feature = "luau"))]
fn main() { eprintln!("run with --features luau"); }
