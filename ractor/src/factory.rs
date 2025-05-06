// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! # Factory actors
//!
//! A factory is a manager of a pool of workers on the same node. This
//! is helpful for job dispatch and load balancing when single-threaded execution
//! of a single [crate::Actor] may not be sufficient. Factories have a set "Job" syntax
//! which denotes a key and message payload for each action. Workers are effectively mindless
//! agents of the factory's will.
//!
//! ## Worker message routing mode
//!  
//! The factory has a series of dispatch modes which are defined in the [routing] module and
//! control the way the factory dispatches work to workers. This should be selected based
//! on the intended workload. Some general guidance:
//!
//! 1. If you need to process a sequence of operations on a given key (i.e. the Job is a user, and
//!    there's a sequential list of updates to that user). You then want the job to land on the same
//!    worker and should select [routing::KeyPersistentRouting] or [routing::StickyQueuerRouting].
//! 2. If you don't need a sequence of operations then [routing::QueuerRouting] is likely a good choice.
//! 3. If your workers are making remote calls to other services/actors you probably want [routing::QueuerRouting]
//!    or [routing::StickyQueuerRouting] to prevent head-of-the-line contention. Otherwise [routing::KeyPersistentRouting]
//!    is sufficient.
//! 4. For some custom defined routing, you can define your own [routing::CustomHashFunction] which will be
//!    used in conjunction with [routing::CustomRouting] to take the incoming job key and
//!    the space which should be hashed to (i.e. the number of workers).
//! 5. If you just want load balancing there's also [routing::RoundRobinRouting] for general 1-off
//!    dispatching of jobs
//!
//! ## Factory queueing
//!
//! The factory can also support factory-side or worker-side queueing of extra work messages based on the definition
//! of the [routing::Router] and [queues::Queue] assigned to the factory.
//!
//! Supported queueing protocols today for factory-side queueing is
//!
//! 1. Default, no-priority, queueing: [queues::DefaultQueue]
//! 2. Priority-based queuing, based on a constant number of priorities [queues::PriorityQueue]
//!
//! ## Worker lifecycle
//!
//! A worker's lifecycle is managed by the factory. If the worker dies or crashes, the factory will
//! replace the worker with a new instance and continue processing jobs for that worker. The
//! factory also maintains the worker's message queue's so messages won't be lost which were in the
//! "worker"'s queue.
//!
//! ## Example Factory
//! ```rust
//! use ractor::concurrency::Duration;
//! use ractor::factory::*;
//! use ractor::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort};
//!
//! #[derive(Debug)]
//! enum ExampleMessage {
//!     PrintValue(u64),
//!     EchoValue(u64, RpcReplyPort<u64>),
//! }
//!
//! #[cfg(feature = "cluster")]
//! impl ractor::Message for ExampleMessage {}
//!
//! /// The worker's specification for the factory. This defines
//! /// the business logic for each message that will be done in parallel.
//! struct ExampleWorker;
//! #[cfg_attr(feature = "async-trait", ractor::async_trait)]
//! impl Worker for ExampleWorker {
//!     type Key = ();
//!     type Message = ExampleMessage;
//!     type State = ();
//!     type Arguments = ();
//!     async fn pre_start(
//!         &self,
//!         wid: WorkerId,
//!         factory: &ActorRef<FactoryMessage<(), ExampleMessage>>,
//!         startup_context: Self::Arguments,
//!     ) -> Result<Self::State, ActorProcessingErr> {
//!         Ok(startup_context)
//!     }
//!     async fn handle(
//!         &self,
//!         wid: WorkerId,
//!         factory: &ActorRef<FactoryMessage<(), ExampleMessage>>,
//!         Job { msg, key, .. }: Job<(), ExampleMessage>,
//!         _state: &mut Self::State,
//!     ) -> Result<(), ActorProcessingErr> {
//!         // Actual business logic that we want to parallelize
//!         tracing::trace!("Worker {} received {:?}", wid, msg);
//!         match msg {
//!             ExampleMessage::PrintValue(value) => {
//!                 tracing::info!("Worker {} printing value {value}", wid);
//!             }
//!             ExampleMessage::EchoValue(value, reply) => {
//!                 tracing::info!("Worker {} echoing value {value}", wid);
//!                 let _ = reply.send(value);
//!             }
//!         }
//!         Ok(key)
//!     }
//! }
//! /// Used by the factory to build new [ExampleWorker]s.
//! struct ExampleWorkerBuilder;
//! impl WorkerBuilder<ExampleWorker, ()> for ExampleWorkerBuilder {
//!     fn build(&mut self, _wid: usize) -> (ExampleWorker, ()) {
//!         (ExampleWorker, ())
//!     }
//! }
//! #[tokio::main]
//! async fn main() {
//!     let factory_def = Factory::<
//!         (),
//!         ExampleMessage,
//!         (),
//!         ExampleWorker,
//!         routing::QueuerRouting<(), ExampleMessage>,
//!         queues::DefaultQueue<(), ExampleMessage>,
//!     >::default();
//!     let factory_args = FactoryArguments::builder()
//!         .worker_builder(Box::new(ExampleWorkerBuilder))
//!         .queue(Default::default())
//!         .router(Default::default())
//!         .num_initial_workers(5)
//!         .build();
//!
//!     let (factory, handle) = Actor::spawn(None, factory_def, factory_args)
//!         .await
//!         .expect("Failed to startup factory");
//!     for i in 0..99 {
//!         factory
//!             .cast(FactoryMessage::Dispatch(Job {
//!                 key: (),
//!                 msg: ExampleMessage::PrintValue(i),
//!                 options: JobOptions::default(),
//!                 accepted: None,
//!             }))
//!             .expect("Failed to send to factory");
//!     }
//!     let reply = factory
//!         .call(
//!             |prt| {
//!                 FactoryMessage::Dispatch(Job {
//!                     key: (),
//!                     msg: ExampleMessage::EchoValue(123, prt),
//!                     options: JobOptions::default(),
//!                     accepted: None,
//!                 })
//!             },
//!             None,
//!         )
//!         .await
//!         .expect("Failed to send to factory")
//!         .expect("Failed to parse reply");
//!     assert_eq!(reply, 123);
//!     factory.stop(None);
//!     handle.await.unwrap();
//! }
//! ```

