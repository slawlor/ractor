// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! [ActorRef] is a strongly-typed wrapper over an [ActorCell]

use std::any::TypeId;
use std::marker::PhantomData;

use crate::{ActorHandler, ActorName, MessagingErr, SupervisionEvent};

use super::ActorCell;

/// An [ActorRef] is a strongly-typed wrapper over an [ActorCell]
/// to provide some syntactic wrapping on the requirement to pass
/// the actor type everywhere
pub struct ActorRef<TActor>
where
    TActor: ActorHandler,
{
    pub(crate) inner: ActorCell,
    _tactor: PhantomData<TActor>,
}

impl<TActor> Clone for ActorRef<TActor>
where
    TActor: ActorHandler,
{
    fn clone(&self) -> Self {
        ActorRef {
            inner: self.inner.clone(),
            _tactor: PhantomData::<TActor>,
        }
    }
}

impl<TActor> std::ops::Deref for ActorRef<TActor>
where
    TActor: ActorHandler,
{
    type Target = ActorCell;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<TActor> From<ActorCell> for ActorRef<TActor>
where
    TActor: ActorHandler,
{
    fn from(value: ActorCell) -> Self {
        Self {
            inner: value,
            _tactor: PhantomData::<TActor>,
        }
    }
}

impl<TActor> From<ActorRef<TActor>> for ActorCell
where
    TActor: ActorHandler,
{
    fn from(value: ActorRef<TActor>) -> Self {
        value.inner
    }
}

impl<TActor> std::fmt::Debug for ActorRef<TActor>
where
    TActor: ActorHandler,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.inner)
    }
}

impl<TActor> ActorRef<TActor>
where
    TActor: ActorHandler,
{
    /// Retrieve a cloned [ActorCell] representing this [ActorRef]
    pub fn get_cell(&self) -> ActorCell {
        self.inner.clone()
    }

    /// Send a strongly-typed message, constructing the boxed message on the fly
    ///
    /// * `message` - The message to send
    ///
    /// Returns [Ok(())] on successful message send, [Err(MessagingErr)] otherwise
    pub fn send_message(&self, message: TActor::Msg) -> Result<(), MessagingErr> {
        self.inner.send_message::<TActor>(message)
    }

    /// Notify the supervisors that a supervision event occurred
    ///
    /// * `evt` - The event to send to this [crate::Actor]'s supervisors
    pub fn notify_supervisors(&self, evt: SupervisionEvent) {
        self.inner.notify_supervisors::<TActor>(evt)
    }

    // ========================== General Actor Operation Aliases ========================== //

    // -------------------------- ActorRegistry -------------------------- //

    /// Try and retrieve a strongly-typed actor from the registry.
    ///
    /// Alias of [crate::registry::where_is]
    pub fn where_is(name: ActorName) -> Option<ActorRef<TActor>> {
        if let Some(actor) = crate::registry::where_is(name) {
            // check the type id when pulling from the registry
            if actor.inner.type_id == TypeId::of::<TActor>() {
                Some(actor.into())
            } else {
                None
            }
        } else {
            None
        }
    }
}
