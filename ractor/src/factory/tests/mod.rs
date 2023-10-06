// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use std::{
    marker::PhantomData,
    sync::{
        atomic::{AtomicU16, Ordering},
        Arc,
    },
};

use crate::{
    common_test::{periodic_async_check, periodic_check},
    concurrency::Duration,
    factory::DiscardHandler,
    Actor, ActorProcessingErr, ActorRef,
};

use super::{
    CustomHashFunction, Factory, FactoryMessage, Job, JobOptions, RoutingMode, WorkerMessage,
    WorkerStartContext,
};

mod worker_lifecycle;

const NUM_TEST_WORKERS: usize = 3;

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
    Ok,
}
#[cfg(feature = "cluster")]
impl crate::Message for TestMessage {}

struct TestWorker {
    counter: Arc<AtomicU16>,
    slow: Option<u64>,
}

#[async_trait::async_trait]
impl Actor for TestWorker {
    type Msg = super::WorkerMessage<TestKey, TestMessage>;
    type State = Self::Arguments;
    type Arguments = WorkerStartContext<TestKey, TestMessage>;

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
                tracing::trace!("Worker received {:?}", job.msg);

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

struct FastTestWorkerBuilder {
    counters: [Arc<AtomicU16>; NUM_TEST_WORKERS],
}

impl super::WorkerBuilder<TestWorker> for FastTestWorkerBuilder {
    fn build(&self, wid: usize) -> TestWorker {
        TestWorker {
            counter: self.counters[wid].clone(),
            slow: None,
        }
    }
}

struct SlowTestWorkerBuilder {
    counters: [Arc<AtomicU16>; NUM_TEST_WORKERS],
}

impl super::WorkerBuilder<TestWorker> for SlowTestWorkerBuilder {
    fn build(&self, wid: usize) -> TestWorker {
        TestWorker {
            counter: self.counters[wid].clone(),
            slow: Some(10),
        }
    }
}

struct InsanelySlowWorkerBuilder {
    counters: [Arc<AtomicU16>; NUM_TEST_WORKERS],
}

impl super::WorkerBuilder<TestWorker> for InsanelySlowWorkerBuilder {
    fn build(&self, wid: usize) -> TestWorker {
        TestWorker {
            counter: self.counters[wid].clone(),
            slow: Some(10000),
        }
    }
}

#[crate::concurrency::test]
#[tracing_test::traced_test]
async fn test_dispatch_key_persistent() {
    let worker_counters: [_; NUM_TEST_WORKERS] = [
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
    ];

    let worker_builder = FastTestWorkerBuilder {
        counters: worker_counters.clone(),
    };
    let factory_definition = Factory::<TestKey, TestMessage, TestWorker> {
        worker_count: NUM_TEST_WORKERS,
        routing_mode: RoutingMode::<TestKey>::KeyPersistent,
        ..Default::default()
    };
    let (factory, factory_handle) =
        Actor::spawn(None, factory_definition, Box::new(worker_builder))
            .await
            .expect("Failed to spawn factory");

    for _ in 0..999 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                key: TestKey { id: 1 },
                msg: TestMessage::Ok,
                options: JobOptions::default(),
            }))
            .expect("Failed to send to factory");
    }

    periodic_check(
        || worker_counters[0].load(Ordering::Relaxed) == 999,
        Duration::from_secs(5),
    )
    .await;

    factory.stop(None);
    factory_handle.await.unwrap();

    tracing::info!(
        "Counters: [{}] [{}] [{}]",
        worker_counters[0].load(Ordering::Relaxed),
        worker_counters[1].load(Ordering::Relaxed),
        worker_counters[2].load(Ordering::Relaxed)
    );
}

// TODO: This test should probably use like a slow queuer or something to check we move to the next item, etc
#[crate::concurrency::test]
async fn test_dispatch_queuer() {
    let worker_counters: [_; NUM_TEST_WORKERS] = [
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
    ];

    let worker_builder = FastTestWorkerBuilder {
        counters: worker_counters.clone(),
    };
    let factory_definition = Factory::<TestKey, TestMessage, TestWorker> {
        worker_count: NUM_TEST_WORKERS,
        routing_mode: RoutingMode::<TestKey>::Queuer,
        ..Default::default()
    };
    let (factory, factory_handle) =
        Actor::spawn(None, factory_definition, Box::new(worker_builder))
            .await
            .expect("Failed to spawn factory");

    for _ in 0..999 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                key: TestKey { id: 1 },
                msg: TestMessage::Ok,
                options: JobOptions::default(),
            }))
            .expect("Failed to send to factory");
    }

    periodic_check(
        || {
            let all_counter: u16 = worker_counters
                .iter()
                .map(|w| w.load(Ordering::Relaxed))
                .sum();
            all_counter == 999
        },
        Duration::from_secs(5),
    )
    .await;

    factory.stop(None);
    factory_handle.await.unwrap();

    tracing::info!(
        "Counters: [{}] [{}] [{}]",
        worker_counters[0].load(Ordering::Relaxed),
        worker_counters[1].load(Ordering::Relaxed),
        worker_counters[2].load(Ordering::Relaxed)
    );
}

