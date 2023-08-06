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

use std::collections::{HashMap, VecDeque};
use std::marker::PhantomData;

use rand::Rng;

use crate::concurrency::{Duration, Instant};
#[cfg(feature = "cluster")]
use crate::message::BoxedDowncastErr;
use crate::{Actor, ActorProcessingErr, ActorRef, Message, RpcReplyPort, SupervisionEvent};

pub mod hash;
pub mod job;
pub mod routing_mode;
pub mod stats;
pub mod worker;

use stats::MessageProcessingStats;

pub use job::{Job, JobKey, JobOptions};
pub use routing_mode::{CustomHashFunction, RoutingMode};
pub use worker::{WorkerMessage, WorkerProperties, WorkerStartContext};

/// Identifier for a worker in a factory
pub type WorkerId = usize;

#[cfg(not(test))]
const PING_FREQUENCY_MS: u64 = 1000;
#[cfg(test)]
const PING_FREQUENCY_MS: u64 = 100;

#[cfg(test)]
mod tests;

/// Trait defining the discard handler for a factory.
pub trait DiscardHandler<TKey, TMsg>: Send + Sync + 'static
where
    TKey: JobKey,
    TMsg: Message,
{
    /// Called on a job
    fn discard(&self, job: Job<TKey, TMsg>);

    /// clone yourself into a box
    fn clone_box(&self) -> Box<dyn DiscardHandler<TKey, TMsg>>;
}

/// Trait defining a builder of workers for a factory
pub trait WorkerBuilder<TWorker>: Send + Sync
where
    TWorker: Actor,
{
    /// Build a new worker
    ///
    /// * `wid`: The worker's "id" or index in the worker pool
    fn build(&self, wid: WorkerId) -> TWorker;
}

/// The configuration for the dead-man's switch functionality
pub struct DeadMansSwitchConfiguration {
    /// Duration before determining worker is stuck
    pub detection_timeout: Duration,
    /// Flag denoting if the stuck worker should be killed
    /// and restarted
    pub kill_worker: bool,
}

/// A factory is a manager to a pool of worker actors used for job dispatching
pub struct Factory<TKey, TMsg, TWorker>
where
    TKey: JobKey,
    TMsg: Message,
    TWorker: Actor<Msg = WorkerMessage<TKey, TMsg>>,
{
    /// Number of workers in the factory
    pub worker_count: usize,

    /// If [true], tells the factory to collect statistics on the workers.
    ///
    /// Default is [false]
    pub collect_worker_stats: bool,

    /// Message routing mode
    ///
    /// Default is [RoutingMode::KeyPersistent]
    pub routing_mode: RoutingMode<TKey>,

    /// Maximum queue length. Any job arriving when the queue is at its max length
    /// will cause an oldest job at the head of the queue will be dropped.
    ///
    /// * For [RoutingMode::Queuer] factories, applied to the factory's internal queue.
    /// * For [RoutingMode::KeyPersistent] and [RoutingMode::CustomHashFunction], this applies to the worker's
    /// message queue
    ///
    /// Default is [None]
    pub discard_threshold: Option<usize>,

    /// Discard callback when a job is discarded.
    ///
    /// Default is [None]
    pub discard_handler: Option<Box<dyn DiscardHandler<TKey, TMsg>>>,

    /// A worker's ability for parallelized work
    ///
    /// Default is 1 (worker is strictly sequential in dispatched work)
    pub worker_parallel_capacity: usize,

    /// Controls the "dead man's" switching logic on the factory. Periodically
    /// the factory will scan for stuck workers. If detected, the worker information
    /// will be logged along with the current job key information. Optionally the worker
    /// can be killed and replaced by the factory
    pub dead_mans_switch: Option<DeadMansSwitchConfiguration>,

    /// The worker type
    pub _worker: PhantomData<TWorker>,
}

