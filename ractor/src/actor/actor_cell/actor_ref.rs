// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! [ActorRef] is a strongly-typed wrapper over an [ActorCell]

use std::any::TypeId;
use std::marker::PhantomData;

use crate::{ActorHandler, ActorId, ActorName, ActorStatus, MessagingErr, SupervisionEvent};

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

    /// Retrieve the [crate::Actor]'s unique identifier [ActorId]
    pub fn get_id(&self) -> ActorId {
        self.inner.get_id()
    }

    /// Retrieve the [crate::Actor]'s name
    pub fn get_name(&self) -> Option<ActorName> {
        self.inner.get_name()
    }

    /// Retrieve the current status of an [crate::Actor]
    ///
    /// Returns the [crate::Actor]'s current [ActorStatus]
    pub fn get_status(&self) -> ActorStatus {
        self.inner.get_status()
    }

    /// Set the status of the [crate::Actor]
    ///
    /// * `status` - The [ActorStatus] to set
    pub(crate) fn set_status(&self, status: ActorStatus) {
        self.inner.set_status(status)
    }

    /// Terminate this [crate::Actor] and all it's children
    pub(crate) fn terminate(&self) {
        self.inner.terminate();
    }

    /// Link this [crate::Actor] to the supervisor
    ///
    /// * `supervisor` - The supervisor to link this [crate::Actor] to
    pub fn link(&self, supervisor: ActorCell) {
        self.inner.link(supervisor);
    }

    /// Unlink this [crate::Actor] from the supervisor
    ///
    /// * `supervisor` - The supervisor to unlink this [crate::Actor] from
    pub fn unlink(&self, supervisor: ActorCell) {
        self.inner.unlink(supervisor)
    }

    /// Kill this [crate::Actor] forcefully (terminates async work)
    pub fn kill(&self) {
        self.inner.kill();
    }

    /// Stop this [crate::Actor] gracefully (stopping message processing)
    ///
    /// * `reason` - An optional static string reason why the stop is occurring
    pub fn stop(&self, reason: Option<String>) {
        self.inner.stop(reason);
    }

    /// Send a strongly-typed message, constructing the boxed message on the fly
    ///
    /// * `message` - The message to send
    ///
    /// Returns [Ok(())] on successful message send, [Err(MessagingErr)] otherwise
    pub fn send_message(&self, message: TActor::Msg) -> Result<(), MessagingErr> {
        self.inner.send_message::<TActor>(message)
    }

    /// Send a supervisor event to the supervisory port
    ///
    /// * `message` - The [SupervisionEvent] to send to the supervisory port
    ///
    /// Returns [Ok(())] on successful message send, [Err(MessagingErr)] otherwise
    #[cfg(test)]
    pub(crate) fn send_supervisor_evt(
        &self,
        message: SupervisionEvent,
    ) -> Result<(), MessagingErr> {
        self.inner.send_supervisor_evt(message)
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
    /// Alias of [crate::registry::try_get]
    pub fn try_get_from_registry(name: ActorName) -> Option<ActorRef<TActor>> {
        if let Some(actor) = crate::registry::try_get(name) {
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
