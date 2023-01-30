// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! TCP server and session actors which transmit [prost::Message] encoded messages

pub mod listener;
pub mod session;

/// A network port
pub type NetworkPort = u16;
