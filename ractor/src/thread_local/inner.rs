// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Inner / private logic for thread_local actors

use std::fmt::Debug;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::AtomicU8;
use std::sync::Arc;
use std::sync::Mutex;

use futures::FutureExt;
use futures::TryFutureExt;
use tracing::Instrument;

use super::ThreadLocalActor;
use super::ThreadLocalActorSpawner;
use crate::actor::actor_cell;
use crate::actor::actor_cell::ActorPortSet;
use crate::actor::actor_properties::ActorProperties;
use crate::actor::actor_properties::MuxedMessage;
use crate::actor::get_panic_string;
use crate::actor::messages::StopMessage;
use crate::actor::ActorLoopResult;
use crate::concurrency as mpsc;
use crate::concurrency::JoinHandle;
use crate::concurrency::OneshotReceiver;
use crate::message::Message;
use crate::ActorCell;
use crate::ActorErr;
use crate::ActorId;
use crate::ActorName;
use crate::ActorProcessingErr;
use crate::ActorRef;
use crate::ActorStatus;
use crate::MessagingErr;
use crate::Signal;
use crate::SpawnErr;
use crate::SupervisionEvent;

impl ActorCell {
    pub(crate) fn new_thread_local<TActor>(
        name: Option<ActorName>,
    ) -> Result<(Self, ActorPortSet<TActor::Msg>), SpawnErr>
    where
        TActor: crate::thread_local::ThreadLocalActor,
    {
        let (props, rx1, rx2, rx3, rx4) =
            crate::actor::actor_properties::ActorProperties::new_thread_local::<TActor>(
                name.clone(),
            );
        let cell = Self {
            inner: Arc::new(props),
        };

        #[cfg(feature = "cluster")]
        {
            // registry to the PID registry
            crate::registry::pid_registry::register_pid(cell.get_id(), cell.clone())?;
        }

        if let Some(r_name) = name {
            crate::registry::register(r_name, cell.clone())?;
        }

        Ok((
            cell,
            ActorPortSet {
                signal_rx: rx1,
                stop_rx: rx2,
                supervisor_rx: rx3,
                message_rx: rx4,
            },
        ))
    }
}

impl ActorProperties {
    pub(crate) fn new_thread_local<TActor>(
        name: Option<ActorName>,
    ) -> (
        Self,
        OneshotReceiver<Signal>,
        OneshotReceiver<StopMessage>,
        mpsc::MpscUnboundedReceiver<SupervisionEvent>,
        mpsc::MpscUnboundedReceiver<MuxedMessage<TActor::Msg>>,
    )
    where
        TActor: crate::thread_local::ThreadLocalActor,
    {
        let id = crate::actor::actor_id::get_new_local_id();
        let (tx_signal, rx_signal) = mpsc::oneshot();
        let (tx_stop, rx_stop) = mpsc::oneshot();
        let (tx_supervision, rx_supervision) = mpsc::mpsc_unbounded();
        let (tx_message, rx_message) = mpsc::mpsc_unbounded();
        (
            Self {
                id,
                name,
                status: AtomicU8::new(ActorStatus::Unstarted as u8),
                signal: Mutex::new(Some(tx_signal)),
                wait_handler: mpsc::Notify::new(),
                stop: Mutex::new(Some(tx_stop)),
                supervision: tx_supervision,
                message: Box::new(tx_message),
                tree: Default::default(),
                type_id: std::any::TypeId::of::<TActor::Msg>(),
                #[cfg(feature = "cluster")]
                supports_remoting: TActor::Msg::serializable(),
            },
            rx_signal,
            rx_stop,
            rx_supervision,
            rx_message,
        )
    }
}

/// Actor runtime specific to thread-local actors, which doesn't require the actors
/// to implement [Send]
pub(crate) struct ThreadLocalActorRuntime<TActor>
where
    TActor: ThreadLocalActor,
{
    actor_ref: ActorRef<TActor::Msg>,
    id: ActorId,
    name: Option<String>,
}

