// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Statistics management + collection for factories

use std::fmt::Display;
use std::time::SystemTime;

use crate::concurrency::Instant;
use crate::factory::JobOptions;

const MICROS_IN_SEC: u128 = 1000000;
const RESET_PINGS_AFTER: u64 = 600;

/// Messaging statistics collected on a factory and/or
/// a worker
#[derive(Clone)]
pub struct MessageProcessingStats {
    // ========== Pings ========== //
    /// number of pings
    pub ping_count: u64,
    /// Running sum of ping time
    pub ping_timing_us: u128,
    /// The time of the last ping (workers only)
    pub last_ping: Instant,
    // ========== Incoming Job QPS ========== //
    /// The time a last job came through
    pub last_job_time: Instant,
    /// Job count
    pub job_count: u64,
    /// Incoming job time
    pub job_incoming_time_us: u128,
    // ========== Finished jobs (factory-only) ========== //
    /// Job processed count
    pub processed_job_count: u64,
    /// Total processed job count
    pub total_processed_job_count: u128,
    /// Factory's processing time for jobs (factory initial handling -> job done)
    pub factory_processing_latency_usec: u128,
    /// Worker's processing time for jobs (worker initial handling -> job done)
    pub worker_processing_latency_usec: u128,
    /// Overall job latency (submission time -> job done)
    pub job_processing_latency_usec: u128,
    // ========== Expired jobs ========== //
    /// Number of expired jobs
    pub total_num_expired_jobs: u128,
    // ========== Discarded jobs ========== //
    /// Number of discarded jobs
    pub total_num_discarded_jobs: u128,

    /// Stats enabled
    pub enabled: bool,
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
Num discarded jobs: {}
            ",
            if self.ping_count > 0 {
                self.ping_timing_us / self.ping_count as u128
            } else {
                0
            },
            self.total_processed_job_count,
            self.avg_job_qps(),
            if self.processed_job_count > 0 {
                self.job_processing_latency_usec / self.processed_job_count as u128
            } else {
                0
            },
            if self.processed_job_count > 0 {
                self.factory_processing_latency_usec / self.processed_job_count as u128
            } else {
                0
            },
            self.total_num_expired_jobs,
            self.total_num_discarded_jobs,
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
            job_processing_latency_usec: 0,
            processed_job_count: 0,
            total_processed_job_count: 0,
            factory_processing_latency_usec: 0,
            total_num_expired_jobs: 0,
            total_num_discarded_jobs: 0,
            worker_processing_latency_usec: 0,
            enabled: false,
        }
    }
}

impl MessageProcessingStats {
    pub(crate) fn enable(&mut self) {
        self.enabled = true;
    }

    pub(crate) fn reset_global_counters(&mut self) {
        self.total_num_discarded_jobs = 0;
        self.total_num_expired_jobs = 0;
        self.total_processed_job_count = 0;
    }

    /// Handle a factory ping, and every 10 minutes adjust the stats to the
    /// average + return a flag to state that we reset the ping counter (every RESET_PINGS_AFTER pings)
    pub(crate) fn ping_received(&mut self, sent_when: Instant) -> bool {
        self.last_ping = Instant::now();
        if self.enabled {
            let duration = self.last_ping - sent_when;
            self.ping_count += 1;
            self.ping_timing_us += duration.as_micros();
            if self.ping_count > RESET_PINGS_AFTER {
                // When we hit 10min, convert the message timing to an average and reset the counters
                // this lets us track degradation without being overwhelmed by old (stale) data
                if self.ping_count > 0 {
                    self.ping_timing_us /= self.ping_count as u128;
                }

                self.ping_count = 1;
                return true;
            }
        }
        false
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

    pub(crate) fn job_ttl_expired(&mut self) {
        if !self.enabled {
            return;
        }
        self.total_num_expired_jobs += 1;
    }

    pub(crate) fn job_discarded(&mut self) {
        if !self.enabled {
            return;
        }
        self.total_num_discarded_jobs += 1;
    }

    pub(crate) fn avg_job_qps(&self) -> u128 {
        if self.job_count > 0 {
            let us_between_jobs = self.job_incoming_time_us / self.job_count as u128;

            if us_between_jobs > 0 {
                MICROS_IN_SEC / us_between_jobs
            } else {
                0
            }
        } else {
            0
        }
    }

    /// Called each time a job is completed to report factory processing time, worker processing time, total processing time
    ///
    /// From these metrics you can derive
    /// 1. Time in factory's queue = factory_job_processing_latency_usec - worker_job_processing_latency_usec
    /// 2. Time in worker's queue + being processed by worker = worker_job_processing_latency_usec
    /// 3. Total time since submission = job_processing_latency_usec
    ///
    /// NOTE: Depending on selected routing metric, these *could* be 0 in some instances (i.e. a key-persistent
    /// routing factory will have near-0 time in the factory's queue since it doesn't exist)
    pub(crate) fn factory_job_done(&mut self, options: &JobOptions) {
        if !self.enabled {
            return;
        }

        self.processed_job_count += 1;
        self.total_processed_job_count += 1;

        let now = SystemTime::now();
        let duration = now
            .duration_since(options.factory_time)
            .unwrap()
            .as_micros();
        self.factory_processing_latency_usec += duration;

        let duration = now.duration_since(options.worker_time).unwrap().as_micros();
        self.worker_processing_latency_usec += duration;

        let duration = now.duration_since(options.submit_time).unwrap().as_micros();
        self.job_processing_latency_usec += duration;

        if self.processed_job_count > 10000 {
            self.job_processing_latency_usec /= self.processed_job_count as u128;
            self.factory_processing_latency_usec /= self.processed_job_count as u128;
            self.processed_job_count = 1;
        }
    }
}
