//! What Luau's C++ needs in a browser (wasm32) that a native build gets
//! from the platform. The browser build compiles Luau against wasi-libc
//! and libc++ (engine/tools/build-web.py) without C++ exception support,
//! so a thrown error cannot be caught: `__cxa_throw` stops the game with
//! the error's message instead. Lua errors (`error()`, a failed step
//! limit, a compile error) therefore end the game in a browser; a game
//! that runs cleanly never throws.
use std::alloc::{Layout, alloc};
use std::ffi::{CStr, c_char, c_void};

/// `__cxa_allocate_exception(size_t)`: room for the thrown object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __cxa_allocate_exception(size: usize) -> *mut c_void {
    unsafe { alloc(Layout::from_size_align(size.max(1), 16).expect("exception size")) as *mut c_void }
}

/// `__cxa_throw(object, type_info, destructor)`: no unwinding in this build,
/// so report what was thrown and stop.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __cxa_throw(object: *mut c_void, tinfo: *const c_void, _dest: *const c_void) -> ! {
    // An Itanium `std::type_info` is a vtable pointer, then its mangled name.
    let name = unsafe { tinfo.cast::<*const c_char>().add(1).read() };
    let name = if name.is_null() { "?".into() } else { unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned() };
    let detail = if name.ends_with("lua_exception") {
        // Luau's `lua_exception`: a vtable pointer, then its lua_State; the
        // message is on top of that state's stack.
        let state = unsafe { object.cast::<*mut mlua::ffi::lua_State>().add(1).read() };
        let mut len = 0;
        let text = unsafe { mlua::ffi::lua_tolstring(state, -1, &mut len) };
        if text.is_null() { "a Lua error".into() } else { String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(text as *const u8, len) }).into_owned() }
    } else {
        format!("a C++ exception ({name}), probably a Luau compile error")
    };
    panic!("Luau error in the browser build, which cannot recover from it: {detail}");
}

/// `clock()`: Luau's `lua_clock` reads it for statistics. Nanoseconds since
/// start (`CLOCKS_PER_SEC` is 10^9 in wasi-libc, `clock_t` 64 bits).
#[unsafe(no_mangle)]
pub extern "C" fn clock() -> i64 {
    thread_local! { static START: web_time::Instant = web_time::Instant::now(); }
    START.with(|s| s.elapsed().as_nanos() as i64)
}

// wasi-libc (which Luau's C++ is linked against) calls these five WASI
// functions; a browser has no WASI host, so they are defined here and the
// module imports nothing from `wasi_snapshot_preview1`. Output on stdout and
// stderr (fd 1, 2) is dropped: Luau prints through the game's own `print`.
const ERRNO_BADF: i32 = 8;

/// `fd_write(fd, iovs, iovs_len, *written)`: count the bytes, write nothing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __imported_wasi_snapshot_preview1_fd_write(fd: i32, iovs: *const [u32; 2], len: i32, written: *mut u32) -> i32 {
    if !(1..=2).contains(&fd) { return ERRNO_BADF; }
    let total = (0..len as usize).map(|i| unsafe { (*iovs.add(i))[1] }).sum();
    unsafe { written.write(total) };
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn __imported_wasi_snapshot_preview1_fd_close(_fd: i32) -> i32 { 0 }

#[unsafe(no_mangle)]
pub extern "C" fn __imported_wasi_snapshot_preview1_fd_seek(_fd: i32, _offset: i64, _whence: i32, _to: *mut u64) -> i32 { ERRNO_BADF }

/// `fd_fdstat_get(fd, *stat)`: stdout and stderr are character devices.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __imported_wasi_snapshot_preview1_fd_fdstat_get(fd: i32, stat: *mut u8) -> i32 {
    if !(0..=2).contains(&fd) { return ERRNO_BADF; }
    // struct fdstat { u8 filetype; u16 flags; u64 rights_base; u64 rights_inheriting } (24 bytes)
    unsafe { std::ptr::write_bytes(stat, 0, 24); stat.write(2) }; // CHARACTER_DEVICE
    0
}

/// `clock_time_get(id, precision, *time)`: nanoseconds, from the page's clock.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __imported_wasi_snapshot_preview1_clock_time_get(id: i32, _precision: i64, time: *mut u64) -> i32 {
    let ns = if id == 0 {
        web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64)
    } else {
        thread_local! { static START: web_time::Instant = web_time::Instant::now(); }
        START.with(|s| s.elapsed().as_nanos() as u64)
    };
    unsafe { time.write(ns) };
    0
}

unsafe extern "C" {
    /// Made by the linker: runs the C++ static constructors (Luau's tables).
    fn __wasm_call_ctors();
}

/// Run Luau's C++ static constructors, once, before any Luau state is made.
/// Call it first thing in a browser build's `main`: when nothing calls
/// `__wasm_call_ctors`, the linker instead runs the constructors again on
/// every call from JavaScript into the module, which hangs the page.
pub fn init() {
    static DONE: std::sync::Once = std::sync::Once::new();
    DONE.call_once(|| unsafe { __wasm_call_ctors() });
}