#[crate::concurrency::test]
#[tracing_test::traced_test]
async fn test_dispatch_round_robin() {
    let worker_counters: [_; NUM_TEST_WORKERS] = [
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
    ];

    let worker_builder = FastTestWorkerBuilder {
        counters: worker_counters.clone(),
    };
    let factory_definition = Factory::<TestKey, TestMessage, TestWorker> {
        worker_count: NUM_TEST_WORKERS,
        routing_mode: RoutingMode::<TestKey>::RoundRobin,
        ..Default::default()
    };
    let (factory, factory_handle) =
        Actor::spawn(None, factory_definition, Box::new(worker_builder))
            .await
            .expect("Failed to spawn factory");

    for _ in 0..999 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                key: TestKey { id: 1 },
                msg: TestMessage::Ok,
                options: JobOptions::default(),
            }))
            .expect("Failed to send to factory");
    }

    periodic_check(
        || {
            worker_counters
                .iter()
                .all(|counter| counter.load(Ordering::Relaxed) == 333)
        },
        Duration::from_secs(5),
    )
    .await;

    factory.stop(None);
    factory_handle.await.unwrap();

    tracing::info!(
        "Counters: [{}] [{}] [{}]",
        worker_counters[0].load(Ordering::Relaxed),
        worker_counters[1].load(Ordering::Relaxed),
        worker_counters[2].load(Ordering::Relaxed)
    );
}

#[crate::concurrency::test]
#[tracing_test::traced_test]
async fn test_dispatch_random() {
    let worker_counters: [_; NUM_TEST_WORKERS] = [
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
    ];

    let worker_builder = FastTestWorkerBuilder {
        counters: worker_counters.clone(),
    };
    let factory_definition = Factory::<TestKey, TestMessage, TestWorker> {
        worker_count: NUM_TEST_WORKERS,
        routing_mode: RoutingMode::<TestKey>::Random,
        ..Default::default()
    };
    let (factory, factory_handle) =
        Actor::spawn(None, factory_definition, Box::new(worker_builder))
            .await
            .expect("Failed to spawn factory");

    for _ in 0..999 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                key: TestKey { id: 1 },
                msg: TestMessage::Ok,
                options: JobOptions::default(),
            }))
            .expect("Failed to send to factory");
    }

    periodic_check(
        || {
            let all_counter: u16 = worker_counters
                .iter()
                .map(|w| w.load(Ordering::Relaxed))
                .sum();
            all_counter == 999
        },
        Duration::from_secs(5),
    )
    .await;

    factory.stop(None);
    factory_handle.await.unwrap();

    tracing::info!(
        "Counters: [{}] [{}] [{}]",
        worker_counters[0].load(Ordering::Relaxed),
        worker_counters[1].load(Ordering::Relaxed),
        worker_counters[2].load(Ordering::Relaxed)
    );
}

#[crate::concurrency::test]
#[tracing_test::traced_test]
async fn test_dispatch_custom_hashing() {
    struct MyHasher<TKey>
    where
        TKey: crate::Message + Sync,
    {
        _key: PhantomData<TKey>,
    }

    impl<TKey> CustomHashFunction<TKey> for MyHasher<TKey>
    where
        TKey: crate::Message + Sync,
    {
        fn hash(&self, _key: &TKey, _worker_count: usize) -> usize {
            2
        }
    }

    let worker_counters: [_; NUM_TEST_WORKERS] = [
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
    ];

    let worker_builder = FastTestWorkerBuilder {
        counters: worker_counters.clone(),
    };
    let factory_definition = Factory::<TestKey, TestMessage, TestWorker> {
        worker_count: NUM_TEST_WORKERS,
        routing_mode: RoutingMode::<TestKey>::CustomHashFunction(Box::new(MyHasher {
            _key: PhantomData,
        })),
        ..Default::default()
    };
    let (factory, factory_handle) =
        Actor::spawn(None, factory_definition, Box::new(worker_builder))
            .await
            .expect("Failed to spawn factory");

    for _ in 0..999 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                key: TestKey { id: 1 },
                msg: TestMessage::Ok,
                options: JobOptions::default(),
            }))
            .expect("Failed to send to factory");
    }

    periodic_check(
        || {
            let all_counter = worker_counters[2].load(Ordering::Relaxed);
            all_counter == 999
        },
        Duration::from_secs(5),
    )
    .await;

    factory.stop(None);
    factory_handle.await.unwrap();

    tracing::info!(
        "Counters: [{}] [{}] [{}]",
        worker_counters[0].load(Ordering::Relaxed),
        worker_counters[1].load(Ordering::Relaxed),
        worker_counters[2].load(Ordering::Relaxed)
    );
}