impl<TKey, TMsg, TWorker> Factory<TKey, TMsg, TWorker>
where
    TKey: JobKey,
    TMsg: Message,
    TWorker: Actor<Msg = WorkerMessage<TKey, TMsg>>,
{
    fn maybe_dequeue(
        &self,
        state: &mut FactoryState<TKey, TMsg, TWorker>,
        who: WorkerId,
    ) -> Result<(), ActorProcessingErr> {
        match &self.routing_mode {
            RoutingMode::Queuer | RoutingMode::StickyQueuer => {
                // pop + discard expired jobs
                let mut next_job = None;
                while let Some(job) = state.messages.pop_front() {
                    if !job.is_expired() {
                        next_job = Some(job);
                        break;
                    } else {
                        state.stats.job_ttl_expired();
                    }
                }
                // de-queue another job
                if let Some(job) = next_job {
                    if let Some(worker) = state.pool.get_mut(&who).filter(|f| f.is_available()) {
                        // Check if this worker is now free (should be the case except potentially in a sticky queuer which may automatically
                        // move to a sticky message)
                        worker.enqueue_job(job)?;
                    } else if let Some(worker) =
                        state.pool.values_mut().find(|worker| worker.is_available())
                    {
                        // If that worker is busy, try and scan the workers to find a free worker
                        worker.enqueue_job(job)?;
                    } else {
                        // no free worker, put the message back into the queue where it was (at the front of the queue)
                        state.messages.push_front(job);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn maybe_enqueue(&self, state: &mut FactoryState<TKey, TMsg, TWorker>, job: Job<TKey, TMsg>) {
        if let Some(limit) = self.discard_threshold {
            state.messages.push_back(job);
            while state.messages.len() > limit {
                // try and shed a job
                if let Some(msg) = state.messages.pop_front() {
                    if let Some(handler) = &self.discard_handler {
                        handler.discard(msg);
                    }
                    state.stats.job_discarded();
                }
            }
        } else {
            // no load-shedding
            state.messages.push_back(job);
        }
    }
}

impl<TKey, TMsg, TWorker> Default for Factory<TKey, TMsg, TWorker>
where
    TKey: JobKey,
    TMsg: Message,
    TWorker: Actor<Msg = WorkerMessage<TKey, TMsg>>,
{
    fn default() -> Self {
        Self {
            worker_count: 1,
            routing_mode: RoutingMode::default(),
            discard_threshold: None,
            discard_handler: None,
            _worker: PhantomData,
            worker_parallel_capacity: 1,
            collect_worker_stats: false,
            dead_mans_switch: None,
        }
    }
}

/// State of a factory (backlogged jobs, handler, etc)
pub struct FactoryState<TKey, TMsg, TWorker>
where
    TKey: JobKey,
    TMsg: Message,
    TWorker: Actor<Msg = WorkerMessage<TKey, TMsg>>,
{
    worker_builder: Box<dyn WorkerBuilder<TWorker>>,
    pool: HashMap<WorkerId, WorkerProperties<TKey, TMsg>>,

    messages: VecDeque<Job<TKey, TMsg>>,
    last_worker: WorkerId,
    stats: MessageProcessingStats,
}

impl<TKey, TMsg, TWorker> FactoryState<TKey, TMsg, TWorker>
where
    TKey: JobKey,
    TMsg: Message,
    TWorker: Actor<Msg = WorkerMessage<TKey, TMsg>, Arguments = WorkerStartContext<TKey, TMsg>>,
{
    fn log_stats(&mut self, factory: &ActorRef<FactoryMessage<TKey, TMsg>>) {
        let factory_identifier = if let Some(factory_name) = factory.get_name() {
            format!("======== Factory {factory_name} stats ========\n")
        } else {
            format!("======== Factory ({}) stats ========\n", factory.get_id())
        };

        tracing::debug!("{factory_identifier}\n{}", self.stats);
        self.stats.reset_global_counters();
    }
}

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

    /// A reply to a factory ping supplying the worker id and the time
    /// of the ping start
    WorkerPong(WorkerId, Duration),

    /// Trigger a scan for stuck worker detection
    IdentifyStuckWorkers,

    /// Retrieve the factory's statsistics
    GetStats(RpcReplyPort<MessageProcessingStats>),
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

#[async_trait::async_trait]
impl<TKey, TMsg, TWorker> Actor for Factory<TKey, TMsg, TWorker>
where
    TKey: JobKey,
    TMsg: Message,
    TWorker: Actor<Msg = WorkerMessage<TKey, TMsg>, Arguments = WorkerStartContext<TKey, TMsg>>,
{
    type Msg = FactoryMessage<TKey, TMsg>;
    type State = FactoryState<TKey, TMsg, TWorker>;
    type Arguments = Box<dyn WorkerBuilder<TWorker>>;

    async fn pre_start(
        &self,
        myself: ActorRef<Self::Msg>,
        builder: Box<dyn WorkerBuilder<TWorker>>,
    ) -> Result<Self::State, ActorProcessingErr> {
        // build the pool
        let mut pool = HashMap::new();
        for wid in 0..self.worker_count {
            let context = WorkerStartContext {
                wid,
                factory: myself.clone(),
            };
            let handler = builder.build(wid);
            let (worker, worker_handle) =
                Actor::spawn_linked(None, handler, context, myself.get_cell()).await?;
            pool.insert(
                wid,
                WorkerProperties::new(
                    wid,
                    worker,
                    self.worker_parallel_capacity,
                    self.discard_threshold,
                    self.discard_handler.as_ref().map(|a| a.clone_box()),
                    self.collect_worker_stats,
                    worker_handle,
                ),
            );
        }

        // Startup worker pinging
        myself.send_after(Duration::from_millis(PING_FREQUENCY_MS), || {
            FactoryMessage::DoPings(Instant::now())
        });

        // startup stuck worker detection
        if let Some(dmd) = &self.dead_mans_switch {
            myself.send_after(dmd.detection_timeout, || {
                FactoryMessage::IdentifyStuckWorkers
            });
        }
        let mut stats = MessageProcessingStats::default();
        stats.enable();

        // initial state
        Ok(Self::State {
            messages: VecDeque::new(),
            worker_builder: builder,
            pool,
            last_worker: 0,
            stats,
        })
    }

    async fn post_stop(
        &self,
        _: ActorRef<Self::Msg>,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        // send the stop signal to all workers
        for worker in state.pool.values() {
            worker.actor.stop(None);
        }
        // wait for all workers to exit
        for worker in state.pool.values_mut() {
            if let Some(handle) = worker.get_handle() {
                let _ = handle.await;
            }
        }
        Ok(())
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: FactoryMessage<TKey, TMsg>,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        // TODO: based on the routing spec, dispatch the message
        match message {
            FactoryMessage::Dispatch(mut job) => {
                // set the time the factory received the message
                job.set_factory_time();
                state.stats.job_submitted();

                match &self.routing_mode {
                    RoutingMode::KeyPersistent => {
                        let key = hash::hash_with_max(&job.key, self.worker_count);
                        if let Some(worker) = state.pool.get_mut(&key) {
                            worker.enqueue_job(job)?;
                        }
                    }
                    RoutingMode::Queuer => {
                        if let Some(worker) =
                            state.pool.values_mut().find(|worker| worker.is_available())
                        {
                            worker.enqueue_job(job)?;
                        } else {
                            // no free worker, maybe backlog the message
                            self.maybe_enqueue(state, job);
                        }
                    }
                    RoutingMode::StickyQueuer => {
                        // See if there's a worker processing this given job key already, if yes route to that worker
                        let maybe_worker = state
                            .pool
                            .values_mut()
                            .find(|worker| worker.is_processing_key(&job.key));
                        if let Some(worker) = maybe_worker {
                            worker.enqueue_job(job)?;
                        } else if let Some(worker) =
                            state.pool.values_mut().find(|worker| worker.is_available())
                        {
                            // If no matching sticky worker, find the first free worker
                            worker.enqueue_job(job)?;
                        } else {
                            // no free worker, maybe backlog the message
                            self.maybe_enqueue(state, job);
                        }
                    }
                    RoutingMode::RoundRobin => {
                        let mut key = state.last_worker + 1;
                        if key >= self.worker_count {
                            key = 0;
                        }
                        if let Some(worker) = state.pool.get_mut(&key) {
                            worker.enqueue_job(job)?;
                        }
                        state.last_worker = key;
                    }
                    RoutingMode::Random => {
                        let key = rand::thread_rng().gen_range(0..self.worker_count);
                        if let Some(worker) = state.pool.get_mut(&key) {
                            worker.enqueue_job(job)?;
                        }
                    }
                    RoutingMode::CustomHashFunction(hasher) => {
                        let key = hasher.hash(&job.key, self.worker_count);
                        if let Some(worker) = state.pool.get_mut(&key) {
                            worker.enqueue_job(job)?;
                        }
                    }
                }
            }
            FactoryMessage::Finished(who, job_key) => {
                let wid = if let Some(worker) = state.pool.get_mut(&who) {
                    if let Some(job_options) = worker.worker_complete(job_key)? {
                        // record the job data on the factory
                        state.stats.factory_job_done(&job_options);
                    }
                    Some(worker.wid)
                } else {
                    None
                };
                if let Some(wid) = wid {
                    self.maybe_dequeue(state, wid)?;
                }
            }
            FactoryMessage::WorkerPong(wid, time) => {
                if let Some(worker) = state.pool.get_mut(&wid) {
                    worker.ping_received(time);
                }
            }
            FactoryMessage::DoPings(when) => {
                if state.stats.ping_received(when.elapsed()) {
                    state.log_stats(&myself);
                }

                for worker in state.pool.values_mut() {
                    worker.send_factory_ping()?;
                }

                // schedule next ping
                myself.send_after(Duration::from_millis(PING_FREQUENCY_MS), || {
                    FactoryMessage::DoPings(Instant::now())
                });
            }
            FactoryMessage::IdentifyStuckWorkers => {
                if let Some(dmd) = &self.dead_mans_switch {
                    for worker in state.pool.values() {
                        if worker.is_stuck(dmd.detection_timeout) && dmd.kill_worker {
                            tracing::info!(
                                "Factory {:?} killing stuck worker {}",
                                myself.get_name(),
                                worker.wid
                            );
                            worker.actor.kill();
                        }
                    }

                    // schedule next check
                    myself.send_after(dmd.detection_timeout, || {
                        FactoryMessage::IdentifyStuckWorkers
                    });
                }
            }
            FactoryMessage::GetStats(reply) => {
                let _ = reply.send(state.stats.clone());
            }
        }

        Ok(())
    }

    async fn handle_supervisor_evt(
        &self,
        myself: ActorRef<Self::Msg>,
        message: SupervisionEvent,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            SupervisionEvent::ActorTerminated(who, _, reason) => {
                let wid = if let Some(worker) = state
                    .pool
                    .values_mut()
                    .find(|actor| actor.is_pid(who.get_id()))
                {
                    tracing::warn!(
                        "Factory {:?}'s worker {} terminated with {reason:?}",
                        myself.get_name(),
                        worker.wid
                    );
                    let new_worker = state.worker_builder.build(worker.wid);
                    let spec = WorkerStartContext {
                        wid: worker.wid,
                        factory: myself.clone(),
                    };
                    let (replacement, replacement_handle) =
                        Actor::spawn_linked(None, new_worker, spec, myself.get_cell()).await?;

                    worker.replace_worker(replacement, replacement_handle)?;
                    Some(worker.wid)
                } else {
                    None
                };
                if let Some(wid) = wid {
                    self.maybe_dequeue(state, wid)?;
                }
            }
            SupervisionEvent::ActorPanicked(who, reason) => {
                let wid = if let Some(worker) = state
                    .pool
                    .values_mut()
                    .find(|actor| actor.is_pid(who.get_id()))
                {
                    tracing::warn!(
                        "Factory {:?}'s worker {} panicked with {reason}",
                        myself.get_name(),
                        worker.wid
                    );
                    let new_worker = state.worker_builder.build(worker.wid);
                    let spec = WorkerStartContext {
                        wid: worker.wid,
                        factory: myself.clone(),
                    };
                    let (replacement, replacement_handle) =
                        Actor::spawn_linked(None, new_worker, spec, myself.get_cell()).await?;

                    worker.replace_worker(replacement, replacement_handle)?;
                    Some(worker.wid)
                } else {
                    None
                };
                if let Some(wid) = wid {
                    self.maybe_dequeue(state, wid)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
