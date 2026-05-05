// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Test for processing_messages counter accounting through queue path
//!
//! This test validates that the processing_messages counter is correctly
//! incremented when jobs are dispatched from the queue to workers, not just
//! when they're dispatched directly.

use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::concurrency::{Duration, Instant};
use crate::factory::*;
use crate::periodic_check;
use crate::Actor;
use crate::ActorProcessingErr;
use crate::ActorRef;

const NUM_TEST_WORKERS: usize = 2;

#[derive(Debug, Hash, Clone, Eq, PartialEq)]
struct TestKey {
    id: u64,
}
#[cfg(feature = "cluster")]
impl crate::BytesConvertable for TestKey {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        Self {
            id: u64::from_bytes(bytes),
        }
    }
    fn into_bytes(self) -> Vec<u8> {
        self.id.into_bytes()
    }
}

#[derive(Debug)]
enum TestMessage {
    Ok,
}
#[cfg(feature = "cluster")]
impl crate::Message for TestMessage {}

type DefaultQueue = crate::factory::queues::DefaultQueue<TestKey, TestMessage>;

struct TestWorker {
    counter: Arc<AtomicU16>,
    slow: Option<u64>,
}

#[cfg_attr(feature = "async-trait", crate::async_trait)]
impl Worker for TestWorker {
    type Key = TestKey;
    type Message = TestMessage;
    type State = ();
    type Arguments = ();

    async fn pre_start(
        &self,
        _wid: WorkerId,
        _factory: &ActorRef<FactoryMessage<TestKey, TestMessage>>,
        startup_context: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(startup_context)
    }

    async fn handle(
        &self,
        _wid: WorkerId,
        _factory: &ActorRef<FactoryMessage<TestKey, TestMessage>>,
        Job { key, .. }: Job<Self::Key, Self::Message>,
        _state: &mut Self::State,
    ) -> Result<TestKey, ActorProcessingErr> {
        self.counter.fetch_add(1, Ordering::Relaxed);

        if let Some(timeout_ms) = self.slow {
            crate::concurrency::sleep(Duration::from_millis(timeout_ms)).await;
        }

        Ok(key)
    }
}

struct SlowTestWorkerBuilder {
    counters: [Arc<AtomicU16>; NUM_TEST_WORKERS],
}

impl WorkerBuilder<TestWorker, ()> for SlowTestWorkerBuilder {
    fn build(&mut self, wid: usize) -> (TestWorker, ()) {
        (
            TestWorker {
                counter: self.counters[wid].clone(),
                // Make workers slow enough to guarantee queueing even in fast CI environments
                slow: Some(200),
            },
            (),
        )
    }
}

/// Test stats layer that captures processing_messages counts
#[derive(Clone)]
struct ProcessingMessagesStatsLayer {
    recorded_counts: Arc<Mutex<Vec<usize>>>,
    max_count: Arc<AtomicUsize>,
}

impl ProcessingMessagesStatsLayer {
    fn new() -> Self {
        Self {
            recorded_counts: Arc::new(Mutex::new(Vec::new())),
            max_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn get_recorded_counts(&self) -> Vec<usize> {
        self.recorded_counts.lock().unwrap().clone()
    }

    fn get_max_count(&self) -> usize {
        self.max_count.load(Ordering::Relaxed)
    }
}

impl FactoryStatsLayer for ProcessingMessagesStatsLayer {
    fn factory_ping_received(&self, _factory: &str, _sent: Instant) {}
    fn worker_ping_received(&self, _factory: &str, _elapsed: Duration) {}
    fn new_job(&self, _factory: &str) {}
    fn job_completed(&self, _factory: &str, _options: &JobOptions) {}
    fn job_discarded(&self, _factory: &str) {}
    fn job_rate_limited(&self, _factory: &str) {}
    fn job_ttl_expired(&self, _factory: &str, _num_removed: usize) {}
    fn record_queue_depth(&self, _factory: &str, _depth: usize) {}
    fn record_worker_count(&self, _factory: &str, _count: usize) {}
    fn record_queue_limit(&self, _factory: &str, _count: usize) {}

    fn record_processing_messages_count(&self, _factory: &str, count: usize) {
        self.recorded_counts.lock().unwrap().push(count);

        // Track the maximum we've seen
        let mut current_max = self.max_count.load(Ordering::Relaxed);
        while count > current_max {
            match self.max_count.compare_exchange(
                current_max,
                count,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(x) => current_max = x,
            }
        }
    }

    fn record_in_flight_messages_count(&self, _factory: &str, _count: usize) {}
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_processing_messages_accounting_through_queue() {
    let worker_counters: [_; NUM_TEST_WORKERS] =
        [Arc::new(AtomicU16::new(0)), Arc::new(AtomicU16::new(0))];

    let worker_builder = SlowTestWorkerBuilder {
        counters: worker_counters.clone(),
    };

    let stats_layer = ProcessingMessagesStatsLayer::new();
    let stats_clone = stats_layer.clone();

    let factory_definition = Factory::<
        TestKey,
        TestMessage,
        (),
        TestWorker,
        routing::QueuerRouting<TestKey, TestMessage>,
        DefaultQueue,
    >::default();

    let (factory, factory_handle) = Actor::spawn(
        None,
        factory_definition,
        FactoryArguments {
            num_initial_workers: NUM_TEST_WORKERS,
            queue: DefaultQueue::default(),
            router: Default::default(),
            capacity_controller: None,
            dead_mans_switch: None,
            discard_handler: None,
            discard_settings: DiscardSettings::None,
            lifecycle_hooks: None,
            worker_builder: Box::new(worker_builder),
            stats: Some(Arc::new(stats_layer)),
        },
    )
    .await
    .expect("Failed to spawn factory");

    // Dispatch a burst of jobs that will queue up
    // Using fewer jobs but with very slow workers to guarantee queueing
    let num_jobs: u16 = 10;
    for i in 0..num_jobs {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                key: TestKey { id: i as u64 },
                msg: TestMessage::Ok,
                options: JobOptions::default(),
                accepted: None,
            }))
            .expect("Failed to send to factory");
    }