#[crate::concurrency::test]
#[tracing_test::traced_test]
async fn test_dispatch_sticky_queueing() {
    let worker_counters: [_; NUM_TEST_WORKERS] = [
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
    ];

    let worker_builder = SlowTestWorkerBuilder {
        counters: worker_counters.clone(),
    };
    let factory_definition = Factory::<TestKey, TestMessage, TestWorker> {
        worker_count: NUM_TEST_WORKERS,
        routing_mode: RoutingMode::<TestKey>::StickyQueuer,
        ..Default::default()
    };
    let (factory, factory_handle) =
        Actor::spawn(None, factory_definition, Box::new(worker_builder))
            .await
            .expect("Failed to spawn factory");

    // Since we're dispatching all of the same key, they should all get routed to the same worker
    for _ in 0..5 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                key: TestKey { id: 1 },
                msg: TestMessage::Ok,
                options: JobOptions::default(),
            }))
            .expect("Failed to send to factory");
    }

    periodic_check(
        || {
            worker_counters
                .iter()
                .map(|c| c.load(Ordering::Relaxed))
                .any(|count| count == 5)
        },
        Duration::from_secs(5),
    )
    .await;

    factory.stop(None);
    factory_handle.await.unwrap();

    tracing::info!(
        "Counters: [{}] [{}] [{}]",
        worker_counters[0].load(Ordering::Relaxed),
        worker_counters[1].load(Ordering::Relaxed),
        worker_counters[2].load(Ordering::Relaxed)
    );
}

#[crate::concurrency::test]
#[tracing_test::traced_test]
async fn test_discards_on_queuer() {
    let worker_counters: [_; NUM_TEST_WORKERS] = [
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
    ];
    let discard_counter = Arc::new(AtomicU16::new(0));

    struct TestDiscarder {
        counter: Arc<AtomicU16>,
    }
    impl DiscardHandler<TestKey, TestMessage> for TestDiscarder {
        fn clone_box(&self) -> Box<dyn DiscardHandler<TestKey, TestMessage>> {
            Box::new(TestDiscarder {
                counter: self.counter.clone(),
            })
        }
        fn discard(&self, _job: Job<TestKey, TestMessage>) {
            let _ = self.counter.fetch_add(1, Ordering::Relaxed);
        }
    }

    let worker_builder = InsanelySlowWorkerBuilder {
        counters: worker_counters.clone(),
    };
    let factory_definition = Factory::<TestKey, TestMessage, TestWorker> {
        worker_count: NUM_TEST_WORKERS,
        routing_mode: RoutingMode::<TestKey>::Queuer,
        discard_handler: Some(Box::new(TestDiscarder {
            counter: discard_counter.clone(),
        })),
        discard_threshold: Some(5),
        ..Default::default()
    };
    let (factory, factory_handle) =
        Actor::spawn(None, factory_definition, Box::new(worker_builder))
            .await
            .expect("Failed to spawn factory");

    // Since we're dispatching all of the same key, they should all get routed to the same worker
    for _ in 0..108 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                key: TestKey { id: 1 },
                msg: TestMessage::Ok,
                options: JobOptions::default(),
            }))
            .expect("Failed to send to factory");
    }

    // each worker only got 1 message, then "slept"
    periodic_check(
        || {
            worker_counters
                .iter()
                .map(|c| c.load(Ordering::Relaxed))
                .all(|count| count == 1)
        },
        Duration::from_secs(5),
    )
    .await;

    // wait for factory termination
    factory.stop(None);
    factory_handle.await.unwrap();

    tracing::info!(
        "Counters: [{}] [{}] [{}]",
        worker_counters[0].load(Ordering::Relaxed),
        worker_counters[1].load(Ordering::Relaxed),
        worker_counters[2].load(Ordering::Relaxed)
    );

    // 5 messages should be left in the factory's queue, while the remaining should get "discarded"
    // (3 in workers) + (5 in queue) + (100 discarded) = 108, the number of msgs we sent to the factory
    assert_eq!(100, discard_counter.load(Ordering::Relaxed));
}

