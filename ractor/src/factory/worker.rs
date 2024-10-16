// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Factory worker properties

use std::collections::{HashMap, VecDeque};
use std::fmt::Debug;
use std::sync::Arc;

use crate::concurrency::{Duration, Instant, JoinHandle};
use crate::{Actor, ActorId, ActorProcessingErr};
use crate::{ActorRef, Message, MessagingErr};

use super::discard::{DiscardMode, WorkerDiscardSettings};
use super::stats::FactoryStatsLayer;
use super::FactoryMessage;
use super::Job;
use super::JobKey;
use super::WorkerId;
use super::{DiscardHandler, DiscardReason, JobOptions};

/// The configuration for the dead-man's switch functionality
#[derive(Debug)]
pub struct DeadMansSwitchConfiguration {
    /// Duration before determining worker is stuck
    pub detection_timeout: Duration,
    /// Flag denoting if the stuck worker should be killed
    /// and restarted
    pub kill_worker: bool,
}

/// The [super::Factory] is responsible for spawning workers
/// and re-spawning workers under failure scenarios. This means that
/// it needs to understand how to build workers. The [WorkerBuilder]
/// trait is used by the factory to construct new workers when needed.
pub trait WorkerBuilder<TWorker, TWorkerStart>: Send + Sync
where
    TWorker: Actor,
    TWorkerStart: Message,
{
    /// Build a new worker
    ///
    /// * `wid`: The worker's "id" or index in the worker pool
    ///
    /// Returns a tuple of the worker and a custom startup definition giving the worker
    /// owned control of some structs that it may need to work.
    fn build(&self, wid: WorkerId) -> (TWorker, TWorkerStart);
}

/// Controls the size of the worker pool by dynamically growing/shrinking the pool
/// to requested size
#[cfg_attr(feature = "async-trait", crate::async_trait)]
pub trait WorkerCapacityController: 'static + Send + Sync {
    /// Retrieve the new pool size
    ///
    /// * `current` - The current pool size
    ///
    /// Returns the "new" pool size. If returns 0, adjustment will be
    /// ignored
    #[cfg(feature = "async-trait")]
    async fn get_pool_size(&mut self, current: usize) -> usize;

    /// Retrieve the new pool size
    ///
    /// * `current` - The current pool size
    ///
    /// Returns the "new" pool size. If returns 0, adjustment will be
    /// ignored
    #[cfg(not(feature = "async-trait"))]
    fn get_pool_size(&mut self, current: usize) -> futures::future::BoxFuture<'_, usize>;
}

/// Message to a worker
#[derive(Debug)]
pub enum WorkerMessage<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    /// A ping from the factory. The worker should send a [super::FactoryMessage::WorkerPong] reply
    /// as soon as received back to the [super::Factory], forwarding this instant value to
    /// track timing information.
    FactoryPing(Instant),
    /// A job is dispatched to the worker. Once the worker is complete with processing, it should
    /// reply with [super::FactoryMessage::Finished] to the [super::Factory] supplying it's
    /// WID and the job key to signify that the job is completed processing and the worker is
    /// available for a new job
    Dispatch(Job<TKey, TMsg>),
}

#[cfg(feature = "cluster")]
impl<TKey, TMsg> crate::Message for WorkerMessage<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
}

/// Startup context data (`Arguments`) which are passed to a worker on start
#[derive(Debug)]
pub struct WorkerStartContext<TKey, TMsg, TCustomStart>
where
    TKey: JobKey,
    TMsg: Message,
    TCustomStart: Message,
{
    /// The worker's identifier
    pub wid: WorkerId,

    /// The factory the worker belongs to
    pub factory: ActorRef<FactoryMessage<TKey, TMsg>>,

    /// Custom startup arguments to the worker
    pub custom_start: TCustomStart,
}

