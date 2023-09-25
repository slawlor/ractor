// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Process groups (PG) are named groups of actors with a friendly name
//! which can be used for retrieval of the process groups. Then within
//! the group, either a random actor (for dispatch) can be selected or
//! the whole group (broadcast), or a subset (partial-broadcast) can have
//! a message sent to them. Common operations are to (a) upcast the group
//! members to a strong-type'd actor then dispatch a message with [crate::call]
//! or [crate::cast].
//!
//! Process groups can also be monitored for changes with calling [monitor] to
//! subscribe to changes and [demonitor] to unsubscribe. Subscribers will receive
//! process group change notifications via a [SupervisionEvent] called on the
//! supervision port of the [crate::Actor]
//!
//! Inspired from [Erlang's `pg` module](https://www.erlang.org/doc/man/pg.html)

use std::collections::HashMap;
use std::sync::Arc;

use dashmap::mapref::entry::Entry::{Occupied, Vacant};
use dashmap::DashMap;

use once_cell::sync::OnceCell;

use crate::{ActorCell, ActorId, GroupName, SupervisionEvent};

/// Key to monitor all of the groups
pub const ALL_GROUPS_NOTIFICATION: &str = "__world__";

#[cfg(test)]
mod tests;

/// Represents a change in a process group's membership
#[derive(Clone)]
pub enum GroupChangeMessage {
    /// Some actors joined a group
    Join(GroupName, Vec<ActorCell>),
    /// Some actors left a group
    Leave(GroupName, Vec<ActorCell>),
}

impl GroupChangeMessage {
    /// Retrieve the group that changed
    pub fn get_group(&self) -> GroupName {
        match self {
            Self::Join(name, _) => name.clone(),
            Self::Leave(name, _) => name.clone(),
        }
    }
}

struct PgState {
    map: Arc<DashMap<GroupName, HashMap<ActorId, ActorCell>>>,
    listeners: Arc<DashMap<GroupName, Vec<ActorCell>>>,
}

static PG_MONITOR: OnceCell<PgState> = OnceCell::new();

fn get_monitor<'a>() -> &'a PgState {
    PG_MONITOR.get_or_init(|| PgState {
        map: Arc::new(DashMap::new()),
        listeners: Arc::new(DashMap::new()),
    })
}

/// Join actors to the group `group`
///
/// * `group` - The statically named group. Will be created if first actors to join
/// * `actors` - The list of [crate::Actor]s to add to the group
pub fn join(group: GroupName, actors: Vec<ActorCell>) {
    let monitor = get_monitor();
    // insert into the monitor group
    match monitor.map.entry(group.clone()) {
        Occupied(mut occupied) => {
            let oref = occupied.get_mut();
            for actor in actors.iter() {
                oref.insert(actor.get_id(), actor.clone());
            }
        }
        Vacant(vacancy) => {
            let map = actors
                .iter()
                .map(|a| (a.get_id(), a.clone()))
                .collect::<HashMap<_, _>>();
            vacancy.insert(map);
        }
    }
    // notify supervisors
    if let Some(listeners) = monitor.listeners.get(&group) {
        for listener in listeners.value() {
            let _ = listener.send_supervisor_evt(SupervisionEvent::ProcessGroupChanged(
                GroupChangeMessage::Join(group.clone(), actors.clone()),
            ));
        }
    }
    // notify the world monitors
    if let Some(listeners) = monitor.listeners.get(ALL_GROUPS_NOTIFICATION) {
        for listener in listeners.value() {
            let _ = listener.send_supervisor_evt(SupervisionEvent::ProcessGroupChanged(
                GroupChangeMessage::Join(group.clone(), actors.clone()),
            ));
        }
    }
}

/// Leaves the specified [crate::Actor]s from the PG group
///
/// * `group` - The statically named group
/// * `actors` - The list of actors to remove from the group
pub fn leave(group: GroupName, actors: Vec<ActorCell>) {
    let monitor = get_monitor();
    match monitor.map.entry(group.clone()) {
        Vacant(_) => {}
        Occupied(mut occupied) => {
            let mut_ref = occupied.get_mut();
            for actor in actors.iter() {
                mut_ref.remove(&actor.get_id());
            }
            // the group is empty, remove it
            if mut_ref.is_empty() {
                occupied.remove();
            }
            if let Some(listeners) = monitor.listeners.get(&group) {
                for listener in listeners.value() {
                    let _ = listener.send_supervisor_evt(SupervisionEvent::ProcessGroupChanged(
                        GroupChangeMessage::Leave(group.clone(), actors.clone()),
                    ));
                }
            }
            // notify the world monitors
            if let Some(listeners) = monitor.listeners.get(ALL_GROUPS_NOTIFICATION) {
                for listener in listeners.value() {
                    let _ = listener.send_supervisor_evt(SupervisionEvent::ProcessGroupChanged(
                        GroupChangeMessage::Leave(group.clone(), actors.clone()),
                    ));
                }
            }
        }
    }
}

