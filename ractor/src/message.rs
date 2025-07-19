// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Message trait definition for inter-actor messaging. Additionally
//! with the `cluster` feature, it controls serialization logic for
//! over-the-wire inter-actor communications

use std::any::Any;

use crate::ActorId;
#[cfg(feature = "cluster")]
use crate::RpcReplyPort;

#[cfg(feature = "derived-actor-from-cell")]
use crate::ActorRef;

/// An error downcasting a boxed item to a strong type
#[derive(Debug, Eq, PartialEq)]
pub struct BoxedDowncastErr;
impl std::fmt::Display for BoxedDowncastErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "An error occurred handling a boxed message")
    }
}

impl std::error::Error for BoxedDowncastErr {}

#[cfg(feature = "derived-actor-from-cell")]
pub(crate) mod request_derived;
#[cfg(feature = "derived-actor-from-cell")]
pub use request_derived::RequestDerived;

/// Represents a serialized call or cast message
#[cfg(feature = "cluster")]
#[derive(Debug)]
pub enum SerializedMessage {
    /// A cast (one-way) with the serialized payload
    Cast {
        /// The index into to variant. Helpful for enum serialization
        variant: String,
        /// The payload of data
        args: Vec<u8>,
        /// Additional (optional) metadata
        metadata: Option<Vec<u8>>,
    },
    /// A call (remote procedure call, waiting on a reply) with the
    /// serialized arguments and reply channel
    Call {
        /// The index into to variant. Helpful for enum serialization
        variant: String,
        /// The argument payload data
        args: Vec<u8>,
        /// The binary reply channel
        reply: RpcReplyPort<Vec<u8>>,
        /// Additional (optional) metadata
        metadata: Option<Vec<u8>>,
    },
    /// A serialized reply from a call operation. Format is
    /// (`message_tag`, `reply_data`). It should not be the output
    /// of [Message::serialize] function, and is only generated
    /// from the `NodeSession`
    CallReply(u64, Vec<u8>),
}

/// A "boxed" message denoting a strong-type message
/// but generic so it can be passed around without type
/// constraints
pub struct BoxedMessage {
    pub(crate) msg: Option<Box<dyn Any + Send>>,
    /// A serialized message for a remote actor, accessed only by the `RemoteActorRuntime`
    #[cfg(feature = "cluster")]
    pub serialized_msg: Option<SerializedMessage>,
    pub(crate) span: Option<tracing::Span>,
}

impl std::fmt::Debug for BoxedMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.msg.is_some() {
            write!(f, "BoxedMessage(Local)")
        } else {
            write!(f, "BoxedMessage(Serialized)")
        }
    }
}

/// Message type for an actor. Generally an enum
/// which muxes the various types of inner-messages the actor
/// supports
///
/// ## Example
///
/// ```rust
/// pub enum MyMessage {
///     /// Record the name to the actor state
///     RecordName(String),
///     /// Print the recorded name from the state to command line
///     PrintName,
/// }
/// ```
pub trait Message: Any + Send + Sized + 'static {
    /// Convert a [BoxedMessage] to this concrete type
    #[cfg(feature = "cluster")]
    fn from_boxed(mut m: BoxedMessage) -> Result<Self, BoxedDowncastErr> {
        if m.msg.is_some() {
            match m.msg.take() {
                Some(m) => {
                    if m.is::<Self>() {
                        Ok(*m.downcast::<Self>().unwrap())
                    } else {
                        Err(BoxedDowncastErr)
                    }
                }
                _ => Err(BoxedDowncastErr),
            }
        } else if m.serialized_msg.is_some() {
            match m.serialized_msg.take() {
                Some(m) => Self::deserialize(m),
                _ => Err(BoxedDowncastErr),
            }
        } else {
            Err(BoxedDowncastErr)
        }
    }

    /// Convert a [BoxedMessage] to this concrete type
    #[cfg(not(feature = "cluster"))]
    fn from_boxed(mut m: BoxedMessage) -> Result<Self, BoxedDowncastErr> {
        match m.msg.take() {
            Some(m) => {
                if m.is::<Self>() {
                    Ok(*m.downcast::<Self>().unwrap())
                } else {
                    Err(BoxedDowncastErr)
                }
            }
            _ => Err(BoxedDowncastErr),
        }
    }

    /// Convert this message to a [BoxedMessage]
    #[cfg(feature = "cluster")]
    fn box_message(self, pid: &ActorId) -> Result<BoxedMessage, BoxedDowncastErr> {
        let span = {
            #[cfg(feature = "message_span_propogation")]
            {
                Some(tracing::Span::current())
            }
            #[cfg(not(feature = "message_span_propogation"))]
            {
                None
            }
        };
        if Self::serializable() && !pid.is_local() {
            // it's a message to a remote actor, serialize it and send it over the wire!
            Ok(BoxedMessage {
                msg: None,
                serialized_msg: Some(self.serialize()?),
                span: None,
            })
        } else if pid.is_local() {
            Ok(BoxedMessage {
                msg: Some(Box::new(self)),
                serialized_msg: None,
                span,
            })
        } else {
            Err(BoxedDowncastErr)
        }
    }

    /// Convert this message to a [BoxedMessage]
    #[cfg(not(feature = "cluster"))]
    #[allow(unused_variables)]
    fn box_message(self, pid: &ActorId) -> Result<BoxedMessage, BoxedDowncastErr> {
        let span = {
            #[cfg(feature = "message_span_propogation")]
            {
                Some(tracing::Span::current())
            }
            #[cfg(not(feature = "message_span_propogation"))]
            {
                None
            }
        };
        Ok(BoxedMessage {
            msg: Some(Box::new(self)),
            span,
        })
    }

    /// Determines if this type is serializable
    #[cfg(feature = "cluster")]
    fn serializable() -> bool {
        false
    }

    /// Serializes this message (if supported)
    #[cfg(feature = "cluster")]
    fn serialize(self) -> Result<SerializedMessage, BoxedDowncastErr> {
        Err(BoxedDowncastErr)
    }

    /// Deserialize binary data to this message type
    #[cfg(feature = "cluster")]
    #[allow(unused_variables)]
    fn deserialize(bytes: SerializedMessage) -> Result<Self, BoxedDowncastErr> {
        Err(BoxedDowncastErr)
    }
}

