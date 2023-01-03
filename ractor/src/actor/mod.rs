// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Holds the logic to a basic actor building-block

use std::panic::{self, AssertUnwindSafe, RefUnwindSafe};
use std::sync::Arc;

pub mod messages;
use messages::*;

pub mod actor_cell;
pub mod errors;
pub mod supervision;

use crate::{Message, State};
use actor_cell::{ActorCell, ActorPortSet, ActorStatus};
use errors::{ActorProcessingErr, SpawnErr};

pub(crate) fn get_panic_string(e: Box<dyn std::any::Any + Send>) -> String {
    match e.downcast::<String>() {
        Ok(v) => *v,
        Err(e) => match e.downcast::<&str>() {
            Ok(v) => v.to_string(),
            _ => "Unknown panic occurred which couldn't be coerced to a string".to_string(),
        },
    }
}

/// The message handling implementation for an actor with a specific type of input message and state
#[async_trait::async_trait]
pub trait ActorHandler: Sync + Send + RefUnwindSafe + 'static {
    /// The message type for this handler
    type Msg: Message;
    /// The type of state this actor deals with
    type State: State;

    /// Invoked when an actor is being started by the system.
    ///
    /// Any initialization inherent to the actor's role should be
    /// performed here hence why it returns the initial state.
    ///
    /// Panics in `pre_start` do not invoke the
    /// supervision strategy and the actor will be terminated.
    fn pre_start(&self, _this_actor: ActorCell) -> Self::State;

    /// Invoked after an actor has started.
    ///
    /// Any post initialization can be performed here, such as writing
    /// to a log file, emmitting metrics.
    ///
    /// Panics in `post_start` follow the supervision strategy.
    fn post_start(&self, _this_actor: ActorCell, _state: &Self::State) -> Option<Self::State> {
        None
    }

    /// Invoked after an actor has been stopped.
    fn post_stop(&self, _state: &Self::State) {}

    /// Handle the incoming message in a basic event processing loop. Unhandled panic's will be captured and
    /// treated as agent death in the supervision tree
    async fn handle(
        &self,
        _this_actor: ActorCell,
        _message: Self::Msg,
        _state: &Self::State,
    ) -> Option<Self::State> {
        None
    }

    /// Handle the incoming supervision event. Unhandled panic's will captured and treated as agent death in
    /// the supervision tree
    async fn handle_supervisor_evt(
        &self,
        _this_actor: ActorCell,
        _message: SupervisionEvent,
        _state: &Self::State,
    ) -> Option<Self::State> {
        None
    }
}

/// The basic actor with a
/// 1. input message type
/// 2. state
/// 3. message handling implementation
pub struct Actor<TMsg, TState, THandler>
where
    TMsg: Message,
    TState: State,
    THandler: ActorHandler<Msg = TMsg, State = TState>,
{
    base: ActorCell,

    handler: Arc<THandler>,
}

