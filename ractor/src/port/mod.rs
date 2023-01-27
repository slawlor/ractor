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

use crate::concurrency;

use crate::MessagingErr;

// ============ Output Ports ============ //
pub mod output;
pub use output::*;

// ============ Rpc (one-use) Ports ============ //

/// A remote procedure call's reply port. Wrapper of [concurrency::OneshotSender] with a
/// consistent error type
pub struct RpcReplyPort<TMsg> {
    port: concurrency::OneshotSender<TMsg>,
    timeout: Option<concurrency::Duration>,
}

impl<TMsg> RpcReplyPort<TMsg> {
    /// Read the timeout of this RPC reply port
    ///
    /// Returns [Some(concurrency::Duration)] if a timeout is set, [None] otherwise
    pub fn get_timeout(&self) -> Option<concurrency::Duration> {
        self.timeout
    }

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

impl<TMsg> From<concurrency::OneshotSender<TMsg>> for RpcReplyPort<TMsg> {
    fn from(value: concurrency::OneshotSender<TMsg>) -> Self {
        Self {
            port: value,
            timeout: None,
        }
    }
}

impl<TMsg> From<(concurrency::OneshotSender<TMsg>, concurrency::Duration)> for RpcReplyPort<TMsg> {
    fn from((value, timeout): (concurrency::OneshotSender<TMsg>, concurrency::Duration)) -> Self {
        Self {
            port: value,
            timeout: Some(timeout),
        }
    }
}