// Auto-Implement the [Message] trait for all types when NOT in the `cluster` configuration
// since there's no need for an override
#[cfg(not(feature = "cluster"))]
impl<T: Any + Send + Sized + 'static> Message for T {}

// Blanket implementation for basic types which are directly bytes serializable which
// are all to be CAST operations
#[cfg(feature = "cluster")]
impl<T: Any + Send + Sized + 'static + crate::serialization::BytesConvertable> Message for T {
    fn serializable() -> bool {
        true
    }

    fn serialize(self) -> Result<SerializedMessage, BoxedDowncastErr> {
        Ok(SerializedMessage::Cast {
            variant: String::new(),
            args: self.into_bytes(),
            metadata: None,
        })
    }

    fn deserialize(bytes: SerializedMessage) -> Result<Self, BoxedDowncastErr> {
        match bytes {
            SerializedMessage::Cast { args, .. } => Ok(T::from_bytes(args)),
            _ => Err(BoxedDowncastErr),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{concurrency::MpscSender, Actor};

    use super::Message;

    struct TestActor;

    #[derive(Debug, Eq, Clone, PartialEq)]
    enum Msg {
        I32(i32),
        I64(i64),
    }
    impl From<i32> for Msg {
        fn from(value: i32) -> Self {
            Msg::I32(value)
        }
    }
    impl From<i64> for Msg {
        fn from(value: i64) -> Self {
            Msg::I64(value)
        }
    }
    impl TryFrom<Msg> for i32 {
        type Error = ();
        fn try_from(value: Msg) -> Result<Self, Self::Error> {
            match value {
                Msg::I32(v) => Ok(v),
                _ => Err(()),
            }
        }
    }
    impl TryFrom<Msg> for i64 {
        type Error = ();
        fn try_from(value: Msg) -> Result<Self, Self::Error> {
            match value {
                Msg::I64(v) => Ok(v),
                _ => Err(()),
            }
        }
    }
    #[cfg(feature = "cluster")]
    impl Message for Msg {}
    impl Actor for TestActor {
        type Msg = Msg;

        type State = MpscSender<Msg>;

        type Arguments = MpscSender<Msg>;

        async fn pre_start(
            &self,
            _myself: crate::ActorRef<Self::Msg>,
            args: Self::Arguments,
        ) -> Result<Self::State, crate::ActorProcessingErr> {
            Ok(args)
        }
        async fn handle(
            &self,
            _myself: crate::ActorRef<Self::Msg>,
            message: Self::Msg,
            state: &mut Self::State,
        ) -> Result<(), crate::ActorProcessingErr> {
            state.send(message).await?;
            Ok(())
        }
        fn provide_derived_actor_ref<'a>(
            myself: crate::ActorRef<Msg>,
            request: &mut super::RequestDerived<'a>,
        ) {
            request.provide_derived_actor(myself.get_derived::<i32>());
            request.provide_derived_actor(myself.get_derived::<i64>());
        }
    }
    #[tokio::test]
    async fn derived_actor_from_cell() {
        let (sx, mut rx) = crate::concurrency::mpsc_bounded(10);
        let (ar, _) = Actor::spawn(None, TestActor, sx).await.unwrap();
        let cell = ar.get_cell();
        let dar = cell.provide_derived::<i32>().unwrap();
        dar.send_message(42).unwrap();
        assert_eq!(rx.recv().await.unwrap(), Msg::I32(42));
        let dar = cell.provide_derived::<i64>().unwrap();
        dar.send_message(33).unwrap();
        assert_eq!(rx.recv().await.unwrap(), Msg::I64(33));
    }
}