impl<TMsg, TState, THandler> Actor<TMsg, TState, THandler>
where
    TMsg: Message,
    TState: State,
    THandler: ActorHandler<Msg = TMsg, State = TState>,
{
    /// Create a new 10-port agent with some handler implementation and initial state
    pub fn new(name: Option<String>, handler: THandler) -> (Self, ActorPortSet) {
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
    pub async fn start(
        self,
        ports: ActorPortSet,
        supervisor: Option<ActorCell>,
    ) -> Result<tokio::task::JoinHandle<()>, SpawnErr> {
        self.base.set_status(ActorStatus::Starting);

        // Perform the pre-start routine, crashing immediately if we fail to start
        let state = Self::do_pre_start(self.base.clone(), self.handler.clone())?;
        // setup supervision
        if let Some(sup) = &supervisor {
            sup.link(self.base.clone()).await;
        }

        let myself = self.base.clone();

        let handle = tokio::spawn(async move {
            Self::processing_loop(ports, state, self.handler.clone(), self.base.clone()).await
        });

        let handle = tokio::spawn(async move {
            let evt = match handle.await {
                Ok(_) => SupervisionEvent::ActorTerminated(myself.clone()),
                Err(maybe_panic) => {
                    if maybe_panic.is_panic() {
                        let panic_message = get_panic_string(maybe_panic.into_panic());
                        SupervisionEvent::ActorPanicked(myself.clone(), panic_message)
                    } else {
                        // task cancelled, just notify of exit
                        SupervisionEvent::ActorTerminated(myself.clone())
                    }
                }
            };
            let _ = myself.notify_supervisors(evt).await;

            myself.set_status(ActorStatus::Stopped);

            // signal received or process exited cleanly, we should already have "handled" the signal, so we can just terminate
            if let Some(sup) = supervisor {
                sup.unlink(myself.clone()).await;
            }
        });
        Ok(handle)
    }

    async fn processing_loop(
        ports: ActorPortSet,
        state: TState,
        handler: Arc<THandler>,
        myself: ActorCell,
    ) {
        // perform the post-start, with supervision enabled
        let mut state = match Self::do_post_start(myself.clone(), handler.clone(), state) {
            Ok(state) => state,
            Err(err) => {
                panic!("{}", err);
            }
        };

        myself.set_status(ActorStatus::Running);
        let _ = myself
            .notify_supervisors(SupervisionEvent::ActorStarted(myself.clone()))
            .await;

        let mut ports = ports;
        let mut last_state = state.clone();
        while let (n_state, None) =
            Self::process_message(myself.clone(), state, handler.clone(), &mut ports).await
        {
            state = n_state;
            last_state = state.clone();
        }

        myself.set_status(ActorStatus::Stopping);

        if let Err(err) = Self::do_post_stop(handler, &last_state) {
            panic!("{}", err);
        };
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
                    panic!("Failed to convert message from `BoxedMessage` to `TMsg`")
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
                myself.terminate().await;
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
        // TODO: process the specific supervision logic (pass to handler?)
        if let Some(evt) = message {
            let maybe_new_state = handler.handle_supervisor_evt(myself, evt, state).await;
            maybe_new_state
        } else {
            None
        }
    }

    fn do_pre_start(myself: ActorCell, handler: Arc<THandler>) -> Result<TState, SpawnErr> {
        let start_result = panic::catch_unwind(AssertUnwindSafe(|| handler.pre_start(myself)));
        match start_result {
            Ok(state) => {
                // intitialize the state
                Ok(state)
            }
            Err(e) => {
                let panic_information = get_panic_string(e);
                Err(SpawnErr::StartupPanic(format!(
                    "Actor panicked during pre_start with '{}'",
                    panic_information
                )))
            }
        }
    }

    fn do_post_start(
        myself: ActorCell,
        handler: Arc<THandler>,
        state: TState,
    ) -> Result<TState, ActorProcessingErr> {
        let post_start_result =
            panic::catch_unwind(AssertUnwindSafe(|| handler.post_start(myself, &state)));
        match post_start_result {
            Ok(Some(new_state)) => Ok(new_state),
            Ok(None) => Ok(state),
            Err(e) => {
                let panic_information = get_panic_string(e);
                Err(ActorProcessingErr::Panic(format!(
                    "Actor panicked in post_start with '{}'",
                    panic_information
                )))
            }
        }
    }

    fn do_post_stop(handler: Arc<THandler>, state: &TState) -> Result<(), ActorProcessingErr> {
        let post_stop_result = panic::catch_unwind(AssertUnwindSafe(|| handler.post_stop(state)));
        match post_stop_result {
            Ok(_) => Ok(()),
            Err(e) => {
                let panic_information = get_panic_string(e);
                Err(ActorProcessingErr::Panic(format!(
                    "Actor panicked in post_start with '{}'",
                    panic_information
                )))
            }
        }
    }
}
