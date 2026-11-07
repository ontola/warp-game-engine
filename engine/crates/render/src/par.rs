//! Scoped threads that also work in a browser. Natively this is
//! `std::thread::scope`; wasm32 has no threads, so there every `spawn` runs
//! its work at once on the calling thread and `join` hands back the result.

#[cfg(not(target_arch = "wasm32"))]
pub use std::thread::scope;

#[cfg(target_arch = "wasm32")]
pub use inline::scope;

#[cfg(target_arch = "wasm32")]
mod inline {
    use std::marker::PhantomData;

    pub struct Scope<'scope, 'env: 'scope> {
        _scope: PhantomData<&'scope mut &'scope ()>,
        _env: PhantomData<&'env mut &'env ()>,
    }

    pub struct ScopedJoinHandle<'scope, T> {
        value: T,
        _scope: PhantomData<&'scope ()>,
    }

    impl<'scope, 'env> Scope<'scope, 'env> {
        pub fn spawn<F, T>(&'scope self, work: F) -> ScopedJoinHandle<'scope, T>
        where F: FnOnce() -> T + Send + 'scope, T: Send + 'scope {
            ScopedJoinHandle { value: work(), _scope: PhantomData }
        }
    }

    impl<T> ScopedJoinHandle<'_, T> {
        pub fn join(self) -> std::thread::Result<T> { Ok(self.value) }
    }

    pub fn scope<'env, F, T>(work: F) -> T
    where F: for<'scope> FnOnce(&'scope Scope<'scope, 'env>) -> T {
        work(&Scope { _scope: PhantomData, _env: PhantomData })
    }
}

/// Workers for cooking work: the machine's threads, or KERN_COOK_THREADS.
pub fn workers() -> usize {
    std::env::var("KERN_COOK_THREADS").ok().and_then(|v| v.parse().ok()).filter(|&n: &usize| n > 0)
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()))
}

/// Extra threads `map` has running, across every caller: nested or
/// concurrent cooks share `workers()` instead of multiplying it.
static EXTRA: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// `f` over every index below `count`, on the calling thread plus as many
/// spare workers as are free; results in index order, so the outcome never
/// depends on the thread count.
pub fn map<R: Send>(count: usize, f: impl Fn(usize) -> R + Sync) -> Vec<R> {
    use std::sync::atomic::Ordering::{AcqRel, Relaxed};
    let want = workers().min(count).saturating_sub(1);
    let extra = if cfg!(target_arch = "wasm32") || want == 0 { 0 } else {
        let mut got = 0;
        let _ = EXTRA.fetch_update(AcqRel, Relaxed, |busy| {
            got = want.min(workers().saturating_sub(1).saturating_sub(busy));
            Some(busy + got)
        });
        got
    };
    if extra == 0 { return (0..count).map(f).collect(); }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let work = || {
        let mut mine = Vec::new();
        loop {
            let i = next.fetch_add(1, Relaxed);
            if i >= count { break; }
            mine.push((i, f(i)));
        }
        mine
    };
    let mut out: Vec<(usize, R)> = scope(|s| {
        let handles: Vec<_> = (0..extra).map(|_| s.spawn(&work)).collect();
        let mut all = work();
        for h in handles { all.extend(h.join().unwrap_or_else(|e| std::panic::resume_unwind(e))); }
        all
    });
    EXTRA.fetch_sub(extra, AcqRel);
    out.sort_by_key(|(i, _)| *i);
    out.into_iter().map(|(_, r)| r).collect()
}
