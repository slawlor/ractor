// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! This module contains the basic building blocks of an actor. They are
//!
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

use crate::{Message, State};
use actor_cell::{ActorCell, ActorPortSet, ActorStatus};
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
    async fn pre_start(&self, myself: ActorCell) -> Self::State;

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
    async fn post_start(&self, myself: ActorCell, state: &Self::State) -> Option<Self::State> {
        None
    }

    /// Invoked after an actor has been stopped.
    ///
    /// * `myself` - A handle to the [ActorCell] representing this actor
    /// * `state` - A reference to the internal actor's state
    ///
    /// Returns: The last [ActorHandler::State] (after doing any teardown) which might be necessary to re-create the actor
    #[allow(unused_variables)]
    async fn post_stop(&self, myself: ActorCell, state: Self::State) -> Self::State {
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
        myself: ActorCell,
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
        myself: ActorCell,
        message: SupervisionEvent,
        state: &Self::State,
    ) -> Option<Self::State> {
        None
    }

    /// Send a message to the specified actor, which is strong-typed to the message handler's type
    ///
    /// * `myself` - A handle to the [ActorCell] representing this actor
    /// * `msg` - The message to send to this actor
    ///
    /// Returns [Ok(())] on successful send, [Err(MessagingError)] in the event sending failed
    fn send_message(&self, myself: ActorCell, msg: Self::Msg) -> Result<(), MessagingErr> {
        myself.send_message::<Self, Self::Msg>(msg)
    }
}

/// [Actor] is a struct which represents the actor. This struct is consumed by the
/// `start` operation, but results in an [ActorCell] to communicate and operate with
pub struct Actor<TMsg, TState, THandler>
where
    TMsg: Message,
    TState: State + Clone,
    THandler: ActorHandler<Msg = TMsg, State = TState>,
{
    base: ActorCell,
    handler: Arc<THandler>,
}

