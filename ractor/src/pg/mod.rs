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

use crate::{ActorCell, ActorId, GroupName, ScopeName, SupervisionEvent};

/// Key to set the default scope
pub const DEFAULT_SCOPE: &str = "__default_scope__";

/// Key to monitor all of the scopes
pub const ALL_SCOPES_NOTIFICATION: &str = "__world_scope__";

/// Key to monitor all of the groups in a scope
pub const ALL_GROUPS_NOTIFICATION: &str = "__world_group_";

#[cfg(test)]
mod tests;

/// Represents a change in a process group's membership
#[derive(Clone)]
pub enum GroupChangeMessage {
    /// Some actors joined a group
    Join(ScopeName, GroupName, Vec<ActorCell>),
    /// Some actors left a group
    Leave(ScopeName, GroupName, Vec<ActorCell>),
}

impl GroupChangeMessage {
    /// Retrieve the group that changed
    pub fn get_group(&self) -> GroupName {
        match self {
            Self::Join(_, name, _) => name.clone(),
            Self::Leave(_, name, _) => name.clone(),
        }
    }

    /// Retrieve the name of the scope in which the group change took place
    pub fn get_scope(&self) -> ScopeName {
        match self {
            Self::Join(scope, _, _) => scope.to_string(),
            Self::Leave(scope, _, _) => scope.to_string(),
        }
    }
}

struct PgState {
    map: Arc<DashMap<(ScopeName, GroupName), HashMap<ActorId, ActorCell>>>,
    listeners: Arc<DashMap<(ScopeName, GroupName), Vec<ActorCell>>>,
}

static PG_MONITOR: OnceCell<PgState> = OnceCell::new();

fn get_monitor<'a>() -> &'a PgState {
    PG_MONITOR.get_or_init(|| PgState {
        map: Arc::new(DashMap::new()),
        listeners: Arc::new(DashMap::new()),
    })
}

