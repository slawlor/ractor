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
pub mod tokio_with_wasm_primitives;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use self::tokio_with_wasm_primitives::*;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod target_specific {
    /// A wrapper for [std::marker::Send] on non-`wasm32-unknown-unknown` targets, or an empty trait on `wasm32-unknown-unknown` targets.
    /// Introduced for compatibility between wasm32 and other targets
    pub trait MaybeSend {}
    impl<T> MaybeSend for T {}
    pub(crate) use web_time::SystemTime;
}
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
mod target_specific {
    /// A wrapper for [std::marker::Send] on non-`wasm32-unknown-unknown` targets, or an empty trait on `wasm32-unknown-unknown` targets.
    /// Introduced for compatibility between wasm32 and other targets
    pub trait MaybeSend: Send {}
    impl<T> MaybeSend for T where T: Send {}
    pub(crate) use std::time::SystemTime;
}

pub use target_specific::MaybeSend;
pub(crate) use target_specific::SystemTime;
