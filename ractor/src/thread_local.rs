// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! The thread-local module provides support for managing tasks which are not [Send]
//! safe, and must remain pinned to the same thread for their lifetime.
//!
//! IMPORTANT: This ONLY works currently with Tokio's `rt` feature, specifically due to the usage
//! of [tokio::task::LocalSet]

use std::future::Future;

#[cfg(feature = "derived-actor-from-cell")]
use crate::actor::RequestDerived;
use crate::concurrency::JoinHandle;
use crate::Actor as SendActor;
use crate::ActorCell;
use crate::ActorName;
use crate::ActorProcessingErr;
use crate::ActorRef;
use crate::Message;
use crate::RpcReplyPort;
use crate::SpawnErr;
use crate::State;
use crate::SupervisionEvent;

mod inner;
#[cfg(test)]
mod supervision_tests;
#[cfg(test)]
mod tests;

/// [ThreadLocalActor] defines the behavior of an Actor. It specifies the
/// Message type, State type, and all processing logic for the actor
///
/// NOTE: All of the implemented trait functions
///
/// * `pre_start`
/// * `post_start`
/// * `post_stop`
/// * `handle`
/// * `handle_serialized` (Available with `cluster` feature only)
/// * `handle_supervisor_evt`
///
/// return a [Result<_, ActorProcessingError>] where the error type is an
/// alias of [Box<dyn std::error::Error + Send + Sync + 'static>]. This is treated
/// as an "unhandled" error and will terminate the actor + execute necessary supervision
/// patterns. Panics are also captured from the inner functions and wrapped into an Error
/// type, however should an [Err(_)] result from any of these functions the **actor will
/// terminate** and cleanup.
///
/// # Example
///
/// ```
/// use ractor::thread_local::ThreadLocalActor;
/// use ractor::thread_local::ThreadLocalActorSpawner;
/// use ractor::ActorProcessingErr;
/// use ractor::ActorRef;
///
/// #[derive(Default)]
/// struct TheActor;
///
/// impl ThreadLocalActor for TheActor {
///     type Msg = ();
///     type Arguments = String;
///     type State = String;
///
///     async fn pre_start(
///         &self,
///         _myself: ActorRef<Self::Msg>,
///         args: Self::Arguments,
///     ) -> Result<Self::State, ActorProcessingErr> {
///         Ok(args)
///     }
///
///     async fn handle(
///         &self,
///         _myself: ActorRef<Self::Msg>,
///         _msg: (),
///         state: &mut Self::State,
///     ) -> Result<(), ActorProcessingErr> {
///         println!("Message! {state}");
///         Ok(())
///     }
/// }
///
/// #[tokio::main]
/// async fn main() {
///     // Create the thread-local spawner
///     let spawner = ThreadLocalActorSpawner::new();
///     // spawn the actor
///     let (who, handle) =
///         ractor::spawn_local::<TheActor>("Something".to_string(), spawner.clone())
///             .await
///             .expect("Failed to spawn thread-local actor!");
///
///     // send messages to the actor
///     who.cast(()).expect("Failed to send");
///     who.cast(()).expect("Failed to send");
///
///     // Tell the actor to drain then stop
///     who.drain();
///
///     // wait for the termination
///     handle.await.unwrap();
/// }
/// ```
pub trait ThreadLocalActor: Default + Sized + 'static {
    /// The message type for this actor
    type Msg: Message;

    /// The type of state this actor manages internally. This type
    /// has no bound requirements, and needs to neither be [Send] nor
    /// [Sync] when used in a [ThreadLocalActor] context.
    type State;

    /// Initialization arguments. These must be [Send] as they are
    /// sent to the pinned thread in order to startup the actor.
    /// However the actor's local [ThreadLocalActor::State] does
    /// NOT need to be [Send] and neither does the actor instance.
    type Arguments: State;

    /// Invoked when an actor is being started by the system.
    ///
    /// Any initialization inherent to the actor's role should be
    /// performed here hence why it returns the initial state.
    ///
    /// Panics in `pre_start` do not invoke the
    /// supervision strategy and the actor won't be started. [ThreadLocalActor]::`spawn`
    /// will return an error to the caller
    ///
    /// * `myself` - A handle to the [ActorCell] representing this actor
    /// * `args` - Arguments that are passed in the spawning of the actor which might
    ///   be necessary to construct the initial state
    ///
    /// Returns an initial [ThreadLocalActor::State] to bootstrap the actor
    fn pre_start(
        &self,
        myself: ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> impl Future<Output = Result<Self::State, ActorProcessingErr>>;

    /// Invoked after an actor has started.
    ///
    /// Any post initialization can be performed here, such as writing
    /// to a log file, emitting metrics.
    ///
    /// Panics in `post_start` follow the supervision strategy.
    ///
    /// * `myself` - A handle to the [ActorCell] representing this actor
    /// * `state` - A mutable reference to the internal actor's state
    #[allow(unused_variables)]
    fn post_start(
        &self,
        myself: ActorRef<Self::Msg>,
        state: &mut Self::State,
    ) -> impl Future<Output = Result<(), ActorProcessingErr>> {
        async { Ok(()) }
    }

    /// Invoked after an actor has been stopped to perform final cleanup. In the
    /// event the actor is terminated with `Signal::Kill` or has self-panicked,
    /// `post_stop` won't be called.
    ///
    /// Panics in `post_stop` follow the supervision strategy.
    ///
    /// * `myself` - A handle to the [ActorCell] representing this actor
    /// * `state` - A mutable reference to the internal actor's last known state
    #[allow(unused_variables)]
    fn post_stop(
        &self,
        myself: ActorRef<Self::Msg>,
        state: &mut Self::State,
    ) -> impl Future<Output = Result<(), ActorProcessingErr>> {
        async { Ok(()) }
    }

    /// Handle the incoming message from the event processing loop. Unhandled panickes will be
    /// captured and sent to the supervisor(s)
    ///
    /// * `myself` - A handle to the [ActorCell] representing this actor
    /// * `message` - The message to process
    /// * `state` - A mutable reference to the internal actor's state
    #[allow(unused_variables)]
    fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> impl Future<Output = Result<(), ActorProcessingErr>> {
        async { Ok(()) }
    }

    /// Handle the remote incoming message from the event processing loop. Unhandled panickes will be
    /// captured and sent to the supervisor(s)
    ///
    /// * `myself` - A handle to the [ActorCell] representing this actor
    /// * `message` - The serialized message to handle
    /// * `state` - A mutable reference to the internal actor's state
    #[allow(unused_variables)]
    #[cfg(feature = "cluster")]
    fn handle_serialized(
        &self,
        myself: ActorRef<Self::Msg>,
        message: crate::message::SerializedMessage,
        state: &mut Self::State,
    ) -> impl Future<Output = Result<(), ActorProcessingErr>> {
        async { Ok(()) }
    }

    /// Handle the incoming supervision event. Unhandled panics will be captured and
    /// sent the the supervisor(s). The default supervision behavior is to exit the
    /// supervisor on any child exit. To override this behavior, implement this function.
    ///
    /// * `myself` - A handle to the [ActorCell] representing this actor
    /// * `message` - The message to process
    /// * `state` - A mutable reference to the internal actor's state
    #[allow(unused_variables)]
    fn handle_supervisor_evt(
        &self,
        myself: ActorRef<Self::Msg>,
        message: SupervisionEvent,
        state: &mut Self::State,
    ) -> impl Future<Output = Result<(), ActorProcessingErr>> {
        async move {
            match message {
                SupervisionEvent::ActorTerminated(who, _, _)
                | SupervisionEvent::ActorFailed(who, _) => {
                    myself.stop(None);
                }
                _ => {}
            }
            Ok(())
        }
    }

    /// Spawn an actor of this type, which is unsupervised, automatically starting
    ///
    /// * `name`: A name to give the actor. Useful for global referencing or debug printing
    /// * `startup_args`: Arguments passed to the `pre_start` call of the [ThreadLocalActor] to facilitate startup and
    ///   initial state creation
    /// * `spawner`: The [ThreadLocalActorSpawner] which will control starting this actor on a specific
    ///   thread
    ///
    /// Returns a [Ok((ActorRef, JoinHandle<()>))] upon successful start, denoting the actor reference
    /// along with the join handle which will complete when the actor terminates. Returns [Err(SpawnErr)] if
    /// the actor failed to start
    fn spawn(
        name: Option<ActorName>,
        startup_args: Self::Arguments,
        spawner: ThreadLocalActorSpawner,
    ) -> impl Future<Output = Result<(ActorRef<Self::Msg>, JoinHandle<()>), SpawnErr>> {
        inner::ThreadLocalActorRuntime::<Self>::spawn(name, startup_args, spawner)
    }
    /// Spawn an actor of this type with a supervisor, automatically starting the actor
    ///
    /// * `name`: A name to give the actor. Useful for global referencing or debug printing
    /// * `startup_args`: Arguments passed to the `pre_start` call of the [ThreadLocalActor] to
    ///   facilitate startup and initial state creation
    /// * `supervisor`: The [ActorCell] which is to become the supervisor (parent) of this actor
    /// * `spawner`: The [ThreadLocalActorSpawner] which will control starting this actor on a specific
    ///   thread
    ///
    /// Returns a [Ok((ActorRef, JoinHandle<()>))] upon successful start, denoting the actor reference
    /// along with the join handle which will complete when the actor terminates. Returns [Err(SpawnErr)] if
    /// the actor failed to start
    fn spawn_linked(
        name: Option<ActorName>,
        startup_args: Self::Arguments,
        supervisor: ActorCell,
        spawner: ThreadLocalActorSpawner,
    ) -> impl Future<Output = Result<(ActorRef<Self::Msg>, JoinHandle<()>), SpawnErr>> {
        inner::ThreadLocalActorRuntime::<Self>::spawn_linked(
            name,
            startup_args,
            spawner,
            supervisor,
        )
    }
    /// Provide to request derived actors that can be derived from actor_ref.
    #[cfg(feature = "derived-actor-from-cell")]
    #[allow(unused_variables)]
    fn provide_derived_actor_ref<'a>(
        my_self: ActorRef<Self::Msg>,
        request: RequestDerived<'a>,
    ) -> RequestDerived<'a> {
        request
    }
}

