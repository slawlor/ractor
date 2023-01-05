// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! A reference counted actor which can be passed around as needed

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc;

use super::messages::{BoxedMessage, Signal};
use super::supervision::SupervisionTree;
use super::SupervisionEvent;
use crate::port::input::InputPortReceiver;
use crate::ActorId;

/// The status of the agent
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

/// The collection of ports an agent needs to listen to
pub struct ActorPortSet {
    pub(crate) signal_rx: mpsc::Receiver<Signal>,
    pub(crate) supervisor_rx: mpsc::UnboundedReceiver<SupervisionEvent>,
    pub(crate) message_rx: InputPortReceiver<BoxedMessage>,
}

/// The inner-properties of an Actor
struct ActorProperties {
    id: ActorId,
    name: Option<String>,
    status: Arc<AtomicU8>,
    signal: mpsc::Sender<Signal>,
    supervision: mpsc::UnboundedSender<SupervisionEvent>,
    message: mpsc::UnboundedSender<BoxedMessage>,
    tree: SupervisionTree,
}
impl ActorProperties {
    pub fn new(
        name: Option<String>,
    ) -> (
        Self,
        mpsc::Receiver<Signal>,
        mpsc::UnboundedReceiver<SupervisionEvent>,
        mpsc::UnboundedReceiver<BoxedMessage>,
    ) {
        let (tx, rx) = mpsc::channel(2);
        let (tx2, rx2) = mpsc::unbounded_channel();
        let (tx3, rx3) = mpsc::unbounded_channel();
        (
            Self {
                id: crate::AGENT_ID_ALLOCATOR.fetch_add(1u64, Ordering::Relaxed),
                name,
                status: Arc::new(AtomicU8::new(ActorStatus::Unstarted as u8)),
                signal: tx,
                supervision: tx2,
                message: tx3,
                tree: SupervisionTree::default(),
            },
            rx,
            rx2,
            rx3,
        )
    }

    pub fn get_status(&self) -> ActorStatus {
        match self.status.load(Ordering::Relaxed) {
            0u8 => ActorStatus::Unstarted,
            1u8 => ActorStatus::Starting,
            2u8 => ActorStatus::Running,
            3u8 => ActorStatus::Upgrading,
            4u8 => ActorStatus::Stopping,
            _ => ActorStatus::Stopped,
        }
    }

    pub fn set_status(&self, status: ActorStatus) {
        self.status.store(status as u8, Ordering::Relaxed);
    }

    pub async fn send_signal(&self, signal: Signal) -> Result<(), mpsc::error::SendError<Signal>> {
        self.signal.send(signal).await
    }

    pub fn send_supervisor_evt(
        &self,
        message: SupervisionEvent,
    ) -> Result<(), mpsc::error::SendError<SupervisionEvent>> {
        self.supervision.send(message)
    }

    pub fn send_message(
        &self,
        message: BoxedMessage,
    ) -> Result<(), mpsc::error::SendError<BoxedMessage>> {
        self.message.send(message)
    }
}

/// A handy-dandy reference to actor's and their inner properties
#[derive(Clone)]
pub struct ActorCell {
    inner: Arc<ActorProperties>,
}

impl std::fmt::Debug for ActorCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Agent {}", self.get_id())
    }
}

impl ActorCell {
    /// Construct a new actor cell and return the message reception channels
    pub fn new(name: Option<String>) -> (Self, ActorPortSet) {
        let (props, rx1, rx2, rx3) = ActorProperties::new(name);
        (
            Self {
                inner: Arc::new(props),
            },
            ActorPortSet {
                signal_rx: rx1,
                supervisor_rx: rx2,
                message_rx: rx3,
            },
        )
    }

    /// Retrieve the agent's id
    pub fn get_id(&self) -> ActorId {
        self.inner.id
    }

    /// Retrieve the agent's name
    pub fn get_name(&self) -> Option<String> {
        self.inner.name.clone()
    }

    /// Retrieve the status of an actor
    pub fn get_status(&self) -> ActorStatus {
        self.inner.get_status()
    }

    /// Set the status of the actor
    pub fn set_status(&self, status: ActorStatus) {
        self.inner.set_status(status)
    }

    // /// Returns the state object, only in the event of a dead actor. Otherwise
    // /// returns None. We cannot read the state of a live actor (for thread safety)
    // pub fn get_state(&self) -> Option<Box<dyn std::any::Any>> {
    //     None
    // }

    /// Terminate yourself and all children beneath you
    pub async fn terminate(&self) {
        // we don't need to nofity of exit if we're already stopping or stopped
        if self.get_status() as u8 <= ActorStatus::Upgrading as u8 {
            // kill myself immediately. Ignores failures, as a failure means either
            // 1. we're already dead or
            // 2. the channel is full of "signals"
            let _ = self.send_signal(Signal::Exit).await;
        }

        // notify children they should die. They will unlink themselves from the supervisor
        self.inner.tree.terminate_children().await;
    }

    /// Link another actor to the supervised list
    pub async fn link(&self, other: ActorCell) {
        other.inner.tree.insert_parent(self.clone()).await;
        self.inner.tree.insert_child(other).await;
    }

    /// Unlink another actor from the supervised list
    pub async fn unlink(&self, other: ActorCell) {
        other.inner.tree.remove_parent(self.clone()).await;
        self.inner.tree.remove_child(other).await;
    }

    /// Send a signal event to the signal port
    pub async fn send_signal(&self, signal: Signal) -> Result<(), mpsc::error::SendError<Signal>> {
        self.inner.send_signal(signal).await
    }

    /// Stop this agent, but sending Signal::Exit
    pub async fn stop(&self) -> Result<(), mpsc::error::SendError<Signal>> {
        self.send_signal(Signal::Exit).await
    }

    /// Send a supervisor event to the supervisory port
    pub fn send_supervisor_evt(
        &self,
        message: SupervisionEvent,
    ) -> Result<(), mpsc::error::SendError<SupervisionEvent>> {
        self.inner.send_supervisor_evt(message)
    }

    /// Send a message to the regular message handling port
    pub fn send_message(
        &self,
        message: BoxedMessage,
    ) -> Result<(), mpsc::error::SendError<BoxedMessage>> {
        self.inner.send_message(message)
    }

    /// Send a strongly-typed message, constructing the boxed message on the fly
    pub fn send_message_t<TMsg: crate::Message>(
        &self,
        message: TMsg,
    ) -> Result<(), mpsc::error::SendError<BoxedMessage>> {
        self.inner.send_message(BoxedMessage::new(message, true))
    }

    /// Notify the supervisors that a supervision event occurred
    pub async fn notify_supervisors(
        &self,
        message: SupervisionEvent,
    ) -> Result<(), mpsc::error::SendError<SupervisionEvent>> {
        self.inner.tree.notify_supervisors(message).await
    }

    #[cfg(test)]
    pub fn get_tree(&self) -> SupervisionTree {
        self.inner.tree.clone()
    }
}
