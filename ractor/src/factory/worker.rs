// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Factory worker properties

use std::collections::{HashMap, VecDeque};

use crate::concurrency::{Duration, Instant, JoinHandle};
use crate::{ActorId, ActorProcessingErr};
use crate::{ActorRef, Message, MessagingErr};

use super::stats::MessageProcessingStats;
use super::FactoryMessage;
use super::Job;
use super::JobKey;
use super::WorkerId;
use super::{DiscardHandler, JobOptions};

/// Message to a worker
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
pub struct WorkerStartContext<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    /// The worker's identifier
    pub wid: WorkerId,

    /// The factory the worker belongs to
    pub factory: ActorRef<FactoryMessage<TKey, TMsg>>,
}

/// Properties of a worker
pub struct WorkerProperties<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    /// Worker identifier
    pub(crate) wid: WorkerId,

    /// Worker's capacity for parallel work
    capacity: usize,

    /// Worker actor
    pub(crate) actor: ActorRef<WorkerMessage<TKey, TMsg>>,

    /// Worker's message queue
    message_queue: VecDeque<Job<TKey, TMsg>>,
    /// Maximum queue length. Any job arriving when the queue is at its max length
    /// will cause an oldest job at the head of the queue will be dropped.
    ///
    /// Default is disabled
    discard_threshold: Option<usize>,

    /// A function to be called for each job to be dropped.
    discard_handler: Option<Box<dyn DiscardHandler<TKey, TMsg>>>,

    /// Flag indicating if this worker has a ping currently pending
    is_ping_pending: bool,

    /// Statistics for the worker
    stats: MessageProcessingStats,

    /// Current pending jobs dispatched to the worker (for tracking stats)
    curr_jobs: HashMap<TKey, JobOptions>,

    /// The join handle for the worker
    handle: Option<JoinHandle<()>>,
}

impl<TKey, TMsg> WorkerProperties<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    fn get_next_non_expired_job(&mut self) -> Option<Job<TKey, TMsg>> {
        while let Some(job) = self.message_queue.pop_front() {
            if !job.is_expired() {
                return Some(job);
            } else {
                self.stats.job_ttl_expired();
            }
        }
        None
    }

    pub(crate) fn new(
        wid: WorkerId,
        actor: ActorRef<WorkerMessage<TKey, TMsg>>,
        capacity: usize,
        discard_threshold: Option<usize>,
        discard_handler: Option<Box<dyn DiscardHandler<TKey, TMsg>>>,
        collect_stats: bool,
        handle: JoinHandle<()>,
    ) -> Self {
        let mut stats = MessageProcessingStats::default();
        if collect_stats {
            stats.enable();
        }
        Self {
            actor,
            discard_handler,
            discard_threshold,
            message_queue: VecDeque::new(),
            curr_jobs: HashMap::new(),
            wid,
            capacity,
            is_ping_pending: false,
            stats,
            handle: Some(handle),
        }
    }

    pub(crate) fn is_pid(&self, pid: ActorId) -> bool {
        self.actor.get_id() == pid
    }

    pub(crate) fn is_processing_key(&self, key: &TKey) -> bool {
        self.curr_jobs.contains_key(key)
    }

    pub(crate) fn replace_worker(
        &mut self,
        nworker: ActorRef<WorkerMessage<TKey, TMsg>>,
        handle: JoinHandle<()>,
    ) -> Result<(), ActorProcessingErr> {
        // these jobs are now "lost" as the worker is going to be killed
        self.is_ping_pending = false;
        self.stats.last_ping = Instant::now();
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

    pub(crate) fn get_handle(&mut self) -> Option<JoinHandle<()>> {
        self.handle.take()
    }

    pub(crate) fn is_available(&self) -> bool {
        self.curr_jobs.len() < self.capacity
    }

    /// Denotes if the worker is stuck (i.e. unable to complete it's current job)

    pub(crate) fn is_stuck(&self, duration: Duration) -> bool {
        if Instant::now() - self.stats.last_ping > duration {
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
    /// it will discard the oldest elements from the message queue
    pub(crate) fn enqueue_job(
        &mut self,
        mut job: Job<TKey, TMsg>,
    ) -> Result<(), MessagingErr<WorkerMessage<TKey, TMsg>>> {
        // track per-job statistics
        self.stats.job_submitted();

        if self.curr_jobs.len() < self.capacity {
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
        if let Some(discard_threshold) = self.discard_threshold {
            while discard_threshold > 0 && self.message_queue.len() > discard_threshold {
                if let Some(discarded) = self.get_next_non_expired_job() {
                    self.stats.job_discarded();
                    if let Some(handler) = &self.discard_handler {
                        handler.discard(discarded);
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
    pub(crate) fn ping_received(&mut self, time: Duration) {
        if self.stats.ping_received(time) {
            // TODO log metrics ? Should be configurable on the factory level
        }
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
}
