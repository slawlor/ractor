// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Shared concurrency primitives utilized within the library for different frameworks (tokio, async-std, etc)

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

#[cfg(not(feature = "async-std"))]
pub mod tokio_primitives;
#[cfg(not(feature = "async-std"))]
pub use self::tokio_primitives::*;

#[cfg(feature = "async-std")]
pub mod async_std_primitives;
#[cfg(feature = "async-std")]
pub use self::async_std_primitives::*;
