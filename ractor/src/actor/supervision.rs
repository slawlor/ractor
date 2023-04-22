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
//! which will be expanded upon as the library develops. Next in line is likely supervision strategies
//! for automatic restart routines.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, RwLock,
};

use dashmap::DashMap;

use super::{actor_cell::ActorCell, messages::SupervisionEvent};
use crate::ActorId;

/// A supervision tree
#[derive(Default)]
pub struct SupervisionTree {
    children: DashMap<ActorId, (u64, ActorCell)>,
    supervisor: Arc<RwLock<Option<ActorCell>>>,
    start_order: AtomicU64,
}

impl SupervisionTree {
    /// Push a child into the tere
    pub fn insert_child(&self, child: ActorCell) {
        let start_order = self.start_order.fetch_add(1, Ordering::Relaxed);
        self.children.insert(child.get_id(), (start_order, child));
    }

    /// Remove a specific actor from the supervision tree (e.g. actor died)
    pub fn remove_child(&self, child: ActorId) {
        match self.children.entry(child) {
            dashmap::mapref::entry::Entry::Occupied(item) => {
                item.remove();
            }
            dashmap::mapref::entry::Entry::Vacant(_) => {}
        }
    }

    /// Push a parent into the tere
    pub fn set_supervisor(&self, parent: ActorCell) {
        *(self.supervisor.write().unwrap()) = Some(parent);
    }

    /// Remove a specific actor from the supervision tree (e.g. actor died)
    pub fn clear_supervisor(&self) {
        *(self.supervisor.write().unwrap()) = None;
    }

    /// Terminate all your supervised children and unlink them
    /// from the supervision tree since the supervisor is shutting down
    /// and can't deal with superivison events anyways
    pub fn terminate_all_children(&self) {
        for kvp in self.children.iter() {
            let child = &kvp.value().1;
            child.terminate();
            child.clear_supervisor();
        }
        self.children.clear();
    }

    /// Terminate the supervised children after a given actor (including the specified actor).
    /// This is necessary to support [Erlang's supervision model](https://www.erlang.org/doc/design_principles/sup_princ.html#flags),
    /// specifically the `rest_for_one` strategy
    ///
    /// * `id` - The id of the actor to terminate + all those that follow
    pub fn terminate_children_after(&self, id: ActorId) {
        let mut reference_point = u64::MAX;
        let mut id_map = std::collections::HashMap::new();

        // keep the lock inside this scope on the map
        {
            for item in self.children.iter_mut() {
                id_map.insert(item.value().0, *item.key());
                if item.value().1.get_id() == id {
                    reference_point = item.value().0;
                    break;
                }
            }
        }

        // if there was a reference point, terminate children from that point on
        if reference_point < u64::MAX {
            for child in self.children.iter() {
                child.1.terminate();
            }
        }
    }

    /// Determine if the specified actor is a parent of this actor
    pub fn is_child_of(&self, id: ActorId) -> bool {
        if let Some(parent) = &*(self.supervisor.read().unwrap()) {
            parent.get_id() == id
        } else {
            false
        }
    }

    /// Send a notification to all supervisors
    pub fn notify_supervisor(&self, evt: SupervisionEvent) {
        if let Some(parent) = &*(self.supervisor.read().unwrap()) {
            let _ = parent.send_supervisor_evt(evt);
        }
    }

    /// Retrieve the number of supervised children
    #[cfg(test)]
    pub fn get_num_children(&self) -> usize {
        self.children.len()
    }

    /// Retrieve the number of supervised children
    #[cfg(test)]
    pub fn get_num_parents(&self) -> usize {
        usize::from(self.supervisor.read().unwrap().is_some())
    }
}
