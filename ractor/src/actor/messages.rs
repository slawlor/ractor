// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Messages which are built-in for `ractor`'s processing routines
//!
//! Additionally contains definitions for [BoxedState]
//! which are used to handle strongly-typed states in a
//! generic way without having to know the strong type in the underlying framework

use std::any::Any;
use std::fmt::Debug;

use crate::message::BoxedDowncastErr;
use crate::State;

/// A "boxed" message denoting a strong-type message
/// but generic so it can be passed around without type
/// constraints
pub struct BoxedState {
    /// The message value
    pub msg: Option<Box<dyn Any + Send>>,
}

impl BoxedState {
    /// Create a new [BoxedState] from a strongly-typed message
    pub fn new<T>(msg: T) -> Self
    where
        T: State,
    {
        Self {
            msg: Some(Box::new(msg)),
        }
    }

    /// Try and take the resulting message as a specific type, consumes
    /// the boxed message
    pub fn take<T>(&mut self) -> Result<T, BoxedDowncastErr>
    where
        T: State,
    {
        match self.msg.take() {
            Some(m) => {
                if m.is::<T>() {
                    Ok(*m.downcast::<T>().unwrap())
                } else {
                    Err(BoxedDowncastErr)
                }
            }
            None => Err(BoxedDowncastErr),
        }
    }
}

/// Messages to stop an actor
pub enum StopMessage {
    /// Normal stop
    Stop,
    /// Stop with a reason
    Reason(String),
}

impl Debug for StopMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Stop message: {}", self)
    }
}

impl std::fmt::Display for StopMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stop => write!(f, "Stop"),
            Self::Reason(reason) => write!(f, "Stop (reason = {})", reason),
        }
    }
}

/// A supervision event from the supervision tree
pub enum SupervisionEvent {
    /// An actor was started
    ActorStarted(super::actor_cell::ActorCell),
    /// An actor terminated. In the event it shutdown cleanly (i.e. didn't panic or get
    /// signaled) we capture the last state of the actor which can be used to re-build an actor
    /// should the need arise. Includes an optional "exit reason" if it could be captured
    /// and was provided
    ActorTerminated(
        super::actor_cell::ActorCell,
        Option<BoxedState>,
        Option<String>,
    ),
    /// An actor panicked
    ActorPanicked(super::actor_cell::ActorCell, String),

    /// A subscribed process group changed
    ProcessGroupChanged(crate::pg::GroupChangeMessage),
}

impl Debug for SupervisionEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Supervision event: {}", self)
    }
}

impl std::fmt::Display for SupervisionEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SupervisionEvent::ActorStarted(actor) => {
                write!(f, "Started actor {:?}", actor)
            }
            SupervisionEvent::ActorTerminated(actor, _, reason) => {
                if let Some(r) = reason {
                    write!(f, "Stopped actor {:?} (reason = {})", actor, r)
                } else {
                    write!(f, "Stopped actor {:?}", actor)
                }
            }
            SupervisionEvent::ActorPanicked(actor, panic_msg) => {
                write!(f, "Actor panicked {:?} - {}", actor, panic_msg)
            }
            SupervisionEvent::ProcessGroupChanged(change) => {
                write!(f, "Process group {} changed", change.get_group())
            }
        }
    }
}

/// A signal message which takes priority above all else
#[derive(Clone)]
pub enum Signal {
    /// Terminate the agent, cancelling all async work immediately
    Kill,
}

impl Debug for Signal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Signal: {}", self)
    }
}

impl std::fmt::Display for Signal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Kill => {
                write!(f, "killed")
            }
        }
    }
}
