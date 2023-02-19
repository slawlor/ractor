// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Factory worker properties

use std::collections::{HashMap, VecDeque};
use std::fmt::Display;
use std::time::SystemTime;

use crate::concurrency::{Duration, Instant};
use crate::{Actor, ActorRef, Message, MessagingErr};
use crate::{ActorId, ActorProcessingErr};

use super::Factory;
use super::Job;
use super::JobKey;
use super::WorkerId;
use super::{DiscardHandler, JobOptions};

const _MICROS_IN_SEC: u128 = 1000000;

pub(crate) struct MessageProcessingStats {
    // ========== Pings ========== //
    ping_count: u64,
    ping_timing_us: u128,
    last_ping: Instant,
    // ========== Incoming Job QPS ========== //
    last_job_time: Instant,
    job_count: u64,
    job_incoming_time_us: u128,
    // ========== Finished jobs ========== //
    job_processing_time_us: u128,
    processed_job_count: u64,
    total_processed_job_count: u128,
    time_in_factory_message_queue_us: u128,
    // ========== Expired jobs ========== //
    total_num_expired_jobs: u128,

    enabled: bool,
}

impl Display for MessageProcessingStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Avg ping time: {}us
Num processed jobs: {}
Job qps: {}
Avg job processing time: {}us
Avg time in factory queue: {}us
Num expired jobs: {}
            ",
            self.ping_timing_us / self.ping_count as u128,
            self.total_processed_job_count,
            self.avg_job_qps(),
            self.job_processing_time_us / self.processed_job_count as u128,
            self.time_in_factory_message_queue_us / self.processed_job_count as u128,
            self.total_num_expired_jobs,
        )
    }
}

impl Default for MessageProcessingStats {
    fn default() -> Self {
        Self {
            ping_count: 0,
            job_incoming_time_us: 0,
            last_ping: Instant::now(),
            last_job_time: Instant::now(),
            ping_timing_us: 0,
            job_count: 0,
            job_processing_time_us: 0,
            processed_job_count: 0,
            total_processed_job_count: 0,
            time_in_factory_message_queue_us: 0,
            total_num_expired_jobs: 0,
            enabled: false,
        }
    }
}

impl MessageProcessingStats {
    pub(crate) fn enable(&mut self) {
        self.enabled = true;
    }

    pub(crate) fn reset_global_counters(&mut self) {
        self.total_num_expired_jobs = 0;
        self.total_processed_job_count = 0;
    }

    /// Handle a factory ping, and every 10 minutes adjust the stats to the
    /// average + return a flag to state that we should log the factory's statistics
    pub(crate) fn handle_ping(&mut self, duration: Duration) -> bool {
        // we ALWAYS record the last ping time
        self.last_ping = Instant::now();

        if !self.enabled {
            return false;
        }

        self.ping_count += 1;
        self.ping_timing_us += duration.as_micros();

        if self.ping_count > 600 {
            // When we hit 10min, convert the message timing to an average and reset the counters
            // this lets us track degradation without being overwhelmed by old (stale) data
            self.ping_timing_us /= self.ping_count as u128;
            self.ping_count = 1;
            true
        } else {
            false
        }
    }

    pub(crate) fn job_submitted(&mut self) {
        if !self.enabled {
            return;
        }

        let time_since_last_job = Instant::now() - self.last_job_time;
        // update the job counter
        self.last_job_time = Instant::now();
        self.job_incoming_time_us += time_since_last_job.as_micros();
        self.job_count += 1;

        if self.job_count > 10000 {
            self.job_incoming_time_us /= self.job_count as u128;
            self.job_count = 1;
        }
    }

    pub(crate) fn job_expired(&mut self) {
        if !self.enabled {
            return;
        }
        self.total_num_expired_jobs += 1;
    }

    pub(crate) fn avg_job_qps(&self) -> u128 {
        let us_between_jobs = self.job_incoming_time_us / self.job_count as u128;

        _MICROS_IN_SEC / us_between_jobs
    }

    pub(crate) fn handle_job_done(&mut self, options: &JobOptions) {
        if !self.enabled {
            return;
        }

        self.processed_job_count += 1;
        self.total_processed_job_count += 1;
        self.job_processing_time_us += options.factory_time.elapsed().unwrap().as_micros();
        self.time_in_factory_message_queue_us += options
            .factory_time
            .duration_since(options.submit_time)
            .unwrap()
            .as_micros();

        if self.processed_job_count > 10000 {
            self.job_processing_time_us /= self.processed_job_count as u128;
            self.time_in_factory_message_queue_us /= self.processed_job_count as u128;
            self.processed_job_count = 1;
        }
    }
}

