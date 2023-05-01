// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Shared concurrency primitives utilized within the library for different frameworks (tokio, async-std, etc)

/// A timoeout error
#[derive(Debug)]
pub struct Timeout;

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

// =============== TOKIO =============== //

/// Tokio-based primitives
// #[cfg(feature = "tokio_runtime")]
pub mod tokio_primatives {
    use std::future::Future;

    /// Represents a task JoinHandle
    pub type JoinHandle<T> = tokio::task::JoinHandle<T>;

    /// A duration of time
    pub type Duration = tokio::time::Duration;

    /// An instant measured on system time
    pub type Instant = tokio::time::Instant;

    /// Sleep the task for a duration of time
    pub async fn sleep(dur: super::Duration) {
        tokio::time::sleep(dur).await;
    }

    /// Spawn a task on the executor runtime
    pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        tokio::task::spawn(future)
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
}
// #[cfg(feature = "tokio_runtime")]
pub use tokio_primatives::*;

// =============== ASYNC-STD =============== //

/// Tokio-based primitives
#[cfg(feature = "async_std_runtime")]
pub mod async_std_primatives {
    use std::future::Future;

    /// Represents a task JoinHandle
    pub type JoinHandle<T> = async_std::task::JoinHandle<T>;

    /// A duration of time
    pub type Duration = std::time::Duration;

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
        async_std::task::spawn(future)
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

    pub use futures::select_biased as select;
    // test macro
    #[cfg(test)]
    pub(crate) use tokio::test;
}
#[cfg(feature = "async_std_runtime")]
pub use async_std_primatives::*;
