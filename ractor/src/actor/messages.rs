// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Messages which are built-in for the actor

use std::any::Any;
use std::fmt::Debug;

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
/// constraints
#[derive(Debug)]
pub struct BoxedState {
    /// The state value
    pub state: Box<dyn Any + Send>,
}

impl BoxedState {
    /// Create a new [BoxedState] from a strongly-typed state
    pub fn new<T>(msg: T) -> Self
    where
        T: Any + State,
    {
        Self {
            state: Box::new(msg),
        }
    }

    /// Try and take the resulting type via cloning, such that we don't
    /// consume the value
    pub fn take<T>(&self) -> Result<T, BoxedDowncastErr>
    where
        T: Any + State,
    {
        let state_ref = &self.state;
        if state_ref.is::<T>() {
            Ok(state_ref.downcast_ref::<T>().cloned().unwrap())
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

/// A supervision event from the supervision tree
#[derive(Debug)]
pub enum SupervisionEvent {
    /// An actor was started
    ActorStarted(super::actor_cell::ActorCell),
    /// An actor terminated. In the event it shutdown cleanly (i.e. didn't panic or get
    /// signaled) we capture the last state of the actor which can be used to re-build an actor
    /// should the need arise
    ActorTerminated(super::actor_cell::ActorCell, Option<BoxedState>),
    /// An actor panicked
    ActorPanicked(super::actor_cell::ActorCell, String),
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
            Self::ActorTerminated(actor, maybe_state) => {
                let cloned_maybe_state = match maybe_state {
                    Some(state) => Some(state.take::<TState>()?),
                    _ => None,
                }
                .map(BoxedState::new);
                Ok(Self::ActorTerminated(actor.clone(), cloned_maybe_state))
            }
            Self::ActorPanicked(actor, message) => {
                Ok(Self::ActorPanicked(actor.clone(), message.clone()))
            }
        }
    }
}

/// A signal message which takes priority above all else
#[derive(Clone, Debug)]
pub enum Signal {
    /// Terminate the agent, cancelling all async work immediately
    Kill,
}