/// Properties of a worker
pub struct WorkerProperties<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    /// Worker identifier
    pub(crate) wid: WorkerId,

    /// Worker actor
    pub(crate) actor: ActorRef<WorkerMessage<TKey, TMsg>>,

    /// Name of the factory that owns this worker
    factory_name: String,

    /// The join handle for the worker
    handle: Option<JoinHandle<()>>,

    /// Worker's message queue
    message_queue: VecDeque<Job<TKey, TMsg>>,

    /// Maximum queue length. Any job arriving when the queue is at its max length
    /// will cause an oldest job at the head of the queue will be dropped.
    ///
    /// Default is [WorkerDiscardSettings::None]
    discard_settings: WorkerDiscardSettings,

    /// A function to be called for each job to be dropped.
    discard_handler: Option<Arc<dyn DiscardHandler<TKey, TMsg>>>,

    /// Flag indicating if this worker has a ping currently pending
    is_ping_pending: bool,

    /// Time the last ping went out to the worker to track ping metrics
    last_ping: Instant,

    /// Statistics for the worker
    stats: Option<Arc<dyn FactoryStatsLayer>>,

    /// Current pending jobs dispatched to the worker (for tracking stats)
    curr_jobs: HashMap<TKey, JobOptions>,

    /// Flag indicating if this worker is currently "draining" work due to resizing
    pub(crate) is_draining: bool,
}

impl<TKey, TMsg> Debug for WorkerProperties<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerProperties")
            .field("wid", &self.wid)
            .field("actor", &self.actor)
            .field("factory_name", &self.factory_name)
            .field("discard_settings", &self.discard_settings)
            .field("is_draining", &self.is_draining)
            .finish()
    }
}

