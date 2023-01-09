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
//! For messages to be transmittable across the [Node] boundaries to other [Node]s in the pool, they need to be
//! serializable to a binary format (say protobuf)

use dashmap::DashMap;

/// Represents messages that can cross the node boundary which can be serialized and sent over the wire
pub trait NodeSerializableMessage {
    /// Serialize the message to binary
    fn serialize(&self) -> &[u8];

    /// Deserialize from binary back into the message type
    fn deserialize(&self, data: &[u8]) -> Self;
}

/// The identifier of a node is a globally unique u64
pub type NodeId = u64;

/// A node in the distributed compute pool.
pub struct Node {
    node_id: u64,
    other_nodes: DashMap<u64, String>,
}