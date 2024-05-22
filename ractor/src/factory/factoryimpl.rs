// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Factory definition

use std::cmp::Ordering;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

use self::routing::RouteResult;
use crate::concurrency::Duration;
use crate::concurrency::Instant;
use crate::Actor;
use crate::ActorProcessingErr;
use crate::ActorRef;
use crate::Message;
use crate::MessagingErr;
use crate::SpawnErr;
use crate::SupervisionEvent;

use super::queues::Queue;
use super::routing::Router;
use super::*;

/// The global execution limit, any more than 1M and realistically
/// we'll get into scheduling problems unless the requests have an
/// incredibly low reception rate and high processing latency. At
/// which point, a factory probably doesn't make great sense for
/// load-shedding customization
const GLOBAL_WORKER_POOL_MAXIMUM: usize = 1_000_000;

#[cfg(test)]
const PING_FREQUENCY: Duration = Duration::from_millis(150);
#[cfg(not(test))]
const PING_FREQUENCY: Duration = Duration::from_millis(10_000);
const CALCULATE_FREQUENCY: Duration = Duration::from_millis(100);

#[derive(Eq, PartialEq)]
enum DrainState {
    NotDraining,
    Draining,
    Drained,
}

/// Factory definition.
///
/// This is a placeholder instance which contains all of the type specifications
/// for the factories properties
pub struct Factory<TKey, TMsg, TWorkerStart, TWorker, TRouter, TQueue>
where
    TKey: JobKey,
    TMsg: Message,
    TWorkerStart: Message,
    TWorker: Actor<
        Msg = WorkerMessage<TKey, TMsg>,
        Arguments = WorkerStartContext<TKey, TMsg, TWorkerStart>,
    >,
    TRouter: Router<TKey, TMsg>,
    TQueue: Queue<TKey, TMsg>,
{
    _key: PhantomData<fn() -> TKey>,
    _msg: PhantomData<fn() -> TMsg>,
    _worker_start: PhantomData<fn() -> TWorkerStart>,
    _worker: PhantomData<fn() -> TWorker>,
    _router: PhantomData<fn() -> TRouter>,
    _queue: PhantomData<fn() -> TQueue>,
}

impl<TKey, TMsg, TWorkerStart, TWorker, TRouter, TQueue> Default
    for Factory<TKey, TMsg, TWorkerStart, TWorker, TRouter, TQueue>
where
    TKey: JobKey,
    TMsg: Message,
    TWorkerStart: Message,
    TWorker: Actor<
        Msg = WorkerMessage<TKey, TMsg>,
        Arguments = WorkerStartContext<TKey, TMsg, TWorkerStart>,
    >,
    TRouter: Router<TKey, TMsg>,
    TQueue: Queue<TKey, TMsg>,
{
    fn default() -> Self {
        Self {
            _key: PhantomData,
            _msg: PhantomData,
            _worker_start: PhantomData,
            _worker: PhantomData,
            _router: PhantomData,
            _queue: PhantomData,
        }
    }
}

