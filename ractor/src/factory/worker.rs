// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Factory worker properties

use std::collections::VecDeque;
use std::hash::Hash;

use crate::ActorId;
use crate::{Actor, ActorRef, Message, MessagingErr};

use super::DiscardHandler;
use super::Factory;
use super::Job;

// TODO:
// 1. Worker stats?

/// Startup context data (`Arguments`) which are passed to a worker on start
pub struct WorkerStartContext<TKey, TMsg, TWorker>
where
    TKey: Message + Hash + Sync,
    TMsg: Message,
    TWorker: Actor<Msg = Job<TKey, TMsg>, Arguments = Self>,
{
    /// The worker's identifier
    pub wid: usize,

    /// The factory the worker belongs to
    pub factory: ActorRef<Factory<TKey, TMsg, TWorker>>,
}

/// Properties of a worker
pub struct WorkerProperties<TKey, TMsg, TWorker>
where
    TKey: Message + Hash + Sync,
    TMsg: Message,
    TWorker: Actor<Msg = Job<TKey, TMsg>>,
{
    /// Worker identifier
    pub wid: usize,

    /// Worker's capacity for parallel work
    capacity: usize,

    /// Worker actor
    actor: ActorRef<TWorker>,

    /// Worker's message queue
    message_queue: VecDeque<Job<TKey, TMsg>>,
    /// Maximum queue length. Any job arriving when the queue is at its max length
    /// will cause an oldest job at the head of the queue will be dropped.
    ///
    /// Default is disabled
    discard_threshold: Option<usize>,

    /// A function to be called for each job to be dropped.
    discard_handler: Option<Box<dyn DiscardHandler<TKey, TMsg>>>,

    /// The current message count being handled by this worker
    current_count: usize,
}

impl<TKey, TMsg, TWorker> WorkerProperties<TKey, TMsg, TWorker>
where
    TKey: Message + Hash + Sync,
    TMsg: Message,
    TWorker: Actor<Msg = Job<TKey, TMsg>, Arguments = WorkerStartContext<TKey, TMsg, TWorker>>,
{
    fn get_next_non_expired_job(&mut self) -> Option<Job<TKey, TMsg>> {
        while let Some(job) = self.message_queue.pop_front() {
            if !job.is_expired() {
                return Some(job);
            }
        }
        None
    }

    pub(crate) fn new(
        wid: usize,
        actor: ActorRef<TWorker>,
        capacity: usize,
        discard_threshold: Option<usize>,
        discard_handler: Option<Box<dyn DiscardHandler<TKey, TMsg>>>,
    ) -> Self {
        Self {
            actor,
            discard_handler,
            discard_threshold,
            message_queue: VecDeque::new(),
            current_count: 0,
            wid,
            capacity,
        }
    }

    pub(crate) fn is_pid(&self, pid: ActorId) -> bool {
        self.actor.get_id() == pid
    }

    pub(crate) fn replace_worker(&mut self, nworker: ActorRef<TWorker>) {
        self.actor = nworker;
    }

    pub(crate) fn is_available(&self) -> bool {
        self.current_count < self.capacity
    }

    /// Enqueue a new job to this worker. If the discard threshold has been exceeded
    /// it will discard the oldest elements from the message queue
    pub(crate) fn enqueue_job(&mut self, job: Job<TKey, TMsg>) -> Result<(), MessagingErr> {
        if self.current_count < self.capacity {
            self.current_count += 1;
            if let Some(older_job) = self.get_next_non_expired_job() {
                self.message_queue.push_back(job);
                self.actor.cast(older_job)?;
            } else {
                self.actor.cast(job)?;
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

    /// Called when the factory is notified a worker completed a job. Will push the next message
    /// if there is any messages in this worker's queue
    pub(crate) fn worker_complete(&mut self, _key: TKey) -> Result<(), MessagingErr> {
        if let Some(job) = self.get_next_non_expired_job() {
            self.actor.cast(job)?;
        } else {
            self.current_count -= 1;
        }
        Ok(())
    }
}
