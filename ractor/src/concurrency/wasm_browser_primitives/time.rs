//! This source file is copied from 'tokio_with_wasm' 0.8.6 and modified to properly mark futures as `Send`.
//! (https://github.com/cunarist/tokio-with-wasm/blob/b4171d75a899c02cea4afc3a3f523c3809959cae/package/src/glue/time/mod.rs)

//! Utilities for tracking time.
//!
//! This module provides a number of types for executing code after a set period
//! of time.

/// from tokio_with_wasm-0.8.6\src\glue\common\mod.rs
mod common {
    use js_sys::Function;
    use wasm_bindgen::{prelude::*, JsValue};

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = console, js_name = error)]
        pub fn error(s: &str);
        #[wasm_bindgen(js_namespace = Date, js_name = now)]
        pub fn now() -> f64;
        #[wasm_bindgen(js_namespace = globalThis, js_name = setTimeout)]
        pub fn set_timeout(callback: &Function, milliseconds: f64);
        #[wasm_bindgen(js_namespace = globalThis, js_name = setInterval)]
        pub fn set_interval(callback: &Function, milliseconds: f64) -> i32;
        #[wasm_bindgen(js_namespace = globalThis, js_name = clearInterval)]
        pub fn clear_interval(id: i32);
    }

    pub(super) trait LogError {
        fn log_error(&self, code: &str);
    }

    impl LogError for JsValue {
        fn log_error(&self, code: &str) {
            error(&format!("Error `{code}` in `tokio_with_wasm`:\n{self:?}"));
        }
    }

    impl<T> LogError for Result<T, JsValue> {
        fn log_error(&self, code: &str) {
            if let Err(js_value) = self {
                error(&format!(
                    "Error `{code}` in `tokio_with_wasm`:\n{js_value:?}"
                ));
            }
        }
    }
}

mod util {
    use std::future::Future;

    use tokio::sync::oneshot::error::RecvError;

    pub(super) fn wrap_future_as_send<F, T>(
        f: F,
    ) -> impl Future<Output = Result<T, RecvError>> + Send
    where
        F: Future<Output = T> + 'static,
        T: Send + 'static,
    {
        let (tx, rx) = crate::concurrency::oneshot();
        tokio_with_wasm::spawn_local(async move {
            let result = f.await;
            let _ = tx.send(result); // note: failures here are ignored, the most likely reason would be a dropped receiver
        });
        rx
    }
}

use util::*;

use common::*;

use js_sys::Promise;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use wasm_bindgen::prelude::{Closure, JsCast};
use wasm_bindgen_futures::JsFuture;

async fn time_future(duration: Duration) {
    let milliseconds = duration.as_millis() as f64;
    let promise = Promise::new(&mut |resolve, _reject| {
        set_timeout(&resolve, milliseconds);
    });
    JsFuture::from(promise).await.log_error("TIME_FUTURE");
}

/// Waits until `duration` has elapsed.
pub(super) fn sleep(duration: Duration) -> Sleep {
    let time_future = time_future(duration);
    let send_safe_future = wrap_future_as_send(time_future);
    Sleep {
        time_future: Box::pin(async move {
            let _ = send_safe_future.await;
        }),
    }
}

/// Future returned by `sleep`.
pub(super) struct Sleep {
    time_future: Pin<Box<dyn Future<Output = ()> + Send>>,
}

impl Future for Sleep {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.time_future.as_mut().poll(cx)
    }
}

/// Poll a future with a timeout.
/// If the future is ready, return the output.
/// If the future is pending, poll the sleep future.
pub(super) fn timeout<F>(duration: Duration, future: F) -> Timeout<F>
where
    F: Future,
{
    let time_future = time_future(duration);
    let send_safe_future = wrap_future_as_send(time_future);
    Timeout {
        future: Box::pin(future),
        time_future: Box::pin(async move {
            let _ = send_safe_future.await;
        }),
    }
}

/// Future returned by `timeout`.
pub(super) struct Timeout<F: Future> {
    future: Pin<Box<F>>,
    time_future: Pin<Box<dyn Future<Output = ()> + Send>>,
}

impl<F: Future> Future for Timeout<F> {
    type Output = Result<F::Output, Elapsed>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Poll the future first.
        // If it's ready, return the output.
        // If it's pending, poll the sleep future.
        match self.future.as_mut().poll(cx) {
            Poll::Ready(output) => Poll::Ready(Ok(output)),
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
pub struct Elapsed(());

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

/// Creates a new interval that ticks every `period` duration.
pub(super) fn interval(period: Duration) -> Interval {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let period_ms = period.as_millis() as f64;
    // Create a closure that sends a tick via the channel.
    let closure = Closure::wrap(Box::new(move || {
        let _ = tx.send(());
    }) as Box<dyn Fn()>);
    // Register an interval with the closure.
    let interval_id = set_interval(closure.as_ref().unchecked_ref(), period_ms);
    // Release memory management of this closure from Rust to the JS GC.
    closure.forget();
    Interval {
        period,
        rx,
        interval_id,
    }
}

/// A structure that represents an interval that ticks at a specified period.
/// It provides methods to wait for the next tick, reset the interval,
/// and ensure the interval is cleaned up when it is dropped.
#[derive(Debug)]
pub struct Interval {
    period: Duration,
    rx: tokio::sync::mpsc::UnboundedReceiver<()>,
    interval_id: i32,
}

impl Interval {
    /// Waits until the next tick.
    pub async fn tick(&mut self) {
        self.rx.recv().await;
    }

    /// Resets the interval, making the next tick occur
    /// after the original period.
    /// This clears the existing interval and establishes a new one.
    pub fn reset(&mut self) {
        // Clear the existing interval.
        clear_interval(self.interval_id);

        // Create a new channel to receive ticks.
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        self.rx = rx;
        let period_ms = self.period.as_millis() as f64;
        // Set up a new interval.
        let closure = Closure::wrap(Box::new(move || {
            let _ = tx.send(());
        }) as Box<dyn Fn()>);
        self.interval_id = set_interval(closure.as_ref().unchecked_ref(), period_ms);
        // Release memory management of this closure from Rust to the JS GC.
        closure.forget();
    }
}

impl Drop for Interval {
    fn drop(&mut self) {
        clear_interval(self.interval_id);
    }
}
