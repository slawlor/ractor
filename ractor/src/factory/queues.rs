// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Queue implementations for Factories

use std::collections::VecDeque;
use std::marker::PhantomData;
use std::sync::Arc;

use crate::factory::DiscardHandler;
use crate::factory::DiscardReason;
use crate::factory::Job;
use crate::factory::JobKey;
use crate::Message;

/// Implementation of backing queue for factory messages when workers are
/// all busy
pub trait Queue<TKey, TMsg>: Send + 'static
where
    TKey: JobKey,
    TMsg: Message,
{
    /// Retrieve the size of the factory's queue
    fn len(&self) -> usize;

    /// Check if the queue is empty
    fn is_empty(&self) -> bool;

    /// Pop the next message from the front of the queue
    fn pop_front(&mut self) -> Option<Job<TKey, TMsg>>;

    /// Try and discard a message according to the queue semantics
    /// in an overload scenario (e.g. lowest priority if priority
    /// queueing). In a basic queueing scenario, this is equivalent
    /// to `pop_front`
    fn discard_oldest(&mut self) -> Option<Job<TKey, TMsg>>;

    /// Peek an item from the head of the queue
    fn peek(&self) -> Option<&Job<TKey, TMsg>>;

    /// Push an item to the back of the queue
    fn push_back(&mut self, job: Job<TKey, TMsg>);

    /// Remove expired items from the queue
    ///
    /// * `discard_handler` - The handler to call for each discarded job. Will be called
    /// with [DiscardReason::Loadshed].
    ///
    /// Returns the number of elements removed from the queue
    fn remove_expired_items(
        &mut self,
        discard_handler: &Option<Arc<dyn DiscardHandler<TKey, TMsg>>>,
    ) -> usize;

    /// Determine if a given job can be discarded. Default is [true] for all jobs.
    ///
    /// This can be overridden to customize discard semantics.
    fn is_job_discardable(&self, _key: &TKey) -> bool {
        true
    }
}

/// Priority trait which denotes the usize value of a [Priority]
pub trait Priority: Default + From<usize> + Send + 'static {
    /// Retrieve the index for the Priority value. This should be
    /// contiguous from 0, 0 being the highest priority.
    fn get_index(&self) -> usize;
}

/// Basic 5-category priority definition. This is probably flexible enough
/// for most use-cases
#[derive(strum::FromRepr, Default, Debug, Clone, Copy, Eq, PartialEq, Hash)]
#[repr(usize)]
pub enum StandardPriority {
    /// Most important
    Highest = 0,
    /// High
    High = 1,
    /// Important
    Important = 2,
    /// Normal
    #[default]
    Normal = 3,
    /// Low/best-effort priority
    BestEffort = 4,
}

#[cfg(feature = "cluster")]
impl crate::BytesConvertable for StandardPriority {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        (u64::from_bytes(bytes) as usize).into()
    }
    fn into_bytes(self) -> Vec<u8> {
        (self as u64).into_bytes()
    }
}

impl StandardPriority {
    /// Retrieve the number of variants of this enum, as a constant
    pub const fn size() -> usize {
        5
    }
}

impl Priority for StandardPriority {
    fn get_index(&self) -> usize {
        *self as usize
    }
}

impl From<usize> for StandardPriority {
    fn from(value: usize) -> Self {
        Self::from_repr(value).unwrap_or_default()
    }
}

/// The [PriorityManager] is responsible for extracting the job priority from
/// a given job's key (`TKey`). Additionally in some scenarios  some jobs may be non-discardable,
/// i.e. can be enqueued regardless of the backpressure status of the factory. This is also
/// responsible for determining if a job can be loadshed.
pub trait PriorityManager<TKey, TPriority>: Send + Sync + 'static
where
    TKey: JobKey,
    TPriority: Priority,
{
    /// Determine if this job can be discarded under load.
    ///
    /// Returns [true] if the job can be discarded, [false] otherwise.
    fn is_discardable(&self, job: &TKey) -> bool;

    /// Retrieve the job's priority.
    ///
    /// Returns [None] if the job does not have a priority, [Some(`TPriority`)] otherwise.
    fn get_priority(&self, job: &TKey) -> Option<TPriority>;
}

// =============== Default Queue ================= //
/// A simple, no-priority queue
///
/// Equivalent to a [VecDeque]
pub struct DefaultQueue<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    q: VecDeque<Job<TKey, TMsg>>,
}

impl<TKey, TMsg> Default for DefaultQueue<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    fn default() -> Self {
        Self { q: VecDeque::new() }
    }
}