impl<TMsg, TState, THandler> Actor<TMsg, TState, THandler>
where
    TMsg: Message,
    TState: State + Clone,
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
        name: Option<String>,
        handler: THandler,
    ) -> Result<(ActorCell, JoinHandle<()>), SpawnErr> {
        let (actor, ports) = Self::new(name, handler);
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
        name: Option<String>,
        handler: THandler,
        supervisor: ActorCell,
    ) -> Result<(ActorCell, JoinHandle<()>), SpawnErr> {
        let (actor, ports) = Self::new(name, handler);
        actor.start(ports, Some(supervisor)).await
    }

    /// Create a new actor with some handler implementation and initial state
    ///
    /// * `name`: A name to give the actor. Useful for global referencing or debug printing
    /// * `handler` The [ActorHandler] defining the logic for this actor
    ///
    /// Returns A tuple [(Actor, ActorPortSet)] to be passed to the `start` function of [Actor]
    fn new(name: Option<String>, handler: THandler) -> (Self, ActorPortSet) {
        let (actor_cell, ports) = ActorCell::new(name);
        (
            Self {
                base: actor_cell,
                handler: Arc::new(handler),
            },
            ports,
        )
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
    ) -> Result<(ActorCell, JoinHandle<()>), SpawnErr> {
        // cannot start an actor more than once
        if self.base.get_status() != ActorStatus::Unstarted {
            return Err(SpawnErr::ActorAlreadyStarted);
        }

        self.base.set_status(ActorStatus::Starting);

        // Perform the pre-start routine, crashing immediately if we fail to start
        let state = Self::do_pre_start(self.base.clone(), self.handler.clone()).await?;

        // setup supervision
        if let Some(sup) = &supervisor {
            sup.link(self.base.clone());
        }

        // run the processing loop, capturing panic's
        let myself = self.base.clone();
        let myself_ret = self.base.clone();
        let handle = tokio::spawn(async move {
            let evt =
                match Self::processing_loop(ports, state, self.handler.clone(), self.base.clone())
                    .await
                {
                    Ok(last_state) => SupervisionEvent::ActorTerminated(
                        myself.clone(),
                        Some(BoxedState::new(last_state)),
                    ),
                    Err(actor_err) => match actor_err {
                        ActorErr::Cancelled => {
                            SupervisionEvent::ActorTerminated(myself.clone(), None)
                        }
                        ActorErr::Panic(msg) => {
                            SupervisionEvent::ActorPanicked(myself.clone(), msg)
                        }
                    },
                };

            // terminate children
            myself.terminate();

            // notify supervisors of the actor's death
            myself.notify_supervisors::<THandler, TState>(evt);

            myself.set_status(ActorStatus::Stopped);
            // signal received or process exited cleanly, we should already have "handled" the signal, so we can just terminate
            if let Some(sup) = supervisor {
                sup.unlink(myself.clone());
            }
        });

        Ok((myself_ret, handle))
    }

    async fn processing_loop(
        ports: ActorPortSet,
        state: TState,
        handler: Arc<THandler>,
        myself: ActorCell,
    ) -> Result<TState, ActorErr> {
        // perform the post-start, with supervision enabled
        let mut state = Self::do_post_start(myself.clone(), handler.clone(), state).await?;

        myself.set_status(ActorStatus::Running);
        myself
            .notify_supervisors::<THandler, TState>(SupervisionEvent::ActorStarted(myself.clone()));

        // let mut last_state = state.clone();
        let myself_clone = myself.clone();
        let handler_clone = handler.clone();

        let last_state = Arc::new(tokio::sync::RwLock::new(state.clone()));

        let last_state_clone = last_state.clone();
        let handle = tokio::spawn(async move {
            let mut ports = ports;
            while let (n_state, None) = Self::process_message(
                myself_clone.clone(),
                state,
                handler_clone.clone(),
                &mut ports,
            )
            .await
            {
                state = n_state;
                *(last_state_clone.write().await) = state.clone();
            }
        });

        match handle_panickable(handle).await {
            PanickableResult::Ok(_) => (),
            PanickableResult::Cancelled => return Err(ActorErr::Cancelled),
            PanickableResult::Panic(msg) => return Err(ActorErr::Panic(msg)),
        }

        myself.set_status(ActorStatus::Stopping);

        let deref_state = last_state.read().await;
        Self::do_post_stop(myself, handler, deref_state.clone()).await
    }

    /// Process a message, returning the "new" state (if changed)
    /// along with optionally whether we were signaled mid-processing or not
    async fn process_message(
        myself: ActorCell,
        state: TState,
        handler: Arc<THandler>,
        ports: &mut ActorPortSet,
    ) -> (TState, Option<Signal>) {
        tokio::select! {
            signal = ports.signal_rx.recv() => {
                (state, Some(Self::handle_signal(myself, signal.unwrap_or(Signal::Exit)).await))
            },
            supervision = ports.supervisor_rx.recv() => {
                let state = Self::handle_supervision_message(myself.clone(), &state, handler.clone(), supervision).await.unwrap_or(state);
                (state, None)
            }
            message = ports.message_rx.recv() => {
                Self::handle_message(myself, ports, state, handler, message).await
            }
        }
    }

    async fn handle_message(
        myself: ActorCell,
        ports: &mut ActorPortSet,
        state: TState,
        handler: Arc<THandler>,
        message: Option<BoxedMessage>,
    ) -> (TState, Option<Signal>) {
        if let Some(mut msg) = message {
            let typed_msg = match msg.take() {
                Ok(m) => m,
                Err(_) => {
                    panic!(
                        "Failed to convert message from `BoxedMessage` to `{}`",
                        std::any::type_name::<TMsg>()
                    )
                }
            };
            // NOTE: We listen for the signal port again during the processing of async work in order
            // to "cancel" any pending work should a signal be received immediately
            tokio::select! {
                signal = ports.signal_rx.recv() => {
                    (state, Some(Self::handle_signal(myself, signal.unwrap_or(Signal::Exit)).await))
                }
                maybe_new_state = handler.handle(myself.clone(), typed_msg, &state) => {
                    if let Some(new_state) = maybe_new_state {
                        (new_state, None)
                    } else {
                        (state, None)
                    }
                }
            }
        } else {
            (state, Some(Self::handle_signal(myself, Signal::Exit).await))
        }
    }

    async fn handle_signal(myself: ActorCell, signal: Signal) -> Signal {
        match &signal {
            Signal::Exit => {
                myself.terminate();
            }
        }
        // signal's always bubble up
        signal
    }

    async fn handle_supervision_message(
        myself: ActorCell,
        state: &TState,
        handler: Arc<THandler>,
        message: Option<SupervisionEvent>,
    ) -> Option<TState> {
        if let Some(evt) = message {
            let maybe_new_state = handler.handle_supervisor_evt(myself, evt, state).await;
            maybe_new_state
        } else {
            None
        }
    }

    async fn do_pre_start(myself: ActorCell, handler: Arc<THandler>) -> Result<TState, SpawnErr> {
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
        myself: ActorCell,
        handler: Arc<THandler>,
        state: TState,
    ) -> Result<TState, ActorErr> {
        // TODO: Not a fan of cloning here :/ but the async move moves ownership of "state" into
        // the async block
        let original_state = state.clone();
        let post_start_result = handle_panickable(tokio::spawn(async move {
            handler.post_start(myself, &state).await
        }))
        .await;
        match post_start_result {
            PanickableResult::Ok(Some(new_state)) => Ok(new_state),
            PanickableResult::Ok(None) => Ok(original_state),
            PanickableResult::Cancelled => Err(ActorErr::Cancelled),
            PanickableResult::Panic(panic_information) => Err(ActorErr::Panic(format!(
                "Actor panicked in post_start with '{}'",
                panic_information
            ))),
        }
    }

    async fn do_post_stop(
        myself: ActorCell,
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
