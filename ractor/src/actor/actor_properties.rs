// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use std::any::Any;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

use crate::actor::messages::StopMessage;
use crate::actor::supervision::SupervisionTree;
use crate::concurrency as mpsc;
use crate::concurrency::MpscUnboundedReceiver as InputPortReceiver;
use crate::concurrency::MpscUnboundedSender as InputPort;
use crate::concurrency::OneshotReceiver;
use crate::concurrency::OneshotSender as OneshotInputPort;
use crate::message::LocalOrSerialized;
use crate::message::RactorMessage;
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
pub(crate) enum MuxedMessage<T: Any + Send> {
    Drain,
    Message(LocalOrSerialized<T>),
}

pub(crate) trait GenericInputPort: Sync + Any + Send + 'static {
    fn send_drain(&self) -> Result<(), MessagingErr<()>>;
    #[cfg(feature = "cluster")]
    fn send_serialized(
        &self,
        message: SerializedMessage,
    ) -> Result<(), MessagingErr<SerializedMessage>>;
}
impl<T: Any + Send> GenericInputPort for InputPort<MuxedMessage<T>> {
    fn send_drain(&self) -> Result<(), MessagingErr<()>> {
        self.send(MuxedMessage::Drain)
            .map_err(|_| MessagingErr::SendErr(()))
    }

    #[cfg(feature = "cluster")]
    fn send_serialized(
        &self,
        message: SerializedMessage,
    ) -> Result<(), MessagingErr<SerializedMessage>> {
        let boxed = LocalOrSerialized::Serialized(message);
        self.send(MuxedMessage::Message(boxed))
            .map_err(|e| match e.0 {
                MuxedMessage::Message(m) => MessagingErr::SendErr(m.into_serialized().unwrap()),
                _ => panic!("Expected a boxed message but got a drain message"),
            })
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
    pub(crate) message: Box<dyn GenericInputPort>,
    pub(crate) tree: SupervisionTree,
    pub(crate) type_id: std::any::TypeId,
    #[cfg(feature = "cluster")]
    pub(crate) supports_remoting: bool,
}

impl ActorProperties {
    #[allow(clippy::type_complexity)]
    pub(crate) fn new<TActor: Actor>(
        name: Option<ActorName>,
    ) -> (
        Self,
        OneshotReceiver<Signal>,
        OneshotReceiver<StopMessage>,
        InputPortReceiver<SupervisionEvent>,
        InputPortReceiver<MuxedMessage<TActor::Msg>>,
    ) {
        Self::new_remote::<TActor>(name, crate::actor::actor_id::get_new_local_id())
    }

    #[allow(clippy::type_complexity)]
    pub(crate) fn new_remote<TActor: Actor>(
        name: Option<ActorName>,
        id: ActorId,
    ) -> (
        Self,
        OneshotReceiver<Signal>,
        OneshotReceiver<StopMessage>,
        InputPortReceiver<SupervisionEvent>,
        InputPortReceiver<MuxedMessage<TActor::Msg>>,
    ) {
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
                message: Box::new(tx_message),
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
        let status = self.get_status();
        if status >= ActorStatus::Draining {
            // if currently draining, stopping or stopped: reject messages directly.
            return Err(MessagingErr::SendErr(message));
        }

        let boxed = message
            .encode(&self.id)
            .map_err(|_e| MessagingErr::InvalidActorType)?;

        match boxed {
            #[cfg(feature = "cluster")]
            LocalOrSerialized::Serialized(m) => self
                .message
                .send_serialized(m)
                .map_err(convert_messaging_error),
            local => {
                let channel: &InputPort<MuxedMessage<TMessage>> = {
                    let ptr: &dyn Any = &*self.message;
                    ptr.downcast_ref().ok_or(MessagingErr::InvalidActorType)?
                };

                channel.send(MuxedMessage::Message(local)).map_err(|ret| {
                    let inner = match ret.0 {
                        MuxedMessage::Message(LocalOrSerialized::Local { msg, .. }) => msg,
                        _ => panic!("Impossible message received at send error point"),
                    };
                    MessagingErr::SendErr(inner)
                })
            }
        }
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
        self.message.send_drain()
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
    ) -> Result<(), Box<MessagingErr<SerializedMessage>>> {
        self.message.send_serialized(message).map_err(Box::new)
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

/// # Panic
/// Panic if the `TMessage` cannot be deserialized from
/// the serialized message passed as argument
#[cfg(feature = "cluster")]
fn convert_messaging_error<TMessage: Message>(
    e: MessagingErr<SerializedMessage>,
) -> MessagingErr<TMessage> {
    match e {
        MessagingErr::SendErr(m) => MessagingErr::SendErr(
            TMessage::decode(LocalOrSerialized::Serialized(m))
                .expect("Failed to decode a serialized message to the provided concrete type"),
        ),
        MessagingErr::ChannelClosed => MessagingErr::ChannelClosed,
        MessagingErr::InvalidActorType => MessagingErr::InvalidActorType,
    }
}
#[cfg(all(test, feature = "cluster"))]
mod tests {
    use super::*;

    #[test]
    fn test_convert_messaging_error() {
        struct TestRemoteMessage;
        // a serializable basic no-op message
        impl Message for TestRemoteMessage {
            fn serializable() -> bool {
                true
            }
            fn deserialize(_bytes: SerializedMessage) -> Result<Self, crate::message::DowncastErr> {
                Ok(TestRemoteMessage)
            }
            fn serialize(self) -> Result<SerializedMessage, crate::message::DowncastErr> {
                Ok(crate::message::SerializedMessage::Cast {
                    args: vec![],
                    variant: "Cast".to_string(),
                    metadata: None,
                })
            }
        }
        let id = ActorId::Remote { node_id: 1, pid: 1 };
        let boxed = TestRemoteMessage.encode(&id).unwrap();
        assert!(matches!(
            convert_messaging_error(MessagingErr::SendErr(boxed.into_serialized().unwrap())),
            MessagingErr::SendErr(TestRemoteMessage)
        ));
        assert!(matches!(
            convert_messaging_error::<TestRemoteMessage>(
                MessagingErr::<SerializedMessage>::ChannelClosed
            ),
            MessagingErr::<TestRemoteMessage>::ChannelClosed
        ));
        assert!(matches!(
            convert_messaging_error::<TestRemoteMessage>(
                MessagingErr::<SerializedMessage>::InvalidActorType
            ),
            MessagingErr::<TestRemoteMessage>::InvalidActorType
        ));
    }
}