impl<TActor: ThreadLocalActor> Debug for ThreadLocalActorRuntime<TActor> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThreadLocalActorRuntime")
            .field("name", &self.name)
            .field("id", &self.id)
            .finish()
    }
}

impl<TActor: ThreadLocalActor> ThreadLocalActorRuntime<TActor> {
    /// Create a new actor with some handler implementation and initial state
    ///
    /// * `name`: A name to give the actor. Useful for global referencing or debug printing
    /// * `handler` The [Actor] defining the logic for this actor
    ///
    /// Returns A tuple [(Actor, ActorPortSet)] to be passed to the `start` function of [Actor]
    fn new(
        name: Option<crate::ActorName>,
    ) -> Result<(Self, crate::actor::actor_cell::ActorPortSet<TActor::Msg>), crate::SpawnErr> {
        let (actor_cell, ports) =
            crate::actor::actor_cell::ActorCell::new_thread_local::<TActor>(name)?;
        let id = actor_cell.get_id();
        let name = actor_cell.get_name();
        Ok((
            Self {
                actor_ref: actor_cell.into(),
                id,
                name,
            },
            ports,
        ))
    }

    /// Spawn an actor, which is unsupervised, automatically starting the actor
    ///
    /// * `name`: A name to give the actor. Useful for global referencing or debug printing
    /// * `handler` The [Actor] defining the logic for this actor
    /// * `startup_args`: Arguments passed to the `pre_start` call of the [Actor] to facilitate startup and
    ///   initial state creation
    /// * `spawner`: The [ThreadLocalActorSpawner] used to spawn [ThreadLocalActor] async instances
    ///
    /// Returns a [Ok((ActorRef, JoinHandle<()>))] upon successful start, denoting the actor reference
    /// along with the join handle which will complete when the actor terminates. Returns [Err(SpawnErr)] if
    /// the actor failed to start
    pub(crate) async fn spawn(
        name: Option<ActorName>,
        startup_args: TActor::Arguments,
        spawner: ThreadLocalActorSpawner,
    ) -> Result<(ActorRef<TActor::Msg>, JoinHandle<()>), SpawnErr> {
        let (actor, ports) = Self::new(name)?;
        let aref = actor.actor_ref.clone();
        let result = actor.start(ports, startup_args, spawner, None).await;
        if result.is_err() {
            aref.set_status(ActorStatus::Stopped);
        }
        result
    }

    /// Spawn an actor with a supervisor, automatically starting the actor
    ///
    /// * `name`: A name to give the actor. Useful for global referencing or debug printing
    /// * `handler` The [Actor] defining the logic for this actor
    /// * `startup_args`: Arguments passed to the `pre_start` call of the [Actor] to facilitate startup and
    ///   initial state creation
    /// * `spawner`: The [ThreadLocalActorSpawner] used to spawn [ThreadLocalActor] async instances
    /// * `supervisor`: The [ActorCell] which is to become the supervisor (parent) of this actor
    ///
    /// Returns a [Ok((ActorRef, JoinHandle<()>))] upon successful start, denoting the actor reference
    /// along with the join handle which will complete when the actor terminates. Returns [Err(SpawnErr)] if
    /// the actor failed to start
    pub(crate) async fn spawn_linked(
        name: Option<ActorName>,
        startup_args: TActor::Arguments,
        spawner: ThreadLocalActorSpawner,
        supervisor: ActorCell,
    ) -> Result<(ActorRef<TActor::Msg>, JoinHandle<()>), SpawnErr> {
        let (actor, ports) = Self::new(name)?;
        let aref = actor.actor_ref.clone();
        let result = actor
            .start(ports, startup_args, spawner, Some(supervisor))
            .await;
        if result.is_err() {
            aref.set_status(ActorStatus::Stopped);
        }
        result
    }

