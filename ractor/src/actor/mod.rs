// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! This module contains the basic building blocks of an actor.
//!
//! They are:
//! [ActorHandler] : The internal logic for how an actor behaves
//! [Actor] : Management structure processing the message handler, signals, and supervision events in a loop

use std::{panic::AssertUnwindSafe, sync::Arc};

use futures::TryFutureExt;
use tokio::task::JoinHandle;

pub mod messages;
use messages::*;

pub mod actor_cell;
pub mod errors;
pub mod supervision;

#[cfg(test)]
mod tests;

use crate::{ActorName, Message, State};
use actor_cell::{ActorCell, ActorPortSet, ActorRef, ActorStatus};
use errors::{ActorErr, MessagingErr, SpawnErr};

pub(crate) fn get_panic_string(e: Box<dyn std::any::Any + Send>) -> String {
    match e.downcast::<String>() {
        Ok(v) => *v,
        Err(e) => match e.downcast::<&str>() {
            Ok(v) => v.to_string(),
            _ => "Unknown panic occurred which couldn't be coerced to a string".to_string(),
        },
    }
}

/// [ActorHandler] is the "business logic" of an Actor. It defines the
/// Message type, State type, and all processing logic for the actor
#[async_trait::async_trait]
pub trait ActorHandler: Sized + Sync + Send + 'static {
    /// The message type for this actor
    type Msg: Message;

    /// The type of state this actor manages internally
    type State: State;

    /// Invoked when an actor is being started by the system.
    ///
    /// Any initialization inherent to the actor's role should be
    /// performed here hence why it returns the initial state.
    ///
    /// Panics in `pre_start` do not invoke the
    /// supervision strategy and the actor won't be started. [Actor]::`spawn`
    /// will return an error to the caller
    ///
    /// * `myself` - A handle to the [ActorCell] representing this actor
    ///
    /// Returns an initial [ActorHandler::State] to bootstrap the actor
    async fn pre_start(&self, myself: ActorRef<Self>) -> Self::State;

    /// Invoked after an actor has started.
    ///
    /// Any post initialization can be performed here, such as writing
    /// to a log file, emmitting metrics.
    ///
    /// Panics in `post_start` follow the supervision strategy.
    ///
    /// * `myself` - A handle to the [ActorCell] representing this actor
    /// * `state` - A mutable reference to the internal actor's state
    #[allow(unused_variables)]
    async fn post_start(&self, myself: ActorRef<Self>, state: &mut Self::State) {}

    /// Invoked after an actor has been stopped to perform final cleanup. In the
    /// event the actor is terminated with [Signal::Kill] or has self-panicked,
    /// `post_stop` won't be called.
    ///
    /// Panics in `post_stop` follow the supervision strategy.
    ///
    /// * `myself` - A handle to the [ActorCell] representing this actor
    /// * `state` - A mutable reference to the internal actor's last known state
    #[allow(unused_variables)]
    async fn post_stop(&self, myself: ActorRef<Self>, state: &mut Self::State) {}

    /// Handle the incoming message from the event processing loop. Unhandled panicks will be
    /// captured and sent to the supervisor(s)
    ///
    /// * `myself` - A handle to the [ActorCell] representing this actor
    /// * `message` - The message to process
    /// * `state` - A mutable reference to the internal actor's state
    #[allow(unused_variables)]
    async fn handle(&self, myself: ActorRef<Self>, message: Self::Msg, state: &mut Self::State) {}

    /// Handle the incoming supervision event. Unhandled panicks will captured and
    /// sent the the supervisor(s)
    ///
    /// * `myself` - A handle to the [ActorCell] representing this actor
    /// * `message` - The message to process
    /// * `state` - A mutable reference to the internal actor's state
    #[allow(unused_variables)]
    async fn handle_supervisor_evt(
        &self,
        myself: ActorRef<Self>,
        message: SupervisionEvent,
        state: &mut Self::State,
    ) {
    }
}