/// Arguments for configuring and starting a [Factory] actor instance.
pub struct FactoryArguments<TKey, TMsg, TWorkerStart, TWorker, TRouter, TQueue>
where
    TKey: JobKey,
    TMsg: Message,
    TWorkerStart: Message,
    TWorker: Actor<
        Msg = WorkerMessage<TKey, TMsg>,
        Arguments = WorkerStartContext<TKey, TMsg, TWorkerStart>,
    >,
    TRouter: Router<TKey, TMsg>,
    TQueue: Queue<TKey, TMsg>,
{
    /// The factory is responsible for spawning workers and re-spawning workers
    /// under failure scenarios. This means that it needs to understand how to
    /// build workers. The WorkerBuilder trait is used by the factory to
    /// construct new workers when needed.
    pub worker_builder: Box<dyn WorkerBuilder<TWorker, TWorkerStart>>,
    /// Number of (initial) workers in the factory
    pub num_initial_workers: usize,
    /// Message routing handler
    pub router: TRouter,
    /// Message queue implementation for the factory
    pub queue: TQueue,
    /// Discard callback when a job is discarded.
    ///
    /// Default is [None]
    pub discard_handler: Option<Arc<dyn DiscardHandler<TKey, TMsg>>>,
    /// Maximum queue length. Any job arriving when the queue is at its max length
    /// will cause a job at the head or tail of the queue to be dropped (which is
    /// controlled by `discard_mode`).
    ///
    /// * For factories using [routing::QueuerRouting], [routing::StickyQueuerRouting] routing, these
    /// are applied to the factory's internal queue.
    /// * For all other routing protocols, this applies to the worker's message queue
    ///
    /// Default is [DiscardSettings::None]
    pub discard_settings: DiscardSettings,
    /// Controls the "dead man's" switching logic on the factory. Periodically
    /// the factory will scan for stuck workers. If detected, the worker information
    /// will be logged along with the current job key information. Optionally the worker
    /// can be killed and replaced by the factory
    pub dead_mans_switch: Option<DeadMansSwitchConfiguration>,
    /// Controls the parallel capacity of the worker pool by dynamically growing/shrinking the pool
    pub capacity_controller: Option<Box<dyn WorkerCapacityController>>,

    /// Lifecycle hooks which provide access to points in the factory's lifecycle
    /// for shutdown/startup/draining
    pub lifecycle_hooks: Option<Box<dyn FactoryLifecycleHooks<TKey, TMsg>>>,

    /// Identifies if the factory should collect statstics around each worker
    pub collect_worker_stats: bool,
}

/// Builder for [FactoryArguments] which can be used to build the
/// [Factory]'s startup arguments.
pub struct FactoryArgumentsBuilder<TKey, TMsg, TWorkerStart, TWorker, TRouter, TQueue>
where
    TKey: JobKey,
    TMsg: Message,
    TWorkerStart: Message,
    TWorker: Actor<
        Msg = WorkerMessage<TKey, TMsg>,
        Arguments = WorkerStartContext<TKey, TMsg, TWorkerStart>,
    >,
    TRouter: Router<TKey, TMsg>,
    TQueue: Queue<TKey, TMsg>,
{
    // Required
    worker_builder: Box<dyn WorkerBuilder<TWorker, TWorkerStart>>,
    num_initial_workers: usize,
    router: TRouter,
    queue: TQueue,
    // Optional
    discard_handler: Option<Arc<dyn DiscardHandler<TKey, TMsg>>>,
    discard_settings: DiscardSettings,
    dead_mans_switch: Option<DeadMansSwitchConfiguration>,
    capacity_controller: Option<Box<dyn WorkerCapacityController>>,
    lifecycle_hooks: Option<Box<dyn FactoryLifecycleHooks<TKey, TMsg>>>,
    collect_worker_stats: bool,
}

impl<TKey, TMsg, TWorkerStart, TWorker, TRouter, TQueue>
    FactoryArgumentsBuilder<TKey, TMsg, TWorkerStart, TWorker, TRouter, TQueue>
