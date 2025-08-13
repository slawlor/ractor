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

/// An error downcasting a boxed item to a strong type
#[derive(Debug, Eq, PartialEq)]
pub struct BoxedDowncastErr;
impl std::fmt::Display for BoxedDowncastErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "An error occurred handling a boxed message")
    }
}

impl std::error::Error for BoxedDowncastErr {}

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
pub struct BoxedMessage<T: Any + Send> {
    pub(crate) msg: LocalOrSerialized<T>,
    pub(crate) span: Option<tracing::Span>,
}
pub(crate) enum LocalOrSerialized<T: Any + Send> {
    Local(T),
    #[cfg(feature = "cluster")]
    /// A serialized message for a remote actor, accessed only by the `RemoteActorRuntime`
    Serialized(SerializedMessage),
}
impl<T: Any + Send> LocalOrSerialized<T> {
    #[cfg(not(feature = "cluster"))]
    pub(crate) fn into_local(self) -> T {
        match self {
            LocalOrSerialized::Local(msg) => msg,
        }
    }
    #[cfg(feature = "cluster")]
    pub(crate) fn into_serialized(self) -> Option<SerializedMessage> {
        match self {
            LocalOrSerialized::Local(_) => None,
            LocalOrSerialized::Serialized(msg) => Some(msg),
        }
    }
}

impl<T: Any + Send> std::fmt::Debug for BoxedMessage<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        #[cfg(not(feature = "cluster"))]
        {
            write!(f, "BoxedMessage(Local)")
        }
        #[cfg(feature = "cluster")]
        if matches!(self.msg, LocalOrSerialized::Local(_)) {
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
    fn from_boxed(m: BoxedMessage<Self>) -> Result<Self, BoxedDowncastErr> {
        match m.msg {
            LocalOrSerialized::Local(m) => Ok(m),
            LocalOrSerialized::Serialized(serialized_message) => {
                Self::deserialize(serialized_message)
            }
        }
    }

    /// Convert a [BoxedMessage] to this concrete type
    #[cfg(not(feature = "cluster"))]
    fn from_boxed(m: BoxedMessage<Self>) -> Result<Self, BoxedDowncastErr> {
        Ok(m.msg.into_local())
    }

    /// Convert this message to a [BoxedMessage]
    #[cfg(feature = "cluster")]
    fn box_message(self, pid: &ActorId) -> Result<BoxedMessage<Self>, BoxedDowncastErr> {
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
                msg: LocalOrSerialized::Serialized(self.serialize()?),
                span: None,
            })
        } else if pid.is_local() {
            Ok(BoxedMessage {
                msg: LocalOrSerialized::Local(self),
                span,
            })
        } else {
            //TODO: this error is not express the right thing
            Err(BoxedDowncastErr)
        }
    }

    /// Convert this message to a [BoxedMessage]
    #[cfg(not(feature = "cluster"))]
    #[allow(unused_variables)]
    fn box_message(self, pid: &ActorId) -> Result<BoxedMessage<Self>, BoxedDowncastErr> {
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
            msg: LocalOrSerialized::Local(self),
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
