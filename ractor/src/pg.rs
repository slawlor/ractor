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
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;

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

/// The process-group relationships owned by one actor.
///
/// Ordinary mutations hold the forward group entry while updating this reverse
/// index. Shutdown first publishes `Stopping`, drains this index, and then
/// removes the forward entries; registrations racing with that drain observe
/// the new status and are rejected. The lock order is forward group/world
/// entry, actor relations, then the scope index; shutdown releases the actor
/// relations lock before acquiring any forward entry.
#[derive(Default)]
struct ActorRelations {
    memberships: HashSet<ScopeGroupKey>,
    group_monitors: HashSet<ScopeGroupKey>,
    world_monitors: HashSet<ScopeGroupKey>,
}

impl ActorRelations {
    fn is_empty(&self) -> bool {
        self.memberships.is_empty()
            && self.group_monitors.is_empty()
            && self.world_monitors.is_empty()
    }
}

type SharedActorRelations = Arc<Mutex<ActorRelations>>;

struct PgState {
    /// Maps (scope, group) to the group's state (members + per-group listeners)
    map: DashMap<ScopeGroupKey, GroupState>,
    /// Secondary index: scope -> set of group names that have members
    index: DashMap<ScopeName, HashSet<GroupName>>,
    /// Scope-level and global monitors (sentinel keys only)
    world_listeners: DashMap<ScopeGroupKey, Vec<ActorCell>>,
    /// Reverse index used to clean up only the relationships owned by an actor
    actor_relations: DashMap<ActorId, SharedActorRelations>,
}

static PG_MONITOR: OnceCell<PgState> = OnceCell::new();

fn get_monitor<'a>() -> &'a PgState {
    PG_MONITOR.get_or_init(|| PgState {
        map: DashMap::new(),
        index: DashMap::new(),
        world_listeners: DashMap::new(),
        actor_relations: DashMap::new(),
    })
}

fn lock_relations(relations: &SharedActorRelations) -> MutexGuard<'_, ActorRelations> {
    relations
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn get_or_create_actor_relations(monitor: &PgState, actor: ActorId) -> SharedActorRelations {
    monitor.actor_relations.entry(actor).or_default().clone()
}

fn get_actor_relations(monitor: &PgState, actor: ActorId) -> Option<SharedActorRelations> {
    monitor
        .actor_relations
        .get(&actor)
        .map(|relations| relations.value().clone())
}

/// Removes the reverse-index entry once an actor owns no relationships.
///
/// A caller may already hold a clone of the same `Arc`, but it cannot add a new
/// relationship once `Stopping` has been published. Callers must therefore use
/// this only for stopping actors. Pointer comparison prevents removing a
/// replacement entry created by a concurrent caller.
fn remove_empty_actor_relations(
    monitor: &PgState,
    actor: ActorId,
    relations: &SharedActorRelations,
) {
    let relations_guard = lock_relations(relations);
    if !relations_guard.is_empty() {
        return;
    }

    if let Occupied(entry) = monitor.actor_relations.entry(actor) {
        if Arc::ptr_eq(entry.get(), relations) {
            entry.remove();
        }
    }
}

fn add_group_to_index(monitor: &PgState, key: &ScopeGroupKey) {
    monitor
        .index
        .entry(key.scope.clone())
        .or_default()
        .insert(key.group.clone());
}

