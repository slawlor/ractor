// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! [ActorCell] is reference counted actor which can be passed around as needed
//!
//! This module contains all the functionality around the [ActorCell], including
//! the internal properties, ports, states, etc. [ActorCell] is the basic primitive
//! for references to a given actor and its communication channels

use std::any::TypeId;
use std::sync::Arc;

#[cfg(feature = "async-std")]
use futures::FutureExt;

use super::messages::{Signal, StopMessage};
use super::SupervisionEvent;
use crate::actor::actor_properties::ActorProperties;
use crate::concurrency::{
    MpscReceiver as BoundedInputPortReceiver, MpscUnboundedReceiver as InputPortReceiver,
};
use crate::errors::MessagingErr;
use crate::message::BoxedMessage;
#[cfg(feature = "cluster")]
use crate::message::SerializedMessage;
use crate::RactorErr;
use crate::{Actor, ActorName, SpawnErr};
use crate::{ActorId, Message};

/// [ActorStatus] represents the status of an actor's lifecycle
#[derive(Debug, Clone, Eq, PartialEq, Copy)]
#[repr(u8)]
pub enum ActorStatus {
    /// Created, but not yet started
    Unstarted = 0u8,
    /// Starting
    Starting = 1u8,
    /// Executing (or waiting on messages)
    Running = 2u8,
    /// Upgrading
    Upgrading = 3u8,
    /// Stopping
    Stopping = 4u8,
    /// Dead
    Stopped = 5u8,
}

/// Actor states where operations can continue to interact with an agent
pub const ACTIVE_STATES: [ActorStatus; 3] = [
    ActorStatus::Starting,
    ActorStatus::Running,
    ActorStatus::Upgrading,
];

/// The collection of ports an actor needs to listen to
pub(crate) struct ActorPortSet {
    /// The inner signal port
    pub(crate) signal_rx: BoundedInputPortReceiver<Signal>,
    /// The inner stop port
    pub(crate) stop_rx: BoundedInputPortReceiver<StopMessage>,
    /// The inner supervisor port
    pub(crate) supervisor_rx: InputPortReceiver<SupervisionEvent>,
    /// The inner message port
    pub(crate) message_rx: InputPortReceiver<BoxedMessage>,
}

impl Drop for ActorPortSet {
    fn drop(&mut self) {
        // Close all the message ports and flush all the message queue backlogs.
        // See: https://docs.rs/tokio/0.1.22/tokio/sync/mpsc/index.html#clean-shutdown
        self.signal_rx.close();
        self.stop_rx.close();
        self.supervisor_rx.close();
        self.message_rx.close();

        while self.signal_rx.try_recv().is_ok() {}
        while self.stop_rx.try_recv().is_ok() {}
        while self.supervisor_rx.try_recv().is_ok() {}
        while self.message_rx.try_recv().is_ok() {}
    }
}

/// Messages that come in off an actor's port, with associated priority
pub(crate) enum ActorPortMessage {
    /// A signal message
    Signal(Signal),
    /// A stop message
    Stop(StopMessage),
    /// A supervision message
    Supervision(SupervisionEvent),
    /// A regular message
    Message(BoxedMessage),
}

impl ActorPortSet {
    /// Run a future beside the signal port, so that
    /// the signal port can terminate the async work
    ///
    /// * `future` - The future to execute
    ///
    /// Returns [Ok(`TState`)] when the future completes without
    /// signal interruption, [Err(Signal)] in the event the
    /// signal interrupts the async work.
    pub async fn run_with_signal<TState>(
        &mut self,
        future: impl std::future::Future<Output = TState>,
    ) -> Result<TState, Signal>
    where
        TState: crate::State,
    {
        #[cfg(feature = "async-std")]
        {
            crate::concurrency::select! {
                // supervision or message processing work
                // can be interrupted by the signal port receiving
                // a kill signal
                signal = self.signal_rx.recv().fuse() => {
                    Err(signal.unwrap_or(Signal::Kill))
                }
                new_state = future.fuse() => {
                    Ok(new_state)
                }
            }
        }
        #[cfg(not(feature = "async-std"))]
        {
            crate::concurrency::select! {
                // supervision or message processing work
                // can be interrupted by the signal port receiving
                // a kill signal
                signal = self.signal_rx.recv() => {
                    Err(signal.unwrap_or(Signal::Kill))
                }
                new_state = future => {
                    Ok(new_state)
                }
            }
        }
    }

