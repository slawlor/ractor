// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Represents an actor registry.
//!
//! It allows unique naming of actors via `'static &str` (not strings)
//! so it works more like a Erlang `atom()`
//!
//! Actors are automatically registered into the global registry, if they
//! provide a name, upon construction.Actors are also
//! automatically unenrolled from the registry upon being dropped, therefore freeing
//! the name for subsequent registration.
//!
//! You can then retrieve actors by name with [try_get]. Note: this
//! function only returns the [ActorCell] reference to the actor, it
//! additionally requires knowledge of the [crate::ActorHandler] in order
//! to send messages to it (since you need to know the message type)
//! or agents will runtime panic on message reception, and supervision
//! processes would need to restart the actors.
//!
//! ## Example
//!
//! ```rust
//! let maybe_actor = ractor::registry::try_get("my_actor");
//! if let Some(actor) = maybe_actor {
//!     // send a message, or interact with the actor
//!     // but you'll need to know the actor's strong type
//! }
//! ```

use std::sync::Arc;

use dashmap::{mapref::entry::Entry, DashMap};
use once_cell::sync::OnceCell;

use crate::{ActorCell, ActorId, ActorName};

#[cfg(test)]
mod tests;

/// Errors involving the [crate::registry]'s actor registry
pub enum ActorRegistryErr {
    /// Actor already registered
    AlreadyRegistered(ActorName),
}

/// The name'd actor registry
static ACTOR_REGISTRY: OnceCell<Arc<DashMap<ActorName, ActorCell>>> = OnceCell::new();

/// Retrieve the named actor registry handle
fn get_actor_registry() -> Arc<DashMap<ActorName, ActorCell>> {
    let reg = ACTOR_REGISTRY.get_or_init(|| Arc::new(DashMap::new()));
    reg.clone()
}

/// Put an actor into the registry
pub(crate) fn enroll(name: ActorName, actor: ActorCell) -> Result<(), ActorRegistryErr> {
    let reg = get_actor_registry();
    let entry = reg.entry(name);

    match entry {
        Entry::Occupied(_) => Err(ActorRegistryErr::AlreadyRegistered(name)),
        Entry::Vacant(vacancy) => {
            vacancy.insert(actor);
            Ok(())
        }
    }
}

/// Check if an actor name is enrolled in the registry
///
/// * `name` - The actor's name
/// * `pid` - the ID of the actor in question
pub(crate) fn is_enrolled(name: ActorName, pid: ActorId) -> bool {
    match get_actor_registry().entry(name) {
        Entry::Occupied(actor) => actor.get().get_id() == pid,
        _ => false,
    }
}

/// Remove an actor from the registry given it's actor name
pub(crate) fn unenroll(name: ActorName) {
    let reg = get_actor_registry();
    let entry = reg.entry(name);
    if let Entry::Occupied(actor) = entry {
        let _ = actor.remove();
    }
}

/// Try and retrieve an actor from the registry
///
/// * `name` - The name of the [ActorCell] to try and retrieve
///
/// Returns: Some(actor) on successful identification of an actor, None if
/// actor not registered
pub fn try_get(name: ActorName) -> Option<ActorCell> {
    let reg = get_actor_registry();
    reg.get(&name).map(|item| item.value().clone())
}
