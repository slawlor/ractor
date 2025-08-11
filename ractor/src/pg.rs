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

use std::borrow::Borrow;
use std::sync::Arc;
use std::sync::OnceLock;

use dashmap::mapref::entry::Entry::Occupied;
use dashmap::mapref::entry::Entry::Vacant;
use dashmap::DashMap;
use dashmap::DashSet;
use once_cell::sync::OnceCell;
use std::hash::Hash;

use crate::ActorCell;
use crate::ActorId;
use crate::GroupName;
use crate::ScopeName;
use crate::SupervisionEvent;

/// Key to set the default scope
pub const DEFAULT_SCOPE: &str = "__default_scope__";

/// Key to monitor all of the scopes
pub const ALL_SCOPES_NOTIFICATION: &str = "__world_scope__";

/// Key to monitor all of the groups in a scope
pub const ALL_GROUPS_NOTIFICATION: &str = "__world_group_";

static ALL_SCOPES_NOTIFICATION_OWNED: OnceLock<ScopeName> = OnceLock::new();
fn all_scopes_notification() -> &'static ScopeName {
    ALL_SCOPES_NOTIFICATION_OWNED.get_or_init(|| ALL_SCOPES_NOTIFICATION.to_owned())
}
static ALL_GROUPS_NOTIFICATION_OWNED: OnceLock<ScopeName> = OnceLock::new();
fn all_groups_notification() -> &'static ScopeName {
    ALL_GROUPS_NOTIFICATION_OWNED.get_or_init(|| ALL_GROUPS_NOTIFICATION.to_owned())
}

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

#[derive(Default)]
struct ScopeData {
    listeners: DashSet<ActorCell>,
    groups: DashMap<GroupName, Arc<GroupData>>,
}
#[derive(Default)]
struct GroupData {
    listeners: DashSet<ActorCell>,
    members: DashSet<ActorCell>,
}

struct PgState {
    world_listeners: Arc<DashSet<ActorCell>>,
    scopes: Arc<DashMap<ScopeName, Arc<ScopeData>>>,
}
static PG_MONITOR: OnceCell<PgState> = OnceCell::new();

fn get_monitor<'a>() -> &'a PgState {
    PG_MONITOR.get_or_init(|| PgState {
        world_listeners: Arc::new(DashSet::new()),
        scopes: Arc::new(DashMap::new()),
    })
}

/// Join actors to the group `group` in the default scope
///
/// * `group` - The named group. Will be created if first actors to join
/// * `actors` - The list of [crate::Actor]s to add to the group
pub fn join(group: GroupName, actors: Vec<ActorCell>) {
    join_scoped(DEFAULT_SCOPE.to_owned(), group, actors);
}

fn notify_listeners(
    listeners: &DashSet<ActorCell>,
    notification: &GroupChangeMessage,
    garbadge: &mut Vec<ActorCell>,
) {
    garbadge.clear();
    for listener in listeners.iter() {
        if listener
            .send_supervisor_evt(SupervisionEvent::ProcessGroupChanged(notification.clone()))
            .is_err()
        {
            garbadge.push(listener.clone())
        }
    }
    for l in garbadge.iter() {
        listeners.remove(l);
    }
}
fn join_actors_to_group(
    monitor: &PgState,
    sd: &ScopeData,
    gd: &GroupData,
    mut actors: Vec<ActorCell>,
    scope: ScopeName,
    group: GroupName,
) {
    let mut garbadge = Vec::new();
    actors.retain(|actor| {
        if gd.members.insert(actor.clone()) {
            if !actor.add_member_ship(scope.to_owned(), group.to_owned()) {
                gd.members.remove(actor);
                false
            } else {
                true
            }
        } else {
            false
        }
    });
    let notif = GroupChangeMessage::Join(scope.to_owned(), group.clone(), actors);
    notify_listeners(&gd.listeners, &notif, &mut garbadge);
    notify_listeners(&sd.listeners, &notif, &mut garbadge);
    notify_listeners(&monitor.world_listeners, &notif, &mut garbadge);
}
fn join_actors_to_scope(
    monitor: &PgState,
    sd: &ScopeData,
    actors: Vec<ActorCell>,
    scope: ScopeName,
    group: GroupName,
) {
    if let Some(gd) = sd.groups.get(&group).map(|r| (*r).clone()) {
        join_actors_to_group(monitor, &sd, &gd, actors, scope, group)
    } else {
        let gd = match sd.groups.entry(group.to_owned()) {
            Occupied(oent) => oent.get().clone(),
            Vacant(vent) => vent.insert(Arc::new(GroupData::default())).clone(),
        };
        join_actors_to_group(monitor, &sd, &*gd, actors, scope, group)
    }
}

