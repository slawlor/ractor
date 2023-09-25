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
/// the actor's message type everywhere.
///
/// An [ActorRef] is the primary means of communication typically used
/// when interfacing with [super::Actor]s
///
/// The [ActorRef] is SPECIFICALLY marked [Sync], regardless of the message type
/// because all usages of the message type are to send an owned instance of a message
/// and in no case is that message instance shared across threads. This is guaranteed
/// by the underlying Tokio channel usages. Without this manual marking of [Sync] on
/// [ActorRef], we would need to constrain the message type [Message] to be [Sync] which
/// is overly restrictive.
pub struct ActorRef<TMessage> {
    pub(crate) inner: ActorCell,
    _tactor: PhantomData<TMessage>,
}

unsafe impl<T> Sync for ActorRef<T> {}

impl<TMessage> Clone for ActorRef<TMessage> {
    fn clone(&self) -> Self {
        ActorRef {
            inner: self.inner.clone(),
            _tactor: PhantomData::<TMessage>,
        }
    }
}

impl<TMessage> std::ops::Deref for ActorRef<TMessage> {
    type Target = ActorCell;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<TMessage> From<ActorCell> for ActorRef<TMessage> {
    fn from(value: ActorCell) -> Self {
        Self {
            inner: value,
            _tactor: PhantomData::<TMessage>,
        }
    }
}

impl<TActor> From<ActorRef<TActor>> for ActorCell {
    fn from(value: ActorRef<TActor>) -> Self {
        value.inner
    }
}

impl<TMessage> std::fmt::Debug for ActorRef<TMessage> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.inner)
    }
}

impl<TMessage> ActorRef<TMessage> {
    /// Retrieve a cloned [ActorCell] representing this [ActorRef]
    pub fn get_cell(&self) -> ActorCell {
        self.inner.clone()
    }

    /// Notify the supervisor and all monitors that a supervision event occurred.
    /// Monitors receive a reduced copy of the supervision event which won't contain
    /// the [crate::actor::BoxedState] and collapses the [crate::ActorProcessingErr]
    /// exception to a [String]
    ///
    /// * `evt` - The event to send to this [crate::Actor]'s supervisors
    pub fn notify_supervisor_and_monitors(&self, evt: SupervisionEvent) {
        self.inner.notify_supervisor_and_monitors(evt)
    }
}

impl<TMessage> crate::actor::ActorRef<TMessage>
where
    TMessage: Message,
{
    /// Send a strongly-typed message, constructing the boxed message on the fly
    ///
    /// * `message` - The message to send
    ///
    /// Returns [Ok(())] on successful message send, [Err(MessagingErr)] otherwise
    pub fn send_message(&self, message: TMessage) -> Result<(), MessagingErr<TMessage>> {
        self.inner.send_message::<TMessage>(message)
    }

    // ========================== General Actor Operation Aliases ========================== //

    // -------------------------- ActorRegistry -------------------------- //

    /// Try and retrieve a strongly-typed actor from the registry.
    ///
    /// Alias of [crate::registry::where_is]
    pub fn where_is(name: ActorName) -> Option<crate::actor::ActorRef<TMessage>> {
        if let Some(actor) = crate::registry::where_is(name) {
            // check the type id when pulling from the registry
            if actor.get_type_id() == TypeId::of::<TMessage>() {
                Some(actor.into())
            } else {
                None
            }
        } else {
            None
        }
    }
}
