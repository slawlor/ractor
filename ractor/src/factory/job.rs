// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Specification for a job sent to a factory

use std::hash::Hash;

use crate::{
    concurrency::{Duration, Instant},
    Message,
};

/// Represents options for the specified job
pub struct JobOptions {
    /// Time job was submitted from the client
    pub submit_time: Instant,
    /// Time job was processed by the factory
    pub factory_time: Instant,
    /// Time-to-live for the job
    pub ttl: Duration,
}

/// Represents a job sent to a factory
pub struct Job<TKey, TMsg>
where
    TKey: Message + Hash + Sync,
    TMsg: Message,
{
    /// The key of the job
    pub key: TKey,
    /// The message of the job
    pub msg: TMsg,
    /// The options of the job
    pub options: JobOptions,
}

#[cfg(feature = "cluster")]
impl<TKey, TMsg> Message for Job<TKey, TMsg>
where
    TKey: Message + Hash + Sync,
    TMsg: Message,
{
}

impl<TKey, TMsg> Job<TKey, TMsg>
where
    TKey: Message + Hash + Sync,
    TMsg: Message,
{
    pub(crate) fn is_expired(&self) -> bool {
        Instant::now() - self.options.submit_time > self.options.ttl
    }

    pub(crate) fn set_factory_time(&mut self) {
        self.options.factory_time = Instant::now()
    }
}
