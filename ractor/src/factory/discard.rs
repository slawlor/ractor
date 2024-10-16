// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Discard handler managing when jobs are discarded
use super::Job;
use super::JobKey;
use crate::Message;

/// The discard mode of a factory
#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub enum DiscardMode {
    /// Discard oldest incoming jobs under backpressure
    Oldest,
    /// Discard newest incoming jobs under backpressure
    Newest,
}

/// A worker's copy of the discard settings.
///
/// Originally we passed a cloned box of the dynamic calculator to the workers,
/// but what that would do is have every worker re-compute the discard limit
/// which if you have many thousands of workers is kind of useless.
///
/// Instead now we have the workers treat the limit as static, but have the
/// factory compute the limit on it's interval and propagate the limit to the
/// workers. The workers "think" it's static, but the factory handles the dynamics.
/// This way the factory can keep the [DynamicDiscardHandler] as a single, uncloned
/// instance. It also moves NUM_WORKER calculations to 1.
#[derive(Debug)]
pub(crate) enum WorkerDiscardSettings {
    None,
    Static { limit: usize, mode: DiscardMode },
}

impl WorkerDiscardSettings {
    pub(crate) fn update_worker_limit(&mut self, new_limit: usize) {
        if let Self::Static { limit, .. } = self {
            *limit = new_limit;
        }
    }

    pub(crate) fn get_limit_and_mode(&self) -> Option<(usize, DiscardMode)> {
        match self {
            Self::None => None,
            Self::Static { limit, mode, .. } => Some((*limit, *mode)),
        }
    }
}
/// If a factory supports job discarding (loadshedding) it can have a few configurations
/// which are defined in this enum. There is
///
/// 1. No discarding
/// 2. A static queueing limit discarding, with a specific discarding mode
/// 3. A dynamic queueing limit for discards, with a specified discarding mode and init discard limit.
pub enum DiscardSettings {
    /// Don't discard jobs
    None,
    /// A static, immutable limit
    Static {
        /// The current limit. If 0, means jobs will never queue and immediately be discarded
        /// once all workers are busy
        limit: usize,
        /// Define the factory messaging discard mode denoting if the oldest or newest messages
        /// should be discarded in back-pressure scenarios
        ///
        /// Default is [DiscardMode::Oldest], meaning discard jobs at the head of the queue
        mode: DiscardMode,
    },
    /// Dynamic discarding is where the discard limit can change over time, controlled
    /// by the `updater` which is an implementation of the [DynamicDiscardController]
    Dynamic {
        /// The current limit. If 0, means jobs will never queue and immediately be discarded
        /// once all workers are busy
        limit: usize,
        /// Define the factory messaging discard mode denoting if the oldest or newest messages
        /// should be discarded in back-pressure scenarios
        ///
        /// Default is [DiscardMode::Oldest], meaning discard jobs at the head of the queue
        mode: DiscardMode,
        /// The [DynamicDiscardController] implementation, which computes new limits dynamically
        /// based on whatever metrics it wants
        updater: Box<dyn DynamicDiscardController>,
    },
}

impl std::fmt::Debug for DiscardSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiscardSettings::None => {
                write!(f, "DiscardSettings::None")
            }
            DiscardSettings::Static { limit, mode } => f
                .debug_struct("DiscardSettings::Static")
                .field("limit", limit)
                .field("mode", mode)
                .finish(),
            DiscardSettings::Dynamic { limit, mode, .. } => f
                .debug_struct("DiscardSettings::Dynamic")
                .field("limit", limit)
                .field("mode", mode)
                .finish(),
        }
    }
}

impl DiscardSettings {
    pub(crate) fn get_worker_settings(&self) -> WorkerDiscardSettings {
        match &self {
            Self::None => WorkerDiscardSettings::None,
            Self::Static { limit, mode } => WorkerDiscardSettings::Static {
                limit: *limit,
                mode: *mode,
            },
            Self::Dynamic { limit, mode, .. } => WorkerDiscardSettings::Static {
                limit: *limit,
                mode: *mode,
            },
        }
    }

    /// Retrieve the discarding limit and [DiscardMode], if configured
    pub fn get_limit_and_mode(&self) -> Option<(usize, DiscardMode)> {
        match self {
            Self::None => None,
            Self::Static { limit, mode, .. } => Some((*limit, *mode)),
            Self::Dynamic { limit, mode, .. } => Some((*limit, *mode)),
        }
    }
}

/// Controls the dynamic concurrency level by receiving periodic snapshots of job statistics
/// and emitting a new concurrency limit
#[cfg_attr(feature = "async-trait", crate::async_trait)]
pub trait DynamicDiscardController: Send + Sync + 'static {
    /// Compute the new threshold for discarding
    ///
    /// If you want to utilize metrics exposed in [crate::factory::stats] you can gather them
    /// by utilizing `stats_facebook::service_data::get_service_data_singleton` to retrieve a
    /// accessor to `ServiceData` which you can then resolve stats by name (either timeseries or
    /// counters)
    ///
    /// The namespace of stats collected on the base controller factory are
    /// `base_controller.factory.{FACTORY_NAME}.{STAT}`
    ///
    /// If no factory name is set, then "all" will be inserted
    async fn compute(&mut self, current_threshold: usize) -> usize;
}

/// Reason for discarding a job
#[derive(Debug)]
pub enum DiscardReason {
    /// The job TTLd
    TtlExpired,
    /// The job was rejected or dropped due to loadshedding
    Loadshed,
    /// The job was dropped due to factory shutting down
    Shutdown,
}

/// Trait defining the discard handler for a factory.
pub trait DiscardHandler<TKey, TMsg>: Send + Sync + 'static
where
    TKey: JobKey,
    TMsg: Message,
{
    /// Called on a job prior to being dropped from the factory.
    ///
    /// Useful scenarios are (a) collecting metrics, (b) logging, (c) tearing
    /// down resources in the job, etc.
    fn discard(&self, reason: DiscardReason, job: &mut Job<TKey, TMsg>);
}