use std::sync::Arc;

use crate::concurrency::{Duration, Instant};
#[cfg(feature = "cluster")]
use crate::message::BoxedDowncastErr;
use crate::{Message, RpcReplyPort};

pub mod discard;
pub mod factoryimpl;
pub mod hash;
pub mod job;
pub mod lifecycle;
pub mod queues;
pub mod ratelim;
pub mod routing;
pub mod stats;
pub mod worker;

#[cfg(test)]
mod tests;

use stats::FactoryStatsLayer;

pub use discard::{
    DiscardHandler, DiscardMode, DiscardReason, DiscardSettings, DynamicDiscardController,
};
pub use factoryimpl::{Factory, FactoryArguments, FactoryArgumentsBuilder};
pub use job::{Job, JobKey, JobOptions, MessageRetryStrategy, RetriableMessage};
pub use lifecycle::FactoryLifecycleHooks;
pub use ratelim::{LeakyBucketRateLimiter, RateLimitedRouter, RateLimiter};
pub use worker::{
    DeadMansSwitchConfiguration, Worker, WorkerBuilder, WorkerCapacityController, WorkerMessage,
    WorkerProperties, WorkerStartContext,
};

/// The settings to change for an update request to the factory at runtime.
///
/// Note: A value of `Some(..)` means that the internal value should be updated
/// inside the factory's state. For values which are originally optional to the factory,
/// we use `Option<Option<T>>`, so if you want to UNSET the value, it would be `Some(None)`.
#[derive(bon::Builder)]
pub struct UpdateSettingsRequest<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    /// The discard handler callback
    pub discard_handler: Option<Option<Arc<dyn DiscardHandler<TKey, TMsg>>>>,
    /// The discard settings
    pub discard_settings: Option<DiscardSettings>,
    /// Dead-man's switch settings
    pub dead_mans_switch: Option<Option<DeadMansSwitchConfiguration>>,
    /// Capacity controller
    pub capacity_controller: Option<Option<Box<dyn WorkerCapacityController>>>,
    /// Lifecycle hooks
    pub lifecycle_hooks: Option<Option<Box<dyn FactoryLifecycleHooks<TKey, TMsg>>>>,
    /// Statistics layer
    pub stats: Option<Option<Arc<dyn FactoryStatsLayer>>>,
    /// The worker count
    pub worker_count: Option<usize>,
}

