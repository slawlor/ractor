// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! [ActorCell] is reference counted actor which can be passed around as needed
//!
//! This module contains all the functionality around the [ActorCell], including
//! the internal properties, ports, states, etc. [ActorCell] is the basic primitive
//! for references to a given actor and its communication channels

use std::sync::Arc;

use super::errors::MessagingErr;
use super::messages::{BoxedMessage, Signal, StopMessage};

use super::SupervisionEvent;
use crate::port::{BoundedInputPortReceiver, InputPortReceiver};
use crate::{ActorHandler, ActorId, ActorName, SpawnErr};

pub mod actor_ref;
pub use actor_ref::ActorRef;

mod actor_properties;
use actor_properties::ActorProperties;

/// [ActorStatus] represents the status of an actor
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
    pub(crate) signal_rx: BoundedInputPortReceiver<Signal>,
    pub(crate) stop_rx: BoundedInputPortReceiver<StopMessage>,
    pub(crate) supervisor_rx: InputPortReceiver<SupervisionEvent>,
    pub(crate) message_rx: InputPortReceiver<BoxedMessage>,
}

pub(crate) enum ActorPortMessage {
    Signal(Signal),
    Stop(StopMessage),
    Supervision(SupervisionEvent),
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
        future: impl futures::Future<Output = TState>,
    ) -> Result<TState, Signal>
    where
        TState: crate::State,
    {
        tokio::select! {
            // Biased ensures that we poll the ports in the order they appear, giving
            // priority to our message reception operations. See:
            // https://docs.rs/tokio/latest/tokio/macro.select.html#fairness
            // for more information
            biased;

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

    /// List to the input ports in priority. The priority of listening for messages is
    /// 1. Signal port
    /// 2. Stop port
    /// 3. Supervision message port
    /// 4. General message port
    ///
    /// Returns [Ok(ActorPortMessage)] on a successful message reception, [MessagingErr]
    /// in the event any of the channels is closed.
    pub async fn listen_in_priority(&mut self) -> Result<ActorPortMessage, MessagingErr> {
        tokio::select! {
            // Biased ensures that we poll the ports in the order they appear, giving
            // priority to our message reception operations. See:
            // https://docs.rs/tokio/latest/tokio/macro.select.html#fairness
            // for more information
            biased;

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

/// A handy-dandy reference to and actor and their inner properties
/// which can be cloned and passed around
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
        TActor: ActorHandler,
    {
        let (props, rx1, rx2, rx3, rx4) = ActorProperties::new::<TActor>(name);
        let cell = Self {
            inner: Arc::new(props),
        };
        if let Some(r_name) = name {
            crate::registry::enroll(r_name, cell.clone())?;
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

    /// Retrieve the [super::Actor]'s unique identifier [ActorId]
    pub fn get_id(&self) -> ActorId {
        self.inner.id
    }

    /// Retrieve the [super::Actor]'s name
    pub fn get_name(&self) -> Option<ActorName> {
        self.inner.name
    }

    /// Retrieve the current status of an [super::Actor]
    ///
    /// Returns the [super::Actor]'s current [ActorStatus]
    pub fn get_status(&self) -> ActorStatus {
        self.inner.get_status()
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
            // If it's enrolled in the registry, remove it
            if let Some(name) = self.get_name() {
                crate::registry::unenroll(name);
            }
            // Leave all + stop monitoring pg groups (if any)
            crate::pg::demonitor_all(self.get_id());
            crate::pg::leave_all(self.get_id());
        }

        self.inner.set_status(status)
    }

    /// Terminate this [super::Actor] and all it's children
    pub(crate) fn terminate(&self) {
        // we don't need to nofity of exit if we're already stopping or stopped
        if self.get_status() as u8 <= ActorStatus::Upgrading as u8 {
            // kill myself immediately. Ignores failures, as a failure means either
            // 1. we're already dead or
            // 2. the channel is full of "signals"
            self.kill();
        }

        // notify children they should die. They will unlink themselves from the supervisor
        self.inner.tree.terminate_children();
    }

    /// Link this [super::Actor] to the supervisor
    ///
    /// * `supervisor` - The supervisor to link this [super::Actor] to
    pub fn link(&self, supervisor: ActorCell) {
        supervisor.inner.tree.insert_parent(self.clone());
        self.inner.tree.insert_child(supervisor);
    }

    /// Unlink this [super::Actor] from the supervisor
    ///
    /// * `supervisor` - The supervisor to unlink this [super::Actor] from
    pub fn unlink(&self, supervisor: ActorCell) {
        supervisor.inner.tree.remove_parent(self.clone());
        self.inner.tree.remove_child(supervisor);
    }

    /// Kill this [super::Actor] forcefully (terminates async work)
    pub fn kill(&self) {
        let _ = self.inner.send_signal(Signal::Kill);
    }

    /// Stop this [super::Actor] gracefully (stopping message processing)
    ///
    /// * `reason` - An optional static string reason why the stop is occurring
    pub fn stop(&self, reason: Option<String>) {
        // ignore failures, since that means the actor is dead already
        let _ = self.inner.send_stop(reason);
    }

    /// Send a supervisor event to the supervisory port
    ///
    /// * `message` - The [SupervisionEvent] to send to the supervisory port
    ///
    /// Returns [Ok(())] on successful message send, [Err(MessagingErr)] otherwise
    pub(crate) fn send_supervisor_evt(
        &self,
        message: SupervisionEvent,
    ) -> Result<(), MessagingErr> {
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
    pub fn send_message<TActor>(&self, message: TActor::Msg) -> Result<(), MessagingErr>
    where
        TActor: ActorHandler,
    {
        self.inner.send_message::<TActor>(message)
    }

    /// Notify the supervisors that a supervision event occurred
    ///
    /// * `evt` - The event to send to this [super::Actor]'s supervisors
    pub fn notify_supervisors<TActor>(&self, evt: SupervisionEvent)
    where
        TActor: ActorHandler,
    {
        self.inner.tree.notify_supervisors::<TActor>(evt)
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
