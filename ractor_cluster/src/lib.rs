// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! # Support for remote nodes in a distributed cluster.
//!
//! A **node** is the same as [Erlang's definition](https://www.erlang.org/doc/reference_manual/distributed.html)
//! for distributed Erlang, in that it's a remote "hosting" process in the distributed pool of processes.
//!
//! In this realization, nodes are simply actors which handle an external connection to the other nodes in the pool.
//! When nodes connect and are authenticated, they spawn their remote-supporting local actors on the remote system
//! as `RemoteActor`s. The additionally handle synchronizing PG groups so the groups can contain both local
//! and remote actors.
//!
//! We have chosen protobuf for our inter-node defined protocol, however you can chose whatever medium you like
//! for binary serialization + deserialization. The "remote" actor will simply encode your message type and send it
//! over the wire for you
//!
//!
//! (Future) When nodes connect, they identify all of the nodes the remote node is also connected to and additionally connect
//! to them as well.
//!
//! ## Important note on message serialization
//!
//! An important note on usage, when utilizing `ractor_cluster` and [ractor] in the cluster configuration
//! (i.e. `ractor/cluster`), you no longer receive the auto-implementation for all types for [ractor::Message]. This
//! is due to specialization (see: <https://github.com/rust-lang/rust/issues/31844>). Ideally we'd have the trait have a
//! "default" non-serializable implementation for all types that could be messages, and specific implementations for
//! those that can be messages sent over the network. However this is presently a `+nightly` only functionality and
//! has a soundness hole in it's definition and usage. Therefore as a workaround, when the `cluster` feature is enabled
//! on [ractor] the default implementation, specifically
//!
//! ```text
//! impl<T: std::any::Any + Send + Sized + 'static> ractor::Message for T {}
//! ```
//! is disabled.
//!
//! This means that you need to specify the implementation of the [ractor::Message] trait on all message types, and when
//! they're not network supported messages, this is just a default empty implementation. When they **are** potentially
//! sent over a network in a dist protocol, then you need to fill out the implementation details for how the message
//! serialization is handled. There however is a procedural macro in `ractor_cluster_derive` to facilitate this, which is
//! re-exposed on this crate under the same naming. Simply derive [RactorMessage] or [RactorClusterMessage] if you want local or
//! remote-supporting messages, respectively.
//!

#![deny(warnings)]
#![warn(unused_imports)]
#![warn(unsafe_code)]
#![warn(missing_docs)]
#![warn(unused_crate_dependencies)]
#![cfg_attr(docsrs, feature(doc_cfg))]

mod hash;
mod net;
mod protocol;
mod remote_actor;

pub mod macros;
pub mod node;

/// Node's are representing by an integer id
pub type NodeId = u64;

// ============== Re-exports ============== //
pub use net::{IncomingEncryptionMode, NetworkStream};
pub use node::client::connect as client_connect;
pub use node::client::connect_enc as client_connect_enc;
pub use node::{
    client::ClientConnectErr, NodeServer, NodeServerMessage, NodeSession, NodeSessionMessage,
};

// Re-export the procedural macros so people don't need to reference them directly
pub use ractor_cluster_derive::RactorClusterMessage;
pub use ractor_cluster_derive::RactorMessage;

pub use ractor::serialization::*;
