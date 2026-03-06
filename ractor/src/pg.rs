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
//! use ractor::pg;
//! use ractor::Actor;
//! use ractor::ActorProcessingErr;
//! use ractor::ActorRef;
//!
//! struct ExampleActor;
//!
//! #[cfg_attr(feature = "async-trait", ractor::async_trait)]
//! impl Actor for ExampleActor {
//!     type Msg = ();
//!     type State = ();
//!     type Arguments = ();
//!
//!     async fn pre_start(
//!         &self,
//!         _myself: ActorRef<Self::Msg>,
//!         _args: Self::Arguments,
//!     ) -> Result<Self::State, ActorProcessingErr> {
//!         println!("Starting");
//!         Ok(())
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() {
//!     let (actor, handle) = Actor::spawn(None, ExampleActor, ())
//!         .await
//!         .expect("Failed to startup dummy actor");
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
use std::collections::HashSet;

use dashmap::mapref::entry::Entry::Occupied;
use dashmap::DashMap;
use once_cell::sync::OnceCell;

use crate::ActorCell;
use crate::ActorId;
use crate::ActorStatus;
use crate::GroupName;
use crate::ScopeName;
use crate::SupervisionEvent;

/// Key to set the default scope
pub const DEFAULT_SCOPE: &str = "__default_scope__";

/// Key to monitor all of the scopes
pub const ALL_SCOPES_NOTIFICATION: &str = "__world_scope__";

/// Key to monitor all of the groups in a scope
pub const ALL_GROUPS_NOTIFICATION: &str = "__world_group_";

#[cfg(test)]
mod tests;

/// Represents a change in a process group's membership
#[derive(Clone, Debug)]
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
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

/// Internal state for a single process group, bundling members and per-group
/// listeners into a single atomically-accessible unit.
#[derive(Default)]
struct GroupState {
    /// The actors that are members of this group
    members: HashMap<ActorId, ActorCell>,
    /// Actors monitoring this specific group for changes
    listeners: Vec<ActorCell>,
}

struct PgState {
    /// Maps (scope, group) to the group's state (members + per-group listeners)
    map: DashMap<ScopeGroupKey, GroupState>,
    /// Secondary index: scope -> set of group names that have members
    index: DashMap<ScopeName, HashSet<GroupName>>,
    /// Scope-level and global monitors (sentinel keys only)
    world_listeners: DashMap<ScopeGroupKey, Vec<ActorCell>>,
}

static PG_MONITOR: OnceCell<PgState> = OnceCell::new();

fn get_monitor<'a>() -> &'a PgState {
    PG_MONITOR.get_or_init(|| PgState {
        map: DashMap::new(),
        index: DashMap::new(),
        world_listeners: DashMap::new(),
    })
}

/// Sends notifications to scope-level and global world listeners.
fn notify_world_listeners(
    monitor: &PgState,
    scope: &ScopeName,
    group: &GroupName,
    actors: &[ActorCell],
    is_join: bool,
) {
    let scoped_key = ScopeGroupKey {
        scope: scope.to_owned(),
        group: ALL_GROUPS_NOTIFICATION.to_owned(),
    };
    let global_key = ScopeGroupKey {
        scope: ALL_SCOPES_NOTIFICATION.to_owned(),
        group: ALL_GROUPS_NOTIFICATION.to_owned(),
    };

    for key in [scoped_key, global_key] {
        let listeners = monitor
            .world_listeners
            .get(&key)
            .map(|entry| entry.value().clone());
        if let Some(listeners) = listeners {
            let change = if is_join {
                GroupChangeMessage::Join(scope.to_owned(), group.clone(), actors.to_vec())
            } else {
                GroupChangeMessage::Leave(scope.to_owned(), group.clone(), actors.to_vec())
            };
            for listener in &listeners {
                let _ = listener
                    .send_supervisor_evt(SupervisionEvent::ProcessGroupChanged(change.clone()));
            }
        }
    }
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

    // Filter out actors that are already stopping or stopped
    let actors: Vec<ActorCell> = actors
        .into_iter()
        .filter(|a| (a.get_status() as u8) <= (ActorStatus::Draining as u8))
        .collect();

    if actors.is_empty() {
        return;
    }

    // Lock map entry, insert members, clone per-group listeners, then drop the guard
    let listeners = {
        let mut entry = monitor.map.entry(key).or_default();
        let gs = entry.value_mut();
        for actor in actors.iter() {
            gs.members.insert(actor.get_id(), actor.clone());
        }
        gs.listeners.clone()
    };

    // Update index
    monitor
        .index
        .entry(scope.to_owned())
        .or_default()
        .insert(group.to_owned());

    // Notify per-group listeners (guard already dropped)
    for listener in &listeners {
        let _ = listener.send_supervisor_evt(SupervisionEvent::ProcessGroupChanged(
            GroupChangeMessage::Join(scope.to_owned(), group.clone(), actors.clone()),
        ));
    }

    // Notify world listeners
    notify_world_listeners(monitor, &scope, &group, &actors, true);
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

    // Lock map entry, remove members, clone listeners, check emptiness
    let result = if let Occupied(mut occupied) = monitor.map.entry(key) {
        let gs = occupied.get_mut();
        for actor in actors.iter() {
            gs.members.remove(&actor.get_id());
        }
        let listeners = gs.listeners.clone();
        let all_members_left = gs.members.is_empty();
        let fully_empty = all_members_left && gs.listeners.is_empty();
        if fully_empty {
            occupied.remove();
        }
        Some((listeners, all_members_left))
    } else {
        None
    };

    let Some((listeners, all_members_left)) = result else {
        return;
    };

    // Update index only if all members left
    if all_members_left {
        if let Some(mut groups) = monitor.index.get_mut(&scope) {
            groups.remove(&group);
            if groups.is_empty() {
                drop(groups);
                monitor.index.remove(&scope);
            }
        }
    }

    // Notify per-group listeners (guard already dropped)
    for listener in &listeners {
        let _ = listener.send_supervisor_evt(SupervisionEvent::ProcessGroupChanged(
            GroupChangeMessage::Leave(scope.to_owned(), group.clone(), actors.clone()),
        ));
    }

    // Notify world listeners
    notify_world_listeners(monitor, &scope, &group, &actors, false);
}

/// Leave all groups for a specific [ActorId].
/// Used only during actor shutdown
pub(crate) fn leave_all(actor: ActorId) {
    let monitor = get_monitor();

    // Phase 1: iterate, remove actor from members, collect notification info
    let mut removal_events: Vec<(ScopeGroupKey, ActorCell, Vec<ActorCell>)> = vec![];
    let mut empty_member_keys: Vec<ScopeGroupKey> = vec![];

    for mut kv in monitor.map.iter_mut() {
        let key = kv.key().clone();
        let gs = kv.value_mut();
        if let Some(actor_cell) = gs.members.remove(&actor) {
            let listeners = gs.listeners.clone();
            removal_events.push((key.clone(), actor_cell, listeners));
        }
        if gs.members.is_empty() {
            empty_member_keys.push(key);
        }
    }

    // Phase 2: clean up empty entries (re-check under lock to handle concurrent joins)
    for key in empty_member_keys {
        if let Occupied(entry) = monitor.map.entry(key.clone()) {
            if entry.get().members.is_empty() {
                // Update index
                if let Some(mut groups) = monitor.index.get_mut(&key.scope) {
                    groups.remove(&key.group);
                    if groups.is_empty() {
                        drop(groups);
                        monitor.index.remove(&key.scope);
                    }
                }
                // Only remove map entry if listeners are also empty
                if entry.get().listeners.is_empty() {
                    entry.remove();
                }
            }
        }
    }

    // Phase 3: send notifications (outside all locks)
    for (scope_and_group, cell, per_group_listeners) in removal_events.iter() {
        // Notify per-group listeners
        for listener in per_group_listeners {
            let _ = listener.send_supervisor_evt(SupervisionEvent::ProcessGroupChanged(
                GroupChangeMessage::Leave(
                    scope_and_group.scope.clone(),
                    scope_and_group.group.clone(),
                    vec![cell.clone()],
                ),
            ));
        }

        // Notify world listeners (scoped + global)
        notify_world_listeners(
            monitor,
            &scope_and_group.scope,
            &scope_and_group.group,
            std::slice::from_ref(cell),
            false,
        );
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
    if let Some(gs) = monitor.map.get(&key) {
        gs.value()
            .members
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
    if let Some(gs) = monitor.map.get(&key) {
        gs.value().members.values().cloned().collect::<Vec<_>>()
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
        .filter(|kvp| !kvp.value().members.is_empty())
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
        Some(groups) => groups.iter().cloned().collect(),
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
        .filter(|kvp| !kvp.value().members.is_empty())
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
        .filter(|kvp| !kvp.value().members.is_empty())
        .map(|kvp| kvp.key().scope.to_owned())
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
    monitor.map.entry(key).or_default().listeners.push(actor);
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
    // Register ONLY in world_listeners (not per-group) to avoid duplicate notifications
    monitor.world_listeners.entry(key).or_default().push(actor);
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
    if let Some(mut gs) = monitor.map.get_mut(&key) {
        gs.listeners.retain(|a| a.get_id() != actor);
    }
}

/// Unsubscribes the provided [crate::Actor] from the scope for updates
///
/// * `scope` - The scope to demonitor
/// * `actor` - The [ActorCell] representing who will no longer receive updates
pub fn demonitor_scope(scope: ScopeName, actor: ActorId) {
    let key = ScopeGroupKey {
        scope: scope.to_owned(),
        group: ALL_GROUPS_NOTIFICATION.to_owned(),
    };
    let monitor = get_monitor();
    if let Occupied(mut entry) = monitor.world_listeners.entry(key) {
        let listeners = entry.get_mut();
        listeners.retain(|a| a.get_id() != actor);
        if listeners.is_empty() {
            entry.remove();
        }
    }
}

/// Remove the specified [ActorId] from monitoring all groups it might be in.
/// Used only during actor shutdown
pub(crate) fn demonitor_all(actor: ActorId) {
    let monitor = get_monitor();

    // Remove from per-group listeners and track potentially empty entries
    let mut maybe_empty = vec![];
    for mut kv in monitor.map.iter_mut() {
        let gs = kv.value_mut();
        gs.listeners.retain(|a| a.get_id() != actor);
        if gs.members.is_empty() && gs.listeners.is_empty() {
            maybe_empty.push(kv.key().clone());
        }
    }

    // Clean up fully empty entries
    for key in maybe_empty {
        if let Occupied(entry) = monitor.map.entry(key.clone()) {
            if entry.get().members.is_empty() && entry.get().listeners.is_empty() {
                entry.remove();
                if let Some(mut groups) = monitor.index.get_mut(&key.scope) {
                    groups.remove(&key.group);
                    if groups.is_empty() {
                        drop(groups);
                        monitor.index.remove(&key.scope);
                    }
                }
            }
        }
    }

    // Remove from world listeners
    let mut empty_world_keys = vec![];
    for mut kv in monitor.world_listeners.iter_mut() {
        kv.value_mut().retain(|a| a.get_id() != actor);
        if kv.value().is_empty() {
            empty_world_keys.push(kv.key().clone());
        }
    }
    for key in empty_world_keys {
        monitor.world_listeners.remove(&key);
    }
}
