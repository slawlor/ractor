// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! # Factory actors
//!
//! A factory is a manager of a pool of workers on the same node. This
//! is helpful for job dispatch and load balancing when single-threaded execution
//! of a since [crate::Actor] may not be sufficient. Factories have a set "Job" syntax
//! which denotes a key and message payload for each action. Workers are effectively mindless
//! agents of the factory's will.
//!
//! ## Worker message routing mode
//!  
//! The factory has a series of dispatch modes which are defined in [RoutingMode] and
//! control the way the factory dispatches work to workers. This should be selected based
//! on the intended workload. Some general guidance:
//!
//! 1. If you need to process a sequence of operations on a given key (i.e. the Job is a user, and
//! there's a sequential list of updates to that user). You then want the job to land on the same
//! worker and should select [RoutingMode::KeyPersistent] or [RoutingMode::StickyQueuer].
//! 2. If you don't need a sequence of operations then [RoutingMode::Queuer] is likely a good choice.
//! 3. If your workers are making remote calls to other services/actors you probably want [RoutingMode::Queuer]
//! or [RoutingMode::StickyQueuer] to prevent head-of-the-line contention. Otherwise [RoutingMode::KeyPersistent]
//! is sufficient.
//! 4. For some custom defined routing, you can define your own [CustomHashFunction] which will be
//! used in conjunction with [RoutingMode::CustomHashFunction] to take the incoming job key and
//! the space which should be hashed to (i.e. the number of workers).
//! 5. If you just want load balancing there's also [RoutingMode::RoundRobin] and [RoutingMode::Random]
//! for general 1-off dispatching of jobs
//!
//! ## Worker lifecycle
//!
//! A worker's lifecycle is managed by the factory. If the worker dies or crashes, the factory will
//! replace the worker with a new instance and continue processing jobs for that worker. The
//! factory also maintains the worker's message queue's so messages won't be lost which were in the
//! "worker"'s queue.
#![cfg_attr(
    not(feature = "cluster"),
    doc = "
## Example Factory
```rust
use ractor::concurrency::Duration;
use ractor::factory::*;
use ractor::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort};
#[derive(Debug)]
enum ExampleMessage {
    PrintValue(u64),
    EchoValue(u64, RpcReplyPort<u64>),
}
/// The worker's specification for the factory. This defines
/// the business logic for each message that will be done in parallel.
struct ExampleWorker;
#[cfg_attr(feature = \"async-trait\", ractor::async_trait)]
impl Actor for ExampleWorker {
    type Msg = WorkerMessage<(), ExampleMessage>;
    type State = WorkerStartContext<(), ExampleMessage, ()>;
    type Arguments = WorkerStartContext<(), ExampleMessage, ()>;
    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        startup_context: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(startup_context)
    }
    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            WorkerMessage::FactoryPing(time) => {
                // This is a message which all factory workers **must**
                // adhere to. It is a background processing message from the
                // factory which is used for (a) metrics and (b) detecting
                // stuck workers, i.e. workers which aren't making progress
                // processing their messages
                state
                    .factory
                    .cast(FactoryMessage::WorkerPong(state.wid, time.elapsed()))?;
            }
            WorkerMessage::Dispatch(job) => {
                // Actual business logic that we want to parallelize
                tracing::trace!(\"Worker {} received {:?}\", state.wid, job.msg);
                match job.msg {
                    ExampleMessage::PrintValue(value) => {
                        tracing::info!(\"Worker {} printing value {value}\", state.wid);
                    }
                    ExampleMessage::EchoValue(value, reply) => {
                        tracing::info!(\"Worker {} echoing value {value}\", state.wid);
                        let _ = reply.send(value);
                    }
                }
                // job finished, on success or err we report back to the factory
                state
                    .factory
                    .cast(FactoryMessage::Finished(state.wid, job.key))?;
            }
        }
        Ok(())
    }
}
/// Used by the factory to build new [ExampleWorker]s.
struct ExampleWorkerBuilder;
impl WorkerBuilder<ExampleWorker, ()> for ExampleWorkerBuilder {
    fn build(&self, _wid: usize) -> (ExampleWorker, ()) {
        (ExampleWorker, ())
    }
}
#[tokio::main]
async fn main() {
    let factory_def = Factory::<
        (), 
        ExampleMessage, 
        (), 
        ExampleWorker, 
        routing::QueuerRouting<(), ExampleMessage>,
        queues::DefaultQueue<(), ExampleMessage>
    >::default();
    let factory_args = FactoryArgumentsBuilder::new(ExampleWorkerBuilder, Default::default(), Default::default())
        .with_number_of_initial_workers(5)
        .build();
    
    let (factory, handle) = Actor::spawn(None, factory_def, factory_args)
        .await
        .expect(\"Failed to startup factory\");
    for i in 0..99 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                key: (),
                msg: ExampleMessage::PrintValue(i),
                options: JobOptions::default(),
            }))
            .expect(\"Failed to send to factory\");
    }
    let reply = factory
        .call(
            |prt| {
                FactoryMessage::Dispatch(Job {
                    key: (),
                    msg: ExampleMessage::EchoValue(123, prt),
                    options: JobOptions::default(),
                })
            },
            None,
        )
        .await
        .expect(\"Failed to send to factory\")
        .expect(\"Failed to parse reply\");
    assert_eq!(reply, 123);
    factory.stop(None);
    handle.await.unwrap();
}
```
"
)]

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
pub mod routing;
pub mod stats;
pub mod worker;

#[cfg(test)]
mod tests;

use stats::MessageProcessingStats;

pub use discard::{
    DiscardHandler, DiscardMode, DiscardReason, DiscardSettings, DynamicDiscardController,
};
pub use factoryimpl::{Factory, FactoryArguments, FactoryArgumentsBuilder};
pub use job::{Job, JobKey, JobOptions};
pub use lifecycle::FactoryLifecycleHooks;
pub use worker::{
    DeadMansSwitchConfiguration, WorkerBuilder, WorkerCapacityController, WorkerMessage,
    WorkerProperties, WorkerStartContext,
};

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

    /// Notify the factory that it's being drained, and to finish jobs
    /// currently in the queue, but discard new work, and once drained
    /// exit
    DrainRequests,
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
