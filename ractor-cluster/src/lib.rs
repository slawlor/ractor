// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Support for remote nodes in a distributed cluster.
//!
//! A node is the same as [Erlang's definition](https://www.erlang.org/doc/reference_manual/distributed.html)
//! for distributed Erlang, in that it's a remote "hosting" process in the distributed pool of processes.
//!
//! In this realization, nodes are simply actors which handle an external connection to the other nodes in the pool.
//! When nodes connect, they identify all of the nodes the remote node is also connected to and additionally connect
//! to them as well. They merge registries and pg groups together in order to create larger clusters of services.
//!
//! We have chosen protobuf for our inter-node defined protocol, however you can chose whatever medium you like
//! for binary serialization + deserialization. The "remote" actor will simply encode your message type and send it
//! over the wire for you
//!
//! ## A note on usage
//!
//! An important note on usage, when utilizing `ractor-cluster` and [ractor] in the cluster configuration
//! (i.e. `ractor/cluster`), you no longer receive the auto-implementation for all types for [ractor::Message]. This
//! is due to specialization (see: <https://github.com/rust-lang/rust/issues/31844>). Ideally we'd have the trait have a
//! "default" non-serializable implementation for all types that could be messages, and specific implementations for
//! those that can be messages sent over the network. However this is presently a `+nightly` only functionality and
//! has a soundness hole in it's processes. Therefore as a workaround, when the `cluster` feature is enabled on [ractor]
//! the default implementation, specifically `impl<T: Any + Send + Sized + 'static> Message for T {}` is disabled.
//!
//! This means that you need to specify the implementation of the [ractor::Message] trait on all message types, and when
//! they're not network supported messages, this is just a default empty implementation. When they **are** potentially
//! sent over a network in a dist protocol, then you need to fill out the implementation details for how the message
//! serialization is handled. See the documentation of [crate::serialized_rpc_forward] for an example.

// #![deny(warnings)]
#![warn(unused_imports)]
#![warn(unsafe_code)]
#![warn(missing_docs)]
#![warn(unused_crate_dependencies)]
#![cfg_attr(docsrs, feature(doc_cfg))]

mod hash;
mod net;
pub mod node;
pub(crate) mod protocol;
pub(crate) mod remote_actor;

pub mod macros;

// Re-exports
pub use node::NodeServer;

/// Node's are representing by an integer id
pub type NodeId = u64;
