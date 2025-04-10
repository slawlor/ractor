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

use std::collections::HashMap;
use std::sync::Mutex;

use super::{actor_cell::ActorCell, messages::SupervisionEvent};
use crate::ActorId;

/// A supervision tree
#[derive(Default, Debug)]
pub(crate) struct SupervisionTree {
    children: Mutex<Option<HashMap<ActorId, ActorCell>>>,
    supervisor: Mutex<Option<ActorCell>>,
    #[cfg(feature = "monitors")]
    monitors: Mutex<Option<HashMap<ActorId, ActorCell>>>,
}

impl SupervisionTree {
    /// Push a child into the tere
    pub(crate) fn insert_child(&self, child: ActorCell) {
        let mut guard = self.children.lock().unwrap();
        if let Some(map) = &mut *(guard) {
            map.insert(child.get_id(), child);
        } else {
            *guard = Some(HashMap::from_iter([(child.get_id(), child)]));
        }
    }

    /// Remove a specific actor from the supervision tree (e.g. actor died)
    pub(crate) fn remove_child(&self, child: ActorId) {
        let mut guard = self.children.lock().unwrap();
        if let Some(map) = &mut *(guard) {
            map.remove(&child);
        }
    }

    /// Push a parent into the tere
    pub(crate) fn set_supervisor(&self, parent: ActorCell) {
        *(self.supervisor.lock().unwrap()) = Some(parent);
    }

    /// Remove a specific actor from the supervision tree (e.g. actor died)
    pub(crate) fn clear_supervisor(&self) {
        *(self.supervisor.lock().unwrap()) = None;
    }

    /// Try and retrieve the set supervisor
    pub(crate) fn try_get_supervisor(&self) -> Option<ActorCell> {
        self.supervisor.lock().unwrap().clone()
    }

    /// Set a monitor of this supervision tree
    #[cfg(feature = "monitors")]
    pub(crate) fn set_monitor(&self, who: ActorCell) {
        let mut guard = self.monitors.lock().unwrap();
        if let Some(map) = &mut *guard {
            map.insert(who.get_id(), who);
        } else {
            *guard = Some(HashMap::from_iter([(who.get_id(), who)]))
        }
    }

    /// Remove a specific monitor from the supervision tree
    #[cfg(feature = "monitors")]
    pub(crate) fn remove_monitor(&self, who: ActorId) {
        let mut guard = self.monitors.lock().unwrap();
        if let Some(map) = &mut *guard {
            map.remove(&who);
            if map.is_empty() {
                *guard = None;
            }
        }
    }

    /// Terminate all your supervised children and unlink them
    /// from the supervision tree since the supervisor is shutting down
    /// and can't deal with superivison events anyways
    pub(crate) fn terminate_all_children(&self) {
        let mut guard = self.children.lock().unwrap();
        let cells = if let Some(map) = &mut *guard {
            map.values().cloned().collect()
        } else {
            vec![]
        };
        *guard = None;
        // drop the guard to not deadlock on double-link
        drop(guard);
        for cell in cells {
            cell.terminate();
            cell.clear_supervisor();
        }
    }

    /// Stop all the linked children, but does NOT unlink them (stop flow will do that)
    pub(crate) fn stop_all_children(&self, reason: Option<String>) {
        let cells = self.get_children();
        for cell in cells {
            cell.stop(reason.clone());
        }
    }

    /// Drain all the linked children, but does NOT unlink them
    pub(crate) fn drain_all_children(&self) {
        let cells = self.get_children();
        for cell in cells {
            _ = cell.drain();
        }
    }

    /// Stop all the linked children, but does NOT unlink them (stop flow will do that),
    /// and wait for them to exit (concurrently)
    pub(crate) async fn stop_all_children_and_wait(
        &self,
        reason: Option<String>,
        timeout: Option<crate::concurrency::Duration>,
    ) {
        let cells = self.get_children();
        let mut js = crate::concurrency::JoinSet::new();
        for cell in cells {
            let lreason = reason.clone();
            let ltimeout = timeout;
            js.spawn(async move { cell.stop_and_wait(lreason, ltimeout).await });
        }
        // drain the tasks
        while let Some(res) = js.join_next().await {
            #[cfg(any(
                feature = "async-std",
                all(target_arch = "wasm32", target_os = "unknown")
            ))]
            if res.is_err() {
                panic!("JoinSet join error");
            }
            #[cfg(not(any(
                feature = "async-std",
                all(target_arch = "wasm32", target_os = "unknown")
            )))]
            {
                match res {
                    Err(err) if err.is_panic() => std::panic::resume_unwind(err.into_panic()),
                    Err(err) => panic!("{err}"),
                    _ => {}
                }
            }
        }
    }

    /// Drain all the linked children, but does NOT unlink them
    pub(crate) async fn drain_all_children_and_wait(
        &self,
        timeout: Option<crate::concurrency::Duration>,
    ) {
        let cells = self.get_children();
        let mut js = crate::concurrency::JoinSet::new();
        for cell in cells {
            let ltimeout = timeout;
            js.spawn(async move { cell.drain_and_wait(ltimeout).await });
        }
        // drain the tasks
        while let Some(res) = js.join_next().await {
            #[cfg(any(
                feature = "async-std",
                all(target_arch = "wasm32", target_os = "unknown")
            ))]
            if res.is_err() {
                panic!("JoinSet join error");
            }
            #[cfg(not(any(
                feature = "async-std",
                all(target_arch = "wasm32", target_os = "unknown")
            )))]
            {
                match res {
                    Err(err) if err.is_panic() => std::panic::resume_unwind(err.into_panic()),
                    Err(err) => panic!("{err}"),
                    _ => {}
                }
            }
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

    /// Return all linked children
    pub(crate) fn get_children(&self) -> Vec<ActorCell> {
        let guard = self.children.lock().unwrap();
        if let Some(map) = &*guard {
            map.values().cloned().collect()
        } else {
            vec![]
        }
    }

    /// Send a notification to the supervisor.
    ///
    /// CAVEAT: Monitors get notified first, in order to save an unnecessary
    /// clone if there are no monitors.
    pub(crate) fn notify_supervisor(&self, evt: SupervisionEvent) {
        #[cfg(feature = "monitors")]
        if let Some(monitors) = &mut *(self.monitors.lock().unwrap()) {
            // We notify the monitors on a best-effort basis, and if we fail to send the event, we remove
            // the monitor
            monitors.retain(|_, v| v.send_supervisor_evt(evt.clone_no_data()).is_ok());
        }

        if let Some(parent) = &*(self.supervisor.lock().unwrap()) {
            _ = parent.send_supervisor_evt(evt);
        }
    }

    /// Retrieve the number of supervised children
    #[cfg(test)]
    pub(crate) fn get_num_children(&self) -> usize {
        let guard = self.children.lock().unwrap();
        if let Some(map) = &*guard {
            map.len()
        } else {
            0
        }
    }

    /// Retrieve the number of supervised children
    #[cfg(test)]
    pub(crate) fn get_num_parents(&self) -> usize {
        usize::from(self.supervisor.lock().unwrap().is_some())
    }
}
