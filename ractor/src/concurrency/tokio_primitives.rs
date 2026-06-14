// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Concurrency primitives based on `tokio`

use std::future::Future;

/// Tokio-based concurrency backend implementation
#[derive(Debug, Clone, Copy)]
pub struct TokioBackend;

impl super::ConcurrencyBackend for TokioBackend {
    type JoinHandle<T> = tokio::task::JoinHandle<T>;
    type Duration = tokio::time::Duration;
    type Instant = tokio::time::Instant;
    type Interval = tokio::time::Interval;
    type JoinSet<T> = tokio::task::JoinSet<T>;

    fn sleep(dur: Self::Duration) -> impl Future<Output = ()> + Send {
        tokio::time::sleep(dur)
    }

    fn interval(dur: Self::Duration) -> Self::Interval {
        tokio::time::interval(dur)
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
        tokio::task::spawn_local(future)
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
        tokio::time::timeout(dur, future)
            .await
            .map_err(|_| super::Timeout)
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
pub use tokio::test;
