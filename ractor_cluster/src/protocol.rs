// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Protobuf specifications for over-the-wire intercommuncation
//! between nodes. Generated via [prost]

/// Node authentication protocol
pub(crate) mod auth {
    #![allow(unreachable_pub)]
    include!(concat!(env!("OUT_DIR"), "/auth.rs"));
}

/// Node actor inter-communication protocol
pub(crate) mod node {
    #![allow(unreachable_pub)]
    include!(concat!(env!("OUT_DIR"), "/node.rs"));
}

/// Control messages between nodes
pub(crate) mod control {
    #![allow(unreachable_pub)]
    include!(concat!(env!("OUT_DIR"), "/control.rs"));
}

/// Meta types which include all base network protocol message types
pub(crate) mod meta {
    #![allow(unreachable_pub)]
    include!(concat!(env!("OUT_DIR"), "/meta.rs"));
}

pub(crate) use meta::NetworkMessage;