impl<T> ThreadLocalActor for T
where
    T: SendActor + Default,
{
    type Msg = <T as SendActor>::Msg;
    type State = <T as SendActor>::State;
    type Arguments = <T as SendActor>::Arguments;

    async fn pre_start(
        &self,
        myself: ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        <Self as SendActor>::pre_start(self, myself, args).await
    }

    async fn post_start(
        &self,
        myself: ActorRef<Self::Msg>,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        <Self as SendActor>::post_start(self, myself, state).await
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        <Self as SendActor>::handle(self, myself, message, state).await
    }

    #[cfg(feature = "cluster")]
    async fn handle_serialized(
        &self,
        myself: ActorRef<Self::Msg>,
        message: crate::message::SerializedMessage,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        <Self as SendActor>::handle_serialized(self, myself, message, state).await
    }

    async fn handle_supervisor_evt(
        &self,
        myself: ActorRef<Self::Msg>,
        message: SupervisionEvent,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        <Self as SendActor>::handle_supervisor_evt(self, myself, message, state).await
    }
}

#[allow(clippy::type_complexity)]
struct SpawnArgs {
    builder: Box<
        dyn FnOnce() -> std::pin::Pin<Box<dyn Future<Output = Result<JoinHandle<()>, SpawnErr>>>>
            + Send,
    >,
    reply: RpcReplyPort<JoinHandle<Result<JoinHandle<()>, SpawnErr>>>,
    name: Option<String>,
}

