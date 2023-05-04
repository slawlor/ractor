// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! [ActorRef] is a strongly-typed wrapper over an [ActorCell]

use std::any::TypeId;
use std::marker::PhantomData;

use crate::{ActorName, Message, MessagingErr, SupervisionEvent};

use super::ActorCell;

/// An [ActorRef] is a strongly-typed wrapper over an [ActorCell]
/// to provide some syntactic wrapping on the requirement to pass
/// the actor type everywhere
pub struct ActorRef<TMessage>
where
    TMessage: Message,
{
    pub(crate) inner: ActorCell,
    _tactor: PhantomData<TMessage>,
}

impl<TMessage> Clone for ActorRef<TMessage>
where
    TMessage: Message,
{
    fn clone(&self) -> Self {
        ActorRef {
            inner: self.inner.clone(),
            _tactor: PhantomData::<TMessage>,
        }
    }
}

impl<TMessage> std::ops::Deref for ActorRef<TMessage>
where
    TMessage: Message,
{
    type Target = ActorCell;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<TMessage> From<ActorCell> for ActorRef<TMessage>
where
    TMessage: Message,
{
    fn from(value: ActorCell) -> Self {
        Self {
            inner: value,
            _tactor: PhantomData::<TMessage>,
        }
    }
}

impl<TActor> From<ActorRef<TActor>> for ActorCell
where
    TActor: Message,
{
    fn from(value: ActorRef<TActor>) -> Self {
        value.inner
    }
}

impl<TMessage> std::fmt::Debug for ActorRef<TMessage>
where
    TMessage: Message,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.inner)
    }
}

impl<TMessage> ActorRef<TMessage>
where
    TMessage: Message,
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
    pub fn send_message(&self, message: TMessage) -> Result<(), MessagingErr<TMessage>> {
        self.inner.send_message::<TMessage>(message)
    }

    /// Notify the supervisors that a supervision event occurred
    ///
    /// * `evt` - The event to send to this [crate::Actor]'s supervisors
    pub fn notify_supervisor(&self, evt: SupervisionEvent) {
        self.inner.notify_supervisor(evt)
    }

    // ========================== General Actor Operation Aliases ========================== //

    // -------------------------- ActorRegistry -------------------------- //

    /// Try and retrieve a strongly-typed actor from the registry.
    ///
    /// Alias of [crate::registry::where_is]
    pub fn where_is(name: ActorName) -> Option<ActorRef<TMessage>> {
        if let Some(actor) = crate::registry::where_is(name) {
            // check the type id when pulling from the registry
            if actor.inner.type_id == TypeId::of::<TMessage>() {
                Some(actor.into())
            } else {
                None
            }
        } else {
            None
        }
    }
}