    /// Start the actor immediately, optionally linking to a parent actor (supervision tree)
    ///
    /// NOTE: This returned [crate::concurrency::JoinHandle] is guaranteed to not panic (unless the runtime is shutting down perhaps).
    /// An inner join handle is capturing panic results from any part of the inner tasks, so therefore
    /// we can safely ignore it, or wait on it to block on the actor's progress
    ///
    /// * `ports` - The [ActorPortSet] for this actor
    /// * `supervisor` - The optional [ActorCell] representing the supervisor of this actor
    ///
    /// Returns a [Ok((ActorRef, JoinHandle<()>))] upon successful start, denoting the actor reference
    /// along with the join handle which will complete when the actor terminates. Returns [Err(SpawnErr)] if
    /// the actor failed to start
    #[tracing::instrument(name = "Actor", skip(self, ports, startup_args, supervisor), fields(id = self.id.to_string(), name = self.name))]
    async fn start(
        self,
        ports: ActorPortSet<TActor::Msg>,
        startup_args: TActor::Arguments,
        spawner: ThreadLocalActorSpawner,
        supervisor: Option<ActorCell>,
    ) -> Result<(ActorRef<TActor::Msg>, JoinHandle<()>), SpawnErr> {
        // cannot start an actor more than once
        if self.actor_ref.get_status() != ActorStatus::Unstarted {
            return Err(SpawnErr::ActorAlreadyStarted);
        }

        let Self {
            actor_ref,
            id,
            name,
        } = self;

        actor_ref.set_status(ActorStatus::Starting);

        // Generate the ActorRef which will be returned
        let spawn_name = name.clone();
        let myself_ret = actor_ref.clone();

        // run the processing loop, backgrounding the work
        let builder = Box::new(move || {
            let handler = TActor::default();
            async move {
                // Perform the pre-start routine, crashing immediately if we fail to start
                let mut state = Self::do_pre_start(actor_ref.clone(), &handler, startup_args)
                    .await?
                    .map_err(SpawnErr::StartupFailed)?;

                // setup supervision
                if let Some(sup) = &supervisor {
                    actor_ref.link(sup.clone());
                }

                // run the processing loop, backgrounding the work
                let handle = tokio::task::spawn_local(async move {
                    let myself = actor_ref.clone();
                    let evt = match Self::processing_loop(
                        ports, &mut state, &handler, actor_ref, id, name,
                    )
                    .await
                    {
                        // IMPORTANT: Because the State in ThreadLocalActor's is not Send, we cannot
                        // construct a boxed state since it can't be sent to the supervisor
                        Ok(exit_reason) => {
                            SupervisionEvent::ActorTerminated(myself.get_cell(), None, exit_reason)
                        }
                        Err(actor_err) => match actor_err {
                            ActorErr::Cancelled => SupervisionEvent::ActorTerminated(
                                myself.get_cell(),
                                None,
                                Some("killed".to_string()),
                            ),
                            ActorErr::Failed(msg) => {
                                SupervisionEvent::ActorFailed(myself.get_cell(), msg)
                            }
                        },
                    };

                    // terminate children
                    myself.terminate();

                    // notify supervisors of the actor's death
                    myself.notify_supervisor_and_monitors(evt);

                    // unlink superisors
                    if let Some(sup) = supervisor {
                        myself.unlink(sup);
                    }

                    // set status to stopped
                    myself.set_status(ActorStatus::Stopped);
                });
                Ok(handle)
            }
            .boxed_local()
        });
        let handle = spawner
            .spawn(builder, spawn_name)
            .await
            .map_err(|e| SpawnErr::StartupFailed(e.into()))?;

        Ok((myself_ret, handle))
    }

