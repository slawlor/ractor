// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

#[cfg(not(feature = "async-trait"))]
use futures::{future::BoxFuture, FutureExt};
use std::sync::atomic::AtomicU16;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::concurrency::Duration;
use crate::factory::*;
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
    /// Doh'k
    #[allow(dead_code)]
    Count(u16),
}
#[cfg(feature = "cluster")]
impl crate::Message for TestMessage {}

struct TestWorker {
    counter: Arc<AtomicU16>,
    slow: Option<u64>,
}

#[cfg_attr(feature = "async-trait", crate::async_trait)]
impl Actor for TestWorker {
    type Msg = WorkerMessage<TestKey, TestMessage>;
    type State = Self::Arguments;
    type Arguments = WorkerStartContext<TestKey, TestMessage, ()>;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        startup_context: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(startup_context)
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            WorkerMessage::FactoryPing(time) => {
                state
                    .factory
                    .cast(FactoryMessage::WorkerPong(state.wid, time.elapsed()))?;
            }
            WorkerMessage::Dispatch(job) => {
                tracing::debug!("Worker received {:?}", job.msg);

                self.counter.fetch_add(1, Ordering::Relaxed);

                if let Some(timeout_ms) = self.slow {
                    crate::concurrency::sleep(Duration::from_millis(timeout_ms)).await;
                }

                // job finished, on success or err we report back to the factory
                state
                    .factory
                    .cast(FactoryMessage::Finished(state.wid, job.key))?;
            }
        }
        Ok(())
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
                slow: Some(10),
            },
            (),
        )
    }
}

struct TestDiscarder {
    counter: Arc<AtomicU16>,
}
impl DiscardHandler<TestKey, TestMessage> for TestDiscarder {
    fn discard(&self, _reason: DiscardReason, job: &mut Job<TestKey, TestMessage>) {
        let TestMessage::Count(count) = job.msg;
        let _ = self.counter.fetch_add(count, Ordering::Relaxed);
    }
}

struct DiscardController {}

#[cfg_attr(feature = "async-trait", crate::async_trait)]
impl DynamicDiscardController for DiscardController {
    #[cfg(feature = "async-trait")]
    async fn compute(&mut self, _current_threshold: usize) -> usize {
        10
    }

    #[cfg(not(feature = "async-trait"))]
    fn compute(&mut self, _current_threshold: usize) -> BoxFuture<'_, usize> {
        async { 10 }.boxed()
    }
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_dynamic_dispatch_basic() {
    // let handle = tokio::runtime::Handle::current();
    // Setup
    let worker_counters: [_; NUM_TEST_WORKERS] =
        [Arc::new(AtomicU16::new(0)), Arc::new(AtomicU16::new(0))];
    let discard_counter = Arc::new(AtomicU16::new(0));

    let worker_builder = SlowTestWorkerBuilder {
        counters: worker_counters.clone(),
    };
    let factory_definition = Factory::<
        TestKey,
        TestMessage,
        (),
        TestWorker,
        routing::QueuerRouting<TestKey, TestMessage>,
        queues::DefaultQueue<TestKey, TestMessage>,
    >::default();
    let (factory, factory_handle) = Actor::spawn(
        None,
        factory_definition,
        FactoryArguments {
            num_initial_workers: NUM_TEST_WORKERS,
            queue: queues::DefaultQueue::default(),
            router: Default::default(),
            capacity_controller: None,
            dead_mans_switch: None,
            discard_handler: Some(Arc::new(TestDiscarder {
                counter: discard_counter.clone(),
            })),
            discard_settings: DiscardSettings::Dynamic {
                limit: 5,
                mode: DiscardMode::Newest,
                updater: Box::new(DiscardController {}),
            },
            lifecycle_hooks: None,
            worker_builder: Box::new(worker_builder),
            stats: None,
        },
    )
    .await
    .expect("Failed to spawn factory");

    // Act
    for i in 0..10 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                key: TestKey { id: 1 },
                msg: TestMessage::Count(i),
                options: JobOptions::default(),
                accepted: None,
            }))
            .expect("Failed to send to factory");
    }
    // give some time to process all the messages (10ms/msg by 2 workers for 7 msgs)
    crate::periodic_check(
        || {
            // Assert
            // we should have shed the 3 newest messages, so 7, 8, 9
            discard_counter.load(Ordering::Relaxed) == 24
        },
        Duration::from_secs(1),
    )
    .await;

    // now we wait for the ping to change the discard threshold to 10
    crate::concurrency::sleep(Duration::from_millis(300)).await;

    // Act again
    for i in 0..14 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                key: TestKey { id: 1 },
                msg: TestMessage::Count(i),
                options: JobOptions::default(),
                accepted: None,
            }))
            .expect("Failed to send to factory");
    }

    // give some time to process all the messages (10ms/msg by 2 workers for 7 msgs)
    crate::periodic_check(
        || {
            // Assert
            // we should have shed the 2 newest messages, so 12 and 13 + original amount of 24
            discard_counter.load(Ordering::Relaxed) == 49
        },
        Duration::from_secs(1),
    )
    .await;

    // Cleanup
    // wait for factory termination
    factory.stop(None);
    factory_handle.await.unwrap();
}