fn remove_group_from_index(monitor: &PgState, key: &ScopeGroupKey) {
    if let Occupied(mut entry) = monitor.index.entry(key.scope.clone()) {
        entry.get_mut().remove(&key.group);
        if entry.get().is_empty() {
            entry.remove();
        }
    }
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

    // Preserve the existing notification contract: every caller-provided actor
    // that is not already stopping remains in the event payload, including
    // duplicate or already-joined actors.
    let actors = actors
        .into_iter()
        .filter(|actor| actor.get_status() <= ActorStatus::Draining)
        .collect::<Vec<_>>();
    if actors.is_empty() {
        return;
    }

    let mut stopped_relations = Vec::new();
    let (joined, listeners) = {
        let mut entry = monitor.map.entry(key.clone()).or_default();
        let group_state = entry.value_mut();
        let mut processed = HashSet::with_capacity(actors.len());
        let mut accepted = HashSet::with_capacity(actors.len());

        for actor in &actors {
            if !processed.insert(actor.get_id()) {
                continue;
            }
            let relations = get_or_create_actor_relations(monitor, actor.get_id());
            let mut relations_guard = lock_relations(&relations);
            if actor.get_status() <= ActorStatus::Draining {
                relations_guard.memberships.insert(key.clone());
                accepted.insert(actor.get_id());
            } else if relations_guard.is_empty() {
                stopped_relations.push((actor.get_id(), relations.clone()));
            }
        }

        let joined = actors
            .into_iter()
            .filter(|actor| accepted.contains(&actor.get_id()))
            .collect::<Vec<_>>();
        for actor in &joined {
            group_state.members.insert(actor.get_id(), actor.clone());
        }

        if !joined.is_empty() {
            add_group_to_index(monitor, &key);
        }

        (joined, group_state.listeners.clone())
    };

    for (actor, relations) in stopped_relations {
        remove_empty_actor_relations(monitor, actor, &relations);
    }

    if joined.is_empty() {
        if let Occupied(entry) = monitor.map.entry(key) {
            if entry.get().members.is_empty() && entry.get().listeners.is_empty() {
                entry.remove();
            }
        }
        return;
    }

    for listener in &listeners {
        let _ = listener.send_supervisor_evt(SupervisionEvent::ProcessGroupChanged(
            GroupChangeMessage::Join(scope.to_owned(), group.clone(), joined.clone()),
        ));
    }

    notify_world_listeners(monitor, &scope, &group, &joined, true);
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

    let result = if let Occupied(mut entry) = monitor.map.entry(key.clone()) {
        let group_state = entry.get_mut();

        for actor in &actors {
            if let Some(relations) = get_actor_relations(monitor, actor.get_id()) {
                lock_relations(&relations).memberships.remove(&key);
            }
            group_state.members.remove(&actor.get_id());
        }

        let listeners = group_state.listeners.clone();
        if group_state.members.is_empty() {
            remove_group_from_index(monitor, &key);
            if group_state.listeners.is_empty() {
                entry.remove();
            }
        }
        Some(listeners)
    } else {
        None
    };

    let Some(listeners) = result else {
        return;
    };

    for listener in &listeners {
        let _ = listener.send_supervisor_evt(SupervisionEvent::ProcessGroupChanged(
            GroupChangeMessage::Leave(scope.to_owned(), group.clone(), actors.clone()),
        ));
    }

    notify_world_listeners(monitor, &scope, &group, &actors, false);
}

