// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Concurrency primitives for the browser based on `tokio-with-wasm` and `wasm-bindgen`.
//! This implementation only works in a browser environment and is not suitable for server-side wasm use.

mod time;

use std::future::Future;

use tokio_with_wasm::alias as tokio;

/// Wasm browser-based concurrency backend implementation
#[derive(Debug, Clone, Copy)]
pub struct WasmBrowserBackend;

impl super::ConcurrencyBackend for WasmBrowserBackend {
    type JoinHandle<T> = tokio::task::JoinHandle<T>;
    type Duration = std::time::Duration;
    type Instant = web_time::Instant;
    type Interval = time::Interval;
    type JoinSet<T> = tokio::task::JoinSet<T>;

    fn sleep(dur: Self::Duration) -> impl Future<Output = ()> + Send {
        time::sleep(dur)
    }

    fn interval(dur: Self::Duration) -> Self::Interval {
        time::interval(dur)
    }

    fn spawn<F>(future: F) -> Self::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        Self::spawn_named(None, future)
    }

    fn spawn_local<F>(future: F) -> Self::JoinHandle<F::Output>
    where
        F: Future + 'static,
    {
        // note: 'tokio_with_wasm::spawn_local' has an incorrect signature and only works with Output=().
        tokio_with_wasm::spawn(future)
    }

    fn spawn_named<F>(name: Option<&str>, future: F) -> Self::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        #[cfg(tokio_unstable)]
        {
            let mut builder = tokio::task::Builder::new();
            if let Some(name) = name {
                builder = builder.name(name);
            }
            builder.spawn(future).expect("Tokio task spawn failed")
        }

        #[cfg(not(tokio_unstable))]
        {
            let _ = name;
            tokio::task::spawn(future)
        }
    }

    async fn timeout<F, T>(dur: Self::Duration, future: F) -> Result<T, super::Timeout>
    where
        F: Future<Output = T>,
    {
        time::timeout(dur, future).await.map_err(|_| super::Timeout)
    }
}

/// Represents a task JoinHandle
pub type JoinHandle<T> = <WasmBrowserBackend as super::ConcurrencyBackend>::JoinHandle<T>;

/// A duration of time
pub type Duration = <WasmBrowserBackend as super::ConcurrencyBackend>::Duration;

/// An instant measured on system time
pub type Instant = <WasmBrowserBackend as super::ConcurrencyBackend>::Instant;

/// Sleep the task for a duration of time
pub fn sleep(dur: Duration) -> impl Future<Output = ()> + Send {
    <WasmBrowserBackend as super::ConcurrencyBackend>::sleep(dur)
}

/// An asynchronous interval calculation which waits until
/// a checkpoint time to tick
pub type Interval = <WasmBrowserBackend as super::ConcurrencyBackend>::Interval;

/// Build a new interval at the given duration starting at now
///
/// Ticks 1 time immediately
pub fn interval(dur: Duration) -> Interval {
    <WasmBrowserBackend as super::ConcurrencyBackend>::interval(dur)
}

/// A set of futures to join on, in an unordered fashion
/// (first-completed, first-served)
pub type JoinSet<T> = <WasmBrowserBackend as super::ConcurrencyBackend>::JoinSet<T>;

/// Spawn a task on the executor runtime
pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    <WasmBrowserBackend as super::ConcurrencyBackend>::spawn(future)
}

/// Spawn a task on the executor runtime which will not be moved to other threads
pub fn spawn_local<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + 'static,
{
    <WasmBrowserBackend as super::ConcurrencyBackend>::spawn_local(future)
}

/// Spawn a (possibly) named task on the executor runtime
pub fn spawn_named<F>(name: Option<&str>, future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    <WasmBrowserBackend as super::ConcurrencyBackend>::spawn_named(name, future)
}

/// Execute the future up to a timeout
///
/// * `dur`: The duration of time to allow the future to execute for
/// * `future`: The future to execute
///
/// Returns [Ok(_)] if the future succeeded before the timeout, [Err(super::Timeout)] otherwise
pub async fn timeout<F, T>(dur: super::Duration, future: F) -> Result<T, super::Timeout>
where
    F: Future<Output = T>,
{
    <WasmBrowserBackend as super::ConcurrencyBackend>::timeout(dur, future).await
}

macro_rules! select {
        ($($tokens:tt)*) => {{
            tokio::select! {
                // Biased ensures that we poll the ports in the order they appear, giving
                // priority to our message reception operations. See:
                // https://docs.rs/tokio/latest/tokio/macro.select.html#fairness
                // for more information
                biased;

                $( $tokens )*
            }
        }}
    }

pub(crate) use select;
// test macro
#[cfg(test)]
pub use wasm_bindgen_test::wasm_bindgen_test as test;
