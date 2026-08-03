// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Routing protocols for Factories

use std::collections::HashMap;
use std::collections::VecDeque;
use std::marker::PhantomData;

use crate::factory::worker::WorkerProperties;
use crate::factory::Job;
use crate::factory::JobKey;
use crate::factory::WorkerId;
use crate::ActorProcessingErr;
use crate::Message;
use crate::State;

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
pub trait Router<TKey, TMsg>: State
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

    /// Notification that a worker's availability has changed.
    ///
    /// Called by the factory when a worker transitions between available and busy states.
    /// Routers can use this to maintain an index of available workers for O(1) dispatch.
    ///
    /// * `wid` - The worker id whose availability changed
    /// * `available` - `true` if the worker is now available, `false` if now busy
    fn on_worker_availability_change(&mut self, _wid: WorkerId, _available: bool) {}

    /// Notification of the key a worker is currently processing.
    ///
    /// The factory calls this after completion, replacement, and removal so routers
    /// that index active keys can keep that index synchronized with worker state.
    fn on_worker_key_change(&mut self, _wid: WorkerId, _key: Option<&TKey>) {}
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
            worker
                .enqueue_job(job)
                .map_err(|err| (*err).discard_message())?;
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
/// Factory will dispatch job to first available worker.
/// Factory will maintain shared internal queue of messages
#[derive(Debug)]
pub struct QueuerRouting<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    _key: PhantomData<fn() -> TKey>,
    _msg: PhantomData<fn() -> TMsg>,
    /// FIFO deque of workers believed to be available
    available_workers: VecDeque<WorkerId>,
    /// Indexed by WorkerId — true if the worker is already in `available_workers`
    worker_in_queue: Vec<bool>,
}

impl<TKey, TMsg> Default for QueuerRouting<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    fn default() -> Self {
        Self {
            _key: PhantomData,
            _msg: PhantomData,
            available_workers: VecDeque::new(),
            worker_in_queue: Vec::new(),
        }
    }
}

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
            worker
                .enqueue_job(job)
                .map_err(|err| (*err).discard_message())?;
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
        // Pop from the available-workers deque, skipping stale entries
        while let Some(wid) = self.available_workers.pop_front() {
            if wid < self.worker_in_queue.len() {
                self.worker_in_queue[wid] = false;
            }
            if let Some(worker) = worker_pool.get(&wid) {
                if worker.is_available() {
                    return Some(wid);
                }
            }
            // Worker removed from pool or no longer available — skip
        }
        None
    }

    fn is_factory_queueing(&self) -> bool {
        true
    }

    fn on_worker_availability_change(&mut self, wid: WorkerId, available: bool) {
        // Grow the tracking vec if needed
        if wid >= self.worker_in_queue.len() {
            self.worker_in_queue.resize(wid + 1, false);
        }
        if available {
            if !self.worker_in_queue[wid] {
                self.worker_in_queue[wid] = true;
                self.available_workers.push_back(wid);
            }
        } else {
            // Mark not-in-queue; lazy removal from deque
            self.worker_in_queue[wid] = false;
        }
    }
}

// ============================ Sticky Queuer routing ======================= //
/// Factory will dispatch jobs to a worker that is processing the same key (if any).
/// Factory will maintain shared internal queue of messages.
///
/// Note: This is helpful for sharded db access style scenarios. If a worker is
/// currently doing something on a given row id for example, we want subsequent updates
/// to land on the same worker so it can serialize updates to the same row consistently.
#[derive(Debug)]
pub struct StickyQueuerRouting<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    _key: PhantomData<fn() -> TKey>,
    _msg: PhantomData<fn() -> TMsg>,
    /// FIFO deque of workers believed to be available
    available_workers: VecDeque<WorkerId>,
    /// Indexed by WorkerId — true if the worker is already in `available_workers`
    worker_in_queue: Vec<bool>,
    /// Active worker for each in-flight job key.
    active_workers: HashMap<TKey, WorkerId>,
    /// Reverse index used to remove a worker's active key in constant time.
    worker_keys: Vec<Option<TKey>>,
}

impl<TKey, TMsg> Default for StickyQueuerRouting<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    fn default() -> Self {
        Self {
            _key: PhantomData,
            _msg: PhantomData,
            available_workers: VecDeque::new(),
            worker_in_queue: Vec::new(),
            active_workers: HashMap::new(),
            worker_keys: Vec::new(),
        }
    }
}

impl<TKey, TMsg> StickyQueuerRouting<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    fn update_worker_key(&mut self, wid: WorkerId, key: Option<&TKey>) {
        match key {
            Some(key)
                if self.worker_keys.get(wid).and_then(Option::as_ref) == Some(key)
                    && self.active_workers.get(key) == Some(&wid) =>
            {
                return;
            }
            None if self.worker_keys.get(wid).is_none_or(Option::is_none) => return,
            _ => {}
        }

        if wid >= self.worker_keys.len() {
            self.worker_keys.resize(wid + 1, None);
        }