where
    TKey: JobKey,
    TMsg: Message,
    TWorkerStart: Message,
    TWorker: Actor<
        Msg = WorkerMessage<TKey, TMsg>,
        Arguments = WorkerStartContext<TKey, TMsg, TWorkerStart>,
    >,
    TRouter: Router<TKey, TMsg>,
    TQueue: Queue<TKey, TMsg>,
{
    /// Construct a new [FactoryArguments] with the required arguments
    ///
    /// * `worker_builder`: The implementation of the [WorkerBuilder] trait which is
    /// used to construct worker instances as needed
    /// * `router`: The message routing implementation the factory should use. Implements
    /// the [Router] trait.
    /// * `queue`: The message queueing implementation the factory should use. Implements
    /// the [Queue] trait.
    pub fn new<TBuilder: WorkerBuilder<TWorker, TWorkerStart> + 'static>(
        worker_builder: TBuilder,
        router: TRouter,
        queue: TQueue,
    ) -> Self {
        Self {
            worker_builder: Box::new(worker_builder),
            num_initial_workers: 1,
            router,
            queue,
            discard_handler: None,
            discard_settings: DiscardSettings::None,
            dead_mans_switch: None,
            capacity_controller: None,
            lifecycle_hooks: None,
            collect_worker_stats: false,
        }
    }

    /// Build the [FactoryArguments] required to start the [Factory]
    pub fn build(self) -> FactoryArguments<TKey, TMsg, TWorkerStart, TWorker, TRouter, TQueue> {
        let Self {
            worker_builder,
            num_initial_workers,
            router,
            queue,
            discard_handler,
            discard_settings,
            dead_mans_switch,
            capacity_controller,
            lifecycle_hooks,
            collect_worker_stats,
        } = self;
        FactoryArguments {
            worker_builder,
            num_initial_workers,
            router,
            queue,
            discard_handler,
            discard_settings,
            dead_mans_switch,
            capacity_controller,
            lifecycle_hooks,
            collect_worker_stats,
        }
    }

    /// Sets the the number initial workers in the factory.
    ///
    /// This controls the factory's initial parallelism for handling
    /// concurrent messages.
    pub fn with_number_of_initial_workers(self, worker_count: usize) -> Self {
        Self {
            num_initial_workers: worker_count,
            ..self
        }
    }

    /// Sets the factory's discard handler. This is a callback which
    /// is used when a job is discarded (e.g. loadshed, timeout, shutdown, etc).
    ///
    /// Default is [None]
    pub fn with_discard_handler<TDiscard: DiscardHandler<TKey, TMsg>>(
        self,
        discard_handler: TDiscard,
    ) -> Self {
        Self {
            discard_handler: Some(Arc::new(discard_handler)),
            ..self
        }
    }

    /// Sets the factory's discard settings.
    ///
    /// This controls the maximum queue length. Any job arriving when the queue is at
    /// its max length will cause a job at the head or tail of the queue to be dropped
    /// (which is controlled by `discard_mode`).
    ///
    /// * For factories using [routing::QueuerRouting], [routing::StickyQueuerRouting] routing, these
    /// are applied to the factory's internal queue.
    /// * For all other routing protocols, this applies to the worker's message queue
    ///
    /// Default is [DiscardSettings::None]
    pub fn with_discard_settings(self, discard_settings: DiscardSettings) -> Self {
        Self {
            discard_settings,
            ..self
        }
    }

    /// Controls the "dead man's" switching logic on the factory.
    ///
    /// Periodically the factory can scan for stuck workers. If detected, the worker information
    /// will be logged along with the current job key information.
    ///
    /// Optionally the worker can be killed and replaced by the factory.
    ///
    /// This can be used to detect and kill stuck jobs that will never compelte in a reasonable
    /// time (e.g. haven't configured internally a job execution timeout or something).
    pub fn with_dead_mans_switch(self, dmd: DeadMansSwitchConfiguration) -> Self {
        Self {
            dead_mans_switch: Some(dmd),
            ..self
        }
    }

    /// Set the factory's dynamic worker capacity controller.
    ///
    /// This, at runtime, controls the factory's capacity (i.e. number of
    /// workers) and can adjust it up and down (bounded in `[1,1_000_000]`).
    pub fn with_capacity_controller<TCapacity: WorkerCapacityController>(
        self,
        capacity_controller: TCapacity,
    ) -> Self {
        Self {
            capacity_controller: Some(Box::new(capacity_controller)),
            ..self
        }
    }

    /// Sets the factory's lifecycle hooks implementation
    ///
    /// Lifecycle hooks provide access to points in the factory's lifecycle
    /// for shutdown/startup/draining where user-defined logic can execute (and
    /// block factory lifecycle at critical points). This means
    /// the factory won't start accepting requests until the complete startup routine
    /// is completed.
    pub fn with_lifecycle_hooks<TLifecycle: FactoryLifecycleHooks<TKey, TMsg>>(
        self,
        lifecycle_hooks: TLifecycle,
    ) -> Self {
        Self {
            lifecycle_hooks: Some(Box::new(lifecycle_hooks)),
            ..self
        }
    }
}

