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
use crate::ActorProcessingErr;
use crate::State;

/// A "boxed" message denoting a strong-type message
/// but generic so it can be passed around without type
/// constraints
pub struct BoxedState {
    /// The message value
    pub msg: Option<Box<dyn Any + Send>>,
}

impl Debug for BoxedState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoxedState").finish()
    }
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
#[derive(Debug)]
pub enum StopMessage {
    /// Normal stop
    Stop,
    /// Stop with a reason
    Reason(String),
}

impl std::fmt::Display for StopMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stop => write!(f, "Stop"),
            Self::Reason(reason) => write!(f, "Stop (reason = {reason})"),
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
    /// An actor failed (due to panic or error case)
    ActorFailed(super::actor_cell::ActorCell, ActorProcessingErr),

    /// A subscribed process group changed
    ProcessGroupChanged(crate::pg::GroupChangeMessage),

    /// A process lifecycle event occurred
    #[cfg(feature = "cluster")]
    PidLifecycleEvent(crate::registry::PidLifecycleEvent),
}

impl SupervisionEvent {
    /// If this supervision event refers to an [Actor] lifecycle event, return
    /// the [ActorCell] for that [actor][Actor].
    ///
    ///
    /// [ActorCell]: crate::ActorCell
    /// [Actor]: crate::Actor
    pub fn actor_cell(&self) -> Option<&super::actor_cell::ActorCell> {
        match self {
            Self::ActorStarted(who)
            | Self::ActorFailed(who, _)
            | Self::ActorTerminated(who, _, _) => Some(who),
            _ => None,
        }
    }

    /// If this supervision event refers to an [Actor] lifecycle event, return
    /// the [ActorId] for that [actor][Actor].
    ///
    /// [ActorId]: crate::ActorId
    /// [Actor]: crate::Actor
    pub fn actor_id(&self) -> Option<super::actor_id::ActorId> {
        self.actor_cell().map(|cell| cell.get_id())
    }

    /// Clone the supervision event, without requiring inner data
    /// be cloneable. This means that the actor error (if present) is converted
    /// to a string and copied as well as the state upon termination being not
    /// propogated. If the state were cloneable, we could propogate it, however
    /// that restriction is overly restrictive, so we've avoided it.
    pub(crate) fn clone_no_data(&self) -> Self {
        match self {
            Self::ActorStarted(who) => Self::ActorStarted(who.clone()),
            Self::ActorFailed(who, what) => {
                Self::ActorFailed(who.clone(), From::from(format!("{what}")))
            }
            Self::ProcessGroupChanged(what) => Self::ProcessGroupChanged(what.clone()),
            Self::ActorTerminated(who, _state, msg) => {
                Self::ActorTerminated(who.clone(), None, msg.as_ref().cloned())
            }
            #[cfg(feature = "cluster")]
            Self::PidLifecycleEvent(evt) => Self::PidLifecycleEvent(evt.clone()),
        }
    }
}

impl Debug for SupervisionEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Supervision event: {self}")
    }
}

impl std::fmt::Display for SupervisionEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SupervisionEvent::ActorStarted(actor) => {
                write!(f, "Started actor {actor:?}")
            }
            SupervisionEvent::ActorTerminated(actor, _, reason) => {
                if let Some(r) = reason {
                    write!(f, "Stopped actor {actor:?} (reason = {r})")
                } else {
                    write!(f, "Stopped actor {actor:?}")
                }
            }
            SupervisionEvent::ActorFailed(actor, panic_msg) => {
                write!(f, "Actor panicked {actor:?} - {panic_msg}")
            }
            SupervisionEvent::ProcessGroupChanged(change) => {
                write!(
                    f,
                    "Process group {} in scope {} changed",
                    change.get_group(),
                    change.get_scope()
                )
            }
            #[cfg(feature = "cluster")]
            SupervisionEvent::PidLifecycleEvent(change) => {
                write!(f, "PID lifecycle event {change:?}")
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

impl std::fmt::Display for Signal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Kill => {
                write!(f, "killed")
            }
        }
    }
}