    #[tracing::instrument(name = "Actor", skip(ports, state, handler, myself, _id, _name), fields(id = _id.to_string(), name = _name))]
    async fn processing_loop(
        mut ports: ActorPortSet<TActor::Msg>,
        state: &mut TActor::State,
        handler: &TActor,
        myself: ActorRef<TActor::Msg>,
        _id: ActorId,
        _name: Option<String>,
    ) -> Result<Option<String>, ActorErr> {
        // perform the post-start, with supervision enabled
        Self::do_post_start(myself.clone(), handler, state)
            .await?
            .map_err(ActorErr::Failed)?;

        myself.set_status(ActorStatus::Running);
        myself.notify_supervisor_and_monitors(SupervisionEvent::ActorStarted(myself.get_cell()));

        let myself_clone = myself.clone();

        let future = async move {
            // the message processing loop. If we get an exit flag, try and capture the exit reason if there
            // is one
            loop {
                let ActorLoopResult {
                    should_exit,
                    exit_reason,
                    was_killed,
                } = Self::process_message(myself.clone(), state, handler, &mut ports)
                    .await
                    .map_err(ActorErr::Failed)?;
                // processing loop exit
                if should_exit {
                    return Ok((state, exit_reason, was_killed));
                }
            }
        };

        // capture any panics in this future and convert to an ActorErr
        let loop_done = futures::FutureExt::catch_unwind(AssertUnwindSafe(future))
            .map_err(|err| ActorErr::Failed(get_panic_string(err)))
            .await;

        // set status to stopping
        myself_clone.set_status(ActorStatus::Stopping);

        let (exit_state, exit_reason, was_killed) = loop_done??;

        // if we didn't exit in error mode, call `post_stop`
        if !was_killed {
            Self::do_post_stop(myself_clone, handler, exit_state)
                .await?
                .map_err(ActorErr::Failed)?;
        }

        Ok(exit_reason)
    }

    /// Process a message, returning the "new" state (if changed)
    /// along with optionally whether we were signaled mid-processing or not
    ///
    /// * `myself` - The current [ActorRef]
    /// * `state` - The current [Actor::State] object
    /// * `handler` - Pointer to the [Actor] definition
    /// * `ports` - The mutable [ActorPortSet] which are the message ports for this actor
    ///
    /// Returns a tuple of the next [Actor::State] and a flag to denote if the processing
    /// loop is done
    async fn process_message(
        myself: ActorRef<TActor::Msg>,
        state: &mut TActor::State,
        handler: &TActor,
        ports: &mut ActorPortSet<TActor::Msg>,
    ) -> Result<ActorLoopResult, ActorProcessingErr> {
        match ports.listen_in_priority().await {
            Ok(actor_port_message) => match actor_port_message {
                actor_cell::ActorPortMessage::Signal(signal) => {
                    Ok(ActorLoopResult::signal(Self::handle_signal(myself, signal)))
                }
                actor_cell::ActorPortMessage::Stop(stop_message) => {
                    let exit_reason = match stop_message {
                        StopMessage::Stop => {
                            tracing::trace!("Actor {:?} stopped with no reason", myself.get_id());
                            None
                        }
                        StopMessage::Reason(reason) => {
                            tracing::trace!(
                                "Actor {:?} stopped with reason '{reason}'",
                                myself.get_id(),
                            );
                            Some(reason)
                        }
                    };
                    Ok(ActorLoopResult::stop(exit_reason))
                }
                actor_cell::ActorPortMessage::Supervision(supervision) => {
                    let future = Self::handle_supervision_message(
                        myself.clone(),
                        state,
                        handler,
                        supervision,
                    );
                    match ports.run_with_signal(future).await {
                        Ok(Ok(())) => Ok(ActorLoopResult::ok()),
                        Ok(Err(internal_err)) => Err(internal_err),
                        Err(signal) => {
                            Ok(ActorLoopResult::signal(Self::handle_signal(myself, signal)))
                        }
                    }
                }
                actor_cell::ActorPortMessage::Message(MuxedMessage::Message(msg)) => {
                    let future = Self::handle_message(myself.clone(), state, handler, msg);
                    match ports.run_with_signal(future).await {
                        Ok(Ok(())) => Ok(ActorLoopResult::ok()),
                        Ok(Err(internal_err)) => Err(internal_err),
                        Err(signal) => {
                            Ok(ActorLoopResult::signal(Self::handle_signal(myself, signal)))
                        }
                    }
                }
                actor_cell::ActorPortMessage::Message(MuxedMessage::Drain) => {
                    // Drain is a stub marker that the actor should now stop, we've processed
                    // all the messages and we want the actor to die now
                    Ok(ActorLoopResult::stop(Some("Drained".to_string())))
                }
            },
            Err(MessagingErr::ChannelClosed) => {
                // one of the channels is closed, this means
                // the receiver was dropped and in this case
                // we should always die. Therefore we flag
                // to terminate
                Ok(ActorLoopResult::signal(Self::handle_signal(
                    myself,
                    Signal::Kill,
                )))
            }
            Err(MessagingErr::InvalidActorType) => {
                // not possible. Treat like a channel closed
                Ok(ActorLoopResult::signal(Self::handle_signal(
                    myself,
                    Signal::Kill,
                )))
            }
            Err(MessagingErr::SendErr(_)) => {
                // not possible. Treat like a channel closed
                Ok(ActorLoopResult::signal(Self::handle_signal(
                    myself,
                    Signal::Kill,
                )))
            }
        }
    }