    /// List to the input ports in priority. The priority of listening for messages is
    /// 1. Signal port
    /// 2. Stop port
    /// 3. Supervision message port
    /// 4. General message port
    ///
    /// Returns [Ok(ActorPortMessage)] on a successful message reception, [MessagingErr]
    /// in the event any of the channels is closed.
    pub async fn listen_in_priority(&mut self) -> Result<ActorPortMessage, MessagingErr<()>> {
        #[cfg(feature = "async-std")]
        {
            crate::concurrency::select! {
                signal = self.signal_rx.recv().fuse() => {
                    signal.map(ActorPortMessage::Signal).ok_or(MessagingErr::ChannelClosed)
                }
                stop = self.stop_rx.recv().fuse() => {
                    stop.map(ActorPortMessage::Stop).ok_or(MessagingErr::ChannelClosed)
                }
                supervision = self.supervisor_rx.recv().fuse() => {
                    supervision.map(ActorPortMessage::Supervision).ok_or(MessagingErr::ChannelClosed)
                }
                message = self.message_rx.recv().fuse() => {
                    message.map(ActorPortMessage::Message).ok_or(MessagingErr::ChannelClosed)
                }
            }
        }
        #[cfg(not(feature = "async-std"))]
        {
            crate::concurrency::select! {
                signal = self.signal_rx.recv() => {
                    signal.map(ActorPortMessage::Signal).ok_or(MessagingErr::ChannelClosed)
                }
                stop = self.stop_rx.recv() => {
                    stop.map(ActorPortMessage::Stop).ok_or(MessagingErr::ChannelClosed)
                }
                supervision = self.supervisor_rx.recv() => {
                    supervision.map(ActorPortMessage::Supervision).ok_or(MessagingErr::ChannelClosed)
                }
                message = self.message_rx.recv() => {
                    message.map(ActorPortMessage::Message).ok_or(MessagingErr::ChannelClosed)
                }
            }
        }
    }
}

/// An [ActorCell] is a reference to an [Actor]'s communication channels
/// and provides external access to send messages, stop, kill, and generally
/// interactor with the underlying [Actor] process.
///
/// The input ports contained in the cell will return an error should the
/// underlying actor have terminated and no longer exist.
#[derive(Clone)]
pub struct ActorCell {
    inner: Arc<ActorProperties>,
}

impl std::fmt::Debug for ActorCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(name) = self.get_name() {
            write!(f, "Actor '{}' (id: {})", name, self.get_id())
        } else {
            write!(f, "Actor with id: {}", self.get_id())
        }
    }
}

impl ActorCell {
    /// Construct a new [ActorCell] pointing to an [super::Actor] and return the message reception channels as a [ActorPortSet]
    ///
    /// * `name` - Optional name for the actor
    ///
    /// Returns a tuple [(ActorCell, ActorPortSet)] to bootstrap the [crate::Actor]
    pub(crate) fn new<TActor>(name: Option<ActorName>) -> Result<(Self, ActorPortSet), SpawnErr>
    where
        TActor: Actor,
    {
        let (props, rx1, rx2, rx3, rx4) = ActorProperties::new::<TActor>(name.clone());
        let cell = Self {
            inner: Arc::new(props),
        };

        #[cfg(feature = "cluster")]
        {
            // registry to the PID registry
            crate::registry::pid_registry::register_pid(cell.get_id(), cell.clone())?;
        }

        if let Some(r_name) = name {
            crate::registry::register(r_name, cell.clone())?;
        }

        Ok((
            cell,
            ActorPortSet {
                signal_rx: rx1,
                stop_rx: rx2,
                supervisor_rx: rx3,
                message_rx: rx4,
            },
        ))
    }

    /// Create a new remote actor, to be called from the `ractor_cluster` crate
    #[cfg(feature = "cluster")]
    pub(crate) fn new_remote<TActor>(
        name: Option<ActorName>,
        id: ActorId,
    ) -> Result<(Self, ActorPortSet), SpawnErr>
    where
        TActor: Actor,
    {
        if id.is_local() {
            return Err(SpawnErr::StartupFailed(From::from("Cannot create a new remote actor handler without the actor id being marked as a remote actor!")));
        }

        let (props, rx1, rx2, rx3, rx4) = ActorProperties::new_remote::<TActor>(name, id);
        let cell = Self {
            inner: Arc::new(props),
        };
        // NOTE: remote actors don't appear in the name registry
        // if let Some(r_name) = name {
        //     crate::registry::register(r_name, cell.clone())?;
        // }
        Ok((
            cell,
            ActorPortSet {
                signal_rx: rx1,
                stop_rx: rx2,
                supervisor_rx: rx3,
                message_rx: rx4,
            },
        ))
    }

