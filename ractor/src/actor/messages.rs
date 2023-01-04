// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Messages which are built-in for the agent

use std::any::Any;
use std::fmt::Debug;

use crate::Message;

/// A "boxed" message denoting a strong-type message
/// but generic so it can be passed around without type
/// constraints
pub struct BoxedMessage {
    /// Flag denoting if message can be consumed 1 time
    pub one_time: bool,
    /// The message value
    pub msg: Option<Box<dyn Any + Send>>,
}

/// An error downcasting a boxed message to a strong type
pub struct DowncastBoxedMessageErr;

impl BoxedMessage {
    /// Create a new [BoxedMessage] from a strongly-typed message
    /// and a flag denoting if the message can be consumed 1 time
    /// or can be cloned and re-consumed
    pub fn new<T>(msg: T, one_time: bool) -> Self
    where
        T: Any + Message,
    {
        Self {
            one_time,
            msg: Some(Box::new(msg)),
        }
    }

    /// Try and take the resulting message as a specific type, consumes
    /// the boxed message if one_time = true
    pub fn take<T>(&mut self) -> Result<T, DowncastBoxedMessageErr>
    where
        T: Any + Message,
    {
        if self.one_time {
            match self.msg.take() {
                Some(m) => {
                    if m.is::<T>() {
                        Ok(*m.downcast::<T>().unwrap())
                    } else {
                        Err(DowncastBoxedMessageErr)
                    }
                }
                None => Err(DowncastBoxedMessageErr),
            }
        } else {
            match self.msg.as_ref() {
                Some(m) if m.is::<T>() => Ok(m.downcast_ref::<T>().cloned().unwrap()),
                Some(_) => Err(DowncastBoxedMessageErr),
                None => Err(DowncastBoxedMessageErr),
            }
        }
    }
}

impl Clone for BoxedMessage {
    fn clone(&self) -> Self {
        panic!("Cloning of an `BoxedMessage` type is not supported");
    }
}

impl Debug for BoxedMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("BoxedMessage")
    }
}

/// A supervision event from the supervision tree
#[derive(Debug, Clone)]
pub enum SupervisionEvent {
    /// An actor was started
    ActorStarted(super::actor_cell::ActorCell),
    /// An actor terminated
    ActorTerminated(super::actor_cell::ActorCell),
    /// An actor panicked
    ActorPanicked(super::actor_cell::ActorCell, String),
}

/// A signal message which takes priority above all else
#[derive(Clone, Debug)]
pub enum Signal {
    /// Exit, cancelling all async work immediately
    Exit,
}
