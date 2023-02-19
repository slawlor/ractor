// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Represents a PID-based registration. Includes all LOCAL actors and their associated pids. It's kept in
//! sync via actor spawn + death management.

use std::fmt::Debug;
use std::sync::Arc;

use dashmap::mapref::entry::Entry::{Occupied, Vacant};
use dashmap::DashMap;
use once_cell::sync::OnceCell;

use crate::{ActorCell, ActorId, SupervisionEvent};

/// Represents a change in group or scope membership
#[derive(Clone)]
pub enum PidLifecycleEvent {
    /// Some actors joined a group
    Spawn(ActorCell),
    /// Some actors left a group
    Terminate(ActorCell),
}

impl Debug for PidLifecycleEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(who) => {
                write!(f, "Spawn {}", who.get_id())
            }
            Self::Terminate(who) => {
                write!(f, "Terminate {}", who.get_id())
            }
        }
    }
}

static PID_REGISTRY: OnceCell<Arc<DashMap<ActorId, ActorCell>>> = OnceCell::new();
static PID_REGISTRY_LISTENERS: OnceCell<Arc<DashMap<ActorId, ActorCell>>> = OnceCell::new();

fn get_pid_registry<'a>() -> &'a Arc<DashMap<ActorId, ActorCell>> {
    PID_REGISTRY.get_or_init(|| Arc::new(DashMap::new()))
}

fn get_pid_listeners<'a>() -> &'a Arc<DashMap<ActorId, ActorCell>> {
    PID_REGISTRY_LISTENERS.get_or_init(|| Arc::new(DashMap::new()))
}

pub(crate) fn register_pid(id: ActorId, actor: ActorCell) -> Result<(), super::ActorRegistryErr> {
    if id.is_local() {
        match get_pid_registry().entry(id) {
            Occupied(_o) => Err(super::ActorRegistryErr::AlreadyRegistered(format!(
                "PID {id} already alive"
            ))),
            Vacant(v) => {
                v.insert(actor.clone());
                // notify lifecycle listeners
                for listener in get_pid_listeners().iter() {
                    let _ =
                        listener
                            .value()
                            .send_supervisor_evt(SupervisionEvent::PidLifecycleEvent(
                                PidLifecycleEvent::Spawn(actor.clone()),
                            ));
                }
                Ok(())
            }
        }
    } else {
        Ok(())
    }
}

pub(crate) fn unregister_pid(id: ActorId) {
    if id.is_local() {
        if let Some((_, cell)) = get_pid_registry().remove(&id) {
            // notify lifecycle listeners
            for listener in get_pid_listeners().iter() {
                let _ = listener
                    .value()
                    .send_supervisor_evt(SupervisionEvent::PidLifecycleEvent(
                        PidLifecycleEvent::Terminate(cell.clone()),
                    ));
            }
        }
    }
}

/// Retrieve all currently registered [crate::Actor]s from the registry
///
/// Returns [Vec<_>] of [crate::ActorCell]s representing the current actors
/// registered
pub fn get_all_pids() -> Vec<ActorCell> {
    get_pid_registry()
        .iter()
        .map(|v| v.value().clone())
        .collect::<Vec<_>>()
}

/// Retrieve an actor from the global registry of all local actors
///
/// * `id` - The **local** id of the actor to retrieve
///
/// Returns [Some(_)] if the actor exists locally, [None] otherwise
pub fn where_is_pid(id: ActorId) -> Option<ActorCell> {
    if id.is_local() {
        get_pid_registry().get(&id).map(|v| v.value().clone())
    } else {
        None
    }
}

/// Subscribes the provided [crate::Actor] to the PID registry lifecycle
/// events
///
/// * `actor` - The [ActorCell] representing who will receive updates
pub fn monitor(actor: ActorCell) {
    get_pid_listeners().insert(actor.get_id(), actor);
}

/// Unsubscribes the provided [crate::Actor] from the PID registry lifecycle
/// events
///
/// * `actor` - The [ActorCell] representing who was receiving updates
pub fn demonitor(actor: ActorId) {
    let _ = get_pid_listeners().remove(&actor);
}
