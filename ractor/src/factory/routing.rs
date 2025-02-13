// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Routing protocols for Factories

use std::collections::HashMap;
use std::marker::PhantomData;

use crate::factory::worker::WorkerProperties;
use crate::factory::Job;
use crate::factory::JobKey;
use crate::factory::WorkerId;
use crate::ActorProcessingErr;
use crate::Message;

/// Custom hashing behavior for factory routing to workers
pub trait CustomHashFunction<TKey>: Send + Sync
where
    TKey: Send + Sync + 'static,
{
    /// Hash the key into the space 0..usize
    fn hash(&self, key: &TKey, worker_count: usize) -> usize;
}

/// The possible results from a routing operation.
#[derive(Debug)]
pub enum RouteResult<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    /// The job has been handled and routed successfully
    Handled,
    /// The job needs to be backlogged into the internal factory's queue (if
    /// configured)
    Backlog(Job<TKey, TMsg>),
    /// The job has exceeded the internal rate limit specification of the router.
    /// This would be returned as a route operation in the event that the router is
    /// tracking the jobs-per-unit-time and has decided that routing this next job
    /// would exceed that limit.
    ///
    /// Returns the job that was rejected
    RateLimited(Job<TKey, TMsg>),
}

/// A routing mode controls how a request is routed from the factory to a
/// designated worker
pub trait Router<TKey, TMsg>: Send + Sync + 'static
where
    TKey: JobKey,
    TMsg: Message,
{
    /// Route a [Job] based on the specific routing methodology
    ///
    /// * `job` - The job to be routed
    /// * `pool_size` - The size of the ACTIVE worker pool (excluding draining workers)
    /// * `worker_hint` - If provided, this is a "hint" at which worker should receive the job,
    ///   if available.
    /// * `worker_pool` - The current worker pool, which may contain draining workers
    ///
    /// Returns [RouteResult::Handled] if the job was routed successfully, otherwise
    /// [RouteResult::Backlog] is returned indicating that the job should be enqueued in
    /// the factory's internal queue.
    fn route_message(
        &mut self,
        job: Job<TKey, TMsg>,
        pool_size: usize,
        worker_hint: Option<WorkerId>,
        worker_pool: &mut HashMap<WorkerId, WorkerProperties<TKey, TMsg>>,
    ) -> Result<RouteResult<TKey, TMsg>, ActorProcessingErr>;

    /// Identifies if a job CAN be routed, and to which worker, without
    /// requiring dequeueing the job
    ///
    /// This prevents the need to support pushing jobs that have been dequeued,
    /// but no worker is available to accept the job, back into the front of the
    /// queue. And given the single-threaded nature of a Factory, this is safe
    /// to call outside of a locked context. It is assumed that if this returns
    /// [Some(WorkerId)], then the job is guaranteed to be routed, as internal state to
    /// the router may be updated.
    ///
    ///  * `job` - A reference to the job to be routed
    /// * `pool_size` - The size of the ACTIVE worker pool (excluding draining workers)
    /// * `worker_hint` - If provided, this is a "hint" at which worker should receive the job,
    ///   if available.
    /// * `worker_pool` - The current worker pool, which may contain draining workers
    ///
    /// Returns [None] if no worker can be identified or no worker is avaialble to accept
    /// the job, otherwise [Some(WorkerId)] indicating the target worker is returned
    fn choose_target_worker(
        &mut self,
        job: &Job<TKey, TMsg>,
        pool_size: usize,
        worker_hint: Option<WorkerId>,
        worker_pool: &HashMap<WorkerId, WorkerProperties<TKey, TMsg>>,
    ) -> Option<WorkerId>;

    /// Returns a flag indicating if the factory does discard/overload management ([true])
    /// or if is handled by the workers worker(s) ([false])
    fn is_factory_queueing(&self) -> bool;
}

// ============================ Macros ======================= //
macro_rules! impl_routing_mode {
    ($routing_mode: ident, $doc:expr) => {
        #[doc = $doc]
        #[derive(Debug)]
        pub struct $routing_mode<TKey, TMsg>
        where
            TKey: JobKey,
            TMsg: Message,
        {
            _key: PhantomData<fn() -> TKey>,
            _msg: PhantomData<fn() -> TMsg>,
        }

        impl<TKey, TMsg> Default for $routing_mode<TKey, TMsg>
        where
            TKey: JobKey,
            TMsg: Message,
        {
            fn default() -> Self {
                Self {
                    _key: PhantomData,
                    _msg: PhantomData,
                }
            }
        }
    };
}

