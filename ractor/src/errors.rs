// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Actor error types

use std::fmt::Display;

use crate::ActorName;

/// Represents an actor's internal processing error
pub type ActorProcessingErr = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Spawn errors starting an actor
#[derive(Debug)]
pub enum SpawnErr {
    /// Actor panic'd or returned an error during startup
    StartupFailed(ActorProcessingErr),
    /// An actor cannot be started > 1 time
    ActorAlreadyStarted,
    /// The named actor is already registered in the registry
    ActorAlreadyRegistered(ActorName),
}

impl std::error::Error for SpawnErr {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self {
            Self::StartupFailed(inner) => Some(inner.as_ref()),
            _ => None,
        }
    }
}

impl Display for SpawnErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StartupFailed(panic_msg) => {
                if f.alternate() {
                    write!(f, "Actor panicked during startup '{panic_msg:#}'")
                } else {
                    write!(f, "Actor panicked during startup '{panic_msg}'")
                }
            }
            Self::ActorAlreadyStarted => {
                write!(f, "Actor cannot be re-started more than once")
            }
            Self::ActorAlreadyRegistered(actor_name) => {
                write!(
                    f,
                    "Actor '{actor_name}' is already registered in the actor registry"
                )
            }
        }
    }
}

impl From<crate::registry::ActorRegistryErr> for SpawnErr {
    fn from(value: crate::registry::ActorRegistryErr) -> Self {
        match value {
            crate::registry::ActorRegistryErr::AlreadyRegistered(actor_name) => {
                SpawnErr::ActorAlreadyRegistered(actor_name)
            }
        }
    }
}

/// Actor processing loop errors
#[derive(Debug)]
pub enum ActorErr {
    /// Actor had a task cancelled internally during processing
    Cancelled,
    /// Actor had an internal panic
    Failed(ActorProcessingErr),
}

impl std::error::Error for ActorErr {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self {
            Self::Failed(inner) => Some(inner.as_ref()),
            _ => None,
        }
    }
}

impl Display for ActorErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Failed(panic_msg) => {
                if f.alternate() {
                    write!(f, "Actor panicked '{panic_msg:#}'")
                } else {
                    write!(f, "Actor panicked '{panic_msg}'")
                }
            }
            Self::Cancelled => {
                write!(f, "Actor operation cancelled")
            }
        }
    }
}

/// A messaging error has occurred
pub enum MessagingErr<T> {
    /// The channel you're trying to send a message too has been dropped/closed.
    /// If you're sending to an [crate::ActorCell] then that means the actor has died
    /// (failure or not).
    ///
    /// Includes the message which failed to send so the caller can perform another operation
    /// with the message if they want to.
    SendErr(T),

    /// The channel you're trying to receive from has had all the senders dropped
    /// and is therefore closed
    ChannelClosed,

    /// Tried to send a message to an actor with an invalid actor type defined.
    /// This happens if you have an [crate::ActorCell] which has the type id of its
    /// handler and you try to use an alternate handler to send a message
    InvalidActorType,
}

impl<T> MessagingErr<T> {
    /// Map any message embedded within the error type. This is primarily useful
    /// for normalizing an error value if the message is not needed.
    pub fn map<F, U>(self, mapper: F) -> MessagingErr<U>
    where
        F: FnOnce(T) -> U,
    {
        match self {
            MessagingErr::SendErr(err) => MessagingErr::SendErr(mapper(err)),
            MessagingErr::ChannelClosed => MessagingErr::ChannelClosed,
            MessagingErr::InvalidActorType => MessagingErr::InvalidActorType,
        }
    }
}

impl<T> std::fmt::Debug for MessagingErr<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SendErr(_) => write!(f, "SendErr"),
            Self::ChannelClosed => write!(f, "RecvErr"),
            Self::InvalidActorType => write!(f, "InvalidActorType"),
        }
    }
}

// SAFETY: This is required in order to map [MessagingErr] to
// ActorProcessingErr which requires errors to be Sync.
impl<T> std::error::Error for MessagingErr<T> {}

// SAFETY: This bound will make the MessagingErr be marked as `Sync`,
// even though all messages must only be `Send`. HOWEVER errors are generally
// never read in a `Sync` required context, and this bound is only
// required due to the auto-conversion to `std::error::Error``
#[allow(unsafe_code)]
unsafe impl<T> Sync for MessagingErr<T> {}

