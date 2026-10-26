//! CPU execution time for the calling thread, separate from elapsed wall time.
//!
//! Windows measures GetThreadTimes user + kernel execution. Blocked/descheduled
//! time and other worker threads are excluded. Counter units are 100 ns; this
//! does not imply 100 ns measurement precision. Unsupported platforms and API
//! failures return None, never a wall-time substitute or a fabricated zero.
//!
//! <https://learn.microsoft.com/windows/win32/api/processthreadsapi/nf-processthreadsapi-getthreadtimes>

#[derive(Clone, Copy, Debug)]
struct ThreadTimes { kernel:u64, user:u64 }

impl ThreadTimes {
    fn elapsed_ms(self, end:Self)->Option<f64> {
        let ticks=end.kernel.checked_sub(self.kernel)?.checked_add(end.user.checked_sub(self.user)?)?;
        Some(ticks as f64 / 10_000.0)
    }
}

/// Start/end must run on the same thread. This reads two OS counters; it does
/// not query GPU work, sum process-wide workers or derive time from CPU cycles.
#[derive(Debug)]
pub struct ThreadCpuTimer {
    thread:std::thread::ThreadId,
    start:Option<ThreadTimes>,
}
impl ThreadCpuTimer {
    pub fn start()->Self {
        Self {thread:std::thread::current().id(),start:current_thread_times()}
    }
    pub fn elapsed_ms(&self)->Option<f64> {
        if self.thread!=std::thread::current().id(){return None;}
        self.start?.elapsed_ms(current_thread_times()?)
    }
}

#[cfg(windows)]
fn current_thread_times()->Option<ThreadTimes> {
    use std::ffi::c_void;
    #[repr(C)]
    #[derive(Default)]
    struct FileTime {low:u32,high:u32}
    impl FileTime {fn ticks(&self)->u64 {(u64::from(self.high)<<32)|u64::from(self.low)}}
    #[link(name="kernel32")]
    unsafe extern "system" {
        fn GetCurrentThread()->*mut c_void;
        fn GetThreadTimes(thread:*mut c_void,creation:*mut FileTime,exit:*mut FileTime,
            kernel:*mut FileTime,user:*mut FileTime)->i32;
    }
    let mut creation=FileTime::default();let mut exit=FileTime::default();
    let mut kernel=FileTime::default();let mut user=FileTime::default();
    // GetCurrentThread returns a non-owning pseudo-handle for this exact thread;
    // all four FILETIME outputs are live, correctly aligned writable objects.
    let okay=unsafe {GetThreadTimes(GetCurrentThread(),&mut creation,&mut exit,&mut kernel,&mut user)};
    (okay!=0).then(||ThreadTimes {kernel:kernel.ticks(),user:user.ticks()})
}

#[cfg(not(windows))]
fn current_thread_times()->Option<ThreadTimes> {None}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn combines_execution_modes_without_wall_time_or_counter_underflow() {
        let start=ThreadTimes {kernel:30_000,user:80_000};
        assert_eq!(start.elapsed_ms(ThreadTimes{kernel:35_000,user:100_000}),Some(2.5));
        assert_eq!(start.elapsed_ms(start),Some(0.0));
        assert_eq!(start.elapsed_ms(ThreadTimes{kernel:29_999,user:200_000}),None);
        assert_eq!(ThreadTimes{kernel:0,user:0}.elapsed_ms(ThreadTimes{kernel:u64::MAX,user:1}),None);
    }
    #[test]
    fn moving_timer_to_another_thread_cannot_measure_that_threads_work() {
        let timer=ThreadCpuTimer::start();
        assert_eq!(std::thread::spawn(move||timer.elapsed_ms()).join().unwrap(),None);
    }
    #[test]
    fn missing_counter_is_unavailable_and_real_counter_is_monotonic() {
        let missing=ThreadCpuTimer {thread:std::thread::current().id(),start:None};
        assert_eq!(missing.elapsed_ms(),None);
        let timer=ThreadCpuTimer::start();
        #[cfg(windows)] {
            let elapsed=timer.elapsed_ms().expect("current Windows thread timing is available");
            assert!(elapsed.is_finite()&&elapsed>=0.0);
        }
        #[cfg(not(windows))] assert_eq!(timer.elapsed_ms(),None);
    }
}
