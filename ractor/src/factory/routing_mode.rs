// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Factory routing mode

/// Custom hashing behavior for factory routing to workers
pub trait CustomHashFunction<TKey>: Send + Sync
where
    TKey: Send + Sync + 'static,
{
    /// Hash the key into the space 0..usize
    fn hash(&self, key: &TKey, worker_count: usize) -> usize;
}

/// Routing mode for jobs through the factory to workers
pub enum RoutingMode<TKey>
where
    TKey: Send + Sync + 'static,
{
    /// Factory will select worker by hashing the job's key.
    /// Workers will have jobs placed into their incoming message queue's
    KeyPersistent,

    /// Factory will dispatch job to first available worker.
    /// Factory will maintain shared internal queue of messages
    Queuer,

    /// Factory will dispatch jobs to a worker that is processing the same key (if any).
    /// Factory will maintain shared internal queue of messages
    StickyQueuer,

    /// Factory will dispatch to the next worker in order
    RoundRobin,

    /// Factory will dispatch to a worker in a random order
    Random,

    /// Similar to [RoutingMode::KeyPersistent] but with a custom hash function
    CustomHashFunction(Box<dyn CustomHashFunction<TKey>>),
}

impl<TKey> Default for RoutingMode<TKey>
where
    TKey: Send + Sync + 'static,
{
    fn default() -> Self {
        Self::KeyPersistent
    }
}
