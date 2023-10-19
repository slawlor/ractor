// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Concurrency primitaves based on a WASM runtime. They however are not completely
//! functional and there are other core problems with Ractor in a WASM runtime, specifically
//! that panic's abort instantly. https://github.com/rust-lang/rust/issues/58874
//! 
//! ## Testing
//! 
//! Test this configuration with
//! 
//! ```text
//! wasm-pack test --headless --safari -r
//! ```

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::FutureExt;

/// Represents a task JoinHandle
pub struct JoinHandle<T> {
    handle: Option<futures::future::RemoteHandle<T>>,
}

impl<T> Drop for JoinHandle<T> {
    fn drop(&mut self) {
        if let Some(h) = self.handle.take() {
            h.forget();
        }
    }
}

impl<T> JoinHandle<T> {
    /// Determine if the handle is currently finished
    pub fn is_finished(&self) -> bool {
        self.handle.is_none()
    }

    /// Abort the handle
    pub fn abort(&mut self) {
        // For a remote handle, being dropped will wake the remote future
        // to be dropped by the executor
        // See: https://docs.rs/futures/latest/futures/prelude/future/struct.RemoteHandle.html
        drop(self.handle.take());
    }
}

impl<T: 'static> Future for JoinHandle<T> {
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
                mutself.abort();
                Poll::Ready(Ok(v))
            }
        }
    }
}

/// Spawn a task on the executor runtime
pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + 'static,
    F::Output: Send + 'static,
{
    spawn_named(None, future)
}

/// Spawn a (possibly) named task on the executor runtime
pub fn spawn_named<F>(name: Option<&str>, future: F) -> JoinHandle<F::Output>
where
    F: Future + 'static,
    F::Output: Send + 'static,
{
    let _ = name;
    let (remote, remote_handle) = future.remote_handle();
    wasm_bindgen_futures::spawn_local(remote);
    JoinHandle {
        handle: Some(remote_handle),
    }
}

/// A duration of time
pub type Duration = std::time::Duration;

/// An instant measured on system time
pub type Instant = wasm_timer::Instant;

/// Sleep the task for a duration of time
pub async fn sleep(dur: Duration) {
    wasmtimer::tokio::sleep(dur).await;
    // let _ = wasm_timer::Delay::new(dur).await;
}

/// An asynchronous interval calculation which waits until
/// a checkpoint time to tick
pub type Interval = wasmtimer::tokio::Interval;

/// Build a new interval at the given duration starting at now
///
/// Ticks 1 time immediately
pub fn interval(dur: Duration) -> Interval {
    wasmtimer::tokio::interval(dur)
}

/// A set of futures to join on, in an unordered fashion
/// (first-completed, first-served)
pub type JoinSet<T> = tokio::task::JoinSet<T>;

/// Execute the future up to a timeout
///
/// * `dur`: The duration of time to allow the future to execute for
/// * `future`: The future to execute
///
/// Returns [Ok(_)] if the future succeeded before the timeout, [Err(Timeout)] otherwise
pub async fn timeout<F, T>(dur: Duration, future: F) -> Result<T, super::Timeout>
where
    F: Future<Output = T>,
{
    wasmtimer::tokio::timeout(dur, future)
        .await
        .map_err(|_| super::Timeout)
}

pub use futures::select_biased as select;

// test macro
pub use wasm_bindgen_test::wasm_bindgen_test as test;
