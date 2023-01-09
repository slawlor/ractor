// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Messages which are built-in for `ractor`'s processing routines
//!
//! Additionally contains definitions for [BoxedMessage] and [BoxedState]
//! which are used to handle strongly-typed messages and states in a
//! generic way without having to know the strong type in the underlying framework

use std::any::Any;
use std::fmt::Debug;
use std::sync::Arc;

use crate::{Message, State};

/// An error downcasting a boxed item to a strong type
#[derive(Debug)]
pub struct BoxedDowncastErr;

/// A "boxed" message denoting a strong-type message
/// but generic so it can be passed around without type
/// constraints
pub struct BoxedMessage {
    /// The message value
    pub msg: Option<Box<dyn Any + Send>>,
}

impl BoxedMessage {
    /// Create a new [BoxedMessage] from a strongly-typed message
    pub fn new<T>(msg: T) -> Self
    where
        T: Any + Message,
    {
        Self {
            msg: Some(Box::new(msg)),
        }
    }

    /// Try and take the resulting message as a specific type, consumes
    /// the boxed message
    pub fn take<T>(&mut self) -> Result<T, BoxedDowncastErr>
    where
        T: Any + Message,
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

/// A "boxed" state denoting a strong-type state
/// but generic so it can be passed around without type
/// constraints.
///
/// It is a shared [Arc] to the last [crate::ActorHandler::State]
/// that the actor reported. This means, that the state shouldn't
/// be mutated once the actor is dead, it should be generally
/// immutable since a shared [Arc] will be sent to every supervisor
pub struct BoxedState {
    /// The state value
    pub state: Box<dyn Any + Send + Sync + 'static>,
}

impl Debug for BoxedState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BoxedState")
    }
}

impl BoxedState {
    /// Create a new [BoxedState] from a strongly-typed state
    pub fn new<T>(msg: T) -> Self
    where
        T: State,
    {
        Self {
            state: Box::new(Arc::new(msg)),
        }
    }

    /// Try and take the resulting type via cloning, such that we don't
    /// consume the value
    pub fn take<T>(&self) -> Result<Arc<T>, BoxedDowncastErr>
    where
        T: State,
    {
        let state_ref = &self.state;
        if state_ref.is::<Arc<T>>() {
            Ok(state_ref.downcast_ref::<Arc<T>>().cloned().unwrap())
        } else {
            Err(BoxedDowncastErr)
        }
    }
}

impl Debug for BoxedMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("BoxedMessage")
    }
}

/// Messages to stop an actor
pub(crate) enum StopMessage {
    // Normal stop
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

impl SupervisionEvent {
    /// A custom "clone" for [SupervisionEvent]s, because they hold a handle to a [BoxedState]
    /// which can't directly implement the clone trait, therefore the [SupervisionEvent]
    /// can't implement [Clone]. This requires that you give the intended strongly typed [State]
    /// which will downcast, clone the underlying type, and create a new boxed state for you
    pub(crate) fn duplicate<TState>(&self) -> Result<Self, BoxedDowncastErr>
    where
        TState: State,
    {
        match self {
            Self::ActorStarted(actor) => Ok(Self::ActorStarted(actor.clone())),
            Self::ActorTerminated(actor, maybe_state, maybe_exit_reason) => {
                let cloned_maybe_state = match maybe_state {
                    Some(state) => Some(state.take::<TState>()?),
                    _ => None,
                }
                .map(BoxedState::new);
                Ok(Self::ActorTerminated(
                    actor.clone(),
                    cloned_maybe_state,
                    maybe_exit_reason.clone(),
                ))
            }
            Self::ActorPanicked(actor, message) => {
                Ok(Self::ActorPanicked(actor.clone(), message.clone()))
            }
            Self::ProcessGroupChanged(change) => Ok(Self::ProcessGroupChanged(change.clone())),
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
