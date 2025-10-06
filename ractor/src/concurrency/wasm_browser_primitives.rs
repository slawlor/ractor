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