// ============================ Key Persistent routing ======================= //
impl_routing_mode! {KeyPersistentRouting, "Factory will select worker by hashing the job's key.
Workers will have jobs placed into their incoming message queue's"}

impl<TKey, TMsg> Router<TKey, TMsg> for KeyPersistentRouting<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    fn route_message(
        &mut self,
        job: Job<TKey, TMsg>,
        pool_size: usize,
        worker_hint: Option<WorkerId>,
        worker_pool: &mut HashMap<WorkerId, WorkerProperties<TKey, TMsg>>,
    ) -> Result<RouteResult<TKey, TMsg>, ActorProcessingErr> {
        if let Some(worker) = self
            .choose_target_worker(&job, pool_size, worker_hint, worker_pool)
            .and_then(|wid| worker_pool.get_mut(&wid))
        {
            worker.enqueue_job(job)?;
        }
        Ok(RouteResult::Handled)
    }

    fn choose_target_worker(
        &mut self,
        job: &Job<TKey, TMsg>,
        pool_size: usize,
        worker_hint: Option<WorkerId>,
        _worker_pool: &HashMap<WorkerId, WorkerProperties<TKey, TMsg>>,
    ) -> Option<WorkerId> {
        let key =
            worker_hint.unwrap_or_else(|| crate::factory::hash::hash_with_max(&job.key, pool_size));
        Some(key)
    }

    fn is_factory_queueing(&self) -> bool {
        false
    }
}

// ============================ Queuer routing ======================= //
impl_routing_mode! {QueuerRouting, "Factory will dispatch job to first available worker.
Factory will maintain shared internal queue of messages"}

impl<TKey, TMsg> Router<TKey, TMsg> for QueuerRouting<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    fn route_message(
        &mut self,
        job: Job<TKey, TMsg>,
        pool_size: usize,
        worker_hint: Option<WorkerId>,
        worker_pool: &mut HashMap<WorkerId, WorkerProperties<TKey, TMsg>>,
    ) -> Result<RouteResult<TKey, TMsg>, ActorProcessingErr> {
        if let Some(worker) = self
            .choose_target_worker(&job, pool_size, worker_hint, worker_pool)
            .and_then(|wid| worker_pool.get_mut(&wid))
        {
            worker.enqueue_job(job)?;
            Ok(RouteResult::Handled)
        } else {
            Ok(RouteResult::Backlog(job))
        }
    }

    fn choose_target_worker(
        &mut self,
        _job: &Job<TKey, TMsg>,
        _pool_size: usize,
        worker_hint: Option<WorkerId>,
        worker_pool: &HashMap<WorkerId, WorkerProperties<TKey, TMsg>>,
    ) -> Option<WorkerId> {
        if let Some(worker) = worker_hint.and_then(|worker| worker_pool.get(&worker)) {
            if worker.is_available() {
                return worker_hint;
            }
        }
        worker_pool
            .iter()
            .find(|(_, worker)| worker.is_available())
            .map(|(wid, _)| *wid)
    }

    fn is_factory_queueing(&self) -> bool {
        true
    }
}

// ============================ Sticky Queuer routing ======================= //
impl_routing_mode! {StickyQueuerRouting, "Factory will dispatch jobs to a worker that is processing the same key (if any).
Factory will maintain shared internal queue of messages.

Note: This is helpful for sharded db access style scenarios. If a worker is
currently doing something on a given row id for example, we want subsequent updates
to land on the same worker so it can serialize updates to the same row consistently."}

impl<TKey, TMsg> Router<TKey, TMsg> for StickyQueuerRouting<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    fn route_message(
        &mut self,
        job: Job<TKey, TMsg>,
        pool_size: usize,
        worker_hint: Option<WorkerId>,
        worker_pool: &mut HashMap<WorkerId, WorkerProperties<TKey, TMsg>>,
    ) -> Result<RouteResult<TKey, TMsg>, ActorProcessingErr> {
        if let Some(worker) = self
            .choose_target_worker(&job, pool_size, worker_hint, worker_pool)
            .and_then(|wid| worker_pool.get_mut(&wid))
        {
            worker.enqueue_job(job)?;
            Ok(RouteResult::Handled)
        } else {
            Ok(RouteResult::Backlog(job))
        }
    }

    fn choose_target_worker(
        &mut self,
        job: &Job<TKey, TMsg>,
        _pool_size: usize,
        worker_hint: Option<WorkerId>,
        worker_pool: &HashMap<WorkerId, WorkerProperties<TKey, TMsg>>,
    ) -> Option<WorkerId> {
        // check sticky first
        if let Some(worker) = worker_hint.and_then(|worker| worker_pool.get(&worker)) {
            if worker.is_processing_key(&job.key) {
                return worker_hint;
            }
        }

        let maybe_worker = worker_pool
            .iter()
            .find(|(_, worker)| worker.is_processing_key(&job.key))
            .map(|(a, _)| *a);
        if maybe_worker.is_some() {
            return maybe_worker;
        }

        // now take first available, based on hint then brute-search
        if let Some(worker) = worker_hint.and_then(|worker| worker_pool.get(&worker)) {
            if worker.is_available() {
                return worker_hint;
            }
        }

        // fallback to first free worker as there's no sticky worker
        worker_pool
            .iter()
            .find(|(_, worker)| worker.is_available())
            .map(|(wid, _)| *wid)
    }

    fn is_factory_queueing(&self) -> bool {
        true
    }
}