impl<T> From<tokio::sync::mpsc::error::SendError<T>> for MessagingErr<T> {
    fn from(e: tokio::sync::mpsc::error::SendError<T>) -> Self {
        Self::SendErr(e.0)
    }
}
impl<T> From<tokio::sync::mpsc::error::TrySendError<T>> for MessagingErr<T> {
    fn from(e: tokio::sync::mpsc::error::TrySendError<T>) -> Self {
        match e {
            tokio::sync::mpsc::error::TrySendError::Closed(c) => Self::SendErr(c),
            tokio::sync::mpsc::error::TrySendError::Full(c) => Self::SendErr(c),
        }
    }
}

impl<T> Display for MessagingErr<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ChannelClosed => {
                write!(f, "Messaging failed because channel is closed")
            }
            Self::InvalidActorType => {
                write!(f, "Messaging failed due to the provided actor type not matching the actor's properties")
            }
            Self::SendErr(_) => {
                write!(f, "Messaging failed to enqueue the message to the specified actor, the actor is likely terminated")
            }
        }
    }
}

/// Error types which can result from Ractor processes
pub enum RactorErr<T> {
    /// An error occurred spawning
    Spawn(SpawnErr),
    /// An error occurred in messaging (sending/receiving)
    Messaging(MessagingErr<T>),
    /// An actor encountered an error while processing (canceled or panicked)
    Actor(ActorErr),
    /// A timeout occurred
    Timeout,
}

impl<T> RactorErr<T> {
    /// Identify if the error has a message payload contained. If [true],
    /// You can utilize `try_get_message` to consume the error and extract the message payload
    /// quickly.
    ///
    /// Returns [true] if the error contains a message payload of type `T`, [false] otherwise.
    pub fn has_message(&self) -> bool {
        matches!(self, Self::Messaging(MessagingErr::SendErr(_)))
    }
    /// Try and extract the message payload from the contained error. This consumes the
    /// [RactorErr] instance in order to not have require cloning the message payload.
    /// Should be used in conjunction with `has_message` to not consume the error if not wanted
    ///
    /// Returns [Some(`T`)] if there is a message payload, [None] otherwise.
    pub fn try_get_message(self) -> Option<T> {
        if let Self::Messaging(MessagingErr::SendErr(msg)) = self {
            Some(msg)
        } else {
            None
        }
    }

    /// Map any message embedded within the error type. This is primarily useful
    /// for normalizing an error value if the message is not needed.
    pub fn map<F, U>(self, mapper: F) -> RactorErr<U>
    where
        F: FnOnce(T) -> U,
    {
        match self {
            RactorErr::Spawn(err) => RactorErr::Spawn(err),
            RactorErr::Messaging(err) => RactorErr::Messaging(err.map(mapper)),
            RactorErr::Actor(err) => RactorErr::Actor(err),
            RactorErr::Timeout => RactorErr::Timeout,
        }
    }
}

impl<T> std::fmt::Debug for RactorErr<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Messaging(m) => write!(f, "Messaging({m:?})"),
            Self::Actor(a) => write!(f, "Actor({a:?})"),
            Self::Spawn(s) => write!(f, "Spawn({s:?})"),
            Self::Timeout => write!(f, "Timeout"),
        }
    }
}

impl<T> std::error::Error for RactorErr<T> {}

impl<T> From<SpawnErr> for RactorErr<T> {
    fn from(value: SpawnErr) -> Self {
        RactorErr::Spawn(value)
    }
}

impl<T> From<MessagingErr<T>> for RactorErr<T> {
    fn from(value: MessagingErr<T>) -> Self {
        RactorErr::Messaging(value)
    }
}

impl<T> From<ActorErr> for RactorErr<T> {
    fn from(value: ActorErr) -> Self {
        RactorErr::Actor(value)
    }
}

impl<T, TResult> From<crate::rpc::CallResult<TResult>> for RactorErr<T> {
    fn from(value: crate::rpc::CallResult<TResult>) -> Self {
        match value {
            crate::rpc::CallResult::SenderError => {
                RactorErr::Messaging(MessagingErr::ChannelClosed)
            }
            crate::rpc::CallResult::Timeout => RactorErr::Timeout,
            _ => panic!("A successful `CallResult` cannot be mapped to a `RactorErr`"),
        }
    }
}

impl<T> std::fmt::Display for RactorErr<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Actor(actor_err) => {
                if f.alternate() {
                    write!(f, "{actor_err:#}")
                } else {
                    write!(f, "{actor_err}")
                }
            }
            Self::Messaging(messaging_err) => {
                if f.alternate() {
                    write!(f, "{messaging_err:#}")
                } else {
                    write!(f, "{messaging_err}")
                }
            }
            Self::Spawn(spawn_err) => {
                if f.alternate() {
                    write!(f, "{spawn_err:#}")
                } else {
                    write!(f, "{spawn_err}")
                }
            }
            Self::Timeout => {
                write!(f, "timeout")
            }
        }
    }
}
