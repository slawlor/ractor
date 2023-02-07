// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! TBFilled out. Representative gen_factory implementation
//!
//! A factory is a supervisor/manager to a pool of workers on the same node. This
//! is helpful for job dispatch
//!
//! TODO:
//! * Batch job processing (incl splitting batches, etc)
//! * Factor stats
//!

use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::marker::PhantomData;

use rand::Rng;

use crate::{Actor, ActorId, ActorProcessingErr, ActorRef, Message, SupervisionEvent};

mod hash;

pub mod worker;
pub use worker::{WorkerProperties, WorkerStartContext};

pub mod job;
pub mod routing_mode;

pub use job::{Job, JobOptions};
pub use routing_mode::{CustomHashFunction, RoutingMode};

#[cfg(test)]
mod tests;

/// Trait defining the discard handler for a factory.
pub trait DiscardHandler<TKey, TMsg>: Send + Sync + 'static
where
    TKey: Message + Hash + Sync,
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
    fn build(&self, wid: usize) -> TWorker;
}

/// A factory is a manager to a pool of worker actors used for job dispatching
pub struct Factory<TKey, TMsg, TWorker>
where
    TKey: Message + Hash + Sync,
    TMsg: Message,
    TWorker: Actor<Msg = Job<TKey, TMsg>>,
{
    /// Number of workers in the factory
    pub worker_count: usize,

    /// Message routing mode
    pub routing_mode: RoutingMode<TKey>,

    /// Maximum queue length. Any job arriving when the queue is at its max length
    /// will cause an oldest job at the head of the queue will be dropped.
    ///
    /// * For [RoutingMode::Queuer] factories, applied to the factory's internal queue.
    /// * For [RoutingMode::KeyPersistent] and [RoutingMode::CustomHashFunction], this applies to the worker's
    /// message queue
    ///
    /// Default is disabled
    pub discard_threshold: Option<usize>,

    /// Discard callback when a job is discarded
    pub discard_handler: Option<Box<dyn DiscardHandler<TKey, TMsg>>>,

    /// A worker's ability for parallelized work
    ///
    /// Default is 1 (worker is strictly sequential in dispatched work)
    pub worker_parallel_capacity: usize,

    _worker: PhantomData<TWorker>,
}

impl<TKey, TMsg, TWorker> Factory<TKey, TMsg, TWorker>
where
    TKey: Message + Hash + Sync,
    TMsg: Message,
    TWorker: Actor<Msg = Job<TKey, TMsg>>,
{
    fn maybe_discard(&self, state: &mut FactoryState<TKey, TMsg, TWorker>) {
        if let Some(threshold) = self.discard_threshold {
            if state.messages.len() > threshold {
                if let Some(msg) = state.messages.pop_front() {
                    if let Some(handler) = &self.discard_handler {
                        handler.discard(msg);
                    }
                }
            }
        }
    }
}

impl<TKey, TMsg, TWorker> Default for Factory<TKey, TMsg, TWorker>
where
    TKey: Message + Hash + Sync,
    TMsg: Message,
    TWorker: Actor<Msg = Job<TKey, TMsg>>,
{
    fn default() -> Self {
        Self {
            worker_count: 1,
            routing_mode: RoutingMode::default(),
            discard_threshold: None,
            discard_handler: None,
            _worker: PhantomData,
            worker_parallel_capacity: 1,
        }
    }
}

/// State of a factory (backlogged jobs, handler, etc)
pub struct FactoryState<TKey, TMsg, TWorker>
where
    TKey: Message + Hash + Sync,
    TMsg: Message,
    TWorker: Actor<Msg = Job<TKey, TMsg>>,
{
    worker_builder: Box<dyn WorkerBuilder<TWorker>>,
    pool: HashMap<usize, WorkerProperties<TKey, TMsg, TWorker>>,

    messages: VecDeque<Job<TKey, TMsg>>,
    last_worker: usize,
}

/// Messages to a factory
pub enum FactoryMessage<TKey, TMsg>
where
    TKey: Message + Hash + Sync,
    TMsg: Message,
{
    /// Dispatch a new message
    Dispatch(Job<TKey, TMsg>),

    /// A job finished
    Finished(ActorId, TKey),
}
#[cfg(feature = "cluster")]
impl<TKey, TMsg> Message for FactoryMessage<TKey, TMsg>
where
    TKey: Message + Hash + Sync,
    TMsg: Message,
{
}

