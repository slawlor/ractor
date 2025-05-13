// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Concurrency primitives based on the `async-std` crate
//!
//! We still rely on tokio for some core executor-independent parts
//! such as channels (see: https://github.com/tokio-rs/tokio/issues/4232#issuecomment-968329443).

use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;

use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
use futures::FutureExt;
use futures::StreamExt;

/// Represents a [JoinHandle] on a spawned task.
/// Adds some syntactic wrapping to support a JoinHandle
/// similar to `tokio`'s.
pub struct JoinHandle<T> {
    handle: Option<async_std::task::JoinHandle<T>>,
    is_done: Arc<AtomicBool>,
}

impl<T> Debug for JoinHandle<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JoinHandle")
            .field("name", &self.is_done.load(Ordering::Relaxed))
            .field("handle", &self.handle.is_some())
            .finish()
    }
}

impl<T> JoinHandle<T> {
    /// Determine if the handle is currently finished
    pub fn is_finished(&self) -> bool {
        self.handle.is_none() || self.is_done.load(Ordering::Relaxed)
    }

    /// Abort the handle
    pub fn abort(&mut self) {
        if let Some(handle) = self.handle.take() {
            let f = handle.cancel();
            drop(f);
        }
    }
}

impl<T> async_std::future::Future for JoinHandle<T> {
    type Output = Result<T, ()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // a little black-magic to poll the inner future, but return a Result instead of a unit
        let mutself = self.get_mut();
        let inner_polled_value = if let Some(inner) = mutself.handle.as_mut() {
            inner.poll_unpin(cx)
        } else {
            return Poll::Ready(Err(()));
        };

        match inner_polled_value {
            Poll::Pending => Poll::Pending,
            Poll::Ready(v) => {
                mutself.handle = None;
                Poll::Ready(Ok(v))
            }
        }
    }
}

/// A duration of time
pub type Duration = std::time::Duration;

/// A system-agnostic point-in-time
pub type Instant = std::time::Instant;

/// An asynchronous interval calculation which waits until
/// a checkpoint time to tick. This is a replication of the
/// basic functionality from `tokio`'s `Interval`.
#[derive(Debug, Clone)]
pub struct Interval {
    dur: Duration,
    next_tick: Instant,
}

impl Interval {
    /// Wait until the next tick time has elapsed, regardless of computation time
    pub async fn tick(&mut self) {
        let now = Instant::now();
        // if the next tick time is in the future, wait until it's time
        if self.next_tick > now {
            sleep(self.next_tick - now).await;
        }
        // set the next tick time
        self.next_tick += self.dur;
    }
}

/// Build a new interval at the given duration starting at now
///
/// Ticks 1 time immediately
pub fn interval(dur: Duration) -> Interval {
    Interval {
        dur,
        next_tick: Instant::now(),
    }
}

/// A set of futures to join on, in an unordered fashion
/// (first-completed, first-served). This is a wrapper
/// to match the signature of `tokio`'s `JoinSet`
#[derive(Default)]
pub struct JoinSet<T> {
    set: FuturesUnordered<BoxFuture<'static, T>>,
}

impl<T> Debug for JoinSet<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JoinSet")
            .field("size", &self.set.len())
            .finish()
    }
}

impl<T> JoinSet<T> {
    /// Creates a new [JoinSet]
    pub fn new() -> JoinSet<T> {
        Self {
            set: FuturesUnordered::new(),
        }
    }

    /// Spawn a new future into the join set
    pub fn spawn<F: Future<Output = T> + Send + 'static>(&mut self, f: F) {
        self.set.push(f.boxed());
    }

    /// Join the next future
    pub async fn join_next(&mut self) -> Option<Result<T, ()>> {
        self.set.next().await.map(|item| Ok(item))
    }

    /// Get the number of futures in the [JoinSet]
    pub fn len(&self) -> usize {
        self.set.len()
    }

    /// Determine if the [JoinSet] has any futures in it
    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }
}

/// Sleep the task for a duration of time
pub async fn sleep(dur: super::Duration) {
    async_std::task::sleep(dur).await;
}

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
    if let Some(name) = name {
        let signal = Arc::new(AtomicBool::new(false));
        let inner_signal = signal.clone();

        let jh = async_std::task::Builder::new()
            .name(name.to_string())
            .spawn(async move {
                let r = future.await;
                inner_signal.fetch_or(true, Ordering::Relaxed);
                r
            })
            .unwrap();

        JoinHandle {
            handle: Some(jh),
            is_done: signal,
        }
    } else {
        let signal = Arc::new(AtomicBool::new(false));
        let inner_signal = signal.clone();

        let jh = async_std::task::spawn(async move {
            let r = future.await;
            inner_signal.fetch_or(true, Ordering::Relaxed);
            r
        });

        JoinHandle {
            handle: Some(jh),
            is_done: signal,
        }
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
    async_std::future::timeout(dur, future)
        .await
        .map_err(|_| super::Timeout)
}

/// test macro
pub use async_std::test;
pub use futures::select_biased as select;

#[cfg(test)]
mod async_std_primitive_tests {

    use super::*;
    use crate::common_test::periodic_check;

    #[super::test]
    async fn join_handle_aborts() {
        let mut jh = spawn(async {
            sleep(Duration::from_millis(1000)).await;
        });
        jh.abort();
        assert!(jh.is_finished());
    }

    #[super::test]
    async fn join_handle_finishes() {
        let jh = spawn(async {
            sleep(Duration::from_millis(5)).await;
            println!("done.");
        });

        periodic_check(|| jh.is_finished(), Duration::from_millis(1000)).await;
    }

    #[super::test]
    async fn test_spawn_named() {
        let jh = spawn_named(Some("something"), async {
            sleep(Duration::from_millis(5)).await;
            println!("done.");
        });
        periodic_check(|| jh.is_finished(), Duration::from_millis(1000)).await;
    }
}