    async fn handle_message(
        myself: ActorRef<TActor::Msg>,
        state: &mut TActor::State,
        handler: &TActor,
        mut msg: crate::message::BoxedMessage<TActor::Msg>,
    ) -> Result<(), ActorProcessingErr> {
        // panic in order to kill the actor
        #[cfg(feature = "cluster")]
        {
            // A `RemoteActor` will handle serialized messages, without decoding them, forwarding them
            // to the remote system for decoding + handling by the real implementation. Therefore `RemoteActor`s
            // can be thought of as a "shim" to a real actor on a remote system
            if !myself.get_id().is_local() {
                match msg.msg.into_serialized() {
                    Some(serialized_msg) => {
                        return handler
                            .handle_serialized(myself, serialized_msg, state)
                            .await;
                    }
                    None => {
                        return Err(From::from(
                            "`RemoteActor` failed to read `SerializedMessage` from `BoxedMessage`",
                        ));
                    }
                }
            }
        }

        // The current [tracing::Span] is retrieved, boxed, and included in every
        // `BoxedMessage` during the conversion of this `TActor::Msg`. It is used
        // to automatically continue tracing span nesting when sending messages to Actors.
        let current_span_when_message_was_sent = msg.span.take();

        // An error here will bubble up to terminate the actor
        let typed_msg = TActor::Msg::from_boxed(msg)?;

        if let Some(span) = current_span_when_message_was_sent {
            handler
                .handle(myself, typed_msg, state)
                .instrument(span)
                .await
        } else {
            handler.handle(myself, typed_msg, state).await
        }
    }

    fn handle_signal(myself: ActorRef<TActor::Msg>, signal: Signal) -> Option<String> {
        match &signal {
            Signal::Kill => {
                myself.terminate();
            }
        }
        Some(signal.to_string())
    }

    async fn handle_supervision_message(
        myself: ActorRef<TActor::Msg>,
        state: &mut TActor::State,
        handler: &TActor,
        message: SupervisionEvent,
    ) -> Result<(), ActorProcessingErr> {
        handler.handle_supervisor_evt(myself, message, state).await
    }

    async fn do_pre_start(
        myself: ActorRef<TActor::Msg>,
        handler: &TActor,
        arguments: TActor::Arguments,
    ) -> Result<Result<TActor::State, ActorProcessingErr>, SpawnErr> {
        let future = handler.pre_start(myself, arguments);
        futures::FutureExt::catch_unwind(AssertUnwindSafe(future))
            .await
            .map_err(|err| SpawnErr::StartupFailed(get_panic_string(err)))
    }

    async fn do_post_start(
        myself: ActorRef<TActor::Msg>,
        handler: &TActor,
        state: &mut TActor::State,
    ) -> Result<Result<(), ActorProcessingErr>, ActorErr> {
        let future = handler.post_start(myself, state);
        futures::FutureExt::catch_unwind(AssertUnwindSafe(future))
            .await
            .map_err(|err| ActorErr::Failed(get_panic_string(err)))
    }

    async fn do_post_stop(
        myself: ActorRef<TActor::Msg>,
        handler: &TActor,
        state: &mut TActor::State,
    ) -> Result<Result<(), ActorProcessingErr>, ActorErr> {
        let future = handler.post_stop(myself, state);
        futures::FutureExt::catch_unwind(AssertUnwindSafe(future))
            .await
            .map_err(|err| ActorErr::Failed(get_panic_string(err)))
    }
}