/// State of a factory (backlogged jobs, handler, etc)
pub struct FactoryState<TKey, TMsg, TWorker, TWorkerStart, TRouter, TQueue>
where
    TKey: JobKey,
    TMsg: Message,
    TWorker: Actor<
        Msg = WorkerMessage<TKey, TMsg>,
        Arguments = WorkerStartContext<TKey, TMsg, TWorkerStart>,
    >,
    TWorkerStart: Message,
    TRouter: Router<TKey, TMsg>,
    TQueue: Queue<TKey, TMsg>,
{
    worker_builder: Box<dyn WorkerBuilder<TWorker, TWorkerStart>>,
    pool_size: usize,
    pool: HashMap<WorkerId, WorkerProperties<TKey, TMsg>>,
    stats: MessageProcessingStats,
    collect_worker_stats: bool,
    router: TRouter,
    queue: TQueue,
    discard_handler: Option<Arc<dyn DiscardHandler<TKey, TMsg>>>,
    discard_settings: DiscardSettings,
    drain_state: DrainState,
    dead_mans_switch: Option<DeadMansSwitchConfiguration>,
    capacity_controller: Option<Box<dyn WorkerCapacityController>>,
    lifecycle_hooks: Option<Box<dyn FactoryLifecycleHooks<TKey, TMsg>>>,
}

impl<TKey, TMsg, TWorker, TWorkerStart, TRouter, TQueue>
    FactoryState<TKey, TMsg, TWorker, TWorkerStart, TRouter, TQueue>
