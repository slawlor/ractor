// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Concurrency primitaves based on `tokio`

use std::future::Future;

/// Represents a task JoinHandle
pub type JoinHandle<T> = tokio::task::JoinHandle<T>;

/// A duration of time
pub type Duration = tokio::time::Duration;

/// An instant measured on system time
pub type Instant = tokio::time::Instant;

/// Sleep the task for a duration of time
pub async fn sleep(dur: Duration) {
    tokio::time::sleep(dur).await;
}

/// An asynchronous interval calculation which waits until
/// a checkpoint time to tick
pub type Interval = tokio::time::Interval;

/// Build a new interval at the given duration starting at now
///
/// Ticks 1 time immediately
pub fn interval(dur: Duration) -> Interval {
    tokio::time::interval(dur)
}

/// A set of futures to join on, in an unordered fashion
/// (first-completed, first-served)
pub type JoinSet<T> = tokio::task::JoinSet<T>;

/// Spawn a task on the executor runtime
pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    spawn_named(None, future)
}

/// Spawn a (possibly) named task on the executor runtime
pub fn spawn_named<F>(name: Option<&str>, future: F) -> JoinHandle<F::Output>
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
    tokio::time::timeout(dur, future)
        .await
        .map_err(|_| super::Timeout)
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