/// The [ThreadLocalActorSpawner] is responsible for spawning [ThreadLocalActor] instances
/// which do not require [Send] restrictions and will be pinned to the current thread. You can
/// make multiple of these to "load balance" actors across threads and can spawn multiple actors
/// on a single one to be shared on a single thread.
#[derive(Clone)]
pub struct ThreadLocalActorSpawner {
    send: crate::concurrency::MpscUnboundedSender<SpawnArgs>,
}

impl std::fmt::Debug for ThreadLocalActorSpawner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ThreadLocalActorSpawner")
    }
}

impl Default for ThreadLocalActorSpawner {
    fn default() -> Self {
        Self::new()
    }
}

impl ThreadLocalActorSpawner {
    /// Create a new [ThreadLocalActorSpawner] on the current thread.
    pub fn new() -> Self {
        let (send, mut recv) = crate::concurrency::mpsc_unbounded();

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        std::thread::spawn(move || {
            let local = tokio::task::LocalSet::new();

            // TODO (seanlawlor): Supported named spawn
            local.spawn_local(async move {
                while let Some(SpawnArgs {
                    builder,
                    reply,
                    name,
                }) = recv.recv().await
                {
                    let fut = builder();
                    #[cfg(tokio_unstable)]
                    {
                        let handle = tokio::task::Builder::new()
                            .name(name.unwrap_or_default().as_str())
                            .spawn_local(fut)
                            .expect("Tokio task spawn failed");
                        _ = reply.send(handle);
                    }
                    #[cfg(not(tokio_unstable))]
                    {
                        _ = name;
                        let handle = tokio::task::spawn_local(fut);
                        _ = reply.send(handle);
                    }
                }
                // If the while loop returns, then all the LocalSpawner
                // objects have been dropped.
            });

            // This will return once all senders are dropped and all
            // spawned tasks have returned.
            rt.block_on(local);
        });

        Self { send }
    }