where
    TKey: JobKey,
    TMsg: Message,
    TWorker: Actor<
        Msg = WorkerMessage<TKey, TMsg>,
        Arguments = WorkerStartContext<TKey, TMsg, TWorkerStart>,
    >,
    TWorkerStart: Message,
    TRouter: Router<TKey, TMsg>,
    TQueue: Queue<TKey, TMsg>,
{
    /// This method tries to
    ///
    /// 1. Cleanup expired jobs at the head of the queue, discarding them
    /// 2. Route the next non-expired job (if any)
    ///     - If a worker-hint was provided, and the worker is available, route it there (this is used
    ///       for when workers have just completed work, and should immediately receive a new job)
    ///     - If no hint provided, route to the next worker by routing protocol.
    fn try_route_next_active_job(
        &mut self,
        worker_hint: WorkerId,
    ) -> Result<(), MessagingErr<WorkerMessage<TKey, TMsg>>> {
        // cleanup expired messages at the head of the queue
        while let Some(true) = self.queue.peek().map(|m| m.is_expired()) {
            // remove the job from the queue
            if let Some(mut job) = self.queue.pop_front() {
                self.stats.job_ttl_expired();
                if let Some(handler) = &self.discard_handler {
                    handler.discard(DiscardReason::TtlExpired, &mut job);
                }
            } else {
                break;
            }
        }

        if let Some(worker) = self.pool.get_mut(&worker_hint).filter(|f| f.is_available()) {
            if let Some(job) = self.queue.pop_front() {
                worker.enqueue_job(job)?;
            }
        } else {
            // target the next available worker
            let target_worker = self
                .queue
                .peek()
                .and_then(|job| {
                    self.router
                        .choose_target_worker(job, self.pool_size, &self.pool)
                })
                .and_then(|wid| self.pool.get_mut(&wid));
            if let (Some(job), Some(worker)) = (self.queue.pop_front(), target_worker) {
                worker.enqueue_job(job)?;
            }
        }
        Ok(())
    }

    fn maybe_enqueue(&mut self, mut job: Job<TKey, TMsg>) {
        let is_discardable = self.queue.is_job_discardable(&job.key);
        let limit_and_mode = self.discard_settings.get_limit_and_mode();

        match limit_and_mode {
            Some((limit, DiscardMode::Newest)) => {
                if is_discardable && self.queue.len() >= limit {
                    // load-shed the job
                    if let Some(handler) = &self.discard_handler {
                        handler.discard(DiscardReason::Loadshed, &mut job);
                    }
                    self.stats.job_discarded();
                } else {
                    self.queue.push_back(job);
                }
            }
            Some((limit, DiscardMode::Oldest)) => {
                self.queue.push_back(job);
                while self.queue.len() > limit {
                    // try and shed a job, of the lowest priority working up
                    if let Some(mut msg) = self.queue.discard_oldest() {
                        self.stats.job_discarded();
                        if let Some(handler) = &self.discard_handler {
                            handler.discard(DiscardReason::Loadshed, &mut msg);
                        }
                    }
                }
            }
            None => {
                // no load-shedding
                self.queue.push_back(job);
            }
        }
    }

    async fn grow_pool(
        &mut self,
        myself: &ActorRef<FactoryMessage<TKey, TMsg>>,
        to_add: usize,
    ) -> Result<(), SpawnErr> {
        let curr_size = self.pool_size;
        for wid in curr_size..(curr_size + to_add) {
            tracing::trace!("Adding worker {}", wid);
            if let Some(existing_worker) = self.pool.get_mut(&wid) {
                // mark the worker as healthy again
                existing_worker.set_draining(false);
            } else {
                // worker doesn't exist, add it
                let (handler, custom_start) = self.worker_builder.build(wid);
                let context = WorkerStartContext {
                    wid,
                    factory: myself.clone(),
                    custom_start,
                };
                let (worker, handle) =
                    Actor::spawn_linked(None, handler, context, myself.get_cell()).await?;
                let discard_settings = if self.router.is_factory_queueing() {
                    discard::WorkerDiscardSettings::None
                } else {
                    self.discard_settings.get_worker_settings()
                };
                self.pool.insert(
                    wid,
                    WorkerProperties::new(
                        wid,
                        worker,
                        discard_settings,
                        self.discard_handler.clone(),
                        self.collect_worker_stats,
                        handle,
                    ),
                );
            }
        }
        Ok(())
    }

    fn shrink_pool(&mut self, to_remove: usize) {
        let curr_size = self.pool_size;
        for wid in (curr_size - to_remove)..curr_size {
            match self.pool.entry(wid) {
                std::collections::hash_map::Entry::Occupied(mut existing_worker) => {
                    let mut_worker = existing_worker.get_mut();
                    if mut_worker.is_working() {
                        // mark the worker as draining
                        mut_worker.set_draining(true);
                    } else {
                        // drained, stop and drop
                        tracing::trace!("Stopping worker {wid}");
                        mut_worker.actor.stop(None);
                        existing_worker.remove();
                    }
                }
                std::collections::hash_map::Entry::Vacant(_) => {
                    // worker doesn't exist, ignore
                }
            }
        }
    }

    async fn resize_pool(
        &mut self,
        myself: &ActorRef<FactoryMessage<TKey, TMsg>>,
        requested_pool_size: usize,
    ) -> Result<(), SpawnErr> {
        if requested_pool_size == 0 {
            return Ok(());
        }

        let curr_size = self.pool_size;
        let new_pool_size = std::cmp::min(GLOBAL_WORKER_POOL_MAXIMUM, requested_pool_size);

        match new_pool_size.cmp(&curr_size) {
            Ordering::Greater => {
                tracing::debug!(
                    factory = ?myself, "Resizing factory worker pool from {} -> {}",
                    curr_size,
                    new_pool_size
                );
                // grow pool
                let to_add = new_pool_size - curr_size;
                self.grow_pool(myself, to_add).await?;
            }
            Ordering::Less => {
                tracing::debug!(
                    factory = ?myself, "Resizing factory worker pool from {} -> {}",
                    curr_size,
                    new_pool_size
                );
                // shrink pool
                let to_remove = curr_size - new_pool_size;
                self.shrink_pool(to_remove);
            }
            Ordering::Equal => {
                // no-op
            }
        }

        self.pool_size = new_pool_size;
        Ok(())
    }

    fn is_drained(&mut self) -> bool {
        match &self.drain_state {
            DrainState::NotDraining => false,
            DrainState::Drained => true,
            DrainState::Draining => {
                let are_all_workers_free = self.pool.values().all(|worker| worker.is_available());
                if are_all_workers_free && self.queue.len() == 0 {
                    tracing::debug!("Worker pool is free and queue is empty.");
                    // everyone is free, all requests are drainined
                    self.drain_state = DrainState::Drained;
                    true
                } else {
                    false
                }
            }
        }
    }

    fn dispatch(&mut self, mut job: Job<TKey, TMsg>) -> Result<(), ActorProcessingErr> {
        // set the time the factory received the message
        job.set_factory_time();
        self.stats.job_submitted();

        if self.drain_state == DrainState::NotDraining {
            if let RouteResult::Backlog(busy_job) =
                self.router
                    .route_message(job, self.pool_size, &mut self.pool)?
            {
                // workers are busy, we need to queue a job
                self.maybe_enqueue(busy_job);
            }
        } else {
            tracing::debug!("Factory is draining but a job was received");
            if let Some(discard_handler) = &self.discard_handler {
                discard_handler.discard(DiscardReason::Shutdown, &mut job);
            }
        }
        Ok(())
    }

    fn worker_finished_job(&mut self, who: WorkerId, key: TKey) -> Result<(), ActorProcessingErr> {
        let (is_worker_draining, should_drop_worker) = if let Some(worker) = self.pool.get_mut(&who)
        {
            if let Some(job_options) = worker.worker_complete(key)? {
                self.stats.factory_job_done(&job_options);
            }

            if worker.is_draining {
                // don't schedule more work
                (true, !worker.is_working())
            } else {
                (false, false)
            }
        } else {
            (false, false)
        };

        if should_drop_worker {
            let worker = self.pool.remove(&who);
            if let Some(w) = worker {
                tracing::trace!("Stopping worker {}", w.wid);
                w.actor.stop(None);
            }
        } else if !is_worker_draining {
            self.try_route_next_active_job(who)?;
        }
        Ok(())
    }

    fn worker_pong(&mut self, wid: usize, time: Duration) {
        let discard_limit = self
            .discard_settings
            .get_limit_and_mode()
            .map_or(0, |(l, _)| l);
        if let Some(worker) = self.pool.get_mut(&wid) {
            worker.ping_received(time, discard_limit);
        }
    }

    async fn calculate_metrics(
        &mut self,
        myself: &ActorRef<FactoryMessage<TKey, TMsg>>,
    ) -> Result<(), ActorProcessingErr> {
        if let Some(capacity_controller) = &mut self.capacity_controller {
            let new_capacity = capacity_controller.get_pool_size(self.pool_size).await;
            if self.pool_size != new_capacity {
                tracing::info!("Factory worker count {}", new_capacity);
                self.resize_pool(myself, new_capacity).await?;
            }
        }

        // TTL expired on these items, remove them before even trying to dequeue & distribute them
        if self.router.is_factory_queueing() {
            let num_removed = self.queue.remove_expired_items(&self.discard_handler);
            self.stats.jobs_ttls_expired(num_removed);
        }

        self.stats.reset_global_counters();

        // schedule next calculation
        myself.send_after(CALCULATE_FREQUENCY, || FactoryMessage::Calculate);
        Ok(())
    }

    async fn send_pings(
        &mut self,
        myself: &ActorRef<FactoryMessage<TKey, TMsg>>,
        when: Instant,
    ) -> Result<(), ActorProcessingErr> {
        self.stats.ping_received(when.elapsed());

        // if we have dyanmic discarding, we update the discard threshold
        if let DiscardSettings::Dynamic { limit, updater, .. } = &mut self.discard_settings {
            *limit = updater.compute(*limit).await;
        }

        for worker in self.pool.values_mut() {
            worker.send_factory_ping()?;
        }

        // schedule next ping
        myself.send_after(PING_FREQUENCY, || FactoryMessage::DoPings(Instant::now()));

        Ok(())
    }

    async fn identify_stuck_workers(&mut self, myself: &ActorRef<FactoryMessage<TKey, TMsg>>) {
        if let Some(dmd) = &self.dead_mans_switch {
            let mut dead_workers = vec![];
            for worker in self.pool.values_mut() {
                if worker.is_stuck(dmd.detection_timeout) && dmd.kill_worker {
                    tracing::warn!(
                        factory = ?myself, "Factory killing stuck worker {}",
                        worker.wid
                    );
                    worker.actor.kill();
                    if let Some(h) = worker.get_join_handle() {
                        dead_workers.push(h);
                    }
                }
            }

            for w in dead_workers.into_iter() {
                let _ = w.await;
            }

            // schedule next check
            myself.send_after(dmd.detection_timeout, || {
                FactoryMessage::IdentifyStuckWorkers
            });
        }
    }

    async fn drain_requests(
        &mut self,
        myself: &ActorRef<FactoryMessage<TKey, TMsg>>,
    ) -> Result<(), ActorProcessingErr> {
        // put us into a draining state
        tracing::debug!("Factory is moving to draining state");
        self.drain_state = DrainState::Draining;
        if let Some(hooks) = &mut self.lifecycle_hooks {
            hooks.on_factory_draining(myself.clone()).await?;
        }
        Ok(())
    }

    fn reply_with_available_capacity(&self, reply: RpcReplyPort<usize>) {
        // calculate the worker's free capacity
        let worker_availability = self
            .pool
            .values()
            .filter(|worker| worker.is_available())
            .count();
        match self.discard_settings.get_limit_and_mode() {
            Some((limit, _)) => {
                // get the queue space and add it to the worker availability
                let count = (limit - self.queue.len()) + worker_availability;
                let _ = reply.send(count);
            }
            None => {
                // there's no queueing limit, so we just report worker
                // availability
                let _ = reply.send(worker_availability);
            }
        }
    }
}

