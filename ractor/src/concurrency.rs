// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Shared concurrency primitives utilized within the library for different frameworks (tokio, async-std, etc)

use std::future::Future;

/// Trait defining the interface for concurrency backends
///
/// This trait provides a common interface for different async runtime backends
/// (tokio, async-std, wasm-browser). Each backend implements this trait with
/// its own runtime-specific types and behavior.
pub trait ConcurrencyBackend {
    /// Task join handle type
    type JoinHandle<T>;

    /// Duration type
    type Duration: Clone;

    /// Instant/time point type
    type Instant: Clone;

    /// Interval type
    type Interval;

    /// JoinSet type for managing multiple futures
    type JoinSet<T>;

    /// Sleep for a duration
    fn sleep(dur: Self::Duration) -> impl Future<Output = ()> + Send;

    /// Create an interval that ticks at a regular duration
    fn interval(dur: Self::Duration) -> Self::Interval;

    /// Spawn a task on the executor runtime
    fn spawn<F>(future: F) -> Self::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static;

    /// Spawn a task on the executor runtime which will not be moved to other threads
    fn spawn_local<F>(future: F) -> Self::JoinHandle<F::Output>
    where
        F: Future + 'static;

    /// Spawn a (possibly) named task on the executor runtime
    fn spawn_named<F>(name: Option<&str>, future: F) -> Self::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static;

    /// Execute a future with a timeout
    fn timeout<F, T>(dur: Self::Duration, future: F) -> impl Future<Output = Result<T, Timeout>>
    where
        F: Future<Output = T>;
}

/// A timeout error
#[derive(Debug)]
pub struct Timeout;
impl std::error::Error for Timeout {}
impl std::fmt::Display for Timeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Timeout")
    }
}

/// A notification
pub type Notify = tokio::sync::Notify;

/// A one-use sender
pub type OneshotSender<T> = tokio::sync::oneshot::Sender<T>;
/// A one-use receiver
pub type OneshotReceiver<T> = tokio::sync::oneshot::Receiver<T>;

/// A bounded MP;SC sender
pub type MpscSender<T> = tokio::sync::mpsc::Sender<T>;
/// A bounded MP;SC receiver
pub type MpscReceiver<T> = tokio::sync::mpsc::Receiver<T>;

/// A bounded MP;SC sender
pub type MpscUnboundedSender<T> = tokio::sync::mpsc::UnboundedSender<T>;
/// A bounded MP;SC receiver
pub type MpscUnboundedReceiver<T> = tokio::sync::mpsc::UnboundedReceiver<T>;

/// A bounded broadcast sender
pub type BroadcastSender<T> = tokio::sync::broadcast::Sender<T>;
/// A bounded broadcast receiver
pub type BroadcastReceiver<T> = tokio::sync::broadcast::Receiver<T>;

/// MPSC bounded channel
pub fn mpsc_bounded<T>(buffer: usize) -> (MpscSender<T>, MpscReceiver<T>) {
    tokio::sync::mpsc::channel(buffer)
}

/// MPSC unbounded channel
pub fn mpsc_unbounded<T>() -> (MpscUnboundedSender<T>, MpscUnboundedReceiver<T>) {
    tokio::sync::mpsc::unbounded_channel()
}

/// Oneshot channel
pub fn oneshot<T>() -> (OneshotSender<T>, OneshotReceiver<T>) {
    tokio::sync::oneshot::channel()
}

/// Broadcast channel
pub fn broadcast<T: Clone>(buffer: usize) -> (BroadcastSender<T>, BroadcastReceiver<T>) {
    tokio::sync::broadcast::channel(buffer)
}

// Backend modules
#[cfg(all(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    not(feature = "async-std")
))]
pub mod tokio_primitives;

#[cfg(all(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    feature = "async-std"
))]
pub mod async_std_primitives;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod wasm_browser_primitives;

// Select the current backend via type alias
#[cfg(all(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    not(feature = "async-std")
))]
type CurrentBackend = tokio_primitives::TokioBackend;

#[cfg(all(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    feature = "async-std"
))]
type CurrentBackend = async_std_primitives::AsyncStdBackend;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
type CurrentBackend = wasm_browser_primitives::WasmBrowserBackend;

// Re-export backend-specific macros (internal use only)
#[cfg(all(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    not(feature = "async-std")
))]
pub(crate) use self::tokio_primitives::select;

#[cfg(all(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    feature = "async-std"
))]
pub(crate) use self::async_std_primitives::select;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) use self::wasm_browser_primitives::select;

// Re-export test macro
#[cfg(all(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    not(feature = "async-std")
))]
pub use self::tokio_primitives::test;

#[cfg(all(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    feature = "async-std"
))]
pub use self::async_std_primitives::test;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use self::wasm_browser_primitives::test;

// Public type aliases using CurrentBackend
/// Represents a task JoinHandle
pub type JoinHandle<T> = <CurrentBackend as ConcurrencyBackend>::JoinHandle<T>;

/// A duration of time
pub type Duration = <CurrentBackend as ConcurrencyBackend>::Duration;

/// An instant measured on system time
pub type Instant = <CurrentBackend as ConcurrencyBackend>::Instant;

/// An asynchronous interval calculation which waits until
/// a checkpoint time to tick
pub type Interval = <CurrentBackend as ConcurrencyBackend>::Interval;

/// A set of futures to join on, in an unordered fashion
/// (first-completed, first-served)
pub type JoinSet<T> = <CurrentBackend as ConcurrencyBackend>::JoinSet<T>;

// Public free functions using CurrentBackend
/// Sleep the task for a duration of time
pub async fn sleep(dur: Duration) {
    <CurrentBackend as ConcurrencyBackend>::sleep(dur).await;
}

/// Build a new interval at the given duration starting at now
///
/// Ticks 1 time immediately
pub fn interval(dur: Duration) -> Interval {
    <CurrentBackend as ConcurrencyBackend>::interval(dur)
}

/// Spawn a task on the executor runtime
pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    <CurrentBackend as ConcurrencyBackend>::spawn(future)
}

/// Spawn a task on the executor runtime which will not be moved to other threads
pub fn spawn_local<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + 'static,
{
    <CurrentBackend as ConcurrencyBackend>::spawn_local(future)
}

/// Spawn a (possibly) named task on the executor runtime
pub fn spawn_named<F>(name: Option<&str>, future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    <CurrentBackend as ConcurrencyBackend>::spawn_named(name, future)
}

/// Execute the future up to a timeout
///
/// * `dur`: The duration of time to allow the future to execute for
/// * `future`: The future to execute
///
/// Returns [Ok(_)] if the future succeeded before the timeout, [Err(Timeout)] otherwise
pub async fn timeout<F, T>(dur: Duration, future: F) -> Result<T, Timeout>
where
    F: Future<Output = T>,
{
    <CurrentBackend as ConcurrencyBackend>::timeout(dur, future).await
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod target_specific {
    pub(crate) use web_time::SystemTime;
}
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
mod target_specific {
    pub(crate) use std::time::SystemTime;
}

pub(crate) use target_specific::SystemTime;
