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
#[derive(Debug)] // TODO: why Eq, PartialEq?
pub enum SpawnErr {
    /// Actor panic'd during startup
    StartupPanic(ActorProcessingErr),
    /// Actor failed to startup because the startup task was cancelled
    StartupCancelled,
    /// An actor cannot be started > 1 time
    ActorAlreadyStarted,
    /// The named actor is already registered in the registry
    ActorAlreadyRegistered(ActorName),
}

impl std::error::Error for SpawnErr {}

impl Display for SpawnErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StartupPanic(panic_msg) => {
                write!(f, "Actor panicked during startup '{panic_msg}'")
            }
            Self::StartupCancelled => {
                write!(
                    f,
                    "Actor failed to startup due to processing task being cancelled"
                )
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
    Panic(ActorProcessingErr),
}

impl std::error::Error for ActorErr {}

impl Display for ActorErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Panic(panic_msg) => {
                write!(f, "Actor panicked '{panic_msg}'")
            }
            Self::Cancelled => {
                write!(f, "Actor operation cancelled")
            }
        }
    }
}

/// A messaging error has occurred
#[derive(Debug)]
pub enum MessagingErr {
    /// The channel you're trying to send a message too has been dropped/closed.
    /// If you're sending to an [crate::ActorCell] then that means the actor has died
    /// (failure or not).
    ChannelClosed,

    /// Tried to send a message to an actor with an invalid actor type defined.
    /// This happens if you have an [crate::ActorCell] which has the type id of its
    /// handler and you try to use an alternate handler to send a message
    InvalidActorType,
}

impl std::error::Error for MessagingErr {}

impl<T> From<tokio::sync::mpsc::error::SendError<T>> for MessagingErr {
    fn from(_: tokio::sync::mpsc::error::SendError<T>) -> Self {
        Self::ChannelClosed
    }
}
impl<T> From<tokio::sync::mpsc::error::TrySendError<T>> for MessagingErr {
    fn from(_: tokio::sync::mpsc::error::TrySendError<T>) -> Self {
        Self::ChannelClosed
    }
}

impl Display for MessagingErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ChannelClosed => {
                write!(f, "Messaging failed because channel is closed")
            }
            Self::InvalidActorType => {
                write!(f, "Messaging failed due to the provided actor type not matching the actor's properties")
            }
        }
    }
}