/// Join actors to the group `group` within the scope `scope`
///
/// * `scope` - The named scope. Will be created if first actors to join
/// * `group` - The named group. Will be created if first actors to join
/// * `actors` - The list of [crate::Actor]s to add to the group
pub fn join_scoped(scope: ScopeName, group: GroupName, actors: Vec<ActorCell>) {
    let monitor = get_monitor();

    if let Some(sd) = monitor.scopes.get(&scope).map(|r| (*r).clone()) {
        join_actors_to_scope(&monitor, &sd, actors, scope, group)
    } else {
        let sd = match monitor.scopes.entry(scope.to_owned()) {
            Occupied(oent) => oent.get().clone(),
            Vacant(vent) => vent.insert(Arc::new(ScopeData::default())).clone(),
        };
        join_actors_to_scope(monitor, &sd, actors, scope, group)
    }
}

#[must_use]
// return true if the group should be removed
fn leave_actors_from_group(
    monitor: &PgState,
    sd: &ScopeData,
    gd: &GroupData,
    mut actors: Vec<ActorCell>,
    scope: ScopeName,
    group: GroupName,
) -> bool {
    let mut garbadge = Vec::new();
    actors.retain(|actor| {
        if gd.members.remove(actor).is_some() {
            actor.remove_member_ship(scope.to_owned(), group.to_owned());
            true
        } else {
            false
        }
    });
    let notif = GroupChangeMessage::Leave(scope.to_owned(), group.clone(), actors);
    notify_listeners(&gd.listeners, &notif, &mut garbadge);
    notify_listeners(&sd.listeners, &notif, &mut garbadge);
    notify_listeners(&monitor.world_listeners, &notif, &mut garbadge);
    gd.members.is_empty()
}
#[must_use]
/// return true if the scope shall be removed
fn leave_actors_from_scope(
    monitor: &PgState,
    sd: &ScopeData,
    actors: Vec<ActorCell>,
    scope: ScopeName,
    group: GroupName,
) -> bool {
    if let Some(gd) = sd.groups.get(&group).map(|r| (*r).clone()) {
        if leave_actors_from_group(monitor, &sd, &gd, actors, scope, group.clone()) {
            sd.groups.remove(&group);
            sd.groups.is_empty()
        } else {
            false
        }
    } else {
        false
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
    let monitor = get_monitor();

    if let Some(sd) = monitor.scopes.get(&scope).map(|r| (*r).clone()) {
        if leave_actors_from_scope(&monitor, &sd, actors, scope.clone(), group) {
            monitor.scopes.remove(&scope);
        }
    }
}

/// Leave all groups for a specific [ActorId].
/// Used only during actor shutdown
pub(crate) fn leave_all(actor: ActorCell, member_ship: Vec<(ScopeName, GroupName)>) {
    for (scope, group) in member_ship {
        leave_scoped(scope, group, vec![actor.clone()])
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
    let monitor = get_monitor();

    if let Some(sd) = monitor.scopes.get(scope).map(|r| (*r).clone()) {
        if let Some(gd) = sd.groups.get(group).map(|r| (*r).clone()) {
            gd.members
                .iter()
                .filter_map(|member| {
                    if member.get_id().is_local() {
                        Some(member.clone())
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    }
}

/// Returns all the actors running on any node in the group `group`
/// in the default scope.
///
/// * `group_name` - A named group
///
/// Returns a [`Vec<ActorCell>`] with the member actors
pub fn get_members<G>(group_name: &G) -> Vec<ActorCell>
where
    G: Hash + Eq + ?Sized,
    GroupName: Borrow<G>,
{
    get_scoped_members::<str, G>(DEFAULT_SCOPE, group_name)
}

/// Returns all the actors running on any node in the group `group`
/// in the scope `scope`.
///
/// * `scope` - A named scope
/// * `group` - A named group
///
/// Returns a [`Vec<ActorCell>`] with the member actors
pub fn get_scoped_members<S, G>(scope: &S, group: &G) -> Vec<ActorCell>
where
    S: Hash + Eq + ?Sized,
    ScopeName: Borrow<S>,
    G: Hash + Eq + ?Sized,
    GroupName: Borrow<G>,
{
    let monitor = get_monitor();

    if let Some(sd) = monitor.scopes.get(scope).map(|r| (*r).clone()) {
        if let Some(gd) = sd.groups.get(group).map(|r| (*r).clone()) {
            gd.members.iter().map(|member| member.clone()).collect()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    }
}

/// Return a list of all known groups
///
/// Returns a [`Vec<GroupName>`] representing all the registered group names
pub fn which_groups() -> Vec<GroupName> {
    let Some(mut groups) = which_scopes()
        .iter()
        .map(|scope| which_scoped_groups(scope))
        .reduce(|mut collected, gs| {
            collected.extend(gs);
            collected
        })
    else {
        return Vec::new();
    };
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
pub fn which_scoped_groups<S>(scope: &S) -> Vec<GroupName>
where
    S: Hash + Eq + ?Sized,
    ScopeName: Borrow<S>,
{
    let monitor = get_monitor();
    if let Some(sd) = monitor.scopes.get(scope).map(|r| (*r).clone()) {
        sd.groups.iter().map(|r| r.key().clone()).collect()
    } else {
        Vec::new()
    }
}

/// Returns a list of all known scope-group combinations.
///
/// Returns a [`Vec<ScopeGroupKey>`] representing all the registered
/// combinations that form an identifying tuple
pub fn which_scopes_and_groups() -> Vec<ScopeGroupKey> {
    which_scopes()
        .into_iter()
        .map(|scope| (which_scoped_groups(&scope), scope.clone()))
        .fold(Vec::new(), |mut collected, (groups, scope)| {
            collected.extend(groups.into_iter().map(|g| ScopeGroupKey {
                scope: scope.clone(),
                group: g,
            }));
            collected
        })
}

/// Returns a list of all known scopes
///
/// Returns a [`Vec<ScopeName>`] representing all the registered scopes
pub fn which_scopes() -> Vec<ScopeName> {
    let monitor = get_monitor();

    monitor.scopes.iter().map(|r| r.key().clone()).collect()
}
/// Subscribes the provided [crate::Actor] to the group in the specified scope
/// for updates
pub fn monitor<G>(group: &G, actor: ActorCell)
where
    G: Hash + Eq + ?Sized + AsRef<str>,
    GroupName: Borrow<G>,
{
    monitor_scoped::<str, G>(DEFAULT_SCOPE, group, actor);
}

/// Subscribes the provided [crate::Actor] to the group in the specified scope
/// for updates
pub fn monitor_scoped<S, G>(scope: &S, group: &G, actor: ActorCell)
where
    S: Hash + Eq + ?Sized,
    G: Hash + Eq + ?Sized,
    ScopeName: Borrow<S>,
    GroupName: Borrow<G>,
{
    if scope == all_scopes_notification().borrow() {
        which_scopes()
            .into_iter()
            .for_each(|scope| monitor_scoped(&scope, group, actor.clone()));
    } else if group == all_groups_notification().borrow() {
        which_scoped_groups(scope)
            .into_iter()
            .for_each(|group| monitor_scoped(scope, &group, actor.clone()));
    } else {
        let monitor = get_monitor();
        if let Some(sd) = monitor.scopes.get(scope).map(|r| (*r).clone()) {
            if let Some(gd) = sd.groups.get(group).map(|r| (*r).clone()) {
                gd.listeners.insert(actor);
            }
        }
    }
}

/// Subscribes the provided [crate::Actor] to the scope for updates
///
/// * `scope` - the scope to monitor
/// * `actor` - The [ActorCell] representing who will receive updates
pub fn monitor_scope<S>(scope: &S, actor: ActorCell)
where
    S: Hash + Eq + ?Sized,
    ScopeName: Borrow<S>,
{
    let monitor = get_monitor();
    if let Some(sd) = monitor.scopes.get(scope).map(|r| (*r).clone()) {
        sd.listeners.insert(actor);
    }
}
/// Unsubscribes the provided [crate::Actor] for updates from the group
/// in default scope
///
/// * `group_name` - The group to demonitor
/// * `actor` - The [ActorCell] representing who will no longer receive updates
pub fn demonitor_scoped<S, G>(scope: &S, group: &G, actor: ActorId)
where
    S: Hash + Eq + ?Sized,
    G: Hash + Eq + ?Sized,
    ScopeName: Borrow<S>,
    GroupName: Borrow<G>,
{
    let monitor = get_monitor();
    if scope == all_scopes_notification().borrow() {
        which_scopes()
            .into_iter()
            .for_each(|scope| demonitor_scoped(&scope, group, actor));
    } else if group == all_groups_notification().borrow() {
        which_scoped_groups(scope)
            .into_iter()
            .for_each(|group| demonitor_scoped(scope, &group, actor));
    } else if let Some(sd) = monitor.scopes.get(scope).map(|r| (*r).clone()) {
        if let Some(gd) = sd.groups.get(group).map(|r| (*r).clone()) {
            gd.listeners.retain(|cell| cell.get_id() != actor);
        }
    }
}

/// Unsubscribes the provided [crate::Actor] for updates from the group
/// in default scope
///
/// * `group_name` - The group to demonitor
/// * `actor` - The [ActorCell] representing who will no longer receive updates
pub fn demonitor<G>(group_name: &G, actor: ActorId)
where
    G: Hash + Eq + ?Sized,
    GroupName: Borrow<G>,
{
    demonitor_scoped::<str, G>(DEFAULT_SCOPE, group_name, actor)
}

/// Unsubscribes the provided [crate::Actor] from the scope for updates
///
/// * `scope` - The scope to demonitor
/// * `actor` - The [ActorCell] representing who will no longer receive updates
pub fn demonitor_scope<S>(scope: &S, actor: ActorId)
where
    S: Hash + Eq + ?Sized,
    ScopeName: Borrow<S>,
{
    let monitor = get_monitor();
    if scope == all_scopes_notification().borrow() {
        which_scopes()
            .into_iter()
            .for_each(|scope| demonitor_scope::<str>(&scope, actor));
    } else if let Some(sd) = monitor.scopes.get(scope).map(|r| (*r).clone()) {
        sd.listeners.retain(|cell| cell.get_id() != actor);
    }
}
