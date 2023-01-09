// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Supervision management logic
//!
//! Supervision is a special notion of "ownership" over actors by a parent (supervisor).
//! Supervisors are responsible for the lifecycle of a child actor such that they get notified
//! when a child actor starts, stops, or panics (when possible). The supervisor can then decide
//! how to handle the event. Should it restart the actor, leave it dead, potentially die itself
//! notifying the supervisor's supervisor? That's up to the implementation of the [super::ActorHandler]
//!
//! This is currently an initial implementation of [Erlang supervisors](https://www.erlang.org/doc/man/supervisor.html)
//! which will be expanded upon as the library develops. Next in line is likely supervision strategies
//! for automatic restart routines.

use dashmap::DashMap;

use super::{actor_cell::ActorCell, messages::SupervisionEvent};
use crate::{ActorHandler, ActorId};

/// A supervision tree
#[derive(Default)]
pub struct SupervisionTree {
    children: DashMap<ActorId, ActorCell>,
    parents: DashMap<ActorId, ActorCell>,
}

impl SupervisionTree {
    /// Push a child into the tere
    pub fn insert_child(&self, child: ActorCell) {
        self.children.insert(child.get_id(), child);
    }

    /// Remove a specific actor from the supervision tree (e.g. actor died)
    pub fn remove_child(&self, child: ActorCell) {
        let id = child.get_id();
        match self.children.entry(id) {
            dashmap::mapref::entry::Entry::Occupied(item) => {
                item.remove();
            }
            dashmap::mapref::entry::Entry::Vacant(_) => {}
        }
    }

    /// Push a parent into the tere
    pub fn insert_parent(&self, parent: ActorCell) {
        self.parents.insert(parent.get_id(), parent);
    }

    /// Remove a specific actor from the supervision tree (e.g. actor died)
    pub fn remove_parent(&self, parent: ActorCell) {
        let id = parent.get_id();
        match self.parents.entry(id) {
            dashmap::mapref::entry::Entry::Occupied(item) => {
                item.remove();
            }
            dashmap::mapref::entry::Entry::Vacant(_) => {}
        }
    }

    /// Terminate all your supervised children
    pub fn terminate_children(&self) {
        for kvp in self.children.iter() {
            kvp.value().terminate();
        }
    }

    /// Determine if the specified actor is a member of this supervision tree
    pub fn is_supervisor_of(&self, id: ActorId) -> bool {
        self.children.contains_key(&id)
    }

    /// Determine if the specified actor is a parent of this actor
    pub fn is_child_of(&self, id: ActorId) -> bool {
        self.parents.contains_key(&id)
    }

    /// Send a notification to all supervisors
    pub fn notify_supervisors<TActor>(&self, evt: SupervisionEvent)
    where
        TActor: ActorHandler,
    {
        for kvp in self.parents.iter() {
            let evt_clone = evt.duplicate::<TActor::State>().unwrap();
            let _ = kvp.value().send_supervisor_evt(evt_clone);
        }
    }

    /// Retrieve the number of supervised children
    pub fn get_num_children(&self) -> usize {
        self.children.len()
    }

    /// Retrieve the number of supervised children
    pub fn get_num_parents(&self) -> usize {
        self.parents.len()
    }
}