/// Message to a worker
pub enum WorkerMessage<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    /// A ping from the factory. The worker should send a [super::FactoryMessage::WorkerPong] reply
    /// as soon as received, forwarding this instant value to track timing information.
    FactoryPing(Instant),
    /// A job is dispatched to the worker. Once the worker is complete with processing, it should
    /// reply with [super::FactoryMessage::Finished] supplying it's WID and the job key to signify
    /// that the job is completed processing and the worker is available for a new job
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
pub struct WorkerStartContext<TKey, TMsg, TWorker>
where
    TKey: JobKey,
    TMsg: Message,
    TWorker: Actor<Msg = WorkerMessage<TKey, TMsg>, Arguments = Self>,
{
    /// The worker's identifier
    pub wid: WorkerId,

    /// The factory the worker belongs to
    pub factory: ActorRef<Factory<TKey, TMsg, TWorker>>,
}

/// Properties of a worker
pub struct WorkerProperties<TKey, TMsg, TWorker>
where
    TKey: JobKey,
    TMsg: Message,
    TWorker: Actor<Msg = WorkerMessage<TKey, TMsg>>,
{
    /// Worker identifier
    pub(crate) wid: WorkerId,

    /// Worker's capacity for parallel work
    capacity: usize,

    /// Worker actor
    pub(crate) actor: ActorRef<TWorker>,

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
}

impl<TKey, TMsg, TWorker> WorkerProperties<TKey, TMsg, TWorker>
where
    TKey: JobKey,
    TMsg: Message,
    TWorker:
        Actor<Msg = WorkerMessage<TKey, TMsg>, Arguments = WorkerStartContext<TKey, TMsg, TWorker>>,
{
    fn get_next_non_expired_job(&mut self) -> Option<Job<TKey, TMsg>> {
        while let Some(job) = self.message_queue.pop_front() {
            if !job.is_expired() {
                return Some(job);
            } else {
                self.stats.job_expired();
            }
        }
        None
    }

    pub(crate) fn new(
        wid: WorkerId,
        actor: ActorRef<TWorker>,
        capacity: usize,
        discard_threshold: Option<usize>,
        discard_handler: Option<Box<dyn DiscardHandler<TKey, TMsg>>>,
        collect_stats: bool,
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
        nworker: ActorRef<TWorker>,
    ) -> Result<(), ActorProcessingErr> {
        self.actor = nworker;
        if let Some(mut job) = self.get_next_non_expired_job() {
            self.curr_jobs.insert(job.key.clone(), job.options.clone());
            job.options.worker_time = SystemTime::now();
            self.actor.cast(WorkerMessage::Dispatch(job))?;
        }
        Ok(())
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
            log::warn!("Stuck worker: {}. Last jobs:\n{}", self.wid, key_strings);
            true
        } else {
            false
        }
    }

    /// Enqueue a new job to this worker. If the discard threshold has been exceeded
    /// it will discard the oldest elements from the message queue
    pub(crate) fn enqueue_job(&mut self, mut job: Job<TKey, TMsg>) -> Result<(), MessagingErr> {
        // track per-job statistics
        self.stats.job_submitted();

        if self.curr_jobs.len() < self.capacity {
            self.curr_jobs.insert(job.key.clone(), job.options.clone());
            if let Some(mut older_job) = self.get_next_non_expired_job() {
                self.message_queue.push_back(job);
                older_job.options.worker_time = SystemTime::now();
                self.actor.cast(WorkerMessage::Dispatch(older_job))?;
            } else {
                job.options.worker_time = SystemTime::now();
                self.actor.cast(WorkerMessage::Dispatch(job))?;
            }
            return Ok(());
        }
        self.message_queue.push_back(job);
        if let Some(discard_threshold) = self.discard_threshold {
            while discard_threshold > 0 && self.message_queue.len() > discard_threshold {
                if let Some(discarded) = self.get_next_non_expired_job() {
                    if let Some(handler) = &self.discard_handler {
                        handler.discard(discarded);
                    }
                }
            }
        }
        Ok(())
    }

    /// Send a ping to the worker
    pub(crate) fn send_factory_ping(&mut self) -> Result<(), MessagingErr> {
        if !self.is_ping_pending {
            self.is_ping_pending = true;
            self.actor.cast(WorkerMessage::FactoryPing(Instant::now()))
        } else {
            // don't send a new ping if one is currently pending
            Ok(())
        }
    }

    /// Comes back when a ping went out
    pub(crate) fn factory_pong(&mut self, time: Instant) {
        let delta = Instant::now() - time;
        if self.stats.handle_ping(delta) {
            // TODO log metrics ? Should be configurable on the factory level
        }
        self.is_ping_pending = false;
    }

    /// Called when the factory is notified a worker completed a job. Will push the next message
    /// if there is any messages in this worker's queue
    pub(crate) fn worker_complete(
        &mut self,
        key: TKey,
    ) -> Result<Option<JobOptions>, MessagingErr> {
        // remove this pending job
        let options = self.curr_jobs.remove(&key);
        if let Some(job_options) = &options {
            self.stats.handle_job_done(job_options);
        }
        // maybe queue up the next job
        if let Some(mut job) = self.get_next_non_expired_job() {
            job.options.worker_time = SystemTime::now();
            self.actor.cast(WorkerMessage::Dispatch(job))?;
        }

        Ok(options)
    }
}
