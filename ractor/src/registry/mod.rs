// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Represents an actor registry.
//!
//! It allows unique naming of actors via `String`
//! so it works more or less like an Erlang `atom()`
//!
//! Actors are automatically registered into the global registry, if they
//! provide a name, upon construction.Actors are also
//! automatically unenrolled from the registry upon being dropped, therefore freeing
//! the name for subsequent registration.
//!
//! You can then retrieve actors by name with [where_is]. Note: this
//! function only returns the [ActorCell] reference to the actor, it
//! additionally requires knowledge of the [crate::Actor] in order
//! to send messages to it (since you need to know the message type)
//! or agents will runtime panic on message reception, and supervision
//! processes would need to restart the actors.
//!
//! ## Example
//!
//! ```rust
//! async fn test() {
//!     let maybe_actor = ractor::registry::where_is("my_actor".to_string());
//!     if let Some(actor) = maybe_actor {
//!         // send a message, or interact with the actor
//!         // but you'll need to know the actor's strong type
//!     }
//! }
//! ```

use std::sync::Arc;

use dashmap::mapref::entry::Entry::{Occupied, Vacant};
use dashmap::DashMap;
use once_cell::sync::OnceCell;

#[cfg(feature = "cluster")]
use crate::ActorId;
use crate::{ActorCell, ActorName};

#[cfg(test)]
mod tests;

/// Errors involving the [crate::registry]'s actor registry
pub enum ActorRegistryErr {
    /// Actor already registered
    AlreadyRegistered(ActorName),
}

/// The name'd actor registry
static ACTOR_REGISTRY: OnceCell<Arc<DashMap<ActorName, ActorCell>>> = OnceCell::new();
#[cfg(feature = "cluster")]
static PID_REGISTRY: OnceCell<Arc<DashMap<ActorId, ActorCell>>> = OnceCell::new();

/// Retrieve the named actor registry handle
fn get_actor_registry<'a>() -> &'a Arc<DashMap<ActorName, ActorCell>> {
    ACTOR_REGISTRY.get_or_init(|| Arc::new(DashMap::new()))
}
#[cfg(feature = "cluster")]
fn get_pid_registry<'a>() -> &'a Arc<DashMap<ActorId, ActorCell>> {
    PID_REGISTRY.get_or_init(|| Arc::new(DashMap::new()))
}

/// Put an actor into the registry
pub(crate) fn register(name: ActorName, actor: ActorCell) -> Result<(), ActorRegistryErr> {
    match get_actor_registry().entry(name.clone()) {
        Occupied(_) => Err(ActorRegistryErr::AlreadyRegistered(name)),
        Vacant(vacancy) => {
            vacancy.insert(actor);
            Ok(())
        }
    }
}
#[cfg(feature = "cluster")]
pub(crate) fn register_pid(id: ActorId, actor: ActorCell) {
    if id.is_local() {
        get_pid_registry().insert(id, actor);
    }
}

/// Remove an actor from the registry given it's actor name
pub(crate) fn unregister(name: ActorName) {
    if let Some(reg) = ACTOR_REGISTRY.get() {
        let _ = reg.remove(&name);
    }
}
#[cfg(feature = "cluster")]
pub(crate) fn unregister_pid(id: ActorId) {
    if id.is_local() {
        let _ = get_pid_registry().remove(&id);
    }
}

/// Try and retrieve an actor from the registry
///
/// * `name` - The name of the [ActorCell] to try and retrieve
///
/// Returns: Some(actor) on successful identification of an actor, None if
/// actor not registered
pub fn where_is(name: ActorName) -> Option<ActorCell> {
    let reg = get_actor_registry();
    reg.get(&name).map(|v| v.value().clone())
}

/// Returns a list of names that have been registered
///
/// Returns: A [Vec<&'str>] of actor names which are registered
/// currently
pub fn registered() -> Vec<ActorName> {
    let reg = get_actor_registry();
    reg.iter().map(|kvp| kvp.key().clone()).collect::<Vec<_>>()
}

/// Retrieve an actor from the global registery of all local actors
///
/// * `id` - The **local** id of the actor to retrieve
///
/// Returns [Some(_)] if the actor exists locally, [None] otherwise
#[cfg(feature = "cluster")]
pub fn get_pid(id: ActorId) -> Option<ActorCell> {
    if id.is_local() {
        get_pid_registry().get(&id).map(|v| v.value().clone())
    } else {
        None
    }
}