#[async_trait::async_trait]
impl<TKey, TMsg, TWorker> Actor for Factory<TKey, TMsg, TWorker>
where
    TKey: Message + Hash + Sync,
    TMsg: Message,
    TWorker: Actor<Msg = Job<TKey, TMsg>, Arguments = WorkerStartContext<TKey, TMsg, TWorker>>,
{
    type Msg = FactoryMessage<TKey, TMsg>;
    type State = FactoryState<TKey, TMsg, TWorker>;
    type Arguments = Box<dyn WorkerBuilder<TWorker>>;

    async fn pre_start(
        &self,
        myself: ActorRef<Self>,
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
            let (worker, _) =
                Actor::spawn_linked(None, handler, context, myself.get_cell()).await?;
            pool.insert(
                wid,
                WorkerProperties::new(
                    wid,
                    worker,
                    self.worker_parallel_capacity,
                    self.discard_threshold,
                    self.discard_handler.as_ref().map(|a| a.clone_box()),
                ),
            );
        }

        // initial state
        Ok(Self::State {
            messages: VecDeque::new(),
            worker_builder: builder,
            pool,
            last_worker: 0,
        })
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self>,
        message: FactoryMessage<TKey, TMsg>,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        // TODO: based on the routing spec, dispatch the message
        match message {
            FactoryMessage::Dispatch(mut job) => {
                // set the time the factory received the message
                job.set_factory_time();

                match &self.routing_mode {
                    RoutingMode::KeyPersistent => {
                        let key = hash::hash_with_max(&job.key, self.worker_count);
                        if let Some(worker) = state.pool.get_mut(&key) {
                            worker.enqueue_job(job)?;
                        }
                    }
                    RoutingMode::Queuer => {
                        let maybe_worker =
                            state.pool.values_mut().find(|worker| worker.is_available());
                        if let Some(worker) = maybe_worker {
                            worker.enqueue_job(job)?;
                        } else {
                            // no free worker, backlog the message
                            state.messages.push_back(job);
                            self.maybe_discard(state);
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
                if let Some(worker) = state.pool.values_mut().find(|v| v.is_pid(who)) {
                    worker.worker_complete(job_key)?;
                }

                if let RoutingMode::Queuer = self.routing_mode {
                    // de-queue another job and send it to the free worker
                    if let Some(job) = state.messages.pop_front() {
                        let maybe_worker =
                            state.pool.values_mut().find(|worker| worker.is_available());
                        if let Some(worker) = maybe_worker {
                            worker.enqueue_job(job)?;
                        } else {
                            // no free worker, backlog the message
                            state.messages.push_front(job);
                            self.maybe_discard(state);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn handle_supervisor_evt(
        &self,
        myself: ActorRef<Self>,
        message: SupervisionEvent,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            SupervisionEvent::ActorTerminated(who, _, reason) => {
                if let Some(worker) = state
                    .pool
                    .values_mut()
                    .find(|actor| actor.is_pid(who.get_id()))
                {
                    log::warn!(
                        "Factory {:?}'s worker {} terminated with {:?}",
                        myself.get_name(),
                        worker.wid,
                        reason
                    );
                    let new_worker = state.worker_builder.build(worker.wid);
                    let spec = WorkerStartContext {
                        wid: worker.wid,
                        factory: myself.clone(),
                    };
                    let (replacement, _) =
                        Actor::spawn_linked(None, new_worker, spec, myself.get_cell()).await?;

                    worker.replace_worker(replacement);
                }
            }
            SupervisionEvent::ActorPanicked(who, reason) => {
                if let Some(worker) = state
                    .pool
                    .values_mut()
                    .find(|actor| actor.is_pid(who.get_id()))
                {
                    log::warn!(
                        "Factory {:?}'s worker {} panicked with {}",
                        myself.get_name(),
                        worker.wid,
                        reason
                    );
                    let new_worker = state.worker_builder.build(worker.wid);
                    let spec = WorkerStartContext {
                        wid: worker.wid,
                        factory: myself.clone(),
                    };
                    let (replacement, _) =
                        Actor::spawn_linked(None, new_worker, spec, myself.get_cell()).await?;

                    worker.replace_worker(replacement);
                }
            }
            _ => {}
        }
        Ok(())
    }
}
