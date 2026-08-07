// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use std::sync::atomic::AtomicU8;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

use crate::actor::messages::StopMessage;
use crate::actor::supervision::SupervisionTree;
use crate::concurrency as mpsc;
use crate::concurrency::MpscUnboundedReceiver as InputPortReceiver;
use crate::concurrency::MpscUnboundedSender as InputPort;
use crate::concurrency::OneshotReceiver;
use crate::concurrency::OneshotSender as OneshotInputPort;
use crate::message::BoxedMessage;
#[cfg(feature = "cluster")]
use crate::message::SerializedMessage;
use crate::Actor;
use crate::ActorId;
use crate::ActorName;
use crate::ActorStatus;
use crate::Message;
use crate::MessagingErr;
use crate::Signal;
use crate::SupervisionEvent;

/// A muxed-message wrapper which allows the message port to receive either a message or a drain
/// request which is a point-in-time marker that the actor's input channel should be drained
pub(crate) enum MuxedMessage {
    Drain,
    Message(BoxedMessage),
}

const MESSAGE_ADMISSION_CLOSED: usize = 1usize << (usize::BITS - 1);
const DRAIN_MARKER_SENT: usize = 1usize << (usize::BITS - 2);
const MESSAGE_ADMISSION_COUNT_MASK: usize = DRAIN_MARKER_SENT - 1;

struct MessageAdmission<'a>(&'a ActorProperties);

impl Drop for MessageAdmission<'_> {
    fn drop(&mut self) {
        let previous = self.0.message_admission.fetch_sub(1, Ordering::AcqRel);
        debug_assert_ne!(previous & MESSAGE_ADMISSION_COUNT_MASK, 0);
        if previous & MESSAGE_ADMISSION_CLOSED != 0 && previous & MESSAGE_ADMISSION_COUNT_MASK == 1
        {
            let _ = self.0.send_drain_marker();
        }
    }
}

// The inner-properties of an Actor
pub(crate) struct ActorProperties {
    pub(crate) id: ActorId,
    pub(crate) name: Option<ActorName>,
    pub(crate) status: AtomicU8,
    pub(crate) wait_handler: mpsc::Notify,
    pub(crate) signal: Mutex<Option<OneshotInputPort<Signal>>>,
    pub(crate) stop: Mutex<Option<OneshotInputPort<StopMessage>>>,
    pub(crate) supervision: InputPort<SupervisionEvent>,
    pub(crate) message: InputPort<MuxedMessage>,
    pub(crate) message_admission: AtomicUsize,
    pub(crate) tree: SupervisionTree,
    pub(crate) type_id: std::any::TypeId,
    #[cfg(feature = "cluster")]
    pub(crate) supports_remoting: bool,
}