    #[allow(clippy::type_complexity)]
    async fn spawn(
        &self,
        builder: Box<
            dyn FnOnce()
                    -> std::pin::Pin<Box<dyn Future<Output = Result<JoinHandle<()>, SpawnErr>>>>
                + Send,
        >,
        name: Option<String>,
    ) -> Result<JoinHandle<()>, SpawnErr> {
        let (tx, rx) = crate::concurrency::oneshot();
        let args = SpawnArgs {
            builder,
            reply: tx.into(),
            name,
        };

        if self.send.send(args).is_err() {
            return Err(SpawnErr::StartupFailed("Spawner dead".into()));
        }

        rx.await
            .map_err(|inner| SpawnErr::StartupFailed(inner.into()))?
            .await
            .map_err(|joinerr| SpawnErr::StartupFailed(joinerr.into()))?
    }
}

impl ActorCell {
    /// Spawn an actor of the given type as a thread-local child of this actor, automatically starting the actor.
    /// This [ActorCell] becomes the supervisor of the child actor.
    ///
    /// * `name`: A name to give the actor. Useful for global referencing or debug printing
    /// * `handler` The implementation of Self
    /// * `startup_args`: Arguments passed to the `pre_start` call of the [ThreadLocalActor] to facilitate startup and
    ///   initial state creation
    ///
    /// Returns a [Ok((ActorRef, JoinHandle<()>))] upon successful start, denoting the actor reference
    /// along with the join handle which will complete when the actor terminates. Returns [Err(SpawnErr)] if
    /// the actor failed to start
    pub async fn spawn_local_linked<T: ThreadLocalActor>(
        &self,
        name: Option<String>,
        startup_args: T::Arguments,
        spawner: ThreadLocalActorSpawner,
    ) -> Result<(ActorRef<T::Msg>, JoinHandle<()>), SpawnErr> {
        T::spawn_linked(name, startup_args, self.clone(), spawner).await
    }
}