impl<TKey, TMsg> WorkerProperties<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    fn get_next_non_expired_job(&mut self) -> Option<Job<TKey, TMsg>> {
        while let Some(mut job) = self.message_queue.pop_front() {
            if !job.is_expired() {
                return Some(job);
            } else {
                if let Some(handler) = &self.discard_handler {
                    handler.discard(DiscardReason::TtlExpired, &mut job);
                }
                self.stats.job_ttl_expired(&self.factory_name, 1);
            }
        }
        None
    }

    pub(crate) fn new(
        factory_name: String,
        wid: WorkerId,
        actor: ActorRef<WorkerMessage<TKey, TMsg>>,
        discard_settings: WorkerDiscardSettings,
        discard_handler: Option<Arc<dyn DiscardHandler<TKey, TMsg>>>,
        handle: JoinHandle<()>,
        stats: Option<Arc<dyn FactoryStatsLayer>>,
    ) -> Self {
        Self {
            factory_name,
            actor,
            discard_settings,
            discard_handler,
            message_queue: VecDeque::new(),
            curr_jobs: HashMap::new(),
            wid,
            is_ping_pending: false,
            stats,
            handle: Some(handle),
            is_draining: false,
            last_ping: Instant::now(),
        }
    }

    pub(crate) fn get_join_handle(&mut self) -> Option<JoinHandle<()>> {
        self.handle.take()
    }

    pub(crate) fn is_pid(&self, pid: ActorId) -> bool {
        self.actor.get_id() == pid
    }

    /// Identifies if a worker is processing a specific job key
    ///
    /// Returns true if the worker is currently processing the given key
    pub fn is_processing_key(&self, key: &TKey) -> bool {
        self.curr_jobs.contains_key(key)
    }

    pub(crate) fn replace_worker(
        &mut self,
        nworker: ActorRef<WorkerMessage<TKey, TMsg>>,
        handle: JoinHandle<()>,
    ) -> Result<(), ActorProcessingErr> {
        // these jobs are now "lost" as the worker is going to be killed
        self.is_ping_pending = false;
        self.last_ping = Instant::now();
        self.curr_jobs.clear();

        self.actor = nworker;
        self.handle = Some(handle);
        if let Some(mut job) = self.get_next_non_expired_job() {
            self.curr_jobs.insert(job.key.clone(), job.options.clone());
            job.set_worker_time();
            self.actor.cast(WorkerMessage::Dispatch(job))?;
        }
        Ok(())
    }

    /// Identify if the worker is available for enqueueing work
    pub fn is_available(&self) -> bool {
        self.curr_jobs.is_empty()
    }

    /// Identify if the worker is currently processing any requests
    pub fn is_working(&self) -> bool {
        !self.curr_jobs.is_empty()
    }

    /// Denotes if the worker is stuck (i.e. unable to complete it's current job)
    pub(crate) fn is_stuck(&self, duration: Duration) -> bool {
        if Instant::now() - self.last_ping > duration {
            let key_strings = self
                .curr_jobs
                .keys()
                .cloned()
                .fold(String::new(), |a, key| format!("{a}\nJob key: {key:?}"));
            tracing::warn!("Stuck worker: {}. Last jobs:\n{key_strings}", self.wid);
            true
        } else {
            false
        }
    }

    /// Enqueue a new job to this worker. If the discard threshold has been exceeded
    /// it will discard the oldest or newest elements from the message queue (based
    /// on discard semantics)
    pub fn enqueue_job(
        &mut self,
        mut job: Job<TKey, TMsg>,
    ) -> Result<(), MessagingErr<WorkerMessage<TKey, TMsg>>> {
        // track per-job statistics
        self.stats.new_job(&self.factory_name);

        if let Some((limit, DiscardMode::Newest)) = self.discard_settings.get_limit_and_mode() {
            if limit > 0 && self.message_queue.len() >= limit {
                // Discard THIS job as it's the newest one
                self.stats.job_discarded(&self.factory_name);
                if let Some(handler) = &self.discard_handler {
                    handler.discard(DiscardReason::Loadshed, &mut job);
                }
                job.reject();
                return Ok(());
            }
        }

        // if the job isn't front-load shedded, it's "accepted"
        job.accept();
        if self.curr_jobs.is_empty() {
            self.curr_jobs.insert(job.key.clone(), job.options.clone());
            if let Some(mut older_job) = self.get_next_non_expired_job() {
                self.message_queue.push_back(job);
                older_job.set_worker_time();
                self.actor.cast(WorkerMessage::Dispatch(older_job))?;
            } else {
                job.set_worker_time();
                self.actor.cast(WorkerMessage::Dispatch(job))?;
            }
            return Ok(());
        }
        self.message_queue.push_back(job);

        if let Some((limit, DiscardMode::Oldest)) = self.discard_settings.get_limit_and_mode() {
            // load-shed the OLDEST jobs
            while limit > 0 && self.message_queue.len() > limit {
                if let Some(mut discarded) = self.get_next_non_expired_job() {
                    self.stats.job_discarded(&self.factory_name);
                    if let Some(handler) = &self.discard_handler {
                        handler.discard(DiscardReason::Loadshed, &mut discarded);
                    }
                }
            }
        }
        Ok(())
    }

    /// Send a ping to the worker
    pub(crate) fn send_factory_ping(
        &mut self,
    ) -> Result<(), MessagingErr<WorkerMessage<TKey, TMsg>>> {
        if !self.is_ping_pending {
            self.is_ping_pending = true;
            self.actor.cast(WorkerMessage::FactoryPing(Instant::now()))
        } else {
            // don't send a new ping if one is currently pending
            Ok(())
        }
    }

    /// Comes back when a ping went out
    pub(crate) fn ping_received(&mut self, time: Duration, discard_limit: usize) {
        self.discard_settings.update_worker_limit(discard_limit);
        self.stats.worker_ping_received(&self.factory_name, time);
        self.is_ping_pending = false;
    }

    /// Called when the factory is notified a worker completed a job. Will push the next message
    /// if there is any messages in this worker's queue
    pub(crate) fn worker_complete(
        &mut self,
        key: TKey,
    ) -> Result<Option<JobOptions>, MessagingErr<WorkerMessage<TKey, TMsg>>> {
        // remove this pending job
        let options = self.curr_jobs.remove(&key);
        // maybe queue up the next job
        if let Some(mut job) = self.get_next_non_expired_job() {
            job.set_worker_time();
            self.actor.cast(WorkerMessage::Dispatch(job))?;
        }

        Ok(options)
    }

    /// Set the draining status of the worker
    pub(crate) fn set_draining(&mut self, is_draining: bool) {
        self.is_draining = is_draining;
    }
}
