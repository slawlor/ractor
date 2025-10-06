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

#[cfg(all(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    not(feature = "async-std")
))]
pub mod tokio_primitives;
#[cfg(all(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    not(feature = "async-std")
))]
pub use self::tokio_primitives::*;

#[cfg(all(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    feature = "async-std"
))]
pub mod async_std_primitives;
#[cfg(all(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    feature = "async-std"
))]
pub use self::async_std_primitives::*;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod wasm_browser_primitives;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use self::wasm_browser_primitives::*;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod target_specific {
    pub(crate) use web_time::SystemTime;
}
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
mod target_specific {
    pub(crate) use std::time::SystemTime;
}

pub(crate) use target_specific::SystemTime;
