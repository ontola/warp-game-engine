//! What the browser (wasm32) build needs that a native build gets from the
//! platform: C++ `operator new`/`delete` for meshoptimizer, which has no C++
//! standard library on wasm32.

use std::alloc::{Layout, alloc, dealloc};

// Each block starts with its size, so `delete` can rebuild the layout.
const HEADER: usize = 16;

fn layout(size: usize) -> Layout { Layout::from_size_align(size + HEADER, HEADER).expect("C++ allocation size") }

unsafe fn allocate(size: usize) -> *mut u8 {
    unsafe {
        let base = alloc(layout(size));
        if base.is_null() { std::alloc::handle_alloc_error(layout(size)); }
        (base as *mut usize).write(size);
        base.add(HEADER)
    }
}

unsafe fn free(ptr: *mut u8) {
    if ptr.is_null() { return; }
    unsafe {
        let base = ptr.sub(HEADER);
        dealloc(base, layout((base as *const usize).read()));
    }
}

/// `operator new(size_t)`
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Znwm(size: usize) -> *mut u8 { unsafe { allocate(size) } }
/// `operator new[](size_t)`
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Znam(size: usize) -> *mut u8 { unsafe { allocate(size) } }
/// `operator delete(void*)`
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _ZdlPv(ptr: *mut u8) { unsafe { free(ptr) } }
/// `operator delete[](void*)`
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _ZdaPv(ptr: *mut u8) { unsafe { free(ptr) } }
/// `operator delete(void*, size_t)`
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _ZdlPvm(ptr: *mut u8, _: usize) { unsafe { free(ptr) } }
/// `operator delete[](void*, size_t)`
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _ZdaPvm(ptr: *mut u8, _: usize) { unsafe { free(ptr) } }