impl ActorProperties {
    fn status_from_u8(status: u8) -> ActorStatus {
        match status {
            0u8 => ActorStatus::Unstarted,
            1u8 => ActorStatus::Starting,
            2u8 => ActorStatus::Running,
            3u8 => ActorStatus::Upgrading,
            4u8 => ActorStatus::Draining,
            5u8 => ActorStatus::Stopping,
            _ => ActorStatus::Stopped,
        }
    }

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
                status: AtomicU8::new(ActorStatus::Unstarted as u8),
                signal: Mutex::new(Some(tx_signal)),
                wait_handler: mpsc::Notify::new(),
                stop: Mutex::new(Some(tx_stop)),
                supervision: tx_supervision,
                message: tx_message,
                message_admission: AtomicUsize::new(0),
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
        Self::status_from_u8(self.status.load(Ordering::SeqCst))
    }

    /// Advances the lifecycle status without allowing a concurrent or stale
    /// transition to move it backwards.
    ///
    /// Returns the status observed immediately before this update.
    pub(crate) fn set_status(&self, status: ActorStatus) -> ActorStatus {
        Self::status_from_u8(self.status.fetch_max(status as u8, Ordering::SeqCst))
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

        // Delegate to unchecked version after type check
        self.send_message_unchecked(message)
    }

    /// Send a message without runtime type checking.
    ///
    /// This is an internal optimization for strongly-typed ActorRef which has compile-time
    /// type safety guarantees, avoiding the redundant runtime TypeId comparison.
    ///
    /// SAFETY: Callers must ensure the message type matches the actor's expected type.
    pub(crate) fn send_message_unchecked<TMessage>(
        &self,
        message: TMessage,
    ) -> Result<(), MessagingErr<TMessage>>
    where
        TMessage: Message,
    {
        let status = self.get_status();
        if status >= ActorStatus::Draining {
            // if currently draining, stopping or stopped: reject messages directly.
            return Err(MessagingErr::SendErr(message));
        }

        let Some(_admission) = self.try_admit_message() else {
            return Err(MessagingErr::SendErr(message));
        };
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

    fn try_admit_message(&self) -> Option<MessageAdmission<'_>> {
        let mut state = self.message_admission.load(Ordering::Relaxed);
        loop {
            if state & MESSAGE_ADMISSION_CLOSED != 0 {
                return None;
            }
            debug_assert!(state & MESSAGE_ADMISSION_COUNT_MASK < MESSAGE_ADMISSION_COUNT_MASK);

            match self.message_admission.compare_exchange_weak(
                state,
                state + 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Some(MessageAdmission(self)),
                Err(observed) => state = observed,
            }
        }
    }

    fn close_message_admission(&self) {
        self.message_admission
            .fetch_or(MESSAGE_ADMISSION_CLOSED, Ordering::AcqRel);
    }

    fn send_drain_marker(&self) -> Result<(), MessagingErr<()>> {
        let mut state = self.message_admission.load(Ordering::Acquire);
        loop {
            if state & MESSAGE_ADMISSION_CLOSED == 0
                || state & MESSAGE_ADMISSION_COUNT_MASK != 0
                || state & DRAIN_MARKER_SENT != 0
            {
                return Ok(());
            }

            match self.message_admission.compare_exchange_weak(
                state,
                state | DRAIN_MARKER_SENT,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return self
                        .message
                        .send(MuxedMessage::Drain)
                        .map_err(|_| MessagingErr::SendErr(()));
                }
                Err(observed) => state = observed,
            }
        }
    }

    pub(crate) fn drain(&self) -> Result<(), MessagingErr<()>> {
        self.close_message_admission();
        let _ = self
            .status
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |f| {
                if f < (ActorStatus::Stopping as u8) {
                    Some(ActorStatus::Draining as u8)
                } else {
                    None
                }
            });
        self.send_drain_marker()
    }

    /// Start draining, and wait for the actor to exit
    pub(crate) async fn drain_and_wait(&self) -> Result<(), MessagingErr<()>> {
        self.drain()?;
        self.wait().await;
        Ok(())
    }

    #[cfg(feature = "cluster")]
    pub(crate) fn send_serialized(
        &self,
        message: SerializedMessage,
    ) -> Result<(), Box<MessagingErr<SerializedMessage>>> {
        if self.get_status() >= ActorStatus::Draining {
            return Err(Box::new(MessagingErr::SendErr(message)));
        }

        let Some(_admission) = self.try_admit_message() else {
            return Err(Box::new(MessagingErr::SendErr(message)));
        };
        let boxed = BoxedMessage {
            msg: None,
            serialized_msg: Some(message),
            #[cfg(feature = "message_span_propogation")]
            span: None,
        };
        Ok(self
            .message
            .send(MuxedMessage::Message(boxed))
            .map_err(|e| match e.0 {
                MuxedMessage::Message(m) => MessagingErr::SendErr(m.serialized_msg.unwrap()),
                _ => panic!("Expected a boxed message but got a drain message"),
            })?)
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
        self.send_stop(reason)?;
        self.wait().await;
        Ok(())
    }

    /// Wait for the actor to exit
    pub(crate) async fn wait(&self) {
        let notified = self.wait_handler.notified();
        if self.get_status() != ActorStatus::Stopped {
            notified.await;
        }
    }

    /// Send the kill signal, threading in a OneShot sender which notifies when the shutdown is completed
    pub(crate) async fn send_signal_and_wait(
        &self,
        signal: Signal,
    ) -> Result<(), MessagingErr<()>> {
        let _ = self.send_signal(signal);
        self.wait().await;
        Ok(())
    }

    pub(crate) fn notify_stop_listener(&self) {
        self.wait_handler.notify_waiters();
        // Preserve one permit for a waiter created after the actor stopped.
        self.wait_handler.notify_one();
    }
}