/// Leave all groups for a specific [ActorId].
/// Used only during actor shutdown
pub(crate) fn leave_all(actor: ActorId) {
    let monitor = get_monitor();
    let Some(relations) = get_actor_relations(monitor, actor) else {
        return;
    };
    let mut relations_guard = lock_relations(&relations);
    let memberships = std::mem::take(&mut relations_guard.memberships);
    drop(relations_guard);
    let mut removal_events = Vec::with_capacity(memberships.len());

    for key in memberships {
        if let Occupied(mut entry) = monitor.map.entry(key.clone()) {
            let group_state = entry.get_mut();
            if let Some(actor_cell) = group_state.members.remove(&actor) {
                let listeners = group_state.listeners.clone();
                if group_state.members.is_empty() {
                    remove_group_from_index(monitor, &key);
                    if group_state.listeners.is_empty() {
                        entry.remove();
                    }
                }
                removal_events.push((key, actor_cell, listeners));
            }
        }
    }

    remove_empty_actor_relations(monitor, actor, &relations);

    for (scope_and_group, cell, per_group_listeners) in &removal_events {
        for listener in per_group_listeners {
            let _ = listener.send_supervisor_evt(SupervisionEvent::ProcessGroupChanged(
                GroupChangeMessage::Leave(
                    scope_and_group.scope.clone(),
                    scope_and_group.group.clone(),
                    vec![cell.clone()],
                ),
            ));
        }

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
    let mut scopes = monitor
        .map
        .iter()
        .filter(|kvp| !kvp.value().members.is_empty())
        .map(|kvp| kvp.key().scope.to_owned())
        .collect::<Vec<_>>();
    scopes.sort_unstable();
    scopes.dedup();
    scopes
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
    let actor_id = actor.get_id();
    let relations = get_or_create_actor_relations(monitor, actor_id);
    let mut entry = monitor.map.entry(key.clone()).or_default();
    let mut relations_guard = lock_relations(&relations);

    if actor.get_status() <= ActorStatus::Draining {
        if !entry
            .listeners
            .iter()
            .any(|listener| listener.get_id() == actor_id)
        {
            entry.listeners.push(actor.clone());
        }
        relations_guard.group_monitors.insert(key.clone());
    }

    drop(relations_guard);
    drop(entry);
    if actor.get_status() >= ActorStatus::Stopping {
        if let Occupied(entry) = monitor.map.entry(key) {
            if entry.get().members.is_empty() && entry.get().listeners.is_empty() {
                entry.remove();
            }
        }
        remove_empty_actor_relations(monitor, actor_id, &relations);
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
    let actor_id = actor.get_id();
    let relations = get_or_create_actor_relations(monitor, actor_id);
    let mut entry = monitor.world_listeners.entry(key.clone()).or_default();
    let mut relations_guard = lock_relations(&relations);

    if actor.get_status() <= ActorStatus::Draining {
        // Register ONLY in world_listeners (not per-group) to avoid duplicate notifications
        if !entry.iter().any(|listener| listener.get_id() == actor_id) {
            entry.push(actor.clone());
        }
        relations_guard.world_monitors.insert(key.clone());
    }

    drop(relations_guard);
    drop(entry);
    if actor.get_status() >= ActorStatus::Stopping {
        if let Occupied(entry) = monitor.world_listeners.entry(key) {
            if entry.get().is_empty() {
                entry.remove();
            }
        }
        remove_empty_actor_relations(monitor, actor_id, &relations);
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
    let relations = get_actor_relations(monitor, actor);

    if let Occupied(mut entry) = monitor.map.entry(key.clone()) {
        let mut relations_guard = relations.as_ref().map(lock_relations);
        let group_state = entry.get_mut();
        group_state
            .listeners
            .retain(|listener| listener.get_id() != actor);
        if let Some(relations_guard) = relations_guard.as_mut() {
            relations_guard.group_monitors.remove(&key);
        }
        if group_state.members.is_empty() && group_state.listeners.is_empty() {
            entry.remove();
        }
    } else if let Some(relations) = relations {
        lock_relations(&relations).group_monitors.remove(&key);
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
    let relations = get_actor_relations(monitor, actor);

    if let Occupied(mut entry) = monitor.world_listeners.entry(key.clone()) {
        let mut relations_guard = relations.as_ref().map(lock_relations);
        let listeners = entry.get_mut();
        listeners.retain(|a| a.get_id() != actor);
        if let Some(relations_guard) = relations_guard.as_mut() {
            relations_guard.world_monitors.remove(&key);
        }
        if listeners.is_empty() {
            entry.remove();
        }
    } else if let Some(relations) = relations {
        lock_relations(&relations).world_monitors.remove(&key);
    }
}

/// Remove the specified [ActorId] from monitoring all groups it might be in.
/// Used only during actor shutdown
pub(crate) fn demonitor_all(actor: ActorId) {
    let monitor = get_monitor();
    let Some(relations) = get_actor_relations(monitor, actor) else {
        return;
    };
    let mut relations_guard = lock_relations(&relations);
    let group_monitors = std::mem::take(&mut relations_guard.group_monitors);
    let world_monitors = std::mem::take(&mut relations_guard.world_monitors);
    drop(relations_guard);

    for key in group_monitors {
        if let Occupied(mut entry) = monitor.map.entry(key) {
            let group_state = entry.get_mut();
            group_state
                .listeners
                .retain(|listener| listener.get_id() != actor);
            if group_state.members.is_empty() && group_state.listeners.is_empty() {
                entry.remove();
            }
        }
    }

    for key in world_monitors {
        if let Occupied(mut entry) = monitor.world_listeners.entry(key) {
            entry
                .get_mut()
                .retain(|listener| listener.get_id() != actor);
            if entry.get().is_empty() {
                entry.remove();
            }
        }
    }
}
