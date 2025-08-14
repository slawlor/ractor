// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Process groups (PG) are named groups of actors with a friendly name
//! which can be used for retrieval of the process groups. Then within
//! the group, either a random actor (for dispatch) can be selected or
//! the whole group (broadcast), or a subset (partial-broadcast) can have
//! a message sent to them. Common operations are to (a) upcast the group
//! members to a strong-typed actor then dispatch a message with [crate::call]
//! or [crate::cast].
//!
//! Process groups can also be monitored for changes by calling [monitor] to
//! subscribe to changes and [demonitor] to unsubscribe. Subscribers will receive
//! process group change notifications via a [SupervisionEvent] called on the
//! supervision port of the [crate::Actor].
//!
//! Inspired by [Erlang's `pg` module](https://www.erlang.org/doc/man/pg.html)
//!
//! ## Examples
//!
//! ### Basic Group Operations
//!
//! ```rust
//! # use ractor::pg;
//! # use ractor::{Actor, ActorProcessingErr, ActorRef};
//! #
//! # struct ExampleActor;
//! #
//! # #[cfg_attr(feature = "async-trait", ractor::async_trait)]
//! # impl Actor for ExampleActor {
//! #     type Msg = ();
//! #     type State = ();
//! #     type Arguments = ();
//! #
//! #     async fn pre_start(
//! #         &self,
//! #         _myself: ActorRef<Self::Msg>,
//! #         _args: Self::Arguments,
//! #     ) -> Result<Self::State, ActorProcessingErr> {
//! #         Ok(())
//! #     }
//! # }
//! #
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let (actor, handle) = Actor::spawn(None, ExampleActor, ()).await?;
//! let group = "worker_pool";
//!
//! // Join the actor to a group
//! pg::join(group, vec![actor.get_cell()]);
//!
//! // Retrieve the group membership
//! let members = pg::get_members(group);
//! assert_eq!(members.len(), 1);
//!
//! // Cleanup
//! actor.stop(None);
//! handle.await;
//! # Ok(())
//! # }
//! ```
//!
//! ### Scoped Operations
//!
//! ```rust
//! # use ractor::pg;
//! # use ractor::{Actor, ActorProcessingErr, ActorRef};
//! #
//! # struct Worker;
//! #
//! # #[cfg_attr(feature = "async-trait", ractor::async_trait)]
//! # impl Actor for Worker {
//! #     type Msg = ();
//! #     type State = ();
//! #     type Arguments = ();
//! #
//! #     async fn pre_start(&self, _: ActorRef<Self::Msg>, _: ()) -> Result<Self::State, ActorProcessingErr> {
//! #         Ok(())
//! #     }
//! # }
//! #
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let (worker, handle) = Actor::spawn(None, Worker, ()).await?;
//!
//! // Join actors to a specific scope
//! pg::join_scoped("production", "workers", vec![worker.get_cell()]);
//!
//! // Get members from a specific scope
//! let members = pg::get_scoped_members("production", "workers");
//! assert_eq!(members.len(), 1);
//!
//! // Cleanup
//! worker.stop(None);
//! handle.await;
//! # Ok(())
//! # }
//! ```

use std::borrow::Borrow;
use std::sync::Arc;

use dashmap::mapref::entry::Entry::Occupied;
use dashmap::mapref::entry::Entry::Vacant;
use dashmap::DashMap;
use dashmap::DashSet;
use once_cell::sync::OnceCell;
use std::hash::Hash;

use crate::actor::actor_properties::MemberShip;
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
    /// Retrieves the group that changed
    ///
    /// # Returns
    ///
    /// The name of the group that experienced the membership change
    ///
    /// # Example
    ///
    /// ```rust
    /// # use ractor::pg::GroupChangeMessage;
    /// let actors = vec![];
    /// let change = GroupChangeMessage::Join("scope".to_string(), "workers".to_string(), actors);
    /// assert_eq!(change.get_group(), "workers");
    /// ```
    pub fn get_group(&self) -> GroupName {
        match self {
            Self::Join(_, name, _) => name.clone(),
            Self::Leave(_, name, _) => name.clone(),
        }
    }

    /// Retrieves the name of the scope in which the group change took place
    ///
    /// # Returns
    ///
    /// The name of the scope containing the group that changed
    ///
    /// # Example
    ///
    /// ```rust
    /// # use ractor::pg::GroupChangeMessage;
    /// let actors = vec![];
    /// let change = GroupChangeMessage::Join("production".to_string(), "workers".to_string(), actors);
    /// assert_eq!(change.get_scope(), "production");
    /// ```
    pub fn get_scope(&self) -> ScopeName {
        match self {
            Self::Join(scope, _, _) => scope.to_string(),
            Self::Leave(scope, _, _) => scope.to_string(),
        }
    }
}

/// Represents the combination of a `ScopeName` and a `GroupName`
/// that uniquely identifies a specific group within a specific scope
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScopeGroupKey {
    /// The scope name
    scope: ScopeName,
    /// The group name
    group: GroupName,
}

impl ScopeGroupKey {
    /// Creates a new scope-group key
    ///
    /// # Arguments
    ///
    /// * `scope` - The scope name
    /// * `group` - The group name
    ///
    /// # Example
    ///
    /// ```rust
    /// # use ractor::pg::ScopeGroupKey;
    /// let key = ScopeGroupKey::new("production".to_string(), "workers".to_string());
    /// assert_eq!(key.get_scope(), "production");
    /// assert_eq!(key.get_group(), "workers");
    /// ```
    pub fn new(scope: ScopeName, group: GroupName) -> Self {
        Self { scope, group }
    }

    /// Retrieves the scope name
    ///
    /// # Returns
    ///
    /// A clone of the scope name
    pub fn get_scope(&self) -> ScopeName {
        self.scope.clone()
    }

