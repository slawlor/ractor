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
//!
//! ## Examples
//!
//! ```rust
//! use ractor::{Actor, ActorRef, ActorProcessingErr};
//! use ractor::pg;
//!
//! struct ExampleActor;
//!
//! #[cfg_attr(feature = "async-trait", ractor::async_trait)]
//! impl Actor for ExampleActor {
//!     type Msg = ();
//!     type State = ();
//!     type Arguments = ();
//!
//!     async fn pre_start(&self, _myself: ActorRef<Self::Msg>, _args: Self::Arguments) -> Result<Self::State, ActorProcessingErr> {
//!         println!("Starting");
//!         Ok(())
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() {
//!     let (actor, handle) = Actor::spawn(None, ExampleActor, ()).await.expect("Failed to startup dummy actor");
//!     let group = "the_group".to_string();    
//!
//!     // Join the actor to a group. This is also commonly done in `pre_start` or `post_start`
//!     // of the actor itself without having to do it externally by some coordinator
//!     pg::join(group.clone(), vec![actor.get_cell()]);
//!     // Retrieve the pg group membership
//!     let members = pg::get_members(&group);
//!     // Send a message to the up-casted actor
//!     let the_actor: ActorRef<()> = members.get(0).unwrap().clone().into();
//!     ractor::cast!(the_actor, ()).expect("Failed to send message");
//!
//!     // wait for actor exit
//!     actor.stop(None);
//!     handle.await.unwrap();
//!     
//!     // The actor will automatically be removed from the group upon shutdown.
//!     let members = pg::get_members(&group);
//!     assert_eq!(members.len(), 0);
//! }
//! ```

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

/// Represents the combination of a `ScopeName` and a `GroupName`
/// that uniquely identifies a specific group in a specific scope
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScopeGroupKey {
    /// the `ScopeName`
    scope: ScopeName,
    /// The `GroupName`
    group: GroupName,
}

impl ScopeGroupKey {
    /// Retrieve the struct's scope
    pub fn get_scope(&self) -> ScopeName {
        self.scope.to_owned()
    }
    /// Retrieve the struct's group
    pub fn get_group(&self) -> GroupName {
        self.group.to_owned()
    }
}

struct PgState {
    map: Arc<DashMap<ScopeGroupKey, HashMap<ActorId, ActorCell>>>,
    index: Arc<DashMap<ScopeName, Vec<GroupName>>>,
    listeners: Arc<DashMap<ScopeGroupKey, Vec<ActorCell>>>,
}

static PG_MONITOR: OnceCell<PgState> = OnceCell::new();

fn get_monitor<'a>() -> &'a PgState {
    PG_MONITOR.get_or_init(|| PgState {
        map: Arc::new(DashMap::new()),
        index: Arc::new(DashMap::new()),
        listeners: Arc::new(DashMap::new()),
    })
}

/// Join actors to the group `group` in the default scope
///
/// * `group` - The named group. Will be created if first actors to join
/// * `actors` - The list of [crate::Actor]s to add to the group
pub fn join(group: GroupName, actors: Vec<ActorCell>) {
    join_scoped(DEFAULT_SCOPE.to_owned(), group, actors);
}

/// Join actors to the group `group` within the scope `scope`
///
/// * `scope` - The named scope. Will be created if first actors to join
/// * `group` - The named group. Will be created if first actors to join
/// * `actors` - The list of [crate::Actor]s to add to the group
pub fn join_scoped(scope: ScopeName, group: GroupName, actors: Vec<ActorCell>) {
    let key = ScopeGroupKey {
        scope: scope.to_owned(),
        group: group.to_owned(),
    };
    let monitor = get_monitor();

    // lock the `PgState`'s `map` and `index` DashMaps.
    let monitor_map = monitor.map.entry(key.to_owned());
    let monitor_idx = monitor.index.entry(scope.to_owned());

    // insert into the monitor group
    match monitor_map {
        Occupied(mut occupied_map) => {
            let oref = occupied_map.get_mut();
            for actor in actors.iter() {
                oref.insert(actor.get_id(), actor.clone());
            }
            match monitor_idx {
                Occupied(mut occupied_idx) => {
                    let oref = occupied_idx.get_mut();
                    if !oref.contains(&group) {
                        oref.push(group.to_owned());
                    }
                }
                Vacant(vacancy) => {
                    vacancy.insert(vec![group.to_owned()]);
                }
            }
        }
        Vacant(vacancy) => {
            let map = actors
                .iter()
                .map(|a| (a.get_id(), a.clone()))
                .collect::<HashMap<_, _>>();
            vacancy.insert(map);
            match monitor_idx {
                Occupied(mut occupied_idx) => {
                    let oref = occupied_idx.get_mut();
                    if !oref.contains(&group) {
                        oref.push(group.to_owned());
                    }
                }
                Vacant(vacancy) => {
                    vacancy.insert(vec![group.to_owned()]);
                }
            }
        }
    }

    // notify supervisors
    if let Some(listeners) = monitor.listeners.get(&key) {
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
/// * `group` - A named group
/// * `actors` - The list of actors to remove from the group
pub fn leave(group: GroupName, actors: Vec<ActorCell>) {
    leave_scoped(DEFAULT_SCOPE.to_owned(), group, actors);
}

/// Leaves the specified [crate::Actor]s from the PG group within the scope `scope`
///
/// * `scope` - A named scope
/// * `group` - A named group
/// * `actors` - The list of actors to remove from the group
pub fn leave_scoped(scope: ScopeName, group: GroupName, actors: Vec<ActorCell>) {
    let key = ScopeGroupKey {
        scope: scope.to_owned(),
        group: group.to_owned(),
    };
    let monitor = get_monitor();

    // lock the `PgState`'s `map` and `index` DashMaps.
    let monitor_map = monitor.map.entry(key.to_owned());
    let monitor_idx = monitor.index.get_mut(&scope);

    match monitor_map {
        Vacant(_) => {}
        Occupied(mut occupied_map) => {
            let mut_ref = occupied_map.get_mut();
            for actor in actors.iter() {
                mut_ref.remove(&actor.get_id());
            }

            // if the scope and group tuple is empty, remove it
            if mut_ref.is_empty() {
                occupied_map.remove();
            }

            // remove the group and possibly the scope from the monitor's index
            if let Some(mut groups_in_scope) = monitor_idx {
                groups_in_scope.retain(|group_name| group_name != &group);
                if groups_in_scope.is_empty() {
                    // drop the `RefMut` to prevent a `DashMap` deadlock
                    drop(groups_in_scope);
                    monitor.index.remove(&scope);
                }
            }

            if let Some(listeners) = monitor.listeners.get(&key) {
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

    let mut empty_scope_group_keys = vec![];
    let mut removal_events = HashMap::new();

    for mut kv in map.iter_mut() {
        if let Some(actor_cell) = kv.value_mut().remove(&actor) {
            removal_events.insert(kv.key().clone(), actor_cell);
        }
        if kv.value().is_empty() {
            empty_scope_group_keys.push(kv.key().clone());
        }
    }

    // notify the listeners
    let all_listeners = pg_monitor.listeners.clone();
    for (scope_and_group, cell) in removal_events.into_iter() {
        if let Some(this_listeners) = all_listeners.get(&scope_and_group) {
            this_listeners.iter().for_each(|listener| {
                let _ = listener.send_supervisor_evt(SupervisionEvent::ProcessGroupChanged(
                    GroupChangeMessage::Leave(
                        scope_and_group.scope.clone(),
                        scope_and_group.group.clone(),
                        vec![cell.clone()],
                    ),
                ));
            });
        }
        // notify the world monitors
        let world_monitor_scoped = ScopeGroupKey {
            scope: scope_and_group.scope,
            group: ALL_GROUPS_NOTIFICATION.to_owned(),
        };
        if let Some(listeners) = all_listeners.get(&world_monitor_scoped) {
            for listener in listeners.value() {
                let _ = listener.send_supervisor_evt(SupervisionEvent::ProcessGroupChanged(
                    GroupChangeMessage::Leave(
                        world_monitor_scoped.scope.clone(),
                        world_monitor_scoped.group.clone(),
                        vec![cell.clone()],
                    ),
                ));
            }
        }
    }

    // Cleanup empty groups
    for scope_group_key in empty_scope_group_keys {
        map.remove(&scope_group_key);
        if let Some(mut groups_in_scope) = pg_monitor.index.get_mut(&scope_group_key.scope) {
            groups_in_scope.retain(|group| group != &scope_group_key.group);
        }
    }
}

/// Returns all actors running on the local node in the group `group`
/// in the default scope.
///
/// * `group` - A named group
///
/// Returns a [`Vec<ActorCell>`] representing the members of this paging group
pub fn get_local_members(group: &GroupName) -> Vec<ActorCell> {
    get_scoped_local_members(&DEFAULT_SCOPE.to_owned(), group)
}

/// Returns all actors running on the local node in the group `group`
/// in scope `scope`
///
/// * `scope_name` - A named scope
/// * `group_name` - A named group
///
/// Returns a [`Vec<ActorCell>`] representing the members of this paging group
pub fn get_scoped_local_members(scope: &ScopeName, group: &GroupName) -> Vec<ActorCell> {
    let key = ScopeGroupKey {
        scope: scope.to_owned(),
        group: group.to_owned(),
    };
    let monitor = get_monitor();
    if let Some(actors) = monitor.map.get(&key) {
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
/// * `group_name` - A named group
///
/// Returns a [`Vec<ActorCell>`] with the member actors
pub fn get_members(group_name: &GroupName) -> Vec<ActorCell> {
    get_scoped_members(&DEFAULT_SCOPE.to_owned(), group_name)
}

/// Returns all the actors running on any node in the group `group`
/// in the scope `scope`.
///
/// * `scope` - A named scope
/// * `group` - A named group
///
/// Returns a [`Vec<ActorCell>`] with the member actors
pub fn get_scoped_members(scope: &ScopeName, group: &GroupName) -> Vec<ActorCell> {
    let key = ScopeGroupKey {
        scope: scope.to_owned(),
        group: group.to_owned(),
    };
    let monitor = get_monitor();
    if let Some(actors) = monitor.map.get(&key) {
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
    let mut groups = monitor
        .map
        .iter()
        .map(|kvp| kvp.key().group.to_owned())
        .collect::<Vec<_>>();
    groups.sort_unstable();
    groups.dedup();
    groups
}

/// Returns a list of all known groups in scope `scope`
///
/// * `scope` - The scope to retrieve the groups from
///
/// Returns a [`Vec<GroupName>`] representing all the registered group names
/// in `scope`
pub fn which_scoped_groups(scope: &ScopeName) -> Vec<GroupName> {
    let monitor = get_monitor();
    match monitor.index.get(scope) {
        Some(groups) => groups.to_owned(),
        None => vec![],
    }
}

/// Returns a list of all known scope-group combinations.
///
/// Returns a [`Vec<ScopeGroupKey>`] representing all the registered
/// combinations that form an identifying tuple
pub fn which_scopes_and_groups() -> Vec<ScopeGroupKey> {
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
        .map(|kvp| {
            let key = kvp.key();
            key.scope.to_owned()
        })
        .collect::<Vec<_>>()
}

/// Subscribes the provided [crate::Actor] to the group in the default scope
/// for updates
///
/// * `group_name` - The group to monitor
/// * `actor` - The [ActorCell] representing who will receive updates
pub fn monitor(group: GroupName, actor: ActorCell) {
    let key = ScopeGroupKey {
        scope: DEFAULT_SCOPE.to_owned(),
        group,
    };
    let monitor = get_monitor();
    match monitor.listeners.entry(key) {
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
    let key = ScopeGroupKey {
        scope: scope.to_owned(),
        group: ALL_GROUPS_NOTIFICATION.to_owned(),
    };
    let monitor = get_monitor();

    // Register at world monitor first
    match monitor.listeners.entry(key) {
        Occupied(mut occupied) => occupied.get_mut().push(actor.clone()),
        Vacant(vacancy) => {
            vacancy.insert(vec![actor.clone()]);
        }
    }

    let groups_in_scope = which_scoped_groups(&scope);
    for group in groups_in_scope {
        let key = ScopeGroupKey {
            scope: scope.to_owned(),
            group,
        };
        match monitor.listeners.entry(key) {
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
    let key = ScopeGroupKey {
        scope: DEFAULT_SCOPE.to_owned(),
        group: group_name,
    };
    let monitor = get_monitor();
    if let Occupied(mut entry) = monitor.listeners.entry(key) {
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
    let groups_in_scope = which_scoped_groups(&scope);
    for group in groups_in_scope {
        let key = ScopeGroupKey {
            scope: scope.to_owned(),
            group,
        };
        if let Occupied(mut entry) = monitor.listeners.entry(key) {
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
/// Returns a `Vec<ScopeGroupKey>` represending all registered tuples
/// for which ane of the values is equivalent to one of the world_monitor_keys
fn get_world_monitor_keys() -> Vec<ScopeGroupKey> {
    let monitor = get_monitor();
    let mut world_monitor_keys = monitor
        .listeners
        .iter()
        .filter_map(|kvp| {
            let key = kvp.key().clone();
            if key.scope == ALL_SCOPES_NOTIFICATION || key.group == ALL_GROUPS_NOTIFICATION {
                Some(key)
            } else {
                None
            }
        })
        .collect::<Vec<ScopeGroupKey>>();
    world_monitor_keys.sort_unstable();
    world_monitor_keys.dedup();
    world_monitor_keys
}