/// [Actor] is a struct which represents the actor. This struct is consumed by the
/// `start` operation, but results in an [ActorRef] to communicate and operate with
pub struct Actor<TMsg, TState, THandler>
where
    TMsg: Message,
    TState: State,
    THandler: ActorHandler<Msg = TMsg, State = TState>,
{
    base: ActorRef<THandler>,
    handler: Arc<THandler>,
}

impl<TMsg, TState, THandler> Actor<TMsg, TState, THandler>
where
    TMsg: Message,
    TState: State,
    THandler: ActorHandler<Msg = TMsg, State = TState>,
{
    /// Spawn an actor, which is unsupervised, automatically starting the actor
    ///
    /// * `name`: A name to give the actor. Useful for global referencing or debug printing
    /// * `handler` The [ActorHandler] defining the logic for this actor
    ///
    /// Returns a [Ok((ActorRef, JoinHandle<()>))] upon successful start, denoting the actor reference
    /// along with the join handle which will complete when the actor terminates. Returns [Err(SpawnErr)] if
    /// the actor failed to start
    pub async fn spawn(
        name: Option<ActorName>,
        handler: THandler,
    ) -> Result<(ActorRef<THandler>, JoinHandle<()>), SpawnErr> {
        let (actor, ports) = Self::new(name, handler)?;
        actor.start(ports, None).await
    }

    /// Spawn an actor with a supervisor, automatically starting the actor
    ///
    /// * `name`: A name to give the actor. Useful for global referencing or debug printing
    /// * `handler` The [ActorHandler] defining the logic for this actor
    /// * `supervisor`: The [ActorCell] which is to become the supervisor (parent) of this actor
    ///
    /// Returns a [Ok((ActorRef, JoinHandle<()>))] upon successful start, denoting the actor reference
    /// along with the join handle which will complete when the actor terminates. Returns [Err(SpawnErr)] if
    /// the actor failed to start
    pub async fn spawn_linked(
        name: Option<ActorName>,
        handler: THandler,
        supervisor: ActorCell,
    ) -> Result<(ActorRef<THandler>, JoinHandle<()>), SpawnErr> {
        let (actor, ports) = Self::new(name, handler)?;
        actor.start(ports, Some(supervisor)).await
    }

    /// Create a new actor with some handler implementation and initial state
    ///
    /// * `name`: A name to give the actor. Useful for global referencing or debug printing
    /// * `handler` The [ActorHandler] defining the logic for this actor
    ///
    /// Returns A tuple [(Actor, ActorPortSet)] to be passed to the `start` function of [Actor]
    fn new(name: Option<ActorName>, handler: THandler) -> Result<(Self, ActorPortSet), SpawnErr> {
        let (actor_cell, ports) = actor_cell::ActorCell::new::<THandler>(name)?;
        Ok((
            Self {
                base: actor_cell.into(),
                handler: Arc::new(handler),
            },
            ports,
        ))
    }

    /// Start the actor immediately, optionally linking to a parent actor (supervision tree)
    ///
    /// NOTE: This returned [tokio::task::JoinHandle] is guaranteed to not panic (unless the runtime is shutting down perhaps).
    /// An inner join handle is capturing panic results from any part of the inner tasks, so therefore
    /// we can safely ignore it, or wait on it to block on the actor's progress
    ///
    /// * `ports` - The [ActorPortSet] for this actor
    /// * `supervisor` - The optional [ActorCell] representing the supervisor of this actor
    ///
    /// Returns a [Ok((ActorRef, JoinHandle<()>))] upon successful start, denoting the actor reference
    /// along with the join handle which will complete when the actor terminates. Returns [Err(SpawnErr)] if
    /// the actor failed to start
    async fn start(
        self,
        ports: ActorPortSet,
        supervisor: Option<ActorCell>,
    ) -> Result<(ActorRef<THandler>, JoinHandle<()>), SpawnErr> {
        // cannot start an actor more than once
        if self.base.get_status() != ActorStatus::Unstarted {
            return Err(SpawnErr::ActorAlreadyStarted);
        }

        self.base.set_status(ActorStatus::Starting);

        // Perform the pre-start routine, crashing immediately if we fail to start
        let mut state = Self::do_pre_start(self.base.clone(), self.handler.clone()).await?;

        // setup supervision
        if let Some(sup) = &supervisor {
            sup.link(self.base.clone().into());
        }

        // run the processing loop, backgrounding the work
        let myself = self.base.clone();
        let myself_ret = self.base.clone();
        let handle = tokio::spawn(async move {
            let evt = match Self::processing_loop(
                ports,
                &mut state,
                self.handler.clone(),
                self.base.clone(),
            )
            .await
            {
                Ok(exit_reason) => SupervisionEvent::ActorTerminated(
                    myself.clone().into(),
                    Some(BoxedState::new(state)),
                    exit_reason,
                ),
                Err(actor_err) => match actor_err {
                    ActorErr::Cancelled => SupervisionEvent::ActorTerminated(
                        myself.clone().into(),
                        None,
                        Some("killed".to_string()),
                    ),
                    ActorErr::Panic(msg) => {
                        SupervisionEvent::ActorPanicked(myself.clone().into(), msg)
                    }
                },
            };

            // terminate children
            myself.terminate();

            // notify supervisors of the actor's death
            myself.notify_supervisors(evt);

            // set status to stopped
            myself.set_status(ActorStatus::Stopped);

            // unlink superisors
            if let Some(sup) = supervisor {
                sup.unlink(myself.clone().into());
            }
        });

        Ok((myself_ret, handle))
    }

    async fn processing_loop(
        ports: ActorPortSet,
        state: &mut TState,
        handler: Arc<THandler>,
        myself: ActorRef<THandler>,
    ) -> Result<Option<String>, ActorErr> {
        // perform the post-start, with supervision enabled
        Self::do_post_start(myself.clone(), handler.clone(), state).await?;

        myself.set_status(ActorStatus::Running);
        myself.notify_supervisors(SupervisionEvent::ActorStarted(myself.clone().into()));

        let myself_clone = myself.clone();
        let handler_clone = handler.clone();

        let future = async move {
            let mut ports = ports;
            // the message processing loop. If we get and exit flag, try and capture the exit reason if there
            // is one
            loop {
                let (should_exit, maybe_exit_reason) = Self::process_message(
                    myself_clone.clone(),
                    state,
                    handler_clone.clone(),
                    &mut ports,
                )
                .await;
                // processing loop exit
                if should_exit {
                    return (state, maybe_exit_reason);
                }
            }
        };
        // capture any panicks in this future and convert to an ActorErr
        let loop_done = futures::FutureExt::catch_unwind(AssertUnwindSafe(future))
            .map_err(|err| ActorErr::Panic(get_panic_string(err)))
            .await;

        // set status to stopping
        myself.set_status(ActorStatus::Stopping);

        let (nstate, exit_reason) = loop_done?;

        // if we didn't exit in error mode, call `post_stop`
        Self::do_post_stop(myself, handler, nstate).await?;

        Ok(exit_reason)
    }

    /// Process a message, returning the "new" state (if changed)
    /// along with optionally whether we were signaled mid-processing or not
    ///
    /// * `myself` - The current [ActorRef]
    /// * `state` - The current [ActorHandler::State] object
    /// * `handler` - Pointer to the [ActorHandler] definition
    /// * `ports` - The mutable [ActorPortSet] which are the message ports for this actor
    ///
    /// Returns a tuple of the next [ActorHandler::State] and a flag to denote if the processing
    /// loop is done
    async fn process_message(
        myself: ActorRef<THandler>,
        state: &mut TState,
        handler: Arc<THandler>,
        ports: &mut ActorPortSet,
    ) -> (bool, Option<String>) {
        match ports.listen_in_priority().await {
            Ok(actor_port_message) => match actor_port_message {
                actor_cell::ActorPortMessage::Signal(signal) => {
                    (true, Self::handle_signal(myself, signal).await)
                }
                actor_cell::ActorPortMessage::Stop(stop_message) => {
                    let exit_reason = match stop_message {
                        StopMessage::Stop => None,
                        StopMessage::Reason(reason) => Some(reason),
                    };
                    (true, exit_reason)
                }
                actor_cell::ActorPortMessage::Supervision(supervision) => {
                    let new_state_future = Self::handle_supervision_message(
                        myself.clone(),
                        state,
                        handler.clone(),
                        supervision,
                    );
                    let new_state = ports.run_with_signal(new_state_future).await;
                    match new_state {
                        Ok(()) => (false, None),
                        Err(signal) => (true, Self::handle_signal(myself, signal).await),
                    }
                }
                actor_cell::ActorPortMessage::Message(msg) => {
                    let new_state_future =
                        Self::handle_message(myself.clone(), state, handler, msg);
                    let new_state = ports.run_with_signal(new_state_future).await;
                    match new_state {
                        Ok(()) => (false, None),
                        Err(signal) => (true, Self::handle_signal(myself, signal).await),
                    }
                }
            },
            Err(MessagingErr::ChannelClosed) => {
                // one of the channels is closed, this means
                // the receiver was dropped and in this case
                // we should always die. Therefore we flag
                // to terminate
                (true, None)
            }
            Err(MessagingErr::InvalidActorType) => {
                // not possible. Treat like a channel closed
                (true, None)
            }
        }
    }

    async fn handle_message(
        myself: ActorRef<THandler>,
        state: &mut TState,
        handler: Arc<THandler>,
        mut msg: BoxedMessage,
    ) {
        // panic in order to kill the actor
        let typed_msg = match msg.take() {
            Ok(m) => m,
            Err(_) => {
                panic!(
                    "Failed to convert message from `BoxedMessage` to `{}`",
                    std::any::type_name::<TMsg>()
                )
            }
        };

        handler.handle(myself, typed_msg, state).await
    }

    async fn handle_signal(myself: ActorRef<THandler>, signal: Signal) -> Option<String> {
        match &signal {
            Signal::Kill => {
                myself.terminate();
            }
        }
        Some(signal.to_string())
    }

    async fn handle_supervision_message(
        myself: ActorRef<THandler>,
        state: &mut TState,
        handler: Arc<THandler>,
        message: SupervisionEvent,
    ) {
        handler.handle_supervisor_evt(myself, message, state).await
    }

    async fn do_pre_start(
        myself: ActorRef<THandler>,
        handler: Arc<THandler>,
    ) -> Result<TState, SpawnErr> {
        let future = handler.pre_start(myself);
        futures::FutureExt::catch_unwind(AssertUnwindSafe(future))
            .await
            .map_err(|err| {
                SpawnErr::StartupPanic(format!(
                    "Actor panicked in pre_start with '{}'",
                    get_panic_string(err)
                ))
            })
    }

    async fn do_post_start(
        myself: ActorRef<THandler>,
        handler: Arc<THandler>,
        state: &mut TState,
    ) -> Result<(), ActorErr> {
        let future = handler.post_start(myself, state);
        futures::FutureExt::catch_unwind(AssertUnwindSafe(future))
            .await
            .map_err(|err| {
                ActorErr::Panic(format!(
                    "Actor panicked in post_start with '{}'",
                    get_panic_string(err)
                ))
            })
    }

    async fn do_post_stop(
        myself: ActorRef<THandler>,
        handler: Arc<THandler>,
        state: &mut TState,
    ) -> Result<(), ActorErr> {
        let future = handler.post_stop(myself, state);
        futures::FutureExt::catch_unwind(AssertUnwindSafe(future))
            .await
            .map_err(|err| {
                ActorErr::Panic(format!(
                    "Actor panicked in post_stop with '{}'",
                    get_panic_string(err)
                ))
            })
    }
}
