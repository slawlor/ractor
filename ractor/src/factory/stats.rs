// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Statistics management + collection for factories

use std::sync::Arc;

use crate::concurrency::{Duration, Instant};
use crate::factory::JobOptions;

/// A wrapper over whatever stats collection a user wishes to utilize
/// in the [super::Factory].
pub trait FactoryStatsLayer: Send + Sync + 'static {
    /// Called when a factory ping has been received, marking the duration
    /// between when it was sent and now.
    ///
    /// Measures ping latency on the factory
    fn factory_ping_received(&self, factory: &str, sent: Instant);

    /// Called when a worker replies to a ping request from the factory,
    /// measuring the duration between the time the ping was sent and the
    /// factory processed the ping response.
    ///
    /// Measures worker "free" latency and this metric relates to identification
    /// of "stuck" or slow workers.
    fn worker_ping_received(&self, factory: &str, elapsed: Duration);

    /// Called for each new incoming job
    fn new_job(&self, factory: &str);

    /// Called when a job is completed to report factory processing time,
    /// worker processing time, total processing time, and job count
    ///
    /// From these metrics you can derive
    /// 1. Time in factory's queue = factory_job_processing_latency_usec - worker_job_processing_latency_usec
    /// 2. Time in worker's queue + being processed by worker = worker_job_processing_latency_usec
    /// 3. Total time since submission = job_processing_latency_usec
    fn job_completed(&self, factory: &str, options: &JobOptions);

    /// Called when a job is discarded
    fn job_discarded(&self, factory: &str);

    /// Called when jobs TTL timeout in the factory's queue
    fn job_ttl_expired(&self, factory: &str, num_removed: usize);

    /// Fixed-period recording of the factory's queue depth
    fn record_queue_depth(&self, factory: &str, depth: usize);

    /// Fixed-period recording of the factory's number of processed messages
    fn record_processing_messages_count(&self, factory: &str, count: usize);

    /// Fixed-period recording of the factory's in-flight message count (processing + queued)
    ///
    /// Default empty implemention for backwards compatibility
    #[allow(unused_variables)]
    fn record_in_flight_messages_count(&self, factory: &str, count: usize) {}

    /// Fixed-period recording of the factory's number of workers
    fn record_worker_count(&self, factory: &str, count: usize);

    /// Fixed-period recording of the factory's maximum allowed queue size
    fn record_queue_limit(&self, factory: &str, count: usize);
}

impl FactoryStatsLayer for Option<Arc<dyn FactoryStatsLayer>> {
    /// Called when a factory ping has been received, marking the duration
    /// between when it was sent and now.
    ///
    /// Measures ping latency on the factory
    fn factory_ping_received(&self, factory: &str, sent: Instant) {
        if let Some(s) = self {
            s.factory_ping_received(factory, sent);
        }
    }

    /// Called when a worker replies to a ping request from the factory,
    /// measuring the duration between the time the ping was sent and the
    /// factory processed the ping response.
    ///
    /// Measures worker "free" latency and this metric relates to identification
    /// of "stuck" or slow workers.
    fn worker_ping_received(&self, factory: &str, elapsed: Duration) {
        if let Some(s) = self {
            s.worker_ping_received(factory, elapsed);
        }
    }

    /// Called for each new incoming job
    fn new_job(&self, factory: &str) {
        if let Some(s) = self {
            s.new_job(factory);
        }
    }

    /// Called when a job is completed to report factory processing time,
    /// worker processing time, total processing time, and job count
    ///
    /// From these metrics you can derive
    /// 1. Time in factory's queue = factory_job_processing_latency_usec - worker_job_processing_latency_usec
    /// 2. Time in worker's queue + being processed by worker = worker_job_processing_latency_usec
    /// 3. Total time since submission = job_processing_latency_usec
    fn job_completed(&self, factory: &str, options: &JobOptions) {
        if let Some(s) = self {
            s.job_completed(factory, options);
        }
    }

    /// Called when a job is discarded
    fn job_discarded(&self, factory: &str) {
        if let Some(s) = self {
            s.job_discarded(factory);
        }
    }

    /// Called when a job TTLs in the factory's queue
    fn job_ttl_expired(&self, factory: &str, num_removed: usize) {
        if let Some(s) = self {
            s.job_ttl_expired(factory, num_removed);
        }
    }

    /// Fixed-period recording of the factory's queue depth
    fn record_queue_depth(&self, factory: &str, depth: usize) {
        if let Some(s) = self {
            s.record_queue_depth(factory, depth);
        }
    }

    /// Fixed-period recording of the factory's number of processed messages
    fn record_processing_messages_count(&self, factory: &str, count: usize) {
        if let Some(s) = self {
            s.record_processing_messages_count(factory, count);
        }
    }

    /// Fixed-period recording of the factory's in-flight message count (processing + queued)
    fn record_in_flight_messages_count(&self, factory: &str, count: usize) {
        if let Some(s) = self {
            s.record_in_flight_messages_count(factory, count);
        }
    }

    /// Fixed-period recording of the factory's number of workers
    fn record_worker_count(&self, factory: &str, count: usize) {
        if let Some(s) = self {
            s.record_worker_count(factory, count);
        }
    }

    /// Fixed-period recording of the factory's maximum allowed queue size
    fn record_queue_limit(&self, factory: &str, count: usize) {
        if let Some(s) = self {
            s.record_queue_limit(factory, count);
        }
    }
}
