// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use crate::actor::messages::StopMessage;
use crate::actor::supervision::SupervisionTree;
use crate::concurrency::{
    MpscUnboundedReceiver as InputPortReceiver, MpscUnboundedSender as InputPort, OneshotReceiver,
    OneshotSender as OneshotInputPort,
};
use crate::message::BoxedMessage;
#[cfg(feature = "cluster")]
use crate::message::SerializedMessage;
use crate::{concurrency as mpsc, Message};
use crate::{Actor, ActorId, ActorName, ActorStatus, MessagingErr, Signal, SupervisionEvent};

/// A muxed-message wrapper which allows the message port to receive either a message or a drain
/// request which is a point-in-time marker that the actor's input channel should be drained
pub(crate) enum MuxedMessage {
    Drain,
    Message(BoxedMessage),
}

// The inner-properties of an Actor
pub(crate) struct ActorProperties {
    pub(crate) id: ActorId,
    pub(crate) name: Option<ActorName>,
    pub(crate) status: Arc<AtomicU8>,
    pub(crate) wait_handler: Arc<mpsc::Notify>,
    pub(crate) signal: Mutex<Option<OneshotInputPort<Signal>>>,
    pub(crate) stop: Mutex<Option<OneshotInputPort<StopMessage>>>,
    pub(crate) supervision: InputPort<SupervisionEvent>,
    pub(crate) message: InputPort<MuxedMessage>,
    pub(crate) tree: SupervisionTree,
    pub(crate) type_id: std::any::TypeId,
    #[cfg(feature = "cluster")]
    pub(crate) supports_remoting: bool,
}

impl ActorProperties {
    pub(crate) fn new<TActor>(
        name: Option<ActorName>,
    ) -> (
        Self,
        OneshotReceiver<Signal>,
        OneshotReceiver<StopMessage>,
        InputPortReceiver<SupervisionEvent>,
        InputPortReceiver<MuxedMessage>,
    )
    where
        TActor: Actor,
    {
        Self::new_remote::<TActor>(name, crate::actor::actor_id::get_new_local_id())
    }

