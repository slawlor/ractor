// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use crate::actor::messages::StopMessage;
use crate::actor::supervision::SupervisionTree;
use crate::concurrency::{
    MpscReceiver as BoundedInputPortReceiver, MpscSender as BoundedInputPort,
    MpscUnboundedReceiver as InputPortReceiver, MpscUnboundedSender as InputPort,
};
use crate::message::BoxedMessage;
#[cfg(feature = "cluster")]
use crate::message::SerializedMessage;
use crate::{concurrency as mpsc, Message};
use crate::{Actor, ActorId, ActorName, ActorStatus, MessagingErr, Signal, SupervisionEvent};

// The inner-properties of an Actor
pub(crate) struct ActorProperties {
    pub(crate) id: ActorId,
    pub(crate) name: Option<ActorName>,
    status: Arc<AtomicU8>,
    wait_handler: Arc<mpsc::BroadcastSender<()>>,
    _wait_handler_rx: mpsc::BroadcastReceiver<()>,
    pub(crate) signal: BoundedInputPort<Signal>,
    pub(crate) stop: BoundedInputPort<StopMessage>,
    pub(crate) supervision: InputPort<SupervisionEvent>,
    pub(crate) message: InputPort<BoxedMessage>,
    pub(crate) tree: SupervisionTree,
    pub(crate) type_id: std::any::TypeId,
    #[cfg(feature = "cluster")]
    pub(crate) supports_remoting: bool,
}

impl ActorProperties {
    pub fn new<TActor>(
        name: Option<ActorName>,
    ) -> (
        Self,
        BoundedInputPortReceiver<Signal>,
        BoundedInputPortReceiver<StopMessage>,
        InputPortReceiver<SupervisionEvent>,
        InputPortReceiver<BoxedMessage>,
    )
    where
        TActor: Actor,
    {
        Self::new_remote::<TActor>(name, crate::actor_id::get_new_local_id())
    }

    pub fn new_remote<TActor>(
        name: Option<ActorName>,
        id: ActorId,
    ) -> (
        Self,
        BoundedInputPortReceiver<Signal>,
        BoundedInputPortReceiver<StopMessage>,
        InputPortReceiver<SupervisionEvent>,
        InputPortReceiver<BoxedMessage>,
    )
    where
        TActor: Actor,
    {
        let (tx_signal, rx_signal) = mpsc::mpsc_bounded(2);
        let (tx_stop, rx_stop) = mpsc::mpsc_bounded(2);
        let (tx_supervision, rx_supervision) = mpsc::mpsc_unbounded();
        let (tx_message, rx_message) = mpsc::mpsc_unbounded();
        let (tx_shutdown, rx_shutdown) = mpsc::broadcast(2);
        (
            Self {
                id,
                name,
                status: Arc::new(AtomicU8::new(ActorStatus::Unstarted as u8)),
                signal: tx_signal,
                wait_handler: Arc::new(tx_shutdown),
                _wait_handler_rx: rx_shutdown,
                stop: tx_stop,
                supervision: tx_supervision,
                message: tx_message,
                tree: SupervisionTree::default(),
                type_id: std::any::TypeId::of::<TActor::Msg>(),
                #[cfg(feature = "cluster")]
                supports_remoting: TActor::Msg::serializable(),
            },
            rx_signal,
            rx_stop,
            rx_supervision,
            rx_message,
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

    pub fn send_signal(&self, signal: Signal) -> Result<(), MessagingErr<()>> {
        self.signal
            .try_send(signal)
            .map_err(|_| MessagingErr::SendErr(()))
    }

    pub fn send_supervisor_evt(
        &self,
        message: SupervisionEvent,
    ) -> Result<(), MessagingErr<SupervisionEvent>> {
        self.supervision.send(message).map_err(|e| e.into())
    }

    pub fn send_message<TMessage>(&self, message: TMessage) -> Result<(), MessagingErr<TMessage>>
    where
        TMessage: Message,
    {
        // Only type-check messages of local actors, remote actors send serialized
        // payloads
        if self.id.is_local() && self.type_id != std::any::TypeId::of::<TMessage>() {
            return Err(MessagingErr::InvalidActorType);
        }

        let boxed = message
            .box_message(&self.id)
            .map_err(|_e| MessagingErr::InvalidActorType)?;
        self.message
            .send(boxed)
            .map_err(|e| MessagingErr::SendErr(TMessage::from_boxed(e.0).unwrap()))
    }

    #[cfg(feature = "cluster")]
    pub fn send_serialized(
        &self,
        message: SerializedMessage,
    ) -> Result<(), MessagingErr<SerializedMessage>> {
        let boxed = BoxedMessage {
            msg: None,
            serialized_msg: Some(message),
        };
        self.message
            .send(boxed)
            .map_err(|e| MessagingErr::SendErr(e.0.serialized_msg.unwrap()))
    }

    pub fn send_stop(&self, reason: Option<String>) -> Result<(), MessagingErr<StopMessage>> {
        let msg = reason.map(StopMessage::Reason).unwrap_or(StopMessage::Stop);
        self.stop.try_send(msg).map_err(|e| e.into())
    }

    /// Send the stop signal, threading in a OneShot sender which notifies when the shutdown is completed
    pub async fn send_stop_and_wait(
        &self,
        reason: Option<String>,
    ) -> Result<(), MessagingErr<StopMessage>> {
        let mut rx = self.wait_handler.subscribe();
        self.send_stop(reason)?;
        rx.recv().await.map_err(|_| MessagingErr::ChannelClosed)
    }

    /// Send the kill signal, threading in a OneShot sender which notifies when the shutdown is completed
    pub async fn send_signal_and_wait(&self, signal: Signal) -> Result<(), MessagingErr<()>> {
        // first bind the wait handler
        let mut rx = self.wait_handler.subscribe();
        let _ = self.send_signal(signal);
        rx.recv().await.map_err(|_| MessagingErr::ChannelClosed)
    }

    pub fn notify_stop_listener(&self) {
        let _ = self.wait_handler.send(());
    }
}
