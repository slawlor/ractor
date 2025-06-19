// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Concurrency primitives based on `tokio`

use std::{
    error::Error,
    fmt::{Display, Formatter},
    future::Future,
    io,
    task::Poll,
};

use ::tokio::sync::oneshot::error::RecvError;
use tokio_with_wasm::alias as tokio;

/// Represents a task JoinHandle
pub type JoinHandle<T> = tokio::task::JoinHandle<T>;

/// A duration of time
pub type Duration = std::time::Duration;

/// An instant measured on system time
pub type Instant = web_time::Instant;

/// Sleep the task for a duration of time
pub fn sleep(dur: Duration) -> impl Future<Output = ()> + Send {
    async move {
        let _ = wrap_future_as_send(tokio::time::sleep(dur)).await;
    }
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
    F: Future + 'static,
    F::Output: 'static,
{
    spawn_named(None, future)
}

pub fn spawn_local<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + 'static,
{
    tokio_with_wasm::spawn_local(future)
}

/// Spawn a (possibly) named task on the executor runtime
pub fn spawn_named<F>(name: Option<&str>, future: F) -> JoinHandle<F::Output>
where
    F: Future + 'static,
    F::Output: 'static,
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

// ------------------------------------------------------------------
// partial reimplementation of tokio_with_wasm.
// The only change is adding 'wrap_future_as_send' to 'timeout' to make it Send

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = globalThis, js_name = setTimeout)]
    pub fn set_timeout(callback: &Function, milliseconds: f64);
}

// note: this future is NOT Send
async fn time_future(duration: Duration) {
    let milliseconds = duration.as_millis() as f64;
    let promise = Promise::new(&mut |resolve, _reject| unsafe {
        set_timeout(&resolve, milliseconds);
    });
    let result = wasm_bindgen_futures::JsFuture::from(promise).await;
    if let Err(err) = result {
        // err.log_error("TIME_FUTURE");
    }
}

fn wrap_future_as_send<F, T>(f: F) -> impl Future<Output = Result<T, RecvError>> + Send
where
    F: Future<Output = T> + 'static,
    T: Send + 'static,
{
    let (tx, rx) = crate::concurrency::oneshot();
    spawn_local(async move {
        let result = f.await;
        let _ = tx.send(result); // note: failures here are ignored, the most likely reason would be a dropped receiver
    });
    async { rx.await }
}

/// Poll a future with a timeout.
/// If the future is ready, return the output.
/// If the future is pending, poll the sleep future.
fn _timeout<F>(
    duration: Duration,
    future: F,
) -> impl Future<Output = Result<F::Output, Elapsed>> + Send
where
    F: Future + Send,
{
    let time_future = async move {
        let _ = wrap_future_as_send(time_future(duration)).await;
    };
    Timeout {
        future: Box::pin(future),
        time_future: Box::pin(time_future),
    }
}

/// Future returned by `timeout`.
struct Timeout<F: Future> {
    future: std::pin::Pin<Box<F>>,
    time_future: std::pin::Pin<Box<dyn Future<Output = ()>>>,
}

unsafe impl<F> Send for Timeout<F> where F: Future + Send {}

impl<F: Future> Future for Timeout<F> {
    type Output = Result<F::Output, Elapsed>;
    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        // Poll the future first.
        // If it's ready, return the output.
        // If it's pending, poll the sleep future.
        match self.future.as_mut().poll(cx) {
            std::task::Poll::Ready(output) => Poll::Ready(Ok(output)),
            Poll::Pending => match self.time_future.as_mut().poll(cx) {
                Poll::Ready(()) => Poll::Ready(Err(Elapsed(()))),
                Poll::Pending => Poll::Pending,
            },
        }
    }
}

/// Errors returned by `Timeout`.
///
/// This error is returned when a timeout expires before the function was able
/// to finish.
#[derive(Debug, PartialEq, Eq)]
struct Elapsed(());

impl Display for Elapsed {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> std::fmt::Result {
        "deadline has elapsed".fmt(fmt)
    }
}

impl Error for Elapsed {}

impl From<Elapsed> for io::Error {
    fn from(_err: Elapsed) -> io::Error {
        io::ErrorKind::TimedOut.into()
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
    F: Future<Output = T> + Send,
    T: 'static,
{
    // tokio::time::timeout(duration, future)
    _timeout(dur, future).await.map_err(|_| super::Timeout)
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
use wasm_bindgen_futures::{
    js_sys::{Function, Promise},
    wasm_bindgen::{self, prelude::wasm_bindgen},
};
// test macro
#[cfg(test)]
pub use wasm_bindgen_test::wasm_bindgen_test as test;