    pub(crate) fn new_remote<TActor>(
        name: Option<ActorName>,
        id: ActorId,
    ) -> (
        Self,
        OneshotReceiver<Signal>,
        OneshotReceiver<StopMessage>,
        InputPortReceiver<SupervisionEvent>,
        InputPortReceiver<MuxedMessage>,
    )
    where
        TActor: Actor,
    {
        let (tx_signal, rx_signal) = mpsc::oneshot();
        let (tx_stop, rx_stop) = mpsc::oneshot();
        let (tx_supervision, rx_supervision) = mpsc::mpsc_unbounded();
        let (tx_message, rx_message) = mpsc::mpsc_unbounded();
        (
            Self {
                id,
                name,
                status: Arc::new(AtomicU8::new(ActorStatus::Unstarted as u8)),
                signal: Mutex::new(Some(tx_signal)),
                wait_handler: Arc::new(mpsc::Notify::new()),
                stop: Mutex::new(Some(tx_stop)),
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

    pub(crate) fn get_status(&self) -> ActorStatus {
        match self.status.load(Ordering::SeqCst) {
            0u8 => ActorStatus::Unstarted,
            1u8 => ActorStatus::Starting,
            2u8 => ActorStatus::Running,
            3u8 => ActorStatus::Upgrading,
            4u8 => ActorStatus::Draining,
            5u8 => ActorStatus::Stopping,
            _ => ActorStatus::Stopped,
        }
    }

    pub(crate) fn set_status(&self, status: ActorStatus) {
        self.status.store(status as u8, Ordering::SeqCst);
    }

    pub(crate) fn send_signal(&self, signal: Signal) -> Result<(), MessagingErr<()>> {
        self.signal
            .lock()
            .unwrap()
            .take()
            .map_or(Err(MessagingErr::ChannelClosed), |prt| {
                prt.send(signal).map_err(|_| MessagingErr::ChannelClosed)
            })
    }

    pub(crate) fn send_supervisor_evt(
        &self,
        message: SupervisionEvent,
    ) -> Result<(), MessagingErr<SupervisionEvent>> {
        self.supervision.send(message).map_err(|e| e.into())
    }

    pub(crate) fn send_message<TMessage>(
        &self,
        message: TMessage,
    ) -> Result<(), MessagingErr<TMessage>>
    where
        TMessage: Message,
    {
        // Only type-check messages of local actors, remote actors send serialized
        // payloads
        if self.id.is_local() && self.type_id != std::any::TypeId::of::<TMessage>() {
            return Err(MessagingErr::InvalidActorType);
        }

        let status = self.get_status();
        if status >= ActorStatus::Draining {
            // if currently draining, stopping or stopped: reject messages directly.
            return Err(MessagingErr::SendErr(message));
        }

        let boxed = message
            .box_message(&self.id)
            .map_err(|_e| MessagingErr::InvalidActorType)?;
        self.message
            .send(MuxedMessage::Message(boxed))
            .map_err(|e| match e.0 {
                MuxedMessage::Message(m) => MessagingErr::SendErr(TMessage::from_boxed(m).unwrap()),
                _ => panic!("Expected a boxed message but got a drain message"),
            })
    }

    pub(crate) fn drain(&self) -> Result<(), MessagingErr<()>> {
        let _ = self
            .status
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |f| {
                if f < (ActorStatus::Stopping as u8) {
                    Some(ActorStatus::Draining as u8)
                } else {
                    None
                }
            });
        self.message
            .send(MuxedMessage::Drain)
            .map_err(|_| MessagingErr::SendErr(()))
    }

    /// Start draining, and wait for the actor to exit
    pub(crate) async fn drain_and_wait(&self) -> Result<(), MessagingErr<()>> {
        let rx = self.wait_handler.notified();
        self.drain()?;
        rx.await;
        Ok(())
    }

    #[cfg(feature = "cluster")]
    pub(crate) fn send_serialized(
        &self,
        message: SerializedMessage,
    ) -> Result<(), MessagingErr<SerializedMessage>> {
        let boxed = BoxedMessage {
            msg: None,
            serialized_msg: Some(message),
            span: None,
        };
        self.message
            .send(MuxedMessage::Message(boxed))
            .map_err(|e| match e.0 {
                MuxedMessage::Message(m) => MessagingErr::SendErr(m.serialized_msg.unwrap()),
                _ => panic!("Expected a boxed message but got a drain message"),
            })
    }

    pub(crate) fn send_stop(
        &self,
        reason: Option<String>,
    ) -> Result<(), MessagingErr<StopMessage>> {
        let msg = reason.map(StopMessage::Reason).unwrap_or(StopMessage::Stop);
        self.stop
            .lock()
            .unwrap()
            .take()
            .map_or(Err(MessagingErr::ChannelClosed), |prt| {
                prt.send(msg).map_err(|_| MessagingErr::ChannelClosed)
            })
    }

    /// Send the stop signal, threading in a OneShot sender which notifies when the shutdown is completed
    pub(crate) async fn send_stop_and_wait(
        &self,
        reason: Option<String>,
    ) -> Result<(), MessagingErr<StopMessage>> {
        let rx = self.wait_handler.notified();
        self.send_stop(reason)?;
        rx.await;
        Ok(())
    }

    /// Wait for the actor to exit
    pub(crate) async fn wait(&self) {
        let rx = self.wait_handler.notified();
        rx.await;
    }

    /// Send the kill signal, threading in a OneShot sender which notifies when the shutdown is completed
    pub(crate) async fn send_signal_and_wait(
        &self,
        signal: Signal,
    ) -> Result<(), MessagingErr<()>> {
        // first bind the wait handler
        let rx = self.wait_handler.notified();
        let _ = self.send_signal(signal);
        rx.await;
        Ok(())
    }

    pub(crate) fn notify_stop_listener(&self) {
        self.wait_handler.notify_waiters();
        // make sure that any future caller immediately returns by pre-storing
        // a notify permit (i.e. the actor stops, but you are only start waiting
        // after the actor has already notified it's dead.)
        self.wait_handler.notify_one();
    }
}
