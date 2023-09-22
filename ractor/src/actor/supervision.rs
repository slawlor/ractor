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
    monitors: DashMap<ActorId, ActorCell>,
    monitored: DashMap<ActorId, ActorCell>,
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

    /// Set a monitor of this supervision tree
    pub fn set_monitor(&self, monitor: ActorCell) {
        self.monitors.insert(monitor.get_id(), monitor);
    }

    /// Mark that this actor is monitoring some other actors
    pub fn mark_monitored(&self, who: ActorCell) {
        self.monitored.insert(who.get_id(), who);
    }

    /// Mark that this actor is no longer monitoring some other actors
    pub fn unmark_monitored(&self, who: ActorId) {
        self.monitored.remove(&who);
    }

    /// Remove a specific monitor from the supervision tree
    pub fn remove_monitor(&self, monitor: ActorId) {
        self.monitors.remove(&monitor);
    }

    /// Get the [ActorCell]s of the monitored actors this actor monitors
    pub fn monitored_actors(&self) -> Vec<ActorCell> {
        self.monitored.iter().map(|a| a.value().clone()).collect()
    }

    /// Terminate all your supervised children and unlink them
    /// from the supervision tree since the supervisor is shutting down
    /// and can't deal with superivison events anyways
    pub fn terminate_all_children(&self) {
        let cells = self
            .children
            .iter()
            .map(|r| r.1.clone())
            .collect::<Vec<_>>();
        // wipe local children to prevent double-link problems
        self.children.clear();

        for cell in cells {
            cell.terminate();
            cell.clear_supervisor();
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

    /// Send a notification to the supervisor and monitors.
    ///
    /// CAVEAT: Monitors get notified first, in order to save an unnecessary
    /// clone if there are no monitors.
    pub fn notify_supervisor_and_monitors(&self, evt: SupervisionEvent) {
        for monitor in self.monitors.iter() {
            let _ = monitor.value().send_supervisor_evt(evt.clone_no_data());
        }
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
