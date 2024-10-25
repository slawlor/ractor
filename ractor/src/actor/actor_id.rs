// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! This module handles everything around actor id's. In the event you have a
//! remote actor, the actor's ID will denote not only the actor's instance id, but
//! the id of the remote node the actor is running on.
//!
//! ActorIds are generally helpful in logging and pattern matching, but not directly
//! used in addressing outside of `ractor_cluster` network-based call internals.

use std::{fmt::Display, sync::atomic::AtomicU64};

/// An actor's globally unique identifier
#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub enum ActorId {
    /// A local pid
    Local(u64),

    /// A remote actor on another system (system, id)
    Remote {
        /// The remote node id
        node_id: u64,
        /// The local id on the remote system
        pid: u64,
    },
}

impl ActorId {
    /// Determine if this actor id is a local or remote actor
    ///
    /// Returns [true] if it is a local actor, [false] otherwise
    pub const fn is_local(&self) -> bool {
        matches!(self, ActorId::Local(_))
    }

    /// Retrieve the actor's PID
    ///
    /// Returns the actor's [u64] instance identifier (process id).
    pub const fn pid(&self) -> u64 {
        match self {
            ActorId::Local(pid) => *pid,
            ActorId::Remote { pid, .. } => *pid,
        }
    }

    /// Retrieve the node id of this PID. 0 = a local actor, while
    /// any non-zero value is the ide of the remote node running this actor
    pub const fn node(&self) -> u64 {
        match self {
            ActorId::Local(_) => 0,
            ActorId::Remote { node_id, .. } => *node_id,
        }
    }
}

impl Display for ActorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActorId::Local(id) => write!(f, "0.{id}"),
            ActorId::Remote { node_id, pid } => write!(f, "{node_id}.{pid}"),
        }
    }
}

/// The local id allocator for actors
static ACTOR_ID_ALLOCATOR: AtomicU64 = AtomicU64::new(0u64);

/// Retrieve a new local id
pub(crate) fn get_new_local_id() -> ActorId {
    ActorId::Local(ACTOR_ID_ALLOCATOR.fetch_add(1, std::sync::atomic::Ordering::AcqRel))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pid() {
        let actor_id = ActorId::Local(123);
        assert_eq!(123, actor_id.pid());
        let actor_id = ActorId::Remote {
            node_id: 1,
            pid: 123,
        };
        assert_eq!(123, actor_id.pid());
    }

    #[test]
    fn test_is_local() {
        let actor_id = ActorId::Local(123);
        assert!(actor_id.is_local());
        let actor_id = ActorId::Remote {
            node_id: 1,
            pid: 123,
        };
        assert!(!actor_id.is_local());
    }
}