impl<TKey, TMsg> Queue<TKey, TMsg> for DefaultQueue<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    /// Retrieve the size of the factory's queue
    fn len(&self) -> usize {
        self.q.len()
    }

    /// Check if the queue is empty
    fn is_empty(&self) -> bool {
        self.q.is_empty()
    }

    /// Pop the next message from the front of the queue
    fn pop_front(&mut self) -> Option<Job<TKey, TMsg>> {
        self.q.pop_front()
    }

    fn discard_oldest(&mut self) -> Option<Job<TKey, TMsg>> {
        self.pop_front()
    }

    fn peek(&self) -> Option<&Job<TKey, TMsg>> {
        self.q.front()
    }

    /// Push an item to the back of the queue, with the given priority
    fn push_back(&mut self, job: Job<TKey, TMsg>) {
        self.q.push_back(job)
    }

    /// Remove expired items from the queue
    fn remove_expired_items(
        &mut self,
        discard_handler: &Option<Arc<dyn DiscardHandler<TKey, TMsg>>>,
    ) -> usize {
        let before = self.q.len();
        // scan backlog for expired jobs and pop, discard, and drop them
        self.q.retain_mut(|queued_item| {
            if queued_item.is_expired() {
                if let Some(handler) = discard_handler {
                    handler.discard(DiscardReason::TtlExpired, queued_item);
                }
                false
            } else {
                true
            }
        });
        self.q.len() - before
    }
}

// =============== Priority Queue ================= //
/// A queue with `NUM_PRIORITIES` priorities
///
/// It requires a [PriorityManager] implementation associated with it in order to
/// determine the priorities of given jobs and inform discard semantics.
pub struct PriorityQueue<TKey, TMsg, TPriority, TPriorityManager, const NUM_PRIORITIES: usize>
where
    TKey: JobKey,
    TMsg: Message,
    TPriority: Priority,
    TPriorityManager: PriorityManager<TKey, TPriority>,
{
    queues: [VecDeque<Job<TKey, TMsg>>; NUM_PRIORITIES],
    priority_manager: TPriorityManager,
    _p: PhantomData<fn() -> TPriority>,
}

impl<TKey, TMsg, TPriority, TPriorityManager, const NUM_PRIORITIES: usize>
    PriorityQueue<TKey, TMsg, TPriority, TPriorityManager, NUM_PRIORITIES>
where
    TKey: JobKey,
    TMsg: Message,
    TPriority: Priority,
    TPriorityManager: PriorityManager<TKey, TPriority>,
{
    /// Construct a new [PriorityQueue] instance with the supplied [PriorityManager]
    /// implementation.
    pub fn new(priority_manager: TPriorityManager) -> Self {
        Self {
            _p: PhantomData,
            priority_manager,
            queues: [(); NUM_PRIORITIES].map(|_| VecDeque::new()),
        }
    }
}

impl<TKey, TMsg, TPriority, TPriorityManager, const NUM_PRIORITIES: usize> Queue<TKey, TMsg>
    for PriorityQueue<TKey, TMsg, TPriority, TPriorityManager, NUM_PRIORITIES>
where
    TKey: JobKey,
    TMsg: Message,
    TPriority: Priority,
    TPriorityManager: PriorityManager<TKey, TPriority>,
{
    /// Retrieve the size of the factory's queue
    fn len(&self) -> usize {
        self.queues.iter().map(|q| q.len()).sum()
    }

    /// Check if the queue is empty
    fn is_empty(&self) -> bool {
        self.queues.iter().all(|q| q.is_empty())
    }

    /// Pop the next message from the front of the queue
    fn pop_front(&mut self) -> Option<Job<TKey, TMsg>> {
        for i in 0..NUM_PRIORITIES {
            if let Some(r) = self.queues[i].pop_front() {
                return Some(r);
            }
        }
        None
    }

    fn discard_oldest(&mut self) -> Option<Job<TKey, TMsg>> {
        for i in (0..NUM_PRIORITIES).rev() {
            if let Some(r) = self.queues[i].pop_front() {
                return Some(r);
            }
        }
        None
    }

    fn peek(&self) -> Option<&Job<TKey, TMsg>> {
        for i in 0..NUM_PRIORITIES {
            let maybe = self.queues[i].front();
            if maybe.is_some() {
                return maybe;
            }
        }
        None
    }

    /// Push an item to the back of the queue
    fn push_back(&mut self, job: Job<TKey, TMsg>) {
        let priority = self
            .priority_manager
            .get_priority(&job.key)
            .unwrap_or_else(Default::default);
        let idx = priority.get_index();
        self.queues[idx].push_back(job);
    }

    /// Remove expired items from the queue
    fn remove_expired_items(
        &mut self,
        discard_handler: &Option<Arc<dyn DiscardHandler<TKey, TMsg>>>,
    ) -> usize {
        let mut num_removed = 0;

        // scan backlog for expired jobs and pop, discard, and drop them
        for i in 0..NUM_PRIORITIES {
            self.queues[i].retain_mut(|queued_item| {
                if queued_item.is_expired() {
                    if let Some(handler) = discard_handler {
                        handler.discard(DiscardReason::TtlExpired, queued_item);
                    }
                    num_removed += 1;
                    false
                } else {
                    true
                }
            });
        }
        num_removed
    }

    fn is_job_discardable(&self, key: &TKey) -> bool {
        self.priority_manager.is_discardable(key)
    }
}
