// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Port implementations for signaling and reception of messages in the Ractor environment

use tokio::sync::mpsc;
use tokio::sync::oneshot;

use crate::MessagingErr;

// ============ Input Ports ============ //

/// A bounded imput port (alias of [mpsc::Sender])
pub(crate) type BoundedInputPort<TMsg> = mpsc::Sender<TMsg>;
/// A bounded imput port (alias of [mpsc::Receiver])
pub(crate) type BoundedInputPortReceiver<TMsg> = mpsc::Receiver<TMsg>;

/// An import-port (alias of [mpsc::UnboundedSender])
pub type InputPort<TMsg> = mpsc::UnboundedSender<TMsg>;
/// An input port's receiver (alias of [mpsc::UnboundedReceiver])
pub(crate) type InputPortReceiver<TMsg> = mpsc::UnboundedReceiver<TMsg>;

// ============ Output Ports ============ //
// TODO: outputs with auto-subscriptions
// pub mod output;
// pub use output::*;

// ============ Rpc (one-use) Ports ============ //

/// A RPC's reply port. Wrapper of [tokio::sync::oneshot::Sender] with a consistent
/// error type
pub struct RpcReplyPort<TMsg> {
    port: oneshot::Sender<TMsg>,
}

impl<TMsg> RpcReplyPort<TMsg> {
    /// Send a message to the Rpc reply port. This consumes the port
    pub fn send(self, msg: TMsg) -> Result<(), MessagingErr> {
        self.port.send(msg).map_err(|_| MessagingErr::ChannelClosed)
    }

    /// Determine if the port is closed (i.e. the receiver has been dropped)
    pub fn is_closed(&self) -> bool {
        self.port.is_closed()
    }
}

impl<TMsg> From<oneshot::Sender<TMsg>> for RpcReplyPort<TMsg> {
    fn from(value: oneshot::Sender<TMsg>) -> Self {
        Self { port: value }
    }
}