    // Wait for all jobs to complete
    let check_counters = [worker_counters[0].clone(), worker_counters[1].clone()];
    periodic_check(
        move || {
            let total = check_counters[0].load(Ordering::Relaxed)
                + check_counters[1].load(Ordering::Relaxed);
            total == num_jobs
        },
        Duration::from_secs(10),
    )
    .await;

    // Wait explicitly for processing_messages to reach 0 instead of using a fixed sleep
    // This is more deterministic and works regardless of metric calculation timing
    let stats_check = stats_clone.clone();
    periodic_check(
        move || {
            let counts = stats_check.get_recorded_counts();
            counts.last().map_or(false, |&c| c == 0)
        },
        Duration::from_secs(2),
    )
    .await;

    factory.stop(None);
    factory_handle.await.unwrap();

    // Verify the processing_messages counter behavior
    let recorded_counts = stats_clone.get_recorded_counts();
    let max_count = stats_clone.get_max_count();

    println!("Recorded processing_messages counts: {:?}", recorded_counts);
    println!("Max processing_messages count: {}", max_count);
    println!(
        "Total jobs processed: {}",
        worker_counters[0].load(Ordering::Relaxed) + worker_counters[1].load(Ordering::Relaxed)
    );

    // Core invariants that must hold regardless of timing:

    // 1. All jobs must complete
    assert_eq!(
        worker_counters[0].load(Ordering::Relaxed) + worker_counters[1].load(Ordering::Relaxed),
        num_jobs,
        "all jobs should complete"
    );

    // 2. The max processing_messages should NEVER exceed worker count
    // This is the critical invariant - without the fix, this could theoretically exceed
    // worker count if the accounting was completely broken
    assert!(
        max_count <= NUM_TEST_WORKERS,
        "processing_messages should never exceed worker count, got {}",
        max_count
    );

    // 3. The processing_messages should have been > 0 at some point
    // (proves the counter was actually incremented during processing)
    assert!(
        max_count > 0,
        "processing_messages should have been > 0 during processing"
    );

    // 4. The final processing_messages count should be 0 (all jobs completed and counter drained)
    // We already waited for this explicitly, but verify it
    assert_eq!(
        *recorded_counts.last().unwrap_or(&usize::MAX),
        0,
        "processing_messages should be 0 after all jobs complete"
    );

    // 5. We should never see the counter "stuck" at 0 while jobs are still being processed.
    // Without the fix, the counter would trend to 0 prematurely and stay there.
    // Count consecutive zeros in the middle of processing (ignoring final zeros).
    let total_samples = recorded_counts.len();
    if total_samples > 2 {
        // Look at samples excluding the first (might be 0 initially) and last few (should be 0 at end)
        let middle_samples = &recorded_counts[1..total_samples.saturating_sub(2)];
        let consecutive_zeros = middle_samples
            .windows(2)
            .filter(|w| w[0] == 0 && w[1] == 0)
            .count();

        // With the fix, we shouldn't see prolonged consecutive zeros in the middle of processing
        // Allow a small number for race conditions, but not the majority
        assert!(
            consecutive_zeros < middle_samples.len() / 2,
            "processing_messages should not be stuck at 0 during active processing (saw {} consecutive zero pairs out of {} middle samples)",
            consecutive_zeros,
            middle_samples.len()
        );
    }
}