impl<TKey, TMsg> std::fmt::Debug for UpdateSettingsRequest<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateSettingsRequest")
            .field("set_discard_handler", &self.discard_handler.is_some())
            .field("set_discard_settings", &self.discard_settings.is_some())
            .field("set_dead_mans_switch", &self.dead_mans_switch.is_some())
            .field(
                "set_capacity_controller",
                &self.capacity_controller.is_some(),
            )
            .field("set_lifecycle_hooks", &self.lifecycle_hooks.is_some())
            .field("set_stats", &self.stats.is_some())
            .field("set_worker_count", &self.worker_count.is_some())
            .finish()
    }
}

/// Unique identifier of a disctinct worker in the factory
pub type WorkerId = usize;
/// Messages to a factory.
///
/// **A special note about factory messages in a distributed context!**
///
/// Factories only support the command [FactoryMessage::Dispatch] over a cluster
/// configuration as the rest of the message types are internal and only intended for
/// in-host communication. This means if you're communicating to a factory you would
/// send only a serialized [Job] which would automatically be converted to a
/// [FactoryMessage::Dispatch(Job)]
#[derive(Debug)]
pub enum FactoryMessage<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    /// Dispatch a new message
    Dispatch(Job<TKey, TMsg>),

    /// A job finished
    Finished(WorkerId, TKey),

    /// Send a ping to the workers of the factory along with
    /// tracking the factory's timing itself
    DoPings(Instant),

    /// Calculate factory properties (loadshedding, concurrency, etc)
    Calculate,

    /// A reply to a factory ping supplying the worker id and the time
    /// of the ping start
    WorkerPong(WorkerId, Duration),

    /// Trigger a scan for stuck worker detection
    IdentifyStuckWorkers,

    /// Retrieve the current queue depth (if in a queueing mode)
    GetQueueDepth(RpcReplyPort<usize>),

    /// Resize the worker pool to the requested size
    AdjustWorkerPool(usize),

    /// Retrieve the available capacity of the worker pool + queue
    GetAvailableCapacity(RpcReplyPort<usize>),

    /// Instantaneous measurement of the number of currently processing
    /// requests
    GetNumActiveWorkers(RpcReplyPort<usize>),

    /// Notify the factory that it's being drained, and to finish jobs
    /// currently in the queue, but discard new work, and once drained
    /// exit
    ///
    /// NOTE: This is different from draining the actor itself, which allows the
    /// pending message queue to flush and then exit. Since the factory
    /// holds an internal queue for jobs, it's possible that the internal
    /// state still has work to do while the factory's input queue is drained.
    /// Therefore in order to propertly drain a factory, you should use the
    /// `DrainRequests` version so the internal pending queue is properly flushed.
    DrainRequests,

    /// Dynamically update the factory's settings, for those which don't require strong-type
    /// guarantees. This allows, at runtime, changing the
    ///
    /// * Worker Count
    /// * Discard Settings
    /// * Lifecycle Hooks
    /// * Statistics collection
    /// * Capacity controller
    /// * Dead-man's switch
    /// * Discard handler
    UpdateSettings(UpdateSettingsRequest<TKey, TMsg>),
}

#[cfg(feature = "cluster")]
impl<TKey, TMsg> Message for FactoryMessage<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    fn serializable() -> bool {
        TMsg::serializable()
    }
    fn serialize(
        self,
    ) -> Result<crate::message::SerializedMessage, crate::message::BoxedDowncastErr> {
        match self {
            Self::Dispatch(job) => job.serialize(),
            _ => Err(BoxedDowncastErr),
        }
    }
    fn deserialize(bytes: crate::message::SerializedMessage) -> Result<Self, BoxedDowncastErr> {
        Ok(Self::Dispatch(Job::<TKey, TMsg>::deserialize(bytes)?))
    }
}