    /// Retrieves the group name
    ///
    /// # Returns
    ///
    /// A clone of the group name
    pub fn get_group(&self) -> GroupName {
        self.group.clone()
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

fn get_monitor() -> &'static PgState {
    PG_MONITOR.get_or_init(|| PgState {
        world_listeners: Arc::new(DashSet::new()),
        scopes: Arc::new(DashMap::new()),
    })
}

/// Joins actors to the specified group in the default scope
///
/// This is a convenience function that calls `join_scoped` with the default scope.
///
/// # Arguments
///
/// * `group` - The name of the group to join (will be created if this is the first join)
/// * `actors` - The list of actors to add to the group
///
/// # Example
///
/// ```rust
/// # use ractor::pg;
/// # use ractor::{Actor, ActorProcessingErr, ActorRef};
/// #
/// # struct Worker;
/// #
/// # #[cfg_attr(feature = "async-trait", ractor::async_trait)]
/// # impl Actor for Worker {
/// #     type Msg = ();
/// #     type State = ();
/// #     type Arguments = ();
/// #
/// #     async fn pre_start(&self, _: ActorRef<Self::Msg>, _: ()) -> Result<Self::State, ActorProcessingErr> {
/// #         Ok(())
/// #     }
/// # }
/// #
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let (worker, handle) = Actor::spawn(None, Worker, ()).await?;
/// pg::join("worker_pool", vec![worker.get_cell()]);
///
/// worker.stop(None);
/// handle.await;
/// # Ok(())
/// # }
/// ```
pub fn join<G>(group: G, actors: Vec<ActorCell>)
where
    G: AsRef<str>,
{
    join_scoped(DEFAULT_SCOPE, group.as_ref(), actors);
}

fn notify_listeners(
    listeners: &DashSet<ActorCell>,
    notification: &GroupChangeMessage,
    garbage: &mut Vec<ActorCell>,
) {
    garbage.clear();
    for listener in listeners.iter() {
        if listener
            .send_supervisor_evt(SupervisionEvent::ProcessGroupChanged(notification.clone()))
            .is_err()
        {
            garbage.push(listener.clone())
        }
    }
    for l in garbage.iter() {
        listeners.remove(l);
    }
}

fn join_actors_to_group(
    monitor: &PgState,
    sd: &ScopeData,
    gd: &GroupData,
    mut actors: Vec<ActorCell>,
    scope: &str,
    group: &str,
) {
    let mut garbage = Vec::new();
    let mut shall_clean_group = false;
    actors.retain(|actor| {
        if gd.members.insert(actor.clone()) {
            if !actor.add_member_ship(scope.to_owned(), group.to_owned()) {
                shall_clean_group |= gd.members.remove(actor).is_some();
                false
            } else {
                true
            }
        } else {
            false
        }
    });
    if shall_clean_group && clean_up_group(sd, group) {
        clean_up_scope(monitor, scope)
    }
    let notif = GroupChangeMessage::Join(scope.to_owned(), group.to_owned(), actors);
    notify_listeners(&gd.listeners, &notif, &mut garbage);
    notify_listeners(&sd.listeners, &notif, &mut garbage);
    notify_listeners(&monitor.world_listeners, &notif, &mut garbage);
}

fn join_actors_to_scope(
    monitor: &PgState,
    sd: &ScopeData,
    actors: Vec<ActorCell>,
    scope: &str,
    group: &str,
) {
    if let Some(gd) = sd.groups.get(group).map(|r| (*r).clone()) {
        join_actors_to_group(monitor, sd, &gd, actors, scope, group)
    } else {
        let gd = match sd.groups.entry(group.to_owned()) {
            Occupied(oent) => oent.get().clone(),
            Vacant(vent) => vent.insert(Arc::new(GroupData::default())).clone(),
        };
        join_actors_to_group(monitor, sd, &gd, actors, scope, group)
    }
}

/// Joins actors to the specified group within the given scope
///
/// If the scope or group doesn't exist, they will be created automatically.
/// Actors are automatically removed from groups when they shut down.
///
/// # Arguments
///
/// * `scope` - The name of the scope (will be created if this is the first join)
/// * `group` - The name of the group within the scope (will be created if this is the first join)
/// * `actors` - The list of actors to add to the group
///
/// # Example
///
/// ```rust
/// # use ractor::pg;
/// # use ractor::{Actor, ActorProcessingErr, ActorRef};
/// #
/// # struct Worker;
/// #
/// # #[cfg_attr(feature = "async-trait", ractor::async_trait)]
/// # impl Actor for Worker {
/// #     type Msg = ();
/// #     type State = ();
/// #     type Arguments = ();
/// #
/// #     async fn pre_start(&self, _: ActorRef<Self::Msg>, _: ()) -> Result<Self::State, ActorProcessingErr> {
/// #         Ok(())
/// #     }
/// # }
/// #
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let (worker, handle) = Actor::spawn(None, Worker, ()).await?;
/// pg::join_scoped("production", "worker_pool", vec![worker.get_cell()]);
///
/// worker.stop(None);
/// handle.await;
/// # Ok(())
/// # }
/// ```
pub fn join_scoped<S, G>(scope: S, group: G, actors: Vec<ActorCell>)
where
    S: AsRef<str>,
    G: AsRef<str>,
{
    let monitor = get_monitor();
    let scope_str = scope.as_ref();
    let group_str = group.as_ref();

    if let Some(sd) = monitor.scopes.get(scope_str).map(|r| (*r).clone()) {
        join_actors_to_scope(monitor, &sd, actors, scope_str, group_str)
    } else {
        let sd = match monitor.scopes.entry(scope_str.to_owned()) {
            Occupied(oent) => oent.get().clone(),
            Vacant(vent) => vent.insert(Arc::new(ScopeData::default())).clone(),
        };
        join_actors_to_scope(monitor, &sd, actors, scope_str, group_str)
    }
}

#[must_use]
/// Returns true if the scope may be cleaned up
fn clean_up_group<G>(sd: &ScopeData, group: &G) -> bool
where
    G: Hash + Eq + ?Sized,
    GroupName: Borrow<G>,
{
    sd.groups
        .remove_if(group, |_, gd| {
            gd.members.is_empty() && gd.listeners.is_empty()
        })
        .is_some()
}

fn clean_up_scope<S>(monitor: &PgState, scope: &S)
where
    S: Hash + Eq + ?Sized,
    ScopeName: Borrow<S>,
{
    monitor.scopes.remove_if(scope, |_, sd| {
        sd.groups.is_empty() && sd.listeners.is_empty()
    });
}

fn leave_actors_from_group(
    monitor: &PgState,
    sd: &ScopeData,
    gd: &GroupData,
    mut actors: Vec<ActorCell>,
    scope: &str,
    group: &str,
) {
    let mut garbage = Vec::new();
    actors.retain(|actor| {
        if gd.members.remove(actor).is_some() {
            actor.remove_member_ship(scope.to_owned(), group.to_owned());
            true
        } else {
            false
        }
    });
    let notif = GroupChangeMessage::Leave(scope.to_owned(), group.to_owned(), actors);
    notify_listeners(&gd.listeners, &notif, &mut garbage);
    notify_listeners(&sd.listeners, &notif, &mut garbage);
    notify_listeners(&monitor.world_listeners, &notif, &mut garbage);
    if clean_up_group(sd, group) {
        clean_up_scope(monitor, scope)
    }
}

fn leave_actors_from_scope(
    monitor: &PgState,
    sd: &ScopeData,
    actors: Vec<ActorCell>,
    scope: &str,
    group: &str,
) {
    if let Some(gd) = sd.groups.get(group).map(|r| (*r).clone()) {
        leave_actors_from_group(monitor, sd, &gd, actors, scope, group)
    }
}

/// Removes the specified actors from the group in the default scope
///
/// This is a convenience function that calls `leave_scoped` with the default scope.
/// If the group becomes empty after removing these actors, it will be automatically cleaned up.
///
/// # Arguments
///
/// * `group` - The name of the group to leave
/// * `actors` - The list of actors to remove from the group
///
/// # Example
///
/// ```rust
/// # use ractor::pg;
/// # use ractor::{Actor, ActorProcessingErr, ActorRef};
/// #
/// # struct Worker;
/// #
/// # #[cfg_attr(feature = "async-trait", ractor::async_trait)]
/// # impl Actor for Worker {
/// #     type Msg = ();
/// #     type State = ();
/// #     type Arguments = ();
/// #
/// #     async fn pre_start(&self, _: ActorRef<Self::Msg>, _: ()) -> Result<Self::State, ActorProcessingErr> {
/// #         Ok(())
/// #     }
/// # }
/// #
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let (worker, handle) = Actor::spawn(None, Worker, ()).await?;
///
/// // First join the group
/// pg::join("worker_pool", vec![worker.get_cell()]);
///
/// // Then leave the group
/// pg::leave("worker_pool", vec![worker.get_cell()]);
///
/// worker.stop(None);
/// handle.await;
/// # Ok(())
/// # }
/// ```
pub fn leave<G>(group: G, actors: Vec<ActorCell>)
where
    G: AsRef<str>,
{
    leave_scoped(DEFAULT_SCOPE, group.as_ref(), actors);
}

/// Removes the specified actors from the group within the given scope
///
/// If the group becomes empty after removing these actors, it will be automatically cleaned up.
/// If the scope becomes empty after group cleanup, it will also be cleaned up.
///
/// # Arguments
///
/// * `scope` - The name of the scope containing the group
/// * `group` - The name of the group to leave
/// * `actors` - The list of actors to remove from the group
///
/// # Example
///
/// ```rust
/// # use ractor::pg;
/// # use ractor::{Actor, ActorProcessingErr, ActorRef};
/// #
/// # struct Worker;
/// #
/// # #[cfg_attr(feature = "async-trait", ractor::async_trait)]
/// # impl Actor for Worker {
/// #     type Msg = ();
/// #     type State = ();
/// #     type Arguments = ();
/// #
/// #     async fn pre_start(&self, _: ActorRef<Self::Msg>, _: ()) -> Result<Self::State, ActorProcessingErr> {
/// #         Ok(())
/// #     }
/// # }
/// #
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let (worker, handle) = Actor::spawn(None, Worker, ()).await?;
///
/// // First join the scoped group
/// pg::join_scoped("production", "worker_pool", vec![worker.get_cell()]);
///
/// // Then leave the scoped group
/// pg::leave_scoped("production", "worker_pool", vec![worker.get_cell()]);
///
/// worker.stop(None);
/// handle.await;
/// # Ok(())
/// # }
/// ```
pub fn leave_scoped<S, G>(scope: S, group: G, actors: Vec<ActorCell>)
where
    S: AsRef<str>,
    G: AsRef<str>,
{
    let monitor = get_monitor();
    let scope_str = scope.as_ref();
    let group_str = group.as_ref();

    if let Some(sd) = monitor.scopes.get(scope_str).map(|r| (*r).clone()) {
        leave_actors_from_scope(monitor, &sd, actors, scope_str, group_str);
    }
}

/// Removes an actor from all groups and stops all monitoring subscriptions
///
/// This function is called automatically during actor shutdown and should not
/// be called manually by user code.
///
/// # Arguments
///
/// * `actor` - The actor cell to remove from all groups
/// * `membership` - The membership information for cleanup
pub(crate) fn leave_and_demonitor_all(actor: ActorCell, membership: MemberShip) {
    for (scope, group) in membership.scope_groups {
        leave_scoped(scope, group, vec![actor.clone()])
    }
    demonitor_world(&actor);
    for (scope, group) in membership.listened_groups {
        demonitor_scoped(&scope, &group, actor.get_id())
    }
    for scope in membership.listened_scopes {
        demonitor_scope(&scope, actor.get_id())
    }
}

/// Returns all actors running on the local node in the specified group
/// within the default scope
///
/// Only returns actors that are running on the current node, not remote actors
/// from other nodes in a cluster.
///
/// # Arguments
///
/// * `group` - The name of the group to query
///
/// # Returns
///
/// A vector of actor cells representing the local members of the group
///
/// # Example
///
/// ```rust
/// # use ractor::pg;
/// # use ractor::{Actor, ActorProcessingErr, ActorRef};
/// #
/// # struct Worker;
/// #
/// # #[cfg_attr(feature = "async-trait", ractor::async_trait)]
/// # impl Actor for Worker {
/// #     type Msg = ();
/// #     type State = ();
/// #     type Arguments = ();
/// #
/// #     async fn pre_start(&self, _: ActorRef<Self::Msg>, _: ()) -> Result<Self::State, ActorProcessingErr> {
/// #         Ok(())
/// #     }
/// # }
/// #
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let (worker, handle) = Actor::spawn(None, Worker, ()).await?;
/// pg::join("worker_pool", vec![worker.get_cell()]);
///
/// let local_workers = pg::get_local_members("worker_pool");
/// assert_eq!(local_workers.len(), 1);
///
/// worker.stop(None);
/// handle.await;
/// # Ok(())
/// # }
/// ```
pub fn get_local_members<G>(group: G) -> Vec<ActorCell>
where
    G: AsRef<str>,
{
    get_scoped_local_members(DEFAULT_SCOPE, group)
}

/// Returns all actors running on the local node in the specified group
/// within the given scope
///
/// Only returns actors that are running on the current node, not remote actors
/// from other nodes in a cluster.
///
/// # Arguments
///
/// * `scope` - The name of the scope containing the group
/// * `group` - The name of the group to query
///
/// # Returns
///
/// A vector of actor cells representing the local members of the group
///
/// # Example
///
/// ```rust
/// # use ractor::pg;
/// # use ractor::{Actor, ActorProcessingErr, ActorRef};
/// #
/// # struct Worker;
/// #
/// # #[cfg_attr(feature = "async-trait", ractor::async_trait)]
/// # impl Actor for Worker {
/// #     type Msg = ();
/// #     type State = ();
/// #     type Arguments = ();
/// #
/// #     async fn pre_start(&self, _: ActorRef<Self::Msg>, _: ()) -> Result<Self::State, ActorProcessingErr> {
/// #         Ok(())
/// #     }
/// # }
/// #
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let (worker, handle) = Actor::spawn(None, Worker, ()).await?;
/// pg::join_scoped("production", "worker_pool", vec![worker.get_cell()]);
///
/// let local_workers = pg::get_scoped_local_members("production", "worker_pool");
/// assert_eq!(local_workers.len(), 1);
///
/// worker.stop(None);
/// handle.await;
/// # Ok(())
/// # }
/// ```
pub fn get_scoped_local_members<S, G>(scope: S, group: G) -> Vec<ActorCell>
where
    S: AsRef<str>,
    G: AsRef<str>,
{
    let monitor = get_monitor();
    let scope_str = scope.as_ref();
    let group_str = group.as_ref();

    if let Some(sd) = monitor.scopes.get(scope_str).map(|r| (*r).clone()) {
        if let Some(gd) = sd.groups.get(group_str).map(|r| (*r).clone()) {
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

/// Returns all actors in the specified group within the default scope
///
/// This includes both local and remote actors in a cluster environment.
///
/// # Arguments
///
/// * `group` - The name of the group to query
///
/// # Returns
///
/// A vector of actor cells representing all members of the group
///
/// # Example
///
/// ```rust
/// # use ractor::pg;
/// # use ractor::{Actor, ActorProcessingErr, ActorRef};
/// #
/// # struct Worker;
/// #
/// # #[cfg_attr(feature = "async-trait", ractor::async_trait)]
/// # impl Actor for Worker {
/// #     type Msg = ();
/// #     type State = ();
/// #     type Arguments = ();
/// #
/// #     async fn pre_start(&self, _: ActorRef<Self::Msg>, _: ()) -> Result<Self::State, ActorProcessingErr> {
/// #         Ok(())
/// #     }
/// # }
/// #
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let (worker, handle) = Actor::spawn(None, Worker, ()).await?;
/// pg::join("worker_pool", vec![worker.get_cell()]);
///
/// let all_workers = pg::get_members("worker_pool");
/// assert_eq!(all_workers.len(), 1);
///
/// worker.stop(None);
/// handle.await;
/// # Ok(())
/// # }
/// ```
pub fn get_members<G>(group: G) -> Vec<ActorCell>
where
    G: AsRef<str>,
{
    get_scoped_members(DEFAULT_SCOPE, group)
}

/// Returns all actors in the specified group within the given scope
///
/// This includes both local and remote actors in a cluster environment.
///
/// # Arguments
///
/// * `scope` - The name of the scope containing the group
/// * `group` - The name of the group to query
///
/// # Returns
///
/// A vector of actor cells representing all members of the group
///
/// # Example
///
/// ```rust
/// # use ractor::pg;
/// # use ractor::{Actor, ActorProcessingErr, ActorRef};
/// #
/// # struct Worker;
/// #
/// # #[cfg_attr(feature = "async-trait", ractor::async_trait)]
/// # impl Actor for Worker {
/// #     type Msg = ();
/// #     type State = ();
/// #     type Arguments = ();
/// #
/// #     async fn pre_start(&self, _: ActorRef<Self::Msg>, _: ()) -> Result<Self::State, ActorProcessingErr> {
/// #         Ok(())
/// #     }
/// # }
/// #
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let (worker1, handle1) = Actor::spawn(None, Worker, ()).await?;
/// let (worker2, handle2) = Actor::spawn(None, Worker, ()).await?;
///
/// pg::join_scoped("production", "worker_pool", vec![worker1.get_cell()]);
/// pg::join_scoped("staging", "worker_pool", vec![worker2.get_cell()]);
///
/// let production_workers = pg::get_scoped_members("production", "worker_pool");
/// let staging_workers = pg::get_scoped_members("staging", "worker_pool");
/// assert_eq!(production_workers.len(), 1);
/// assert_eq!(staging_workers.len(), 1);
///
/// worker1.stop(None);
/// worker2.stop(None);
/// handle1.await?;
/// handle2.await?;
/// # Ok(())
/// # }
/// ```
pub fn get_scoped_members<S, G>(scope: S, group: G) -> Vec<ActorCell>
where
    S: AsRef<str>,
    G: AsRef<str>,
{
    let monitor = get_monitor();
    let scope_str = scope.as_ref();
    let group_str = group.as_ref();

    if let Some(sd) = monitor.scopes.get(scope_str).map(|r| (*r).clone()) {
        if let Some(gd) = sd.groups.get(group_str).map(|r| (*r).clone()) {
            gd.members.iter().map(|member| member.clone()).collect()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    }
}

/// Returns a list of all known group names across all scopes
///
/// The returned list is sorted and deduplicated. This function aggregates
/// groups from all existing scopes.
///
/// # Returns
///
/// A vector of group names representing all registered groups
///
/// # Example
///
/// ```rust
/// # use ractor::pg;
/// # use ractor::{Actor, ActorProcessingErr, ActorRef};
/// #
/// # struct Worker;
/// #
/// # #[cfg_attr(feature = "async-trait", ractor::async_trait)]
/// # impl Actor for Worker {
/// #     type Msg = ();
/// #     type State = ();
/// #     type Arguments = ();
/// #
/// #     async fn pre_start(&self, _: ActorRef<Self::Msg>, _: ()) -> Result<Self::State, ActorProcessingErr> {
/// #         Ok(())
/// #     }
/// # }
/// #
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let (worker, handle) = Actor::spawn(None, Worker, ()).await?;
/// pg::join("group1", vec![worker.get_cell()]);
/// pg::join_scoped("scope1", "group2", vec![worker.get_cell()]);
///
/// let all_groups = pg::which_groups();
/// assert!(all_groups.len() >= 2);
///
/// worker.stop(None);
/// handle.await;
/// # Ok(())
/// # }
/// ```
pub fn which_groups() -> Vec<GroupName> {
    let Some(mut groups) = which_scopes()
        .iter()
        .map(|scope| which_scoped_groups(scope.clone()))
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

/// Returns a list of all known group names within the specified scope
///
/// # Arguments
///
/// * `scope` - The scope to query for group names
///
/// # Returns
///
/// A vector of group names within the specified scope
///
/// # Example
///
/// ```rust
/// # use ractor::pg;
/// # use ractor::{Actor, ActorProcessingErr, ActorRef};
/// #
/// # struct Worker;
/// #
/// # #[cfg_attr(feature = "async-trait", ractor::async_trait)]
/// # impl Actor for Worker {
/// #     type Msg = ();
/// #     type State = ();
/// #     type Arguments = ();
/// #
/// #     async fn pre_start(&self, _: ActorRef<Self::Msg>, _: ()) -> Result<Self::State, ActorProcessingErr> {
/// #         Ok(())
/// #     }
/// # }
/// #
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let (worker, handle) = Actor::spawn(None, Worker, ()).await?;
/// pg::join_scoped("production", "workers", vec![worker.get_cell()]);
/// pg::join_scoped("staging", "workers", vec![worker.get_cell()]);
///
/// let production_groups = pg::which_scoped_groups("production");
/// let staging_groups = pg::which_scoped_groups("staging");
/// assert_eq!(production_groups.len(), 1);
/// assert_eq!(staging_groups.len(), 1);
///
/// worker.stop(None);
/// handle.await;
/// # Ok(())
/// # }
/// ```
pub fn which_scoped_groups<S>(scope: S) -> Vec<GroupName>
where
    S: AsRef<str>,
{
    let monitor = get_monitor();
    let scope_str = scope.as_ref();
    if let Some(sd) = monitor.scopes.get(scope_str).map(|r| (*r).clone()) {
        sd.groups.iter().map(|r| r.key().clone()).collect()
    } else {
        Vec::new()
    }
}

/// Returns a list of all known scope-group combinations
///
/// This function provides a complete mapping of all existing scope and group
/// combinations in the system.
///
/// # Returns
///
/// A vector of `ScopeGroupKey` instances representing all registered combinations
///
/// # Example
///
/// ```rust
/// # use ractor::pg;
/// # use ractor::{Actor, ActorProcessingErr, ActorRef};
/// #
/// # struct Worker;
/// #
/// # #[cfg_attr(feature = "async-trait", ractor::async_trait)]
/// # impl Actor for Worker {
/// #     type Msg = ();
/// #     type State = ();
/// #     type Arguments = ();
/// #
/// #     async fn pre_start(&self, _: ActorRef<Self::Msg>, _: ()) -> Result<Self::State, ActorProcessingErr> {
/// #         Ok(())
/// #     }
/// # }
/// #
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let (worker, handle) = Actor::spawn(None, Worker, ()).await?;
/// pg::join_scoped("production", "workers", vec![worker.get_cell()]);
/// pg::join_scoped("staging", "testers", vec![worker.get_cell()]);
///
/// let all_combinations = pg::which_scopes_and_groups();
/// assert!(all_combinations.len() >= 2);
///
/// worker.stop(None);
/// handle.await;
/// # Ok(())
/// # }
/// ```
pub fn which_scopes_and_groups() -> Vec<ScopeGroupKey> {
    which_scopes()
        .into_iter()
        .map(|scope| (which_scoped_groups(scope.clone()), scope.clone()))
        .fold(Vec::new(), |mut collected, (groups, scope)| {
            collected.extend(
                groups
                    .into_iter()
                    .map(|g| ScopeGroupKey::new(scope.clone(), g)),
            );
            collected
        })
}

/// Returns a list of all known scope names
///
/// # Returns
///
/// A vector of scope names representing all registered scopes
///
/// # Example
///
/// ```rust
/// # use ractor::pg;
/// # use ractor::{Actor, ActorProcessingErr, ActorRef};
/// #
/// # struct Worker;
/// #
/// # #[cfg_attr(feature = "async-trait", ractor::async_trait)]
/// # impl Actor for Worker {
/// #     type Msg = ();
/// #     type State = ();
/// #     type Arguments = ();
/// #
/// #     async fn pre_start(&self, _: ActorRef<Self::Msg>, _: ()) -> Result<Self::State, ActorProcessingErr> {
/// #         Ok(())
/// #     }
/// # }
/// #
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let (worker, handle) = Actor::spawn(None, Worker, ()).await?;
/// pg::join_scoped("production", "workers", vec![worker.get_cell()]);
/// pg::join_scoped("staging", "workers", vec![worker.get_cell()]);
///
/// let all_scopes = pg::which_scopes();
/// assert!(all_scopes.len() >= 2);
///
/// worker.stop(None);
/// handle.await;
/// # Ok(())
/// # }
/// ```
pub fn which_scopes() -> Vec<ScopeName> {
    let monitor = get_monitor();
    monitor.scopes.iter().map(|r| r.key().clone()).collect()
}

#[must_use]
/// Returns true if cleanup should be performed
fn add_listener_scope(listeners: &DashSet<ActorCell>, actor: &ActorCell, scope: &str) -> bool {
    listeners.insert(actor.clone());
    if !actor.add_listen_scope(scope.to_owned()) {
        listeners.remove(actor);
        true
    } else {
        false
    }
}

#[must_use]
/// Returns true if cleanup should be performed
fn add_listener_group(
    listeners: &DashSet<ActorCell>,
    actor: &ActorCell,
    scope: &str,
    group: &str,
) -> bool {
    listeners.insert(actor.clone());
    if !actor.add_listen_group(scope.to_owned(), group.to_owned()) {
        listeners.remove(actor);
        true
    } else {
        false
    }
}

#[must_use]
/// Returns true if the group may be cleaned up from the scope
fn add_listener_to_group(sd: &ScopeData, actor: &ActorCell, scope: &str, group: &str) -> bool {
    if let Some(gd) = sd.groups.get(group).map(|r| (*r).clone()) {
        if add_listener_group(&gd.listeners, actor, scope, group) {
            clean_up_group(sd, group)
        } else {
            false
        }
    } else {
        let gd = match sd.groups.entry(group.to_owned()) {
            Occupied(oent) => oent.get().clone(),
            Vacant(vent) => vent.insert(Arc::new(GroupData::default())).clone(),
        };
        if add_listener_group(&gd.listeners, actor, scope, group) {
            clean_up_group(sd, group)
        } else {
            false
        }
    }
}

/// Subscribes the provided actor to group changes in the default scope
///
/// The actor will receive `GroupChangeMessage` notifications via its supervision
/// port whenever actors join or leave the specified group.
///
/// # Arguments
///
/// * `group` - The name of the group to monitor
/// * `actor` - The actor that will receive change notifications
///
/// # Example
///
/// ```rust
/// # use ractor::pg;
/// # use ractor::{Actor, ActorProcessingErr, ActorRef};
/// #
/// # struct Monitor;
/// #
/// # #[cfg_attr(feature = "async-trait", ractor::async_trait)]
/// # impl Actor for Monitor {
/// #     type Msg = ();
/// #     type State = ();
/// #     type Arguments = ();
/// #
/// #     async fn pre_start(&self, _: ActorRef<Self::Msg>, _: ()) -> Result<Self::State, ActorProcessingErr> {
/// #         Ok(())
/// #     }
/// # }
/// #
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let (monitor_actor, handle) = Actor::spawn(None, Monitor, ()).await?;
///
/// // Monitor the "worker_pool" group for changes
/// pg::monitor("worker_pool", monitor_actor.get_cell());
///
/// monitor_actor.stop(None);
/// handle.await;
/// # Ok(())
/// # }
/// ```
pub fn monitor<G>(group: G, actor: ActorCell)
where
    G: AsRef<str>,
{
    monitor_scoped(DEFAULT_SCOPE, group.as_ref(), actor);
}

/// Subscribes the provided actor to group changes in the specified scope
///
/// The actor will receive `GroupChangeMessage` notifications via its supervision
/// port whenever actors join or leave the specified group within the given scope.
///
/// Special cases:
/// - If `scope` is `ALL_SCOPES_NOTIFICATION`, monitors the group across all **existing** scopes only
/// - If `group` is `ALL_GROUPS_NOTIFICATION`, monitors all **existing** groups within the scope only
///
/// **Important:** This function only monitors existing scopes/groups at the time of the call.
/// To monitor future scopes/groups as they are created, use `monitor_world` or `monitor_scope`.
///
/// # Arguments
///
/// * `scope` - The name of the scope containing the group
/// * `group` - The name of the group to monitor
/// * `actor` - The actor that will receive change notifications
///
/// # Example
///
/// ```rust
/// # use ractor::pg;
/// # use ractor::{Actor, ActorProcessingErr, ActorRef};
/// #
/// # struct Monitor;
/// #
/// # #[cfg_attr(feature = "async-trait", ractor::async_trait)]
/// # impl Actor for Monitor {
/// #     type Msg = ();
/// #     type State = ();
/// #     type Arguments = ();
/// #
/// #     async fn pre_start(&self, _: ActorRef<Self::Msg>, _: ()) -> Result<Self::State, ActorProcessingErr> {
/// #         Ok(())
/// #     }
/// # }
/// #
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let (monitor_actor, handle) = Actor::spawn(None, Monitor, ()).await?;
///
/// // Monitor the "worker_pool" group in the "production" scope
/// pg::monitor_scoped("production", "worker_pool", monitor_actor.get_cell());
///
/// // Monitor all existing groups in the "production" scope
/// pg::monitor_scoped("production", pg::ALL_GROUPS_NOTIFICATION, monitor_actor.get_cell());
///
/// monitor_actor.stop(None);
/// handle.await;
/// # Ok(())
/// # }
/// ```
pub fn monitor_scoped<S, G>(scope: S, group: G, actor: ActorCell)
where
    S: AsRef<str>,
    G: AsRef<str>,
{
    let scope_str = scope.as_ref();
    let group_str = group.as_ref();

    if scope_str == ALL_SCOPES_NOTIFICATION {
        which_scopes()
            .into_iter()
            .for_each(|existing_scope| monitor_scoped(existing_scope, group_str, actor.clone()));
    } else if group_str == ALL_GROUPS_NOTIFICATION {
        which_scoped_groups(scope_str)
            .into_iter()
            .for_each(|existing_group| monitor_scoped(scope_str, existing_group, actor.clone()));
    } else {
        let monitor = get_monitor();
        if let Some(sd) = monitor.scopes.get(scope_str).map(|r| (*r).clone()) {
            if add_listener_to_group(&sd, &actor, scope_str, group_str) {
                clean_up_scope(monitor, scope_str)
            }
        } else {
            let sd = match monitor.scopes.entry(scope_str.to_owned()) {
                Occupied(oent) => oent.get().clone(),
                Vacant(vent) => vent.insert(Arc::new(ScopeData::default())).clone(),
            };
            if add_listener_to_group(&sd, &actor, scope_str, group_str) {
                clean_up_scope(monitor, scope_str)
            }
        }
    }
}

/// Monitors any modification in any group across all scopes
///
/// This function registers the actor to receive notifications for every group
/// change that occurs in any scope, **including new scopes and groups that are
/// created in the future**. This is the only way to monitor future scopes.
///
/// # Arguments
///
/// * `actor` - The actor that will receive all group change notifications
///
/// # Example
///
/// ```rust
/// # use ractor::pg;
/// # use ractor::{Actor, ActorProcessingErr, ActorRef};
/// #
/// # struct GlobalMonitor;
/// #
/// # #[cfg_attr(feature = "async-trait", ractor::async_trait)]
/// # impl Actor for GlobalMonitor {
/// #     type Msg = ();
/// #     type State = ();
/// #     type Arguments = ();
/// #
/// #     async fn pre_start(&self, _: ActorRef<Self::Msg>, _: ()) -> Result<Self::State, ActorProcessingErr> {
/// #         Ok(())
/// #     }
/// # }
/// #
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let (monitor_actor, handle) = Actor::spawn(None, GlobalMonitor, ()).await?;
///
/// // Monitor all groups across all scopes (current and future)
/// pg::monitor_world(&monitor_actor.get_cell());
///
/// monitor_actor.stop(None);
/// handle.await;
/// # Ok(())
/// # }
/// ```
///
/// # Note
///
/// This can generate a lot of notifications in systems with many groups.
/// Consider using more targeted monitoring with `monitor_scope` or `monitor_scoped`.
pub fn monitor_world(actor: &ActorCell) {
    let monitor = get_monitor();
    monitor.world_listeners.insert(actor.clone());
    if !actor.can_monitor() {
        monitor.world_listeners.remove(actor);
    }
}

/// Monitors the specified scope for group changes
///
/// The actor will receive notifications for all group changes within the specified
/// scope, **including new groups that are created in the future** within that scope.
///
/// Special case: If `scope` is `ALL_SCOPES_NOTIFICATION`, monitors all **existing**
/// scopes only (not future scopes). To monitor future scopes, use `monitor_world`.
///
/// # Arguments
///
/// * `scope` - The name of the scope to monitor
/// * `actor` - The actor that will receive change notifications
///
/// # Example
///
/// ```rust
/// # use ractor::pg;
/// # use ractor::{Actor, ActorProcessingErr, ActorRef};
/// #
/// # struct ScopeMonitor;
/// #
/// # #[cfg_attr(feature = "async-trait", ractor::async_trait)]
/// # impl Actor for ScopeMonitor {
/// #     type Msg = ();
/// #     type State = ();
/// #     type Arguments = ();
/// #
/// #     async fn pre_start(&self, _: ActorRef<Self::Msg>, _: ()) -> Result<Self::State, ActorProcessingErr> {
/// #         Ok(())
/// #     }
/// # }
/// #
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let (monitor_actor, handle) = Actor::spawn(None, ScopeMonitor, ()).await?;
///
/// // Monitor all groups in the "production" scope (current and future groups)
/// pg::monitor_scope("production", monitor_actor.get_cell());
///
/// // Monitor all existing scopes (current scopes only, not future ones)
/// pg::monitor_scope(pg::ALL_SCOPES_NOTIFICATION, monitor_actor.get_cell());
///
/// monitor_actor.stop(None);
/// handle.await;
/// # Ok(())
/// # }
/// ```
pub fn monitor_scope<S>(scope: S, actor: ActorCell)
where
    S: AsRef<str>,
{
    let scope_str = scope.as_ref();

    if scope_str == ALL_SCOPES_NOTIFICATION {
        // Special case: monitor all existing scopes and any future scopes
        which_scopes()
            .into_iter()
            .for_each(|existing_scope| monitor_scope(existing_scope, actor.clone()));
    } else {
        let monitor = get_monitor();

        if let Some(sd) = monitor.scopes.get(scope_str).map(|r| (*r).clone()) {
            if add_listener_scope(&sd.listeners, &actor, scope_str) {
                clean_up_scope(monitor, scope_str)
            }
        } else {
            let sd = match monitor.scopes.entry(scope_str.to_owned()) {
                Occupied(oent) => oent.get().clone(),
                Vacant(vent) => vent.insert(Arc::new(ScopeData::default())).clone(),
            };
            if add_listener_scope(&sd.listeners, &actor, scope_str) {
                clean_up_scope(monitor, scope_str)
            }
        }
    }
}

/// Unsubscribes the provided actor from group changes in the specified scope and group
///
/// Special cases:
/// - If `scope` is `ALL_SCOPES_NOTIFICATION`, removes monitoring from the group across all scopes
/// - If `group` is `ALL_GROUPS_NOTIFICATION`, removes monitoring from all groups within the scope
///
/// # Arguments
///
/// * `scope` - The name of the scope containing the group
/// * `group` - The name of the group to stop monitoring
/// * `actor` - The ID of the actor to unsubscribe
///
/// # Example
///
/// ```rust
/// # use ractor::pg;
/// # use ractor::{Actor, ActorProcessingErr, ActorRef};
/// #
/// # struct TestActor;
/// #
/// # #[cfg_attr(feature = "async-trait", ractor::async_trait)]
/// # impl Actor for TestActor {
/// #     type Msg = ();
/// #     type State = ();
/// #     type Arguments = ();
/// #
/// #     async fn pre_start(&self, _: ActorRef<Self::Msg>, _: ()) -> Result<Self::State, ActorProcessingErr> {
/// #         Ok(())
/// #     }
/// # }
/// #
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let (actor, handle) = Actor::spawn(None, TestActor, ()).await?;
/// let actor_id = actor.get_id();
///
/// // First monitor a group
/// pg::monitor_scoped("production", "worker_pool", actor.get_cell());
///
/// // Then stop monitoring the "worker_pool" group in the "production" scope
/// pg::demonitor_scoped("production", "worker_pool", actor_id);
///
/// actor.stop(None);
/// handle.await;
/// # Ok(())
/// # }
/// ```
pub fn demonitor_scoped<S, G>(scope: S, group: G, actor: ActorId)
where
    S: AsRef<str>,
    G: AsRef<str>,
{
    let monitor = get_monitor();
    let scope_str = scope.as_ref();
    let group_str = group.as_ref();

    if scope_str == ALL_SCOPES_NOTIFICATION {
        which_scopes()
            .into_iter()
            .for_each(|existing_scope| demonitor_scoped(existing_scope, group_str, actor));
    } else if group_str == ALL_GROUPS_NOTIFICATION {
        which_scoped_groups(scope_str)
            .into_iter()
            .filter(|g| g != ALL_GROUPS_NOTIFICATION)
            .for_each(|existing_group| demonitor_scoped(scope_str, existing_group, actor));
    } else if let Some(sd) = monitor.scopes.get(scope_str).map(|r| (*r).clone()) {
        if let Some(gd) = sd.groups.get(group_str).map(|r| (*r).clone()) {
            if let Some(actor_cell) = gd.listeners.remove(&actor) {
                actor_cell.remove_listen_group(scope_str, group_str);
                if clean_up_group(&sd, group_str) {
                    clean_up_scope(monitor, scope_str)
                }
            }
        }
    }
}

/// Removes the actor from world monitoring
///
/// This removes the actor from receiving notifications about all group changes
/// across all scopes. This is the opposite of `monitor_world`.
///
/// Note: Any specific registrations for individual scopes or groups will remain active.
/// To completely remove the actor from all monitoring, also call:
/// - `demonitor_scope(ALL_SCOPES_NOTIFICATION, actor_id)`
/// - `demonitor_scoped(ALL_SCOPES_NOTIFICATION, ALL_GROUPS_NOTIFICATION, actor_id)`
///
/// # Arguments
///
/// * `actor` - The actor to remove from world monitoring
///
/// # Example
///
/// ```rust
/// # use ractor::pg;
/// # use ractor::{Actor, ActorProcessingErr, ActorRef};
/// #
/// # struct TestActor;
/// #
/// # #[cfg_attr(feature = "async-trait", ractor::async_trait)]
/// # impl Actor for TestActor {
/// #     type Msg = ();
/// #     type State = ();
/// #     type Arguments = ();
/// #
/// #     async fn pre_start(&self, _: ActorRef<Self::Msg>, _: ()) -> Result<Self::State, ActorProcessingErr> {
/// #         Ok(())
/// #     }
/// # }
/// #
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let (actor, handle) = Actor::spawn(None, TestActor, ()).await?;
///
/// // First start monitoring globally
/// pg::monitor_world(&actor.get_cell());
///
/// // Stop monitoring all global changes
/// pg::demonitor_world(&actor.get_cell());
///
/// actor.stop(None);
/// handle.await;
/// # Ok(())
/// # }
/// ```
pub fn demonitor_world(actor: &ActorCell) {
    let monitor = get_monitor();
    monitor.world_listeners.remove(actor);
}

/// Unsubscribes the provided actor from group changes in the default scope
///
/// This is a convenience function that calls `demonitor_scoped` with the default scope.
///
/// # Arguments
///
/// * `group` - The name of the group to stop monitoring
/// * `actor` - The ID of the actor to unsubscribe
///
/// # Example
///
/// ```rust
/// # use ractor::pg;
/// # use ractor::{Actor, ActorProcessingErr, ActorRef};
/// #
/// # struct TestActor;
/// #
/// # #[cfg_attr(feature = "async-trait", ractor::async_trait)]
/// # impl Actor for TestActor {
/// #     type Msg = ();
/// #     type State = ();
/// #     type Arguments = ();
/// #
/// #     async fn pre_start(&self, _: ActorRef<Self::Msg>, _: ()) -> Result<Self::State, ActorProcessingErr> {
/// #         Ok(())
/// #     }
/// # }
/// #
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let (actor, handle) = Actor::spawn(None, TestActor, ()).await?;
/// let actor_id = actor.get_id();
///
/// // First monitor a group
/// pg::monitor("worker_pool", actor.get_cell());
///
/// // Stop monitoring the "worker_pool" group
/// pg::demonitor("worker_pool", actor_id);
///
/// actor.stop(None);
/// handle.await;
/// # Ok(())
/// # }
/// ```
pub fn demonitor<G>(group: G, actor: ActorId)
where
    G: AsRef<str>,
{
    demonitor_scoped(DEFAULT_SCOPE, group.as_ref(), actor)
}

/// Unsubscribes the provided actor from scope monitoring
///
/// This removes the actor from receiving notifications about all group changes
/// within the specified scope. This is the opposite of `monitor_scope`.
///
/// Note: If the actor has been registered for updates on specific groups within
/// this scope using `monitor_scoped`, those registrations will remain active.
/// To unregister from all groups within the scope, also call:
/// `demonitor_scoped(scope, ALL_GROUPS_NOTIFICATION, actor_id)`
///
/// Special case: If `scope` is `ALL_SCOPES_NOTIFICATION`, removes the actor from
/// world monitoring and from monitoring all existing scopes.
///
/// # Arguments
///
/// * `scope` - The name of the scope to stop monitoring
/// * `actor` - The ID of the actor to unsubscribe
///
/// # Example
///
/// ```rust
/// # use ractor::pg;
/// # use ractor::{Actor, ActorProcessingErr, ActorRef};
/// #
/// # struct TestActor;
/// #
/// # #[cfg_attr(feature = "async-trait", ractor::async_trait)]
/// # impl Actor for TestActor {
/// #     type Msg = ();
/// #     type State = ();
/// #     type Arguments = ();
/// #
/// #     async fn pre_start(&self, _: ActorRef<Self::Msg>, _: ()) -> Result<Self::State, ActorProcessingErr> {
/// #         Ok(())
/// #     }
/// # }
/// #
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let (actor, handle) = Actor::spawn(None, TestActor, ()).await?;
/// let actor_id = actor.get_id();
///
/// // First monitor a scope
/// pg::monitor_scope("production", actor.get_cell());
///
/// // Stop monitoring all groups in the "production" scope
/// pg::demonitor_scope("production", actor_id);
///
/// actor.stop(None);
/// handle.await;
/// # Ok(())
/// # }
/// ```
pub fn demonitor_scope<S>(scope: S, actor: ActorId)
where
    S: AsRef<str>,
{
    let monitor = get_monitor();
    let scope_str = scope.as_ref();

    if scope_str == ALL_SCOPES_NOTIFICATION {
        // Special case: remove from world listeners (for new scope notifications)
        // and demonitor all existing scopes (excluding special notification scopes)
        monitor.world_listeners.remove(&actor);
        which_scopes()
            .into_iter()
            .filter(|s| s != ALL_SCOPES_NOTIFICATION)
            .for_each(|existing_scope| demonitor_scope(existing_scope, actor));
    } else if let Some(sd) = monitor.scopes.get(scope_str).map(|r| (*r).clone()) {
        if let Some(actor_cell) = sd.listeners.remove(&actor) {
            actor_cell.remove_listen_scope(scope_str);
            clean_up_scope(monitor, scope_str)
        }
    }
}