        if let Some(previous_key) = self.worker_keys[wid].take() {
            if self.active_workers.get(&previous_key) == Some(&wid) {
                self.active_workers.remove(&previous_key);
            }
        }

        if let Some(key) = key {
            if let Some(previous_wid) = self.active_workers.insert(key.clone(), wid) {
                if previous_wid != wid
                    && self.worker_keys.get(previous_wid).and_then(Option::as_ref) == Some(key)
                {
                    self.worker_keys[previous_wid] = None;
                }
            }
            self.worker_keys[wid] = Some(key.clone());
        }
    }
}

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
        let Some(wid) = self.choose_target_worker(&job, pool_size, worker_hint, worker_pool) else {
            return Ok(RouteResult::Backlog(job));
        };
        let Some(worker) = worker_pool.get_mut(&wid) else {
            self.update_worker_key(wid, None);
            return Ok(RouteResult::Backlog(job));
        };

        let key = job.key.clone();
        worker
            .enqueue_job(job)
            .map_err(|err| (*err).discard_message())?;
        self.update_worker_key(wid, Some(&key));
        Ok(RouteResult::Handled)
    }

    fn choose_target_worker(
        &mut self,
        job: &Job<TKey, TMsg>,
        _pool_size: usize,
        worker_hint: Option<WorkerId>,
        worker_pool: &HashMap<WorkerId, WorkerProperties<TKey, TMsg>>,
    ) -> Option<WorkerId> {
        // Look up the active key directly. A draining worker retains the binding until
        // it finishes so the same key cannot execute concurrently on another worker.
        if let Some(wid) = self.active_workers.get(&job.key).copied() {
            if let Some(worker) = worker_pool.get(&wid) {
                if worker.is_processing_key(&job.key) {
                    return (!worker.is_draining).then_some(wid);
                }
            }
            self.update_worker_key(wid, None);
        }

        // A hint provides an O(1) recovery path if a custom factory transition did not
        // report its current key to the router.
        if let Some(wid) = worker_hint {
            if let Some(worker) = worker_pool.get(&wid) {
                if worker.is_processing_key(&job.key) {
                    let is_draining = worker.is_draining;
                    self.update_worker_key(wid, Some(&job.key));
                    return (!is_draining).then_some(wid);
                }
            }
        }

        // now take first available, based on hint then deque
        if let Some(worker) = worker_hint.and_then(|worker| worker_pool.get(&worker)) {
            if worker.is_available() {
                return worker_hint;
            }
        }

        // fallback to first free worker via the available-workers deque
        while let Some(wid) = self.available_workers.pop_front() {
            if wid < self.worker_in_queue.len() {
                self.worker_in_queue[wid] = false;
            }
            if let Some(worker) = worker_pool.get(&wid) {
                if worker.is_available() {
                    return Some(wid);
                }
            }
            // Worker removed from pool or no longer available — skip
        }
        None
    }

    fn is_factory_queueing(&self) -> bool {
        true
    }

    fn on_worker_availability_change(&mut self, wid: WorkerId, available: bool) {
        // Grow the tracking vec if needed
        if wid >= self.worker_in_queue.len() {
            self.worker_in_queue.resize(wid + 1, false);
        }
        if available {
            if !self.worker_in_queue[wid] {
                self.worker_in_queue[wid] = true;
                self.available_workers.push_back(wid);
            }
        } else {
            // Mark not-in-queue; lazy removal from deque
            self.worker_in_queue[wid] = false;
        }
    }

    fn on_worker_key_change(&mut self, wid: WorkerId, key: Option<&TKey>) {
        self.update_worker_key(wid, key);
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
            worker
                .enqueue_job(job)
                .map_err(|err| (*err).discard_message())?;
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
            worker
                .enqueue_job(job)
                .map_err(|err| (*err).discard_message())?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sticky_key_indexes_follow_worker_lifecycle() {
        let mut router = StickyQueuerRouting::<u64, ()>::default();

        router.on_worker_key_change(2, Some(&10));
        assert_eq!(Some(&2), router.active_workers.get(&10));
        assert_eq!(Some(&10), router.worker_keys[2].as_ref());

        // Completion followed by another assignment on the same worker replaces the key.
        router.on_worker_key_change(2, Some(&11));
        assert!(!router.active_workers.contains_key(&10));
        assert_eq!(Some(&2), router.active_workers.get(&11));

        // Replacement or retry on another worker repairs both sides of the index.
        router.on_worker_key_change(3, Some(&11));
        assert_eq!(None, router.worker_keys[2]);
        assert_eq!(Some(&11), router.worker_keys[3].as_ref());
        assert_eq!(Some(&3), router.active_workers.get(&11));

        // Failure, removal, and pool shrink all publish an empty current key.
        router.on_worker_key_change(3, None);
        assert!(!router.active_workers.contains_key(&11));
        assert_eq!(None, router.worker_keys[3]);
    }
}