/// Join actors to the group `group` in the default scope
///
/// * `group` - The statically named group. Will be created if first actors to join
/// * `actors` - The list of [crate::Actor]s to add to the group
pub fn join(group: GroupName, actors: Vec<ActorCell>) {
    let monitor = get_monitor();
    // insert into the monitor group
    match monitor
        .map
        .entry((DEFAULT_SCOPE.to_owned().clone(), group.clone()))
    {
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
    if let Some(listeners) = monitor
        .listeners
        .get(&(DEFAULT_SCOPE.to_owned(), group.clone()))
    {
        for listener in listeners.value() {
            let _ = listener.send_supervisor_evt(SupervisionEvent::ProcessGroupChanged(
                GroupChangeMessage::Join(DEFAULT_SCOPE.to_owned(), group.clone(), actors.clone()),
            ));
        }
    }

    // notify the world monitors
    let world_monitor_keys = get_world_monitor_keys();
    for key in world_monitor_keys {
        if let Some(listeners) = monitor.listeners.get(&key) {
            for listener in listeners.value() {
                let _ = listener.send_supervisor_evt(SupervisionEvent::ProcessGroupChanged(
                    GroupChangeMessage::Join(
                        DEFAULT_SCOPE.to_owned(),
                        group.clone(),
                        actors.clone(),
                    ),
                ));
            }
        }
    }
}

/// Join actors to the group `group` within the scope `scope`
///
/// * `scope` - the statically named scope. Will be created if first actors join
/// * `group` - The statically named group. Will be created if first actors to join
/// * `actors` - The list of [crate::Actor]s to add to the group
pub fn join_with_named_scope(scope: ScopeName, group: GroupName, actors: Vec<ActorCell>) {
    let monitor = get_monitor();
    // insert into the monitor group
    match monitor.map.entry((scope.to_owned().clone(), group.clone())) {
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
    if let Some(listeners) = monitor.listeners.get(&(scope.to_owned(), group.clone())) {
        for listener in listeners.value() {
            let _ = listener.send_supervisor_evt(SupervisionEvent::ProcessGroupChanged(
                GroupChangeMessage::Join(scope.to_owned(), group.clone(), actors.clone()),
            ));
        }
    }
    // notify the world monitors
    let world_monitor_keys = get_world_monitor_keys();
    for key in world_monitor_keys {
        if let Some(listeners) = monitor.listeners.get(&key) {
            for listener in listeners.value() {
                let _ = listener.send_supervisor_evt(SupervisionEvent::ProcessGroupChanged(
                    GroupChangeMessage::Join(scope.to_owned(), group.clone(), actors.clone()),
                ));
            }
        }
    }
}

/// Leaves the specified [crate::Actor]s from the PG group in the default scope
///
/// * `group` - The statically named group
/// * `actors` - The list of actors to remove from the group
pub fn leave(group: GroupName, actors: Vec<ActorCell>) {
    let monitor = get_monitor();
    match monitor.map.entry((DEFAULT_SCOPE.to_owned(), group.clone())) {
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
            if let Some(listeners) = monitor
                .listeners
                .get(&(DEFAULT_SCOPE.to_owned(), group.clone()))
            {
                for listener in listeners.value() {
                    let _ = listener.send_supervisor_evt(SupervisionEvent::ProcessGroupChanged(
                        GroupChangeMessage::Leave(
                            DEFAULT_SCOPE.to_owned(),
                            group.clone(),
                            actors.clone(),
                        ),
                    ));
                }
            }
            // notify the world monitors
            if let Some(listeners) = monitor
                .listeners
                .get(&(DEFAULT_SCOPE.to_owned(), ALL_GROUPS_NOTIFICATION.to_owned()))
            {
                for listener in listeners.value() {
                    let _ = listener.send_supervisor_evt(SupervisionEvent::ProcessGroupChanged(
                        GroupChangeMessage::Leave(
                            DEFAULT_SCOPE.to_owned(),
                            group.clone(),
                            actors.clone(),
                        ),
                    ));
                }
            }
        }
    }
}

/// Leaves the specified [crate::Actor]s from the PG group within the scope `scope`
///
/// * `scope` - The statically named scope
/// * `group` - The statically named group
/// * `actors` - The list of actors to remove from the group
pub fn leave_with_named_scope(scope: ScopeName, group: GroupName, actors: Vec<ActorCell>) {
    let monitor = get_monitor();
    match monitor.map.entry((scope.to_owned(), group.clone())) {
        Vacant(_) => {}
        Occupied(mut occupied) => {
            let mut_ref = occupied.get_mut();
            for actor in actors.iter() {
                mut_ref.remove(&actor.get_id());
            }

            // the scope and group tuple is empty, remove it
            if mut_ref.is_empty() {
                occupied.remove();
            }
            if let Some(listeners) = monitor.listeners.get(&(scope.to_owned(), group.clone())) {
                for listener in listeners.value() {
                    let _ = listener.send_supervisor_evt(SupervisionEvent::ProcessGroupChanged(
                        GroupChangeMessage::Leave(scope.to_owned(), group.clone(), actors.clone()),
                    ));
                }
            }
            // notify the world monitors
            let world_monitor_keys = get_world_monitor_keys();
            for key in world_monitor_keys {
                if let Some(listeners) = monitor.listeners.get(&key) {
                    for listener in listeners.value() {
                        let _ = listener.send_supervisor_evt(
                            SupervisionEvent::ProcessGroupChanged(GroupChangeMessage::Leave(
                                scope.to_owned(),
                                group.clone(),
                                actors.clone(),
                            )),
                        );
                    }
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
    for ((scope_name, group_name), cell) in removal_events.into_iter() {
        if let Some(this_listeners) = all_listeners.get(&(scope_name.clone(), group_name.clone())) {
            this_listeners.iter().for_each(|listener| {
                let _ = listener.send_supervisor_evt(SupervisionEvent::ProcessGroupChanged(
                    GroupChangeMessage::Leave(
                        scope_name.clone(),
                        group_name.clone(),
                        vec![cell.clone()],
                    ),
                ));
            });
        }
        // notify the world monitors
        if let Some(listeners) =
            all_listeners.get(&(scope_name.clone(), ALL_GROUPS_NOTIFICATION.to_owned()))
        {
            for listener in listeners.value() {
                let _ = listener.send_supervisor_evt(SupervisionEvent::ProcessGroupChanged(
                    GroupChangeMessage::Leave(
                        scope_name.clone(),
                        group_name.clone(),
                        vec![cell.clone()],
                    ),
                ));
            }
        }
    }

    // Cleanup empty groups
    for group in empty_groups {
        map.remove(&group);
    }
}

/// Returns all actors running on the local node in the group `group`
/// in the default scope.
///
/// * `group_name` - Either a statically named group
///
/// Returns a [`Vec<ActorCell>`] representing the members of this paging group
pub fn get_local_members(group_name: &GroupName) -> Vec<ActorCell> {
    let monitor = get_monitor();
    if let Some(actors) = monitor
        .map
        .get(&(DEFAULT_SCOPE.to_owned(), group_name.to_owned()))
    {
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

/// Returns all actors running on the local node in the group `group`
/// in scope `scope`
///
/// * `scope_name` - A statically named scope
/// * `group_name` - Either a statically named group
///
/// Returns a [`Vec<ActorCell>`] representing the members of this paging group
pub fn get_local_members_with_scope(scope: &ScopeName, group_name: &GroupName) -> Vec<ActorCell> {
    let monitor = get_monitor();
    if let Some(actors) = monitor.map.get(&(scope.to_owned(), group_name.to_owned())) {
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

/// Returns all the actors running on any node in the group `group`
/// in the default scope.
///
/// * `group_name` - Either a statically named group or scope
///
/// Returns a [`Vec<ActorCell>`] with the member actors
pub fn get_members(group_name: &GroupName) -> Vec<ActorCell> {
    let monitor = get_monitor();
    if let Some(actors) = monitor
        .map
        .get(&(DEFAULT_SCOPE.to_owned(), group_name.to_owned()))
    {
        actors.value().values().cloned().collect::<Vec<_>>()
    } else {
        vec![]
    }
}

/// Returns all the actors running on any node in the group `group`
/// in the scope `scope`.
///
/// * `group_name` - Either a statically named group or scope
///
/// Returns a [`Vec<ActorCell>`] with the member actors
pub fn get_members_with_scope(scope: &ScopeName, group_name: &GroupName) -> Vec<ActorCell> {
    let monitor = get_monitor();
    if let Some(actors) = monitor.map.get(&(scope.to_owned(), group_name.to_owned())) {
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
        .map(|(_scope, group)| group.clone())
        .collect::<Vec<_>>()
}

/// Returns a list of all known groups in scope `scope`
///
/// * `scope` - The scope to retrieve the groups from
///
/// Returns a [`Vec<GroupName>`] representing all the registered group names
/// in `scope`
pub fn which_groups_in_named_scope(scope: &ScopeName) -> Vec<GroupName> {
    let monitor = get_monitor();
    monitor
        .map
        .iter()
        .map(|kvp| kvp.key().clone())
        .filter(|(scope_name, _group)| scope == scope_name)
        .map(|(_scope_name, group)| group.clone())
        .collect::<Vec<_>>()
}

/// Returns a list of all known scope-group combinations.
///
/// Returns a [`Vec<(ScopeName,GroupName)>`] representing all the registered
/// combinations that form an identifying tuple
pub fn which_scopes_and_groups() -> Vec<(ScopeName, GroupName)> {
    let monitor = get_monitor();
    monitor
        .map
        .iter()
        .map(|kvp| kvp.key().clone())
        .collect::<Vec<_>>()
}

/// Returns a list of all known scopes
///
/// Returns a [`Vec<ScopeName>`] representing all the registered scopes
pub fn which_scopes() -> Vec<ScopeName> {
    let monitor = get_monitor();
    monitor
        .map
        .iter()
        .map(|kvp| kvp.key().clone())
        .map(|(scope, _group)| scope.clone())
        .collect::<Vec<_>>()
}

/// Subscribes the provided [crate::Actor] to the group in the default scope
/// for updates
///
/// * `group_name` - The group to monitor
/// * `actor` - The [ActorCell] representing who will receive updates
pub fn monitor(group_name: GroupName, actor: ActorCell) {
    let monitor = get_monitor();
    match monitor
        .listeners
        .entry((DEFAULT_SCOPE.to_owned(), group_name))
    {
        Occupied(mut occupied) => occupied.get_mut().push(actor),
        Vacant(vacancy) => {
            vacancy.insert(vec![actor]);
        }
    }
}

/// Subscribes the provided [crate::Actor] to the scope for updates
///
/// * `scope` - the scope to monitor
/// * `actor` - The [ActorCell] representing who will receive updates
pub fn monitor_scope(scope: ScopeName, actor: ActorCell) {
    let monitor = get_monitor();

    // Register at world monitor first
    match monitor
        .listeners
        .entry((scope.clone(), ALL_GROUPS_NOTIFICATION.to_owned()))
    {
        Occupied(mut occupied) => occupied.get_mut().push(actor.clone()),
        Vacant(vacancy) => {
            vacancy.insert(vec![actor.clone()]);
        }
    }

    let groups_in_scope = which_groups_in_named_scope(&scope);
    for group in groups_in_scope {
        match monitor.listeners.entry((scope.to_owned(), group)) {
            Occupied(mut occupied) => occupied.get_mut().push(actor.clone()),
            Vacant(vacancy) => {
                vacancy.insert(vec![actor.clone()]);
            }
        }
    }
}

/// Unsubscribes the provided [crate::Actor] for updates from the group
/// in default scope
///
/// * `group_name` - The group to demonitor
/// * `actor` - The [ActorCell] representing who will no longer receive updates
pub fn demonitor(group_name: GroupName, actor: ActorId) {
    let monitor = get_monitor();
    if let Occupied(mut entry) = monitor
        .listeners
        .entry((DEFAULT_SCOPE.to_owned(), group_name))
    {
        let mut_ref = entry.get_mut();
        mut_ref.retain(|a| a.get_id() != actor);
        if mut_ref.is_empty() {
            entry.remove();
        }
    }
}

/// Unsubscribes the provided [crate::Actor] from the scope for updates
///
/// * `scope` - The scope to demonitor
/// * `actor` - The [ActorCell] representing who will no longer receive updates
pub fn demonitor_scope(scope: ScopeName, actor: ActorId) {
    let monitor = get_monitor();
    let groups_in_scope = which_groups_in_named_scope(&scope);
    for group in groups_in_scope {
        if let Occupied(mut entry) = monitor.listeners.entry((scope.to_owned(), group)) {
            let mut_ref = entry.get_mut();
            mut_ref.retain(|a| a.get_id() != actor);
            if mut_ref.is_empty() {
                entry.remove();
            }
        }
    }
}

/// Remove the specified [ActorId] from monitoring all groups it might be in.
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

/// Gets the keys for the world monitors.
///
/// Returns a `Vec<ScopeName, GroupName>` represending all registered tuples
/// for which ane of the values is equivalent to one of the world_monitor_keys
fn get_world_monitor_keys() -> Vec<(ScopeName, GroupName)> {
    let monitor = get_monitor();
    let mut world_monitor_keys = monitor
        .listeners
        .iter()
        .map(|kvp| kvp.key().clone())
        .filter(|(scope_name, group_name)| {
            scope_name == ALL_SCOPES_NOTIFICATION || group_name == ALL_GROUPS_NOTIFICATION
        })
        .collect::<Vec<(String, String)>>();
    world_monitor_keys.sort_unstable();
    world_monitor_keys.dedup();
    world_monitor_keys
}