struct StuckWorker {
    counter: Arc<AtomicU16>,
    slow: Option<u64>,
}

#[async_trait::async_trait]
impl Actor for StuckWorker {
    type Msg = super::WorkerMessage<TestKey, TestMessage>;
    type State = Self::Arguments;
    type Arguments = WorkerStartContext<TestKey, TestMessage>;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        startup_context: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        self.counter.fetch_add(1, Ordering::Relaxed);
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

#[crate::concurrency::test]
#[tracing_test::traced_test]
async fn test_stuck_workers() {
    let worker_counters: [_; NUM_TEST_WORKERS] = [
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
    ];

    struct StuckWorkerBuilder {
        counters: [Arc<AtomicU16>; NUM_TEST_WORKERS],
    }

    impl super::WorkerBuilder<TestWorker> for StuckWorkerBuilder {
        fn build(&self, wid: usize) -> TestWorker {
            TestWorker {
                counter: self.counters[wid].clone(),
                slow: Some(10000),
            }
        }
    }

    let worker_builder = StuckWorkerBuilder {
        counters: worker_counters.clone(),
    };

    let factory_definition = Factory::<TestKey, TestMessage, TestWorker> {
        worker_count: NUM_TEST_WORKERS,
        routing_mode: RoutingMode::<TestKey>::RoundRobin,
        dead_mans_switch: Some(super::DeadMansSwitchConfiguration {
            detection_timeout: Duration::from_millis(50),
            kill_worker: true,
        }),
        ..Default::default()
    };
    let (factory, factory_handle) =
        Actor::spawn(None, factory_definition, Box::new(worker_builder))
            .await
            .expect("Failed to spawn factory");

    // Since we're dispatching all of the same key, they should all get routed to the same worker
    for _ in 0..9 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                key: TestKey { id: 1 },
                msg: TestMessage::Ok,
                options: JobOptions::default(),
            }))
            .expect("Failed to send to factory");
    }

    periodic_check(
        || {
            worker_counters
                .iter()
                .map(|c| c.load(Ordering::Relaxed))
                .all(|count| count > 1)
        },
        Duration::from_secs(5),
    )
    .await;

    // wait for factory termination
    factory.stop(None);
    factory_handle.await.unwrap();

    tracing::info!(
        "Counters: [{}] [{}] [{}]",
        worker_counters[0].load(Ordering::Relaxed),
        worker_counters[1].load(Ordering::Relaxed),
        worker_counters[2].load(Ordering::Relaxed)
    );
}

#[crate::concurrency::test]
#[tracing_test::traced_test]
async fn test_worker_pings() {
    let worker_counters: [_; NUM_TEST_WORKERS] = [
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
    ];

    let worker_builder = FastTestWorkerBuilder {
        counters: worker_counters.clone(),
    };
    let factory_definition = Factory::<TestKey, TestMessage, TestWorker> {
        worker_count: NUM_TEST_WORKERS,
        routing_mode: RoutingMode::<TestKey>::KeyPersistent,
        collect_worker_stats: true,
        ..Default::default()
    };
    let (factory, factory_handle) =
        Actor::spawn(None, factory_definition, Box::new(worker_builder))
            .await
            .expect("Failed to spawn factory");

    for _ in 0..999 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                key: TestKey { id: 1 },
                msg: TestMessage::Ok,
                options: JobOptions::default(),
            }))
            .expect("Failed to send to factory");
    }

    periodic_async_check(
        || async {
            let stats = crate::call_t!(factory, FactoryMessage::GetStats, 200)
                .expect("Failed to get statistics");
            stats.ping_count > 0
        },
        Duration::from_secs(10),
    )
    .await;

    factory.stop(None);
    factory_handle.await.unwrap();

    tracing::info!(
        "Counters: [{}] [{}] [{}]",
        worker_counters[0].load(Ordering::Relaxed),
        worker_counters[1].load(Ordering::Relaxed),
        worker_counters[2].load(Ordering::Relaxed)
    );

    periodic_check(
        || {
            let all_counter = worker_counters[0].load(Ordering::Relaxed);
            all_counter == 999
        },
        Duration::from_secs(10),
    )
    .await;
}