    /// Retrieve the [super::Actor]'s unique identifier [ActorId]
    pub fn get_id(&self) -> ActorId {
        self.inner.id
    }

    /// Retrieve the [super::Actor]'s name
    pub fn get_name(&self) -> Option<ActorName> {
        self.inner.name.clone()
    }

    /// Retrieve the current status of an [super::Actor]
    ///
    /// Returns the [super::Actor]'s current [ActorStatus]
    pub fn get_status(&self) -> ActorStatus {
        self.inner.get_status()
    }

    /// Identifies if this actor supports remote (dist) communication
    ///
    /// Returns [true] if the actor's messaging protocols support remote calls, [false] otherwise
    #[cfg(feature = "cluster")]
    pub fn supports_remoting(&self) -> bool {
        self.inner.supports_remoting
    }

    /// Set the status of the [super::Actor]. If the status is set to
    /// [ActorStatus::Stopping] or [ActorStatus::Stopped] the actor
    /// will also be unenrolled from both the named registry ([crate::registry])
    /// and the PG groups ([crate::pg]) if it's enrolled in any
    ///
    /// * `status` - The [ActorStatus] to set
    pub(crate) fn set_status(&self, status: ActorStatus) {
        // The actor is shut down
        if status == ActorStatus::Stopped || status == ActorStatus::Stopping {
            #[cfg(feature = "cluster")]
            {
                // stop monitoring for updates
                crate::registry::pid_registry::demonitor(self.get_id());
                // unregistry from the PID registry
                crate::registry::pid_registry::unregister_pid(self.get_id());
            }
            // If it's enrolled in the registry, remove it
            if let Some(name) = self.get_name() {
                crate::registry::unregister(name);
            }
            // Leave all + stop monitoring pg groups (if any)
            crate::pg::demonitor_all(self.get_id());
            crate::pg::leave_all(self.get_id());
        }

        // Fix for #254. We should only notify the stop listener AFTER post_stop
        // has executed, which is when the state gets set to `Stopped`.
        if status == ActorStatus::Stopped {
            // notify whoever might be waiting on the stop signal
            self.inner.notify_stop_listener();
        }

        self.inner.set_status(status)
    }

    /// Terminate this [super::Actor] and all it's children
    pub(crate) fn terminate(&self) {
        // we don't need to notify of exit if we're already stopping or stopped
        if self.get_status() as u8 <= ActorStatus::Upgrading as u8 {
            // kill myself immediately. Ignores failures, as a failure means either
            // 1. we're already dead or
            // 2. the channel is full of "signals"
            self.kill();
        }

        // notify children they should die. They will unlink themselves from the supervisor
        self.inner.tree.terminate_all_children();
    }

    /// Link this [super::Actor] to the provided supervisor
    ///
    /// * `supervisor` - The supervisor [super::Actor] of this actor
    pub fn link(&self, supervisor: ActorCell) {
        supervisor.inner.tree.insert_child(self.clone());
        self.inner.tree.set_supervisor(supervisor);
    }

    /// Unlink this [super::Actor] from the supervisor if it's
    /// currently linked (if self's supervisor is `supervisor`)
    ///
    /// * `supervisor` - The supervisor to unlink this [super::Actor] from
    pub fn unlink(&self, supervisor: ActorCell) {
        if self.inner.tree.is_child_of(supervisor.get_id()) {
            supervisor.inner.tree.remove_child(self.get_id());
            self.inner.tree.clear_supervisor();
        }
    }

    /// Clear the supervisor field
    pub(crate) fn clear_supervisor(&self) {
        self.inner.tree.clear_supervisor();
    }

    /// Kill this [super::Actor] forcefully (terminates async work)
    pub fn kill(&self) {
        let _ = self.inner.send_signal(Signal::Kill);
    }

