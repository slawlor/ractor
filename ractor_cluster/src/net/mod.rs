// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! TCP server and session actors which transmit [prost::Message] encoded messages

// TODO: we need a way to identify which session messages are coming from + going to. Therefore
// we should actually have a notification when a new session is launched, which can be used
// to match which session is tied to which actor id

pub mod listener;
pub mod session;

/// A trait which implements [prost::Message], [Default], and has a static lifetime
/// denoting protobuf-encoded messages which can be transmitted over the wire
pub trait NetworkMessage: prost::Message + Default + 'static {}
impl<T: prost::Message + Default + 'static> NetworkMessage for T {}

/// A network port
pub type NetworkPort = u16;
