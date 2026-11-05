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
