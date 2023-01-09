// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! This module handles everything around actor id's. In the event you have a
//! remote actor, this id will demonstrate that

use std::{fmt::Display, sync::atomic::AtomicU64};

/// An actor's globally unique identifier
#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq)]
pub enum ActorId {
    /// A local pid
    Local(u64),

    /// A remote actor on another system (system, id)
    Remote(u64, u64),
}

impl ActorId {
    /// Determine if this actor id is a local or remote actor
    ///
    /// Returns [true] if it is a local actor, [false] otherwise
    pub fn is_local(&self) -> bool {
        matches!(self, ActorId::Local(_))
    }
}

impl Display for ActorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActorId::Local(id) => write!(f, "0.{}", id),
            ActorId::Remote(system_id, id) => write!(f, "{}.{}", system_id, id),
        }
    }
}

/// The local id allocator for actors
static ACTOR_ID_ALLOCATOR: AtomicU64 = AtomicU64::new(0u64);

/// Retreiev a new local id
pub(crate) fn get_new_local_id() -> ActorId {
    ActorId::Local(ACTOR_ID_ALLOCATOR.fetch_add(1, std::sync::atomic::Ordering::AcqRel))
}

impl ActorId {
    /// Retrieve the PID of the actor, ignoring local/remote properties
    pub fn get_pid(&self) -> u64 {
        match self {
            ActorId::Local(pid) => *pid,
            ActorId::Remote(_, pid) => *pid,
        }
    }
}
