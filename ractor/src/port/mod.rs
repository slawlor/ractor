// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Port implementations for signaling and reception of messages in the Ractor environment
//!
//! Most of the ports we utilize are direct aliases of [tokio]'s channels
//! (in the `sync` feature of the crate), however there are some helpful wrappers
//! and utilities to make working with mailbox processing in `ractor` easier in
//! the actor framework.

use tokio::sync::mpsc;
use tokio::sync::oneshot;

use crate::MessagingErr;

// ============ Input Ports ============ //

/// A bounded-depth message port (alias of [mpsc::Sender])
pub(crate) type BoundedInputPort<TMsg> = mpsc::Sender<TMsg>;
/// A bounded-depth message port's receiver (alias of [mpsc::Receiver])
pub(crate) type BoundedInputPortReceiver<TMsg> = mpsc::Receiver<TMsg>;

/// An unbounded message port (alias of [mpsc::UnboundedSender])
pub type InputPort<TMsg> = mpsc::UnboundedSender<TMsg>;
/// An unbounded message port's receiver (alias of [mpsc::UnboundedReceiver])
pub(crate) type InputPortReceiver<TMsg> = mpsc::UnboundedReceiver<TMsg>;

// ============ Output Ports ============ //
pub mod output;
pub use output::*;

// ============ Rpc (one-use) Ports ============ //

/// A remote procedure call's reply port. Wrapper of [tokio::sync::oneshot::Sender] with a
/// consistent error type
pub struct RpcReplyPort<TMsg> {
    port: oneshot::Sender<TMsg>,
}

impl<TMsg> RpcReplyPort<TMsg> {
    /// Send a message to the Rpc reply port. This consumes the port
    ///
    /// * `msg` - The message to send
    ///
    /// Returns [Ok(())] if the message send was successful, [Err(MessagingErr)] otherwise
    pub fn send(self, msg: TMsg) -> Result<(), MessagingErr> {
        self.port.send(msg).map_err(|_| MessagingErr::ChannelClosed)
    }

    /// Determine if the port is closed (i.e. the receiver has been dropped)
    ///
    /// Returns [true] if the receiver has been dropped and the channel is
    /// closed, this means sends will fail, [false] if channel is open and
    /// receiving messages
    pub fn is_closed(&self) -> bool {
        self.port.is_closed()
    }
}

impl<TMsg> From<oneshot::Sender<TMsg>> for RpcReplyPort<TMsg> {
    fn from(value: oneshot::Sender<TMsg>) -> Self {
        Self { port: value }
    }
}