/// Leave all groups for a specific [ActorId].
/// Used only during actor shutdown
pub(crate) fn leave_all(actor: ActorId) {
    let pg_monitor = get_monitor();
    let map = pg_monitor.map.clone();

    let mut empty_groups = vec![];
    let mut removal_events = HashMap::new();

    for mut kv in map.iter_mut() {
        if let Some(actor_cell) = kv.value_mut().remove(&actor) {
            removal_events.insert(kv.key().clone(), actor_cell);
        }
        if kv.value().is_empty() {
            empty_groups.push(kv.key().clone());
        }
    }

    // notify the listeners
    let all_listeners = pg_monitor.listeners.clone();
    for (group, cell) in removal_events.into_iter() {
        if let Some(this_listeners) = all_listeners.get(&group) {
            this_listeners.iter().for_each(|listener| {
                let _ = listener.send_supervisor_evt(SupervisionEvent::ProcessGroupChanged(
                    GroupChangeMessage::Leave(group.clone(), vec![cell.clone()]),
                ));
            });
        }
        // notify the world monitors
        if let Some(listeners) = all_listeners.get(ALL_GROUPS_NOTIFICATION) {
            for listener in listeners.value() {
                let _ = listener.send_supervisor_evt(SupervisionEvent::ProcessGroupChanged(
                    GroupChangeMessage::Leave(group.clone(), vec![cell.clone()]),
                ));
            }
        }
    }

    // Cleanup empty groups
    for group in empty_groups {
        map.remove(&group);
    }
}

/// Returns all the actors running on the local node in the group `group`.
///
/// * `group_name` - Either a statically named group or scope
///
/// Returns a [`Vec<ActorCell>`] representing the members of this paging group
pub fn get_local_members(group_name: &GroupName) -> Vec<ActorCell> {
    let monitor = get_monitor();
    if let Some(actors) = monitor.map.get(group_name) {
        actors
            .value()
            .values()
            .filter(|a| a.get_id().is_local())
            .cloned()
            .collect::<Vec<_>>()
    } else {
        vec![]
    }
}

/// Returns all the actors running on any node in the group `group`.
///
/// * `group_name` - Either a statically named group or scope
///
/// Returns a [`Vec<ActorCell>`] with the member actors
pub fn get_members(group_name: &GroupName) -> Vec<ActorCell> {
    let monitor = get_monitor();
    if let Some(actors) = monitor.map.get(group_name) {
        actors.value().values().cloned().collect::<Vec<_>>()
    } else {
        vec![]
    }
}

/// Return a list of all known groups
///
/// Returns a [`Vec<GroupName>`] representing all the registered group names
pub fn which_groups() -> Vec<GroupName> {
    let monitor = get_monitor();
    monitor
        .map
        .iter()
        .map(|kvp| kvp.key().clone())
        .collect::<Vec<_>>()
}

/// Subscribes the provided [crate::Actor] to the scope or group for updates
///
/// * `group_name` - The scope or group to monitor
/// * `actor` - The [ActorCell] representing who will receive updates
pub fn monitor(group_name: GroupName, actor: ActorCell) {
    let monitor = get_monitor();
    match monitor.listeners.entry(group_name) {
        Occupied(mut occupied) => occupied.get_mut().push(actor),
        Vacant(vacancy) => {
            vacancy.insert(vec![actor]);
        }
    }
}

/// Unsubscribes the provided [crate::Actor] from the scope or group for updates
///
/// * `group_name` - The scope or group to monitor
/// * `actor` - The [ActorCell] representing who will receive updates
pub fn demonitor(group_name: GroupName, actor: ActorId) {
    let monitor = get_monitor();
    if let Occupied(mut entry) = monitor.listeners.entry(group_name) {
        let mut_ref = entry.get_mut();
        mut_ref.retain(|a| a.get_id() != actor);
        if mut_ref.is_empty() {
            entry.remove();
        }
    }
}

/// Remove the specified [ActorId] from monitoring all groups it might be.
/// Used only during actor shutdown
pub(crate) fn demonitor_all(actor: ActorId) {
    let monitor = get_monitor();
    let mut empty_groups = vec![];

    for mut kvp in monitor.listeners.iter_mut() {
        let v = kvp.value_mut();
        v.retain(|v| v.get_id() != actor);
        if v.is_empty() {
            empty_groups.push(kvp.key().clone());
        }
    }

    // cleanup empty listener groups
    for empty_group in empty_groups {
        monitor.listeners.remove(&empty_group);
    }
}
