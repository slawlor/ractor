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
//!     let maybe_actor = ractor::registry::where_is("my_actor");
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

/// Retrieve the named actor registry handle
fn get_actor_registry<'a>() -> &'a Arc<DashMap<ActorName, ActorCell>> {
    ACTOR_REGISTRY.get_or_init(|| Arc::new(DashMap::new()))
}

/// Put an actor into the registry
pub(crate) fn register(name: ActorName, actor: ActorCell) -> Result<(), ActorRegistryErr> {
    match get_actor_registry().entry(name) {
        Occupied(_) => Err(ActorRegistryErr::AlreadyRegistered(name)),
        Vacant(vacancy) => {
            vacancy.insert(actor);
            Ok(())
        }
    }
}

/// Remove an actor from the registry given it's actor name
pub(crate) fn unregister(name: ActorName) {
    if let Some(reg) = ACTOR_REGISTRY.get() {
        let _ = reg.remove(&name);
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
    reg.iter().map(|kvp| *kvp.key()).collect::<Vec<_>>()
}
