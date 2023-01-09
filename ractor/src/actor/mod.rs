// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! This module contains the basic building blocks of an actor.
//!
//! They are:
//! [ActorHandler] : The internal logic for how an actor behaves
//! [Actor] : Management structure processing the message handler, signals, and supervision events in a loop

use std::sync::Arc;

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

enum PanickableResult<T> {
    Ok(T),
    Cancelled,
    Panic(String),
}

async fn handle_panickable<TResult>(
    handle: tokio::task::JoinHandle<TResult>,
) -> PanickableResult<TResult> {
    match handle.await {
        Ok(t) => PanickableResult::Ok(t),
        Err(maybe_panic) => {
            if maybe_panic.is_panic() {
                let panic_message = get_panic_string(maybe_panic.into_panic());
                PanickableResult::Panic(panic_message)
            } else {
                // task cancelled, just notify of exit
                PanickableResult::Cancelled
            }
        }
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
    /// supervision strategy and the actor will be terminated.
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
    /// * `state` - A reference to the internal actor's state
    ///
    /// Returns [Some(ActorHandler::State)] if the state should be updated with a new value, [None] otherwise
    #[allow(unused_variables)]
    async fn post_start(&self, myself: ActorRef<Self>, state: &Self::State) -> Option<Self::State> {
        None
    }

    /// Invoked after an actor has been stopped.
    ///
    /// * `myself` - A handle to the [ActorCell] representing this actor
    /// * `state` - A reference to the internal actor's state
    ///
    /// Returns: The last [ActorHandler::State] (after doing any teardown) which might be necessary to re-create the actor
    #[allow(unused_variables)]
    async fn post_stop(&self, myself: ActorRef<Self>, state: Self::State) -> Self::State {
        state
    }

    /// Handle the incoming message in a basic event processing loop. Unhandled panic's will be captured and
    /// treated as actor death in the supervision tree
    ///
    /// * `myself` - A handle to the [ActorCell] representing this actor
    /// * `message` - The message to process
    /// * `state` - A reference to the internal actor's state
    ///
    /// Returns [Some(ActorHandler::State)] if the state should be updated with a new value, [None] otherwise
    #[allow(unused_variables)]
    async fn handle(
        &self,
        myself: ActorRef<Self>,
        message: Self::Msg,
        state: &Self::State,
    ) -> Option<Self::State> {
        None
    }

    /// Handle the incoming supervision event. Unhandled panic's will captured and treated as actor death in
    /// the supervision tree
    ///
    /// * `myself` - A handle to the [ActorCell] representing this actor
    /// * `message` - The message to process
    /// * `state` - A reference to the internal actor's state
    ///
    /// Returns [Some(ActorHandler::State)] if the state should be updated with a new value, [None] otherwise
    #[allow(unused_variables)]
    async fn handle_supervisor_evt(
        &self,
        myself: ActorRef<Self>,
        message: SupervisionEvent,
        state: &Self::State,
    ) -> Option<Self::State> {
        None
    }
}

/// [Actor] is a struct which represents the actor. This struct is consumed by the
/// `start` operation, but results in an [ActorCell] to communicate and operate with
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
    /// Returns a [Ok((ActorCell, JoinHandle<()>))] upon successful start, denoting the actor reference
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
    /// Returns a [Ok((ActorCell, JoinHandle<()>))] upon successful start, denoting the actor reference
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
    /// Returns a [Ok((ActorCell, JoinHandle<()>))] upon successful start, denoting the actor reference
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
        let state = Self::do_pre_start(self.base.clone(), self.handler.clone()).await?;

        // setup supervision
        if let Some(sup) = &supervisor {
            sup.link(self.base.clone().into());
        }

        // run the processing loop, capturing panic's
        let myself = self.base.clone();
        let myself_ret = self.base.clone();
        let handle = tokio::spawn(async move {
            let evt =
                match Self::processing_loop(ports, state, self.handler.clone(), self.base.clone())
                    .await
                {
                    Ok((last_state, exit_reason)) => SupervisionEvent::ActorTerminated(
                        myself.clone().into(),
                        Some(BoxedState::new(last_state)),
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

            myself.set_status(ActorStatus::Stopped);
            // signal received or process exited cleanly, we should already have "handled" the signal, so we can just terminate
            if let Some(sup) = supervisor {
                sup.unlink(myself.clone().into());
            }
        });

        Ok((myself_ret, handle))
    }

    async fn processing_loop(
        ports: ActorPortSet,
        state: TState,
        handler: Arc<THandler>,
        myself: ActorRef<THandler>,
    ) -> Result<(TState, Option<String>), ActorErr> {
        // perform the post-start, with supervision enabled
        let state = Self::do_post_start(myself.clone(), handler.clone(), state).await?;

        myself.set_status(ActorStatus::Running);
        myself.notify_supervisors(SupervisionEvent::ActorStarted(myself.clone().into()));

        // let mut last_state = state.clone();
        let myself_clone = myself.clone();
        let handler_clone = handler.clone();

        // let last_state = Arc::new(tokio::sync::RwLock::new(state.clone()));

        // let last_state_clone = last_state.clone();
        let handle: JoinHandle<(TState, Option<String>)> = tokio::spawn(async move {
            let mut ports = ports;
            let mut p_state = state;
            // the message processing loop. If we get and exit flag, try and capture the exit reason if there
            // is one
            loop {
                let (n_state, should_exit, maybe_exit_reason) = Self::process_message(
                    myself_clone.clone(),
                    p_state,
                    handler_clone.clone(),
                    &mut ports,
                )
                .await;
                p_state = n_state;
                // processing loop exit
                if should_exit {
                    return (p_state, maybe_exit_reason);
                }
            }
        });

        let loop_done = match handle_panickable(handle).await {
            PanickableResult::Ok(r) => Ok(r),
            PanickableResult::Cancelled => Err(ActorErr::Cancelled),
            PanickableResult::Panic(msg) => Err(ActorErr::Panic(msg)),
        };

        myself.set_status(ActorStatus::Stopping);

        let (last_state, exit_reason) = loop_done?;

        let last_state = Self::do_post_stop(myself, handler, last_state).await?;
        Ok((last_state, exit_reason))
    }

    /// Process a message, returning the "new" state (if changed)
    /// along with optionally whether we were signaled mid-processing or not
    ///
    /// * `myself` - The current [ActorCell]
    /// * `state` - The current [ActorHandler::State] object
    /// * `handler` - Pointer to the [ActorHandler] definition
    /// * `ports` - The mutable [ActorPortSet] which are the message ports for this actor
    ///
    /// Returns a tuple of the next [ActorHandler::State] and a flag to denote if the processing
    /// loop is done
    async fn process_message(
        myself: ActorRef<THandler>,
        state: TState,
        handler: Arc<THandler>,
        ports: &mut ActorPortSet,
    ) -> (TState, bool, Option<String>) {
        match ports.listen_in_priority().await {
            Ok(actor_port_message) => match actor_port_message {
                actor_cell::ActorPortMessage::Signal(signal) => {
                    (state, true, Self::handle_signal(myself, signal).await)
                }
                actor_cell::ActorPortMessage::Stop(stop_message) => {
                    let exit_reason = match stop_message {
                        StopMessage::Stop => None,
                        StopMessage::Reason(reason) => Some(reason),
                    };
                    (state, true, exit_reason)
                }
                actor_cell::ActorPortMessage::Supervision(supervision) => {
                    let new_state_future = Self::handle_supervision_message(
                        myself.clone(),
                        &state,
                        handler.clone(),
                        supervision,
                    );
                    let new_state = ports.run_with_signal(new_state_future).await;
                    match new_state {
                        Ok(s) => (s.unwrap_or(state), false, None),
                        Err(signal) => (state, true, Self::handle_signal(myself, signal).await),
                    }
                }
                actor_cell::ActorPortMessage::Message(msg) => {
                    let new_state_future =
                        Self::handle_message(myself.clone(), &state, handler, msg);
                    let new_state = ports.run_with_signal(new_state_future).await;
                    match new_state {
                        Ok(s) => (s.unwrap_or(state), false, None),
                        Err(signal) => (state, true, Self::handle_signal(myself, signal).await),
                    }
                }
            },
            Err(MessagingErr::ChannelClosed) => {
                // one of the channels is closed, this means
                // the receiver was dropped and in this case
                // we should always die. Therefore we simply return
                // the state and the flag to terminate
                (state, true, None)
            }
            Err(MessagingErr::InvalidActorType) => {
                // not possible. Treat like a channel closed
                (state, true, None)
            }
        }
    }

    async fn handle_message(
        myself: ActorRef<THandler>,
        state: &TState,
        handler: Arc<THandler>,
        mut msg: BoxedMessage,
    ) -> Option<TState> {
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
        state: &TState,
        handler: Arc<THandler>,
        message: SupervisionEvent,
    ) -> Option<TState> {
        handler.handle_supervisor_evt(myself, message, state).await
    }

    async fn do_pre_start(
        myself: ActorRef<THandler>,
        handler: Arc<THandler>,
    ) -> Result<TState, SpawnErr> {
        let start_result =
            handle_panickable(tokio::spawn(async move { handler.pre_start(myself).await })).await;
        match start_result {
            PanickableResult::Ok(state) => {
                // intitialize the state
                Ok(state)
            }
            PanickableResult::Cancelled => Err(SpawnErr::StartupCancelled),
            PanickableResult::Panic(panic_information) => Err(SpawnErr::StartupPanic(format!(
                "Actor panicked during pre_start with '{}'",
                panic_information
            ))),
        }
    }

    async fn do_post_start(
        myself: ActorRef<THandler>,
        handler: Arc<THandler>,
        state: TState,
    ) -> Result<TState, ActorErr> {
        let post_start_result = handle_panickable(tokio::spawn(async move {
            handler.post_start(myself, &state).await.unwrap_or(state)
        }))
        .await;
        match post_start_result {
            PanickableResult::Ok(new_state) => Ok(new_state),
            PanickableResult::Cancelled => Err(ActorErr::Cancelled),
            PanickableResult::Panic(panic_information) => Err(ActorErr::Panic(format!(
                "Actor panicked in post_start with '{}'",
                panic_information
            ))),
        }
    }

    async fn do_post_stop(
        myself: ActorRef<THandler>,
        handler: Arc<THandler>,
        state: TState,
    ) -> Result<TState, ActorErr> {
        let post_stop_result = handle_panickable(tokio::spawn(async move {
            handler.post_stop(myself, state).await
        }))
        .await;
        match post_stop_result {
            PanickableResult::Ok(last_state) => Ok(last_state),
            PanickableResult::Cancelled => Err(ActorErr::Cancelled),
            PanickableResult::Panic(panic_information) => Err(ActorErr::Panic(format!(
                "Actor panicked in post_start with '{}'",
                panic_information
            ))),
        }
    }
}
