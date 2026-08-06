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

use super::actor_cell::ActorCell;
use super::messages::SupervisionEvent;
use crate::ActorId;

// Structural updates span multiple actors, so they share one private writer lock.
static TREE_MUTATION_LOCK: Mutex<()> = Mutex::new(());

/// A supervision tree
#[derive(Debug)]
pub(crate) struct SupervisionTree {
    children: Mutex<Option<HashMap<ActorId, ActorCell>>>,
    supervisor: Mutex<Option<ActorCell>>,
    #[cfg(feature = "monitors")]
    monitors: Mutex<Option<HashMap<ActorId, ActorCell>>>,
}

impl Default for SupervisionTree {
    fn default() -> Self {
        Self {
            // `None` permanently closes the tree to new children once termination starts.
            children: Mutex::new(Some(HashMap::new())),
            supervisor: Mutex::new(None),
            #[cfg(feature = "monitors")]
            monitors: Mutex::new(None),
        }
    }
}

impl SupervisionTree {
    /// Transactionally replace a child's supervisor and update both parents' child sets.
    pub(crate) fn link(child: &ActorCell, supervisor: ActorCell) -> bool {
        let _mutation_guard = TREE_MUTATION_LOCK.lock().unwrap();

        if child.get_status() >= super::actor_cell::ActorStatus::Draining
            || supervisor.get_status() >= super::actor_cell::ActorStatus::Draining
        {
            return false;
        }

        let child_id = child.get_id();
        let mut new_children_guard = supervisor.inner.tree.children.lock().unwrap();
        let Some(new_children) = new_children_guard.as_mut() else {
            return false;
        };

        let mut current_supervisor = child.inner.tree.supervisor.lock().unwrap();
        if current_supervisor
            .as_ref()
            .is_some_and(|current| current.get_id() == supervisor.get_id())
        {
            new_children.insert(child_id, child.clone());
            return true;
        }

        new_children.insert(child_id, child.clone());
        let previous_supervisor = current_supervisor.replace(supervisor.clone());
        drop(current_supervisor);
        drop(new_children_guard);

        if let Some(previous_supervisor) = previous_supervisor {
            let mut previous_children = previous_supervisor.inner.tree.children.lock().unwrap();
            if let Some(previous_children) = previous_children.as_mut() {
                previous_children.remove(&child_id);
            }
        }
        true
    }

    /// Unlink a child if `supervisor` is still its current supervisor.
    pub(crate) fn unlink(child: &ActorCell, supervisor: &ActorCell) {
        let _mutation_guard = TREE_MUTATION_LOCK.lock().unwrap();
        let mut current_supervisor = child.inner.tree.supervisor.lock().unwrap();
        if !current_supervisor
            .as_ref()
            .is_some_and(|current| current.get_id() == supervisor.get_id())
        {
            return;
        }

        let mut children = supervisor.inner.tree.children.lock().unwrap();
        if let Some(children) = children.as_mut() {
            children.remove(&child.get_id());
        }
        *current_supervisor = None;
    }

    /// Close this actor's child set and detach the children for iterative termination.
    pub(crate) fn take_children(parent: &ActorCell) -> Vec<ActorCell> {
        let _mutation_guard = TREE_MUTATION_LOCK.lock().unwrap();
        let mut children = parent.inner.tree.children.lock().unwrap();
        let cells = children
            .take()
            .map_or_else(Vec::new, |children| children.into_values().collect());

        for child in &cells {
            let mut supervisor = child.inner.tree.supervisor.lock().unwrap();
            if supervisor
                .as_ref()
                .is_some_and(|supervisor| supervisor.get_id() == parent.get_id())
            {
                *supervisor = None;
            }
        }

        cells
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

    /// Stop all the linked children, but does NOT unlink them (stop flow will do that)
    pub(crate) fn stop_all_children(&self, reason: Option<String>) {
        self.for_each_child(|cell| {
            cell.stop(reason.clone());
        });
    }

    /// Drain all the linked children, but does NOT unlink them
    pub(crate) fn drain_all_children(&self) {
        self.for_each_child(|cell| {
            _ = cell.drain();
        });
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

    /// Return all linked children
    pub(crate) fn get_children(&self) -> Vec<ActorCell> {
        let guard = self.children.lock().unwrap();
        if let Some(map) = &*guard {
            map.values().cloned().collect()
        } else {
            vec![]
        }
    }

    /// Execute a closure for each child without allocating a Vec.
    /// This is more efficient when you just need to iterate over children.
    pub(crate) fn for_each_child<F>(&self, mut f: F)
    where
        F: FnMut(&ActorCell),
    {
        let guard = self.children.lock().unwrap();
        if let Some(map) = &*guard {
            for cell in map.values() {
                f(cell);
            }
        }
    }

    /// Send a notification to the supervisor.
    ///
    /// Optimized to collect all targets under a single lock, then send outside the lock
    /// to minimize lock contention.
    pub(crate) fn notify_supervisor(&self, evt: SupervisionEvent) {
        // Collect all notification targets under a single lock acquisition
        #[cfg(feature = "monitors")]
        let monitor_targets: Vec<ActorCell> = {
            let guard = self.monitors.lock().unwrap();
            if let Some(monitors) = &*guard {
                monitors.values().cloned().collect()
            } else {
                Vec::new()
            }
        };

        let supervisor_target = {
            let guard = self.supervisor.lock().unwrap();
            (*guard).clone()
        };

        // Send to all monitors (best-effort, outside the lock)
        #[cfg(feature = "monitors")]
        if !monitor_targets.is_empty() {
            for monitor in monitor_targets.iter() {
                // Clone the event for each monitor (without requiring inner data to be Clone)
                let monitor_evt = evt.clone_no_data();
                if monitor.send_supervisor_evt(monitor_evt).is_err() {
                    // Best-effort delivery - if send fails, remove the monitor
                    let mut guard = self.monitors.lock().unwrap();
                    if let Some(monitors) = &mut *guard {
                        monitors.remove(&monitor.get_id());
                    }
                }
            }
        }

        // Send to supervisor
        if let Some(parent) = supervisor_target {
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