#[cfg_attr(feature = "async-trait", crate::async_trait)]
impl<TKey, TMsg, TWorkerStart, TWorker, TRouter, TQueue> Actor
    for Factory<TKey, TMsg, TWorkerStart, TWorker, TRouter, TQueue>
where
    TKey: JobKey,
    TMsg: Message,
    TWorkerStart: Message,
    TWorker: Actor<
        Msg = WorkerMessage<TKey, TMsg>,
        Arguments = WorkerStartContext<TKey, TMsg, TWorkerStart>,
    >,
    TRouter: Router<TKey, TMsg>,
    TQueue: Queue<TKey, TMsg>,
{
    type Msg = FactoryMessage<TKey, TMsg>;
    type State = FactoryState<TKey, TMsg, TWorker, TWorkerStart, TRouter, TQueue>;
    type Arguments = FactoryArguments<TKey, TMsg, TWorkerStart, TWorker, TRouter, TQueue>;

    async fn pre_start(
        &self,
        myself: ActorRef<FactoryMessage<TKey, TMsg>>,
        FactoryArguments {
            worker_builder,
            num_initial_workers,
            router,
            queue,
            discard_handler,
            discard_settings,
            dead_mans_switch,
            capacity_controller,
            lifecycle_hooks,
            collect_worker_stats,
        }: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        tracing::debug!(factory = ?myself, "Factory starting");

        // build the pool
        let mut pool = HashMap::new();
        for wid in 0..num_initial_workers {
            let (handler, custom_start) = worker_builder.build(wid);
            let context = WorkerStartContext {
                wid,
                factory: myself.clone(),
                custom_start,
            };
            let (worker, worker_handle) =
                Actor::spawn_linked(None, handler, context, myself.get_cell()).await?;
            let worker_discard_settings = if router.is_factory_queueing() {
                discard::WorkerDiscardSettings::None
            } else {
                discard_settings.get_worker_settings()
            };

            pool.insert(
                wid,
                WorkerProperties::new(
                    wid,
                    worker,
                    worker_discard_settings,
                    discard_handler.clone(),
                    collect_worker_stats,
                    worker_handle,
                ),
            );
        }

        // Startup worker pinging
        myself.send_after(PING_FREQUENCY, || FactoryMessage::DoPings(Instant::now()));

        // startup calculations
        myself.send_after(CALCULATE_FREQUENCY, || FactoryMessage::Calculate);

        // startup stuck worker detection
        if let Some(dmd) = &dead_mans_switch {
            myself.send_after(dmd.detection_timeout, || {
                FactoryMessage::IdentifyStuckWorkers
            });
        }

        // initial state
        Ok(FactoryState {
            worker_builder,
            pool_size: num_initial_workers,
            pool,
            drain_state: DrainState::NotDraining,
            capacity_controller,
            dead_mans_switch,
            discard_handler,
            discard_settings,
            lifecycle_hooks,
            queue,
            router,
            stats: {
                let mut s = MessageProcessingStats::default();
                s.enable();
                s
            },
            collect_worker_stats,
        })
    }

    async fn post_start(
        &self,
        myself: ActorRef<FactoryMessage<TKey, TMsg>>,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        tracing::debug!(factory = ?myself, "Factory started");
        if let Some(hooks) = &mut state.lifecycle_hooks {
            hooks.on_factory_started(myself).await?;
        }
        Ok(())
    }

    async fn post_stop(
        &self,
        myself: ActorRef<Self::Msg>,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        tracing::debug!(factory = ?myself, "Factory stopped");

        if let Some(handler) = &state.discard_handler {
            while let Some(mut msg) = state.queue.pop_front() {
                handler.discard(DiscardReason::Shutdown, &mut msg);
            }
        }

        // cleanup the pool and wait for it to exit
        for worker_props in state.pool.values() {
            worker_props.actor.stop(None);
        }
        // now wait on the handles until the workers finish
        for worker_props in state.pool.values_mut() {
            if let Some(handle) = worker_props.get_join_handle() {
                let _ = handle.await;
            }
        }

        if let Some(hooks) = &mut state.lifecycle_hooks {
            hooks.on_factory_stopped().await?;
        }

        Ok(())
    }

    async fn handle_supervisor_evt(
        &self,
        myself: ActorRef<FactoryMessage<TKey, TMsg>>,
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
                        factory = ?myself, "Factory's worker {} terminated with {:?}",
                        worker.wid,
                        reason
                    );
                    let (new_worker, custom_start) = state.worker_builder.build(worker.wid);
                    let spec = WorkerStartContext {
                        wid: worker.wid,
                        factory: myself.clone(),
                        custom_start,
                    };
                    let (replacement, replacement_handle) =
                        Actor::spawn_linked(None, new_worker, spec, myself.get_cell()).await?;

                    worker.replace_worker(replacement, replacement_handle)?;
                    Some(worker.wid)
                } else {
                    None
                };
                if let Some(wid) = wid {
                    state.try_route_next_active_job(wid)?;
                }
            }
            SupervisionEvent::ActorFailed(who, reason) => {
                let wid = if let Some(worker) = state
                    .pool
                    .values_mut()
                    .find(|actor| actor.is_pid(who.get_id()))
                {
                    tracing::warn!(
                        factory = ?myself, "Factory's worker {} panicked with {}",
                        worker.wid,
                        reason
                    );
                    let (new_worker, custom_start) = state.worker_builder.build(worker.wid);
                    let spec = WorkerStartContext {
                        wid: worker.wid,
                        factory: myself.clone(),
                        custom_start,
                    };
                    let (replacement, replacement_handle) =
                        Actor::spawn_linked(None, new_worker, spec, myself.get_cell()).await?;

                    worker.replace_worker(replacement, replacement_handle)?;
                    Some(worker.wid)
                } else {
                    None
                };
                if let Some(wid) = wid {
                    state.try_route_next_active_job(wid)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle(
        &self,
        myself: ActorRef<FactoryMessage<TKey, TMsg>>,
        message: FactoryMessage<TKey, TMsg>,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            FactoryMessage::Dispatch(job) => {
                state.dispatch(job)?;
            }
            FactoryMessage::Finished(who, key) => {
                state.worker_finished_job(who, key)?;
            }
            FactoryMessage::WorkerPong(wid, time) => {
                state.worker_pong(wid, time);
            }
            FactoryMessage::Calculate => {
                state.calculate_metrics(&myself).await?;
            }
            FactoryMessage::DoPings(when) => {
                state.send_pings(&myself, when).await?;
            }
            FactoryMessage::IdentifyStuckWorkers => {
                state.identify_stuck_workers(&myself).await;
            }
            FactoryMessage::GetQueueDepth(reply) => {
                let _ = reply.send(state.queue.len());
            }
            FactoryMessage::AdjustWorkerPool(requested_pool_size) => {
                tracing::info!("Adjusting pool size to {}", requested_pool_size);
                state.resize_pool(&myself, requested_pool_size).await?;
            }
            FactoryMessage::GetAvailableCapacity(reply) => {
                state.reply_with_available_capacity(reply);
            }
            FactoryMessage::DrainRequests => {
                state.drain_requests(&myself).await?;
            }
        }

        if state.is_drained() {
            // If we're in a draining state, and all requests are now drained
            // stop the factory
            myself.stop(None);
        }
        Ok(())
    }
}
