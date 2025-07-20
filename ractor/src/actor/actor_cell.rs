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

use super::actor_properties::MuxedMessage;
use super::messages::Signal;
use super::messages::StopMessage;
use super::SupervisionEvent;
use crate::actor::actor_properties::ActorProperties;
#[cfg(feature = "derived-actor-from-cell")]
use crate::actor::request_derived::OwnedRequest;
use crate::concurrency::JoinHandle;
use crate::concurrency::MpscUnboundedReceiver as InputPortReceiver;
use crate::concurrency::OneshotReceiver;
use crate::errors::MessagingErr;
#[cfg(feature = "cluster")]
use crate::message::SerializedMessage;
use crate::Actor;
use crate::ActorId;
use crate::ActorName;
use crate::ActorRef;
#[cfg(feature = "derived-actor-from-cell")]
use crate::DerivedActorRef;
use crate::Message;
use crate::RactorErr;
use crate::SpawnErr;

/// [ActorStatus] represents the status of an actor's lifecycle
#[derive(Debug, Clone, Eq, PartialEq, Copy, PartialOrd, Ord)]
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
    /// Draining
    Draining = 4u8,
    /// Stopping
    Stopping = 5u8,
    /// Dead
    Stopped = 6u8,
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
    pub(crate) signal_rx: OneshotReceiver<Signal>,
    /// The inner stop port
    pub(crate) stop_rx: OneshotReceiver<StopMessage>,
    /// The inner supervisor port
    pub(crate) supervisor_rx: InputPortReceiver<SupervisionEvent>,
    /// The inner message port
    pub(crate) message_rx: InputPortReceiver<MuxedMessage>,
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
    Message(MuxedMessage),
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
    pub(crate) async fn run_with_signal<TState>(
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
                signal = (&mut self.signal_rx).fuse() => {
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
                signal = &mut self.signal_rx => {
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
    pub(crate) async fn listen_in_priority(
        &mut self,
    ) -> Result<ActorPortMessage, MessagingErr<()>> {
        #[cfg(feature = "async-std")]
        {
            crate::concurrency::select! {
                signal = (&mut self.signal_rx).fuse() => {
                    signal.map(ActorPortMessage::Signal).map_err(|_| MessagingErr::ChannelClosed)
                }
                stop = (&mut self.stop_rx).fuse() => {
                    stop.map(ActorPortMessage::Stop).map_err(|_| MessagingErr::ChannelClosed)
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
                signal = &mut self.signal_rx => {
                    signal.map(ActorPortMessage::Signal).map_err(|_| MessagingErr::ChannelClosed)
                }
                stop = &mut self.stop_rx => {
                    stop.map(ActorPortMessage::Stop).map_err(|_| MessagingErr::ChannelClosed)
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
    pub(crate) inner: Arc<ActorProperties>,
}

impl std::fmt::Debug for ActorCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Actor")
            .field("name", &self.get_name())
            .field("id", &self.get_id())
            .finish()
    }
}

impl PartialEq for ActorCell {
    fn eq(&self, other: &Self) -> bool {
        other.get_id() == self.get_id()
    }
}

impl Eq for ActorCell {}

impl std::hash::Hash for ActorCell {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.get_id().hash(state)
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

    /// Retrieve the current status of an [super::Actor]
    ///
    /// Returns the [super::Actor]'s current [ActorStatus]
    #[cfg(feature = "derived-actor-from-cell")]
    pub fn provide_derived<T: Message>(&self) -> Option<DerivedActorRef<T>> {
        let mut owned_request = OwnedRequest::<T>::new();
        self.inner
            .derived_provider
            .provide_derived_actor_ref(self.clone(), owned_request.as_request());
        owned_request.extract()
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

    /// Monitor the provided [super::Actor] for supervision events. An actor in `ractor` can
    /// only have a single supervisor, denoted by the `link` function, however they
    /// may have multiple `monitors`. Monitor's receive copies of the [SupervisionEvent]s,
    /// with non-cloneable information removed.
    ///
    /// * `who`: The actor to monitor
    #[cfg(feature = "monitors")]
    pub fn monitor(&self, who: ActorCell) {
        who.inner.tree.set_monitor(self.clone());
    }

    /// Stop monitoring the provided [super::Actor] for supervision events.
    ///
    /// * `who`: The actor to stop monitoring
    #[cfg(feature = "monitors")]
    pub fn unmonitor(&self, who: ActorCell) {
        who.inner.tree.remove_monitor(self.get_id());
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

    /// Wait for the actor to exit, optionally within a timeout
    ///
    /// * `timeout`: If supplied, the amount of time to wait before
    ///   returning an error and cancelling the wait future.
    ///
    /// IMPORTANT: If the timeout is hit, the actor is still running.
    /// You should wait again for its exit.
    pub async fn wait(
        &self,
        timeout: Option<crate::concurrency::Duration>,
    ) -> Result<(), crate::concurrency::Timeout> {
        if let Some(to) = timeout {
            crate::concurrency::timeout(to, self.inner.wait()).await
        } else {
            self.inner.wait().await;
            Ok(())
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

    /// Drain the actor's message queue and when finished processing, terminate the actor.
    ///
    /// Any messages received after the drain marker but prior to shutdown will be rejected
    pub fn drain(&self) -> Result<(), MessagingErr<()>> {
        self.inner.drain()
    }

    /// Drain the actor's message queue and when finished processing, terminate the actor,
    /// notifying on this handler that the actor has drained and exited (stopped).
    ///
    /// * `timeout`: The optional amount of time to wait for the drain to complete.
    ///
    /// Any messages received after the drain marker but prior to shutdown will be rejected
    pub async fn drain_and_wait(
        &self,
        timeout: Option<crate::concurrency::Duration>,
    ) -> Result<(), RactorErr<()>> {
        if let Some(to) = timeout {
            match crate::concurrency::timeout(to, self.inner.drain_and_wait()).await {
                Err(_) => Err(RactorErr::Timeout),
                Ok(Err(e)) => Err(e.into()),
                Ok(_) => Ok(()),
            }
        } else {
            Ok(self.inner.drain_and_wait().await?)
        }
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
    ) -> Result<(), Box<MessagingErr<SerializedMessage>>> {
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

    /// Stop any children of this actor, not waiting for their exit, and threading
    /// the optional reason to all children
    ///
    /// * `reason`: The stop reason to send to all the children
    ///
    /// This swallows and communication errors because if you can't send a message
    /// to the child, it's dropped the message channel, and is dead/stopped already.
    pub fn stop_children(&self, reason: Option<String>) {
        self.inner.tree.stop_all_children(reason);
    }

    /// Tries to retrieve this actor's supervisor.
    ///
    /// Returns [None] if this actor has no supervisor at the given instance or
    /// [Some(ActorCell)] supervisor if one is configured.
    pub fn try_get_supervisor(&self) -> Option<ActorCell> {
        self.inner.tree.try_get_supervisor()
    }

    /// Stop any children of this actor, and wait for their collective exit, optionally
    /// threading the optional reason to all children
    ///
    /// * `reason`: The stop reason to send to all the children
    /// * `timeout`: An optional timeout which is the maximum time to wait for the actor stop
    ///   operation to complete
    ///
    /// This swallows and communication errors because if you can't send a message
    /// to the child, it's dropped the message channel, and is dead/stopped already.
    pub async fn stop_children_and_wait(
        &self,
        reason: Option<String>,
        timeout: Option<crate::concurrency::Duration>,
    ) {
        self.inner
            .tree
            .stop_all_children_and_wait(reason, timeout)
            .await
    }

    /// Drain any children of this actor, not waiting for their exit
    ///
    /// This swallows and communication errors because if you can't send a message
    /// to the child, it's dropped the message channel, and is dead/stopped already.
    pub fn drain_children(&self) {
        self.inner.tree.drain_all_children();
    }

    /// Drain any children of this actor, and wait for their collective exit
    ///
    /// * `timeout`: An optional timeout which is the maximum time to wait for the actor stop
    ///   operation to complete
    pub async fn drain_children_and_wait(&self, timeout: Option<crate::concurrency::Duration>) {
        self.inner.tree.drain_all_children_and_wait(timeout).await
    }

    /// Retrieve the supervised children of this actor (if any)
    ///
    /// Returns a [Vec] of [ActorCell]s which are the children that are
    /// presently linked to this actor.
    pub fn get_children(&self) -> Vec<ActorCell> {
        self.inner.tree.get_children()
    }

    /// Retrieve the [TypeId] of this [ActorCell] which can be helpful
    /// for quick type-checking.
    ///
    /// HOWEVER: Note this is an unstable identifier, and changes between
    /// Rust releases and may not be stable over a network call.
    pub fn get_type_id(&self) -> TypeId {
        self.inner.type_id
    }

    /// Runtime check the message type of this actor, which only works for
    /// local actors, as remote actors send serializable messages, and can't
    /// have their message type runtime checked.
    ///
    /// Returns [None] if the actor is a remote actor, and we cannot perform a
    /// runtime message type check. Otherwise [Some(true)] for the correct message
    /// type or [Some(false)] for an incorrect type will returned.
    pub fn is_message_type_of<TMessage: Message>(&self) -> Option<bool> {
        if self.get_id().is_local() {
            Some(self.get_type_id() == std::any::TypeId::of::<TMessage>())
        } else {
            None
        }
    }

    /// Spawn an actor of the given type as a child of this actor, automatically starting the actor.
    /// This [ActorCell] becomes the supervisor of the child actor.
    ///
    /// * `name`: A name to give the actor. Useful for global referencing or debug printing
    /// * `handler` The implementation of Self
    /// * `startup_args`: Arguments passed to the `pre_start` call of the [Actor] to facilitate startup and
    ///   initial state creation
    ///
    /// Returns a [Ok((ActorRef, JoinHandle<()>))] upon successful start, denoting the actor reference
    /// along with the join handle which will complete when the actor terminates. Returns [Err(SpawnErr)] if
    /// the actor failed to start
    pub async fn spawn_linked<T: Actor>(
        &self,
        name: Option<String>,
        handler: T,
        startup_args: T::Arguments,
    ) -> Result<(ActorRef<T::Msg>, JoinHandle<()>), SpawnErr> {
        crate::actor::ActorRuntime::spawn_linked(name, handler, startup_args, self.clone()).await
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