// ============================ Round-robin routing ======================= //
/// Factory will dispatch to the next worker in order.
///
/// Workers will have jobs placed into their incoming message queue's
#[derive(Debug)]
pub struct RoundRobinRouting<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    _key: PhantomData<fn() -> TKey>,
    _msg: PhantomData<fn() -> TMsg>,
    last_worker: WorkerId,
}

impl<TKey, TMsg> Default for RoundRobinRouting<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    fn default() -> Self {
        Self {
            _key: PhantomData,
            _msg: PhantomData,
            last_worker: 0,
        }
    }
}

impl<TKey, TMsg> Router<TKey, TMsg> for RoundRobinRouting<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    fn route_message(
        &mut self,
        job: Job<TKey, TMsg>,
        pool_size: usize,
        worker_hint: Option<WorkerId>,
        worker_pool: &mut HashMap<WorkerId, WorkerProperties<TKey, TMsg>>,
    ) -> Result<RouteResult<TKey, TMsg>, ActorProcessingErr> {
        if let Some(worker) = self
            .choose_target_worker(&job, pool_size, worker_hint, worker_pool)
            .and_then(|wid| worker_pool.get_mut(&wid))
        {
            worker.enqueue_job(job)?;
        }
        Ok(RouteResult::Handled)
    }

    fn choose_target_worker(
        &mut self,
        _job: &Job<TKey, TMsg>,
        pool_size: usize,
        worker_hint: Option<WorkerId>,
        worker_pool: &HashMap<WorkerId, WorkerProperties<TKey, TMsg>>,
    ) -> Option<WorkerId> {
        if let Some(worker) = worker_hint.and_then(|worker| worker_pool.get(&worker)) {
            if worker.is_available() {
                return worker_hint;
            }
        }

        let mut key = self.last_worker + 1;
        if key >= pool_size {
            key = 0;
        }
        self.last_worker = key;
        Some(key)
    }

    fn is_factory_queueing(&self) -> bool {
        false
    }
}

// ============================ Custom routing ======================= //
/// Factory will dispatch to workers based on a custom hash function.
///
/// The factory maintains no queue in this scenario, and jobs are pushed
/// to worker's queues.
#[derive(Debug)]
pub struct CustomRouting<TKey, TMsg, THasher>
where
    TKey: JobKey,
    TMsg: Message,
    THasher: CustomHashFunction<TKey>,
{
    _key: PhantomData<fn() -> TKey>,
    _msg: PhantomData<fn() -> TMsg>,
    hasher: THasher,
}

impl<TKey, TMsg, THasher> CustomRouting<TKey, TMsg, THasher>
where
    TKey: JobKey,
    TMsg: Message,
    THasher: CustomHashFunction<TKey>,
{
    /// Construct a new [CustomRouting] instance with the supplied hash function
    pub fn new(hasher: THasher) -> Self {
        Self {
            _key: PhantomData,
            _msg: PhantomData,
            hasher,
        }
    }
}

impl<TKey, TMsg, THasher> Router<TKey, TMsg> for CustomRouting<TKey, TMsg, THasher>
where
    TKey: JobKey,
    TMsg: Message,
    THasher: CustomHashFunction<TKey> + 'static,
{
    fn route_message(
        &mut self,
        job: Job<TKey, TMsg>,
        pool_size: usize,
        worker_hint: Option<WorkerId>,
        worker_pool: &mut HashMap<WorkerId, WorkerProperties<TKey, TMsg>>,
    ) -> Result<RouteResult<TKey, TMsg>, ActorProcessingErr> {
        if let Some(worker) = self
            .choose_target_worker(&job, pool_size, worker_hint, worker_pool)
            .and_then(|wid| worker_pool.get_mut(&wid))
        {
            worker.enqueue_job(job)?;
        }
        Ok(RouteResult::Handled)
    }

    fn choose_target_worker(
        &mut self,
        job: &Job<TKey, TMsg>,
        pool_size: usize,
        _worker_hint: Option<WorkerId>,
        _worker_pool: &HashMap<WorkerId, WorkerProperties<TKey, TMsg>>,
    ) -> Option<WorkerId> {
        let key = self.hasher.hash(&job.key, pool_size);
        Some(key)
    }

    fn is_factory_queueing(&self) -> bool {
        false
    }
}
