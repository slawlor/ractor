// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! A reference counted actor which can be passed around as needed

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::time::Duration;

use super::errors::MessagingErr;
use super::messages::{BoxedMessage, Signal};
use super::supervision::SupervisionTree;
use super::SupervisionEvent;
use crate::port::{
    BoundedInputPort, BoundedInputPortReceiver, InputPort, InputPortReceiver, RpcReplyPort,
};
use crate::rpc::{self, CallResult};
use crate::{ActorHandler, ActorId, Message};

/// The status of the actor
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

/// Denotes states where operations can continue to interact with an agent
pub const ACTIVE_STATES: [ActorStatus; 3] = [
    ActorStatus::Starting,
    ActorStatus::Running,
    ActorStatus::Upgrading,
];

/// The collection of ports an actor needs to listen to
pub struct ActorPortSet {
    pub(crate) signal_rx: BoundedInputPortReceiver<Signal>,
    pub(crate) supervisor_rx: InputPortReceiver<SupervisionEvent>,
    pub(crate) message_rx: InputPortReceiver<BoxedMessage>,
}

/// The inner-properties of an Actor
struct ActorProperties {
    id: ActorId,
    name: Option<String>,
    status: Arc<AtomicU8>,
    signal: BoundedInputPort<Signal>,
    supervision: InputPort<SupervisionEvent>,
    message: InputPort<BoxedMessage>,
    tree: SupervisionTree,
}
impl ActorProperties {
    pub fn new(
        name: Option<String>,
    ) -> (
        Self,
        BoundedInputPortReceiver<Signal>,
        InputPortReceiver<SupervisionEvent>,
        InputPortReceiver<BoxedMessage>,
    ) {
        let (tx, rx) = mpsc::channel(2);
        let (tx2, rx2) = mpsc::unbounded_channel();
        let (tx3, rx3) = mpsc::unbounded_channel();
        (
            Self {
                id: crate::ACTOR_ID_ALLOCATOR.fetch_add(1u64, Ordering::Relaxed),
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

    pub fn send_signal(&self, signal: Signal) -> Result<(), MessagingErr> {
        self.signal.try_send(signal).map_err(|e| e.into())
    }

    pub fn send_supervisor_evt(&self, message: SupervisionEvent) -> Result<(), MessagingErr> {
        self.supervision.send(message).map_err(|e| e.into())
    }

    pub fn send_message(&self, message: BoxedMessage) -> Result<(), MessagingErr> {
        self.message.send(message).map_err(|e| e.into())
    }
}

/// A handy-dandy reference to actor's and their inner properties
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

    /// Retrieve the actor's id
    pub fn get_id(&self) -> ActorId {
        self.inner.id
    }

    /// Retrieve the actor's name
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

    /// Terminate yourself and all children beneath you
    pub fn terminate(&self) {
        // we don't need to nofity of exit if we're already stopping or stopped
        if self.get_status() as u8 <= ActorStatus::Upgrading as u8 {
            // kill myself immediately. Ignores failures, as a failure means either
            // 1. we're already dead or
            // 2. the channel is full of "signals"
            self.stop();
        }

        // notify children they should die. They will unlink themselves from the supervisor
        self.inner.tree.terminate_children();
    }

    /// Link another actor to the supervised list
    pub fn link(&self, other: ActorCell) {
        other.inner.tree.insert_parent(self.clone());
        self.inner.tree.insert_child(other);
    }

    /// Unlink another actor from the supervised list
    pub fn unlink(&self, other: ActorCell) {
        other.inner.tree.remove_parent(self.clone());
        self.inner.tree.remove_child(other);
    }
    /// Stop this actor, by sending Signal::Exit
    pub fn stop(&self) {
        // ignore failures, since either the actor is already dead
        // or the channel is full of "signals" which is also fine
        // since it'll die shortly
        let _ = self.inner.send_signal(Signal::Exit);
    }

    /// Send a supervisor event to the supervisory port
    pub fn send_supervisor_evt(&self, message: SupervisionEvent) -> Result<(), MessagingErr> {
        self.inner.send_supervisor_evt(message)
    }

    /// Send a strongly-typed message, constructing the boxed message on the fly
    ///
    /// Note: The type requirement of `TActor` assures that `TMsg` is the supported
    /// message type for `TActor` such that we can't send boxed messages of an unsupported
    /// type to the specified actor.
    pub fn send_message<TActor, TMsg>(&self, message: TMsg) -> Result<(), MessagingErr>
    where
        TActor: ActorHandler<Msg = TMsg>,
        TMsg: Message,
    {
        self.inner.send_message(BoxedMessage::new(message))
    }

    /// Notify the supervisors that a supervision event occurred
    pub fn notify_supervisors<TActor, TState>(&self, evt: SupervisionEvent)
    where
        TActor: ActorHandler<State = TState>,
        TState: crate::State,
    {
        self.inner.tree.notify_supervisors::<TActor, _>(evt)
    }

    /// Alias of [rpc::cast]
    pub fn cast<TActor, TMsg>(&self, msg: TMsg) -> Result<(), MessagingErr>
    where
        TActor: ActorHandler<Msg = TMsg>,
        TMsg: Message,
    {
        rpc::cast::<TActor, _>(self, msg)
    }

    /// Alias of [rpc::call]
    pub async fn call<TActor, TMsg, TReply, TMsgBuilder>(
        &self,
        msg_builder: TMsgBuilder,
        timeout_option: Option<Duration>,
    ) -> Result<CallResult<TReply>, MessagingErr>
    where
        TActor: ActorHandler<Msg = TMsg>,
        TMsg: Message,
        TMsgBuilder: FnOnce(RpcReplyPort<TReply>) -> TMsg,
    {
        rpc::call::<TActor, TMsg, TReply, TMsgBuilder>(self, msg_builder, timeout_option).await
    }

    /// Alias of [rpc::call_and_forward]
    pub fn call_and_forward<
        TActor,
        TForwardActor,
        TMsg,
        TReply,
        TMsgBuilder,
        FwdMapFn,
        TForwardMessage,
    >(
        &self,
        msg_builder: TMsgBuilder,
        response_forward: ActorCell,
        forward_mapping: FwdMapFn,
        timeout_option: Option<Duration>,
    ) -> Result<tokio::task::JoinHandle<CallResult<Result<(), MessagingErr>>>, MessagingErr>
    where
        TActor: ActorHandler<Msg = TMsg>,
        TMsg: Message,
        TReply: Message,
        TMsgBuilder: FnOnce(RpcReplyPort<TReply>) -> TMsg,
        TForwardActor: ActorHandler<Msg = TForwardMessage>,
        FwdMapFn: FnOnce(TReply) -> TForwardMessage + Send + 'static,
        TForwardMessage: Message,
    {
        rpc::call_and_forward::<
            TActor,
            TForwardActor,
            TMsg,
            TReply,
            TMsgBuilder,
            FwdMapFn,
            TForwardMessage,
        >(
            self,
            msg_builder,
            response_forward,
            forward_mapping,
            timeout_option,
        )
    }

    /// Test utility to retrieve a clone of the underlying supervision tree
    #[cfg(test)]
    pub(crate) fn get_tree(&self) -> SupervisionTree {
        self.inner.tree.clone()
    }
}
