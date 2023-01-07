// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! An actor framework for Rust

#![warn(missing_docs)]
#![warn(unused_crate_dependencies)]
#![cfg_attr(docsrs, feature(doc_cfg))]

use std::sync::atomic::AtomicU64;

/// An actor's globally unique identifier
pub type ActorId = u64;
/// The global id allocator for actors
pub static ACTOR_ID_ALLOCATOR: AtomicU64 = AtomicU64::new(0u64);

pub mod actor;
pub mod port;
pub mod rpc;
pub mod time;

// re-exports
pub use actor::actor_cell::{ActorCell, ActorStatus, ACTIVE_STATES};
pub use actor::errors::{ActorErr, MessagingErr, SpawnErr};
pub use actor::messages::{Signal, SupervisionEvent};
pub use actor::{Actor, ActorHandler};
pub use port::{InputPort, RpcReplyPort};

/// Message type for an actor
pub trait Message: Send + 'static {}
impl<T: Send + 'static> Message for T {}

/// Represents the state of an actor
pub trait State: Clone + Sync + Send + 'static {}
impl<T: Clone + Sync + Send + 'static> State for T {}

/// Error types which can result from Ractor processes
#[derive(Debug)]
pub enum RactorErr {
    /// An error occurred spawning
    Spawn(SpawnErr),
    /// An error occurred in messaging (sending/receiving)
    Messaging(MessagingErr),
    /// An actor encountered an error while processing (canceled or panicked)
    Actor(ActorErr),
}

impl From<SpawnErr> for RactorErr {
    fn from(value: SpawnErr) -> Self {
        RactorErr::Spawn(value)
    }
}

impl From<MessagingErr> for RactorErr {
    fn from(value: MessagingErr) -> Self {
        RactorErr::Messaging(value)
    }
}

impl From<ActorErr> for RactorErr {
    fn from(value: ActorErr) -> Self {
        RactorErr::Actor(value)
    }
}

impl std::fmt::Display for RactorErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Actor(actor_err) => {
                write!(f, "{}", actor_err)
            }
            Self::Messaging(messaging_err) => {
                write!(f, "{}", messaging_err)
            }
            Self::Spawn(spawn_err) => {
                write!(f, "{}", spawn_err)
            }
        }
    }
}