    /// Kill this [super::Actor] forcefully (terminates async work)
    /// and wait for the actor shutdown to complete
    ///
    /// * `timeout` - An optional timeout duration to wait for shutdown to occur
    ///
    /// Returns [Ok(())] upon the actor being stopped/shutdown. [Err(RactorErr::Messaging(_))] if the channel is closed
    /// or dropped (which may indicate some other process is trying to shutdown this actor) or [Err(RactorErr::Timeout)]
    /// if timeout was hit before the actor was successfully shut down (when set)
    pub async fn kill_and_wait(
        &self,
        timeout: Option<crate::concurrency::Duration>,
    ) -> Result<(), RactorErr<()>> {
        if let Some(to) = timeout {
            match crate::concurrency::timeout(to, self.inner.send_signal_and_wait(Signal::Kill))
                .await
            {
                Err(_) => Err(RactorErr::Timeout),
                Ok(Err(e)) => Err(e.into()),
                Ok(_) => Ok(()),
            }
        } else {
            Ok(self.inner.send_signal_and_wait(Signal::Kill).await?)
        }
    }

    /// Stop this [super::Actor] gracefully (stopping message processing)
    ///
    /// * `reason` - An optional string reason why the stop is occurring
    pub fn stop(&self, reason: Option<String>) {
        // ignore failures, since that means the actor is dead already
        let _ = self.inner.send_stop(reason);
    }

    /// Stop the [super::Actor] gracefully (stopping messaging processing)
    /// and wait for the actor shutdown to complete
    ///
    /// * `reason` - An optional string reason why the stop is occurring
    /// * `timeout` - An optional timeout duration to wait for shutdown to occur
    ///
    /// Returns [Ok(())] upon the actor being stopped/shutdown. [Err(RactorErr::Messaging(_))] if the channel is closed
    /// or dropped (which may indicate some other process is trying to shutdown this actor) or [Err(RactorErr::Timeout)]
    /// if timeout was hit before the actor was successfully shut down (when set)
    pub async fn stop_and_wait(
        &self,
        reason: Option<String>,
        timeout: Option<crate::concurrency::Duration>,
    ) -> Result<(), RactorErr<StopMessage>> {
        if let Some(to) = timeout {
            match crate::concurrency::timeout(to, self.inner.send_stop_and_wait(reason)).await {
                Err(_) => Err(RactorErr::Timeout),
                Ok(Err(e)) => Err(e.into()),
                Ok(_) => Ok(()),
            }
        } else {
            Ok(self.inner.send_stop_and_wait(reason).await?)
        }
    }

    /// Send a supervisor event to the supervisory port
    ///
    /// * `message` - The [SupervisionEvent] to send to the supervisory port
    ///
    /// Returns [Ok(())] on successful message send, [Err(MessagingErr)] otherwise
    pub(crate) fn send_supervisor_evt(
        &self,
        message: SupervisionEvent,
    ) -> Result<(), MessagingErr<SupervisionEvent>> {
        self.inner.send_supervisor_evt(message)
    }

    /// Send a strongly-typed message, constructing the boxed message on the fly
    ///
    /// Note: The type requirement of `TActor` assures that `TMsg` is the supported
    /// message type for `TActor` such that we can't send boxed messages of an unsupported
    /// type to the specified actor.
    ///
    /// * `message` - The message to send
    ///
    /// Returns [Ok(())] on successful message send, [Err(MessagingErr)] otherwise
    pub fn send_message<TMessage>(&self, message: TMessage) -> Result<(), MessagingErr<TMessage>>
    where
        TMessage: Message,
    {
        self.inner.send_message::<TMessage>(message)
    }

    /// Send a serialized binary message to the actor.
    ///
    /// * `message` - The message to send
    ///
    /// Returns [Ok(())] on successful message send, [Err(MessagingErr)] otherwise
    #[cfg(feature = "cluster")]
    pub fn send_serialized(
        &self,
        message: SerializedMessage,
    ) -> Result<(), MessagingErr<SerializedMessage>> {
        self.inner.send_serialized(message)
    }

    /// Notify the supervisor and all monitors that a supervision event occurred.
    /// Monitors receive a reduced copy of the supervision event which won't contain
    /// the [crate::actor::BoxedState] and collapses the [crate::ActorProcessingErr]
    /// exception to a [String]
    ///
    /// * `evt` - The event to send to this [super::Actor]'s supervisors
    pub fn notify_supervisor(&self, evt: SupervisionEvent) {
        self.inner.tree.notify_supervisor(evt)
    }

    pub(crate) fn get_type_id(&self) -> TypeId {
        self.inner.type_id
    }

    // ================== Test Utilities ================== //

    #[cfg(test)]
    pub(crate) fn get_num_children(&self) -> usize {
        self.inner.tree.get_num_children()
    }

    #[cfg(test)]
    pub(crate) fn get_num_parents(&self) -> usize {
        self.inner.tree.get_num_parents()
    }
}
