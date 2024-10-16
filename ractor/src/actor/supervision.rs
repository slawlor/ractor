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
//! notifying the supervisor's supervisor? That's up to the implementation of the [super::Actor]
//!
//! This is currently an initial implementation of [Erlang supervisors](https://www.erlang.org/doc/man/supervisor.html)
//! which will be expanded upon as the library develops.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::{actor_cell::ActorCell, messages::SupervisionEvent};
use crate::ActorId;

/// A supervision tree
#[derive(Default, Debug)]
pub(crate) struct SupervisionTree {
    children: Arc<Mutex<HashMap<ActorId, ActorCell>>>,
    supervisor: Arc<Mutex<Option<ActorCell>>>,
}

impl SupervisionTree {
    /// Push a child into the tere
    pub(crate) fn insert_child(&self, child: ActorCell) {
        self.children.lock().unwrap().insert(child.get_id(), child);
    }

    /// Remove a specific actor from the supervision tree (e.g. actor died)
    pub(crate) fn remove_child(&self, child: ActorId) {
        self.children.lock().unwrap().remove(&child);
    }

    /// Push a parent into the tere
    pub(crate) fn set_supervisor(&self, parent: ActorCell) {
        *(self.supervisor.lock().unwrap()) = Some(parent);
    }

    /// Remove a specific actor from the supervision tree (e.g. actor died)
    pub(crate) fn clear_supervisor(&self) {
        *(self.supervisor.lock().unwrap()) = None;
    }

    /// Terminate all your supervised children and unlink them
    /// from the supervision tree since the supervisor is shutting down
    /// and can't deal with superivison events anyways
    pub(crate) fn terminate_all_children(&self) {
        let mut guard = self.children.lock().unwrap();
        let cells = guard.iter().map(|(_, a)| a.clone()).collect::<Vec<_>>();
        guard.clear();
        // drop the guard to not deadlock on double-link
        drop(guard);
        for cell in cells {
            cell.terminate();
            cell.clear_supervisor();
        }
    }

    /// Determine if the specified actor is a parent of this actor
    pub(crate) fn is_child_of(&self, id: ActorId) -> bool {
        if let Some(parent) = &*(self.supervisor.lock().unwrap()) {
            parent.get_id() == id
        } else {
            false
        }
    }

    /// Send a notification to the supervisor.
    pub(crate) fn notify_supervisor(&self, evt: SupervisionEvent) {
        if let Some(parent) = &*(self.supervisor.lock().unwrap()) {
            let _ = parent.send_supervisor_evt(evt);
        }
    }

    /// Retrieve the number of supervised children
    #[cfg(test)]
    pub(crate) fn get_num_children(&self) -> usize {
        self.children.lock().unwrap().len()
    }

    /// Retrieve the number of supervised children
    #[cfg(test)]
    pub(crate) fn get_num_parents(&self) -> usize {
        usize::from(self.supervisor.lock().unwrap().is_some())
    }
}
