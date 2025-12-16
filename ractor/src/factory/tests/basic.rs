// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use std::marker::PhantomData;
use std::sync::atomic::AtomicU16;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::concurrency::Duration;
use crate::factory::routing::CustomHashFunction;
use crate::factory::*;
use crate::periodic_check;
use crate::Actor;
use crate::ActorProcessingErr;
use crate::ActorRef;

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
    /// Doh'k
    #[allow(dead_code)]
    Count(u16),
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
        Job { msg, key, .. }: Job<Self::Key, Self::Message>,
        _state: &mut Self::State,
    ) -> Result<TestKey, ActorProcessingErr> {
        tracing::debug!("Worker received {:?}", msg);

        self.counter.fetch_add(1, Ordering::Relaxed);

        if let Some(timeout_ms) = self.slow {
            crate::concurrency::sleep(Duration::from_millis(timeout_ms)).await;
        }

        Ok(key)
    }
}

struct FastTestWorkerBuilder {
    counters: [Arc<AtomicU16>; NUM_TEST_WORKERS],
}

impl WorkerBuilder<TestWorker, ()> for FastTestWorkerBuilder {
    fn build(&mut self, wid: usize) -> (TestWorker, ()) {
        (
            TestWorker {
                counter: self.counters[wid].clone(),
                slow: None,
            },
            (),
        )
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

struct InsanelySlowWorkerBuilder {
    counters: [Arc<AtomicU16>; NUM_TEST_WORKERS],
}

impl WorkerBuilder<TestWorker, ()> for InsanelySlowWorkerBuilder {
    fn build(&mut self, wid: usize) -> (TestWorker, ()) {
        (
            TestWorker {
                counter: self.counters[wid].clone(),
                slow: Some(10000),
            },
            (),
        )
    }
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_dispatch_key_persistent() {
    let worker_counters: [_; NUM_TEST_WORKERS] = [
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
    ];

    let worker_builder = FastTestWorkerBuilder {
        counters: worker_counters.clone(),
    };
    let factory_definition = Factory::<
        TestKey,
        TestMessage,
        (),
        TestWorker,
        routing::KeyPersistentRouting<TestKey, TestMessage>,
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
            stats: None,
        },
    )
    .await
    .expect("Failed to spawn factory");

    for _ in 0..999 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                key: TestKey { id: 1 },
                msg: TestMessage::Ok,
                options: JobOptions::default(),
                accepted: None,
            }))
            .expect("Failed to send to factory");
    }

    // give some time to process all the messages
    let check_counters = worker_counters[0].clone();
    periodic_check(
        move || {
            let all_counter = check_counters.load(Ordering::Relaxed);
            all_counter == 999
        },
        Duration::from_secs(3),
    )
    .await;

    factory.stop(None);
    factory_handle.await.unwrap();

    println!(
        "Counters: [{}] [{}] [{}]",
        worker_counters[0].load(Ordering::Relaxed),
        worker_counters[1].load(Ordering::Relaxed),
        worker_counters[2].load(Ordering::Relaxed)
    );
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_dispatch_queuer() {
    let worker_counters: [_; NUM_TEST_WORKERS] = [
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
    ];

    let worker_builder = FastTestWorkerBuilder {
        counters: worker_counters.clone(),
    };
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
            stats: None,
        },
    )
    .await
    .expect("Failed to spawn factory");

    for id in 0..100 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                key: TestKey { id },
                msg: TestMessage::Ok,
                options: JobOptions::default(),
                accepted: None,
            }))
            .expect("Failed to send to factory");
    }

    // give some time to process all the messages
    let check_counters = worker_counters.clone();
    periodic_check(
        || {
            check_counters
                .iter()
                .map(|c| c.load(Ordering::Relaxed))
                .sum::<u16>()
                == 100
        },
        Duration::from_secs(5),
    )
    .await;

    factory.stop(None);
    factory_handle.await.unwrap();

    println!(
        "Counters: [{}] [{}] [{}]",
        worker_counters[0].load(Ordering::Relaxed),
        worker_counters[1].load(Ordering::Relaxed),
        worker_counters[2].load(Ordering::Relaxed)
    );
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_dispatch_round_robin() {
    let worker_counters: [_; NUM_TEST_WORKERS] = [
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
    ];

    let worker_builder = FastTestWorkerBuilder {
        counters: worker_counters.clone(),
    };
    let factory_definition = Factory::<
        TestKey,
        TestMessage,
        (),
        TestWorker,
        routing::RoundRobinRouting<TestKey, TestMessage>,
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
            stats: None,
        },
    )
    .await
    .expect("Failed to spawn factory");

    for _ in 0..999 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                key: TestKey { id: 1 },
                msg: TestMessage::Ok,
                options: JobOptions::default(),
                accepted: None,
            }))
            .expect("Failed to send to factory");
    }

    // give some time to process all the messages
    let check_counters = worker_counters.clone();
    periodic_check(
        move || {
            check_counters
                .iter()
                .all(|counter| counter.load(Ordering::Relaxed) == 333)
        },
        Duration::from_secs(3),
    )
    .await;

    factory.stop(None);
    factory_handle.await.unwrap();

    println!(
        "Counters: [{}] [{}] [{}]",
        worker_counters[0].load(Ordering::Relaxed),
        worker_counters[1].load(Ordering::Relaxed),
        worker_counters[2].load(Ordering::Relaxed)
    );
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
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
    let factory_definition = Factory::<
        TestKey,
        TestMessage,
        (),
        TestWorker,
        routing::CustomRouting<TestKey, TestMessage, MyHasher<TestKey>>,
        DefaultQueue,
    >::default();
    let (factory, factory_handle) = Actor::spawn(
        None,
        factory_definition,
        FactoryArguments {
            num_initial_workers: NUM_TEST_WORKERS,
            queue: DefaultQueue::default(),
            router: routing::CustomRouting::new(MyHasher { _key: PhantomData }),
            capacity_controller: None,
            dead_mans_switch: None,
            discard_handler: None,
            discard_settings: DiscardSettings::None,
            lifecycle_hooks: None,
            worker_builder: Box::new(worker_builder),
            stats: None,
        },
    )
    .await
    .expect("Failed to spawn factory");

    for _ in 0..999 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                key: TestKey { id: 1 },
                msg: TestMessage::Ok,
                options: JobOptions::default(),
                accepted: None,
            }))
            .expect("Failed to send to factory");
    }

    // give some time to process all the messages
    let check_counters = worker_counters.clone();
    periodic_check(
        move || check_counters[2].load(Ordering::Relaxed) == 999,
        Duration::from_secs(3),
    )
    .await;

    factory.stop(None);
    factory_handle.await.unwrap();

    println!(
        "Counters: [{}] [{}] [{}]",
        worker_counters[0].load(Ordering::Relaxed),
        worker_counters[1].load(Ordering::Relaxed),
        worker_counters[2].load(Ordering::Relaxed)
    );
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_dispatch_sticky_queueing() {
    let worker_counters: [_; NUM_TEST_WORKERS] = [
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
    ];

    let worker_builder = SlowTestWorkerBuilder {
        counters: worker_counters.clone(),
    };
    let factory_definition = Factory::<
        TestKey,
        TestMessage,
        (),
        TestWorker,
        routing::StickyQueuerRouting<TestKey, TestMessage>,
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
            stats: None,
        },
    )
    .await
    .expect("Failed to spawn factory");

    // Since we're dispatching all of the same key, they should all get routed to the same worker
    for _ in 0..5 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                key: TestKey { id: 1 },
                msg: TestMessage::Ok,
                options: JobOptions::default(),
                accepted: None,
            }))
            .expect("Failed to send to factory");
    }

    // give some time to process all the messages
    let check_counters = worker_counters.clone();
    periodic_check(
        move || {
            // one worker got all 5 messages due to sticky queueing
            check_counters
                .iter()
                .any(|counter| counter.load(Ordering::Relaxed) == 5)
        },
        Duration::from_secs(3),
    )
    .await;

    factory.stop(None);
    factory_handle.await.unwrap();

    println!(
        "Counters: [{}] [{}] [{}]",
        worker_counters[0].load(Ordering::Relaxed),
        worker_counters[1].load(Ordering::Relaxed),
        worker_counters[2].load(Ordering::Relaxed)
    );
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_discarding_old_records_on_queuer() {
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
        fn discard(&self, _reason: DiscardReason, _job: &mut Job<TestKey, TestMessage>) {
            let _ = self.counter.fetch_add(1, Ordering::Relaxed);
        }
    }

    let worker_builder = InsanelySlowWorkerBuilder {
        counters: worker_counters.clone(),
    };
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
            discard_handler: Some(Arc::new(TestDiscarder {
                counter: discard_counter.clone(),
            })),
            discard_settings: DiscardSettings::Static {
                limit: 5,
                mode: DiscardMode::Oldest,
            },
            lifecycle_hooks: None,
            worker_builder: Box::new(worker_builder),
            stats: None,
        },
    )
    .await
    .expect("Failed to spawn factory");

    for _ in 0..108 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                key: TestKey { id: 1 },
                msg: TestMessage::Ok,
                options: JobOptions::default(),
                accepted: None,
            }))
            .expect("Failed to send to factory");
    }

    // give some time to process all the messages
    crate::concurrency::sleep(Duration::from_millis(250)).await;

    println!(
        "Counters: [{}] [{}] [{}]",
        worker_counters[0].load(Ordering::Relaxed),
        worker_counters[1].load(Ordering::Relaxed),
        worker_counters[2].load(Ordering::Relaxed)
    );

    // assert

    // each worker only got 1 message, then "slept"
    assert!(worker_counters
        .iter()
        .map(|c| c.load(Ordering::Relaxed))
        .all(|count| count == 1));

    // 5 messages should be left in the factory's queue, while the remaining should get "discarded"
    // (3 in workers) + (5 in queue) + (100 discarded) = 108, the number of msgs we sent to the factory
    assert_eq!(100, discard_counter.load(Ordering::Relaxed));

    // wait for factory termination
    factory.stop(None);
    factory_handle.await.unwrap();
}

struct StuckWorker {
    counter: Arc<AtomicU16>,
    slow: Option<u64>,
}

#[cfg_attr(feature = "async-trait", crate::async_trait)]
impl Actor for StuckWorker {
    type Msg = WorkerMessage<TestKey, TestMessage>;
    type State = Self::Arguments;
    type Arguments = WorkerStartContext<TestKey, TestMessage, ()>;

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
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_stuck_workers() {
    let worker_counters: [_; NUM_TEST_WORKERS] = [
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
    ];

    struct StuckWorkerBuilder {
        counters: [Arc<AtomicU16>; NUM_TEST_WORKERS],
    }

    impl WorkerBuilder<StuckWorker, ()> for StuckWorkerBuilder {
        fn build(&mut self, wid: usize) -> (StuckWorker, ()) {
            (
                StuckWorker {
                    counter: self.counters[wid].clone(),
                    slow: Some(10000),
                },
                (),
            )
        }
    }

    let worker_builder = StuckWorkerBuilder {
        counters: worker_counters.clone(),
    };
    let factory_definition = Factory::<
        TestKey,
        TestMessage,
        (),
        StuckWorker,
        routing::RoundRobinRouting<TestKey, TestMessage>,
        DefaultQueue,
    >::default();
    let dms = DeadMansSwitchConfiguration::builder()
        .detection_timeout(Duration::from_millis(50))
        .kill_worker(true)
        .build();
    tracing::debug!("DMS settings: {dms:?}");
    let args = FactoryArguments::builder()
        .num_initial_workers(NUM_TEST_WORKERS)
        .queue(Default::default())
        .router(Default::default())
        .worker_builder(Box::new(worker_builder))
        .dead_mans_switch(dms)
        .build();
    tracing::debug!("Factory args {args:?}");
    let (factory, factory_handle) = Actor::spawn(None, factory_definition, args)
        .await
        .expect("Failed to spawn factory");

    tracing::debug!(
        "Actor node {}, pid {}",
        factory.get_id().node(),
        factory.get_id().pid()
    );

    for _ in 0..9 {
        factory
            .cast(FactoryMessage::Dispatch(
                Job::builder()
                    .key(TestKey { id: 1 })
                    .msg(TestMessage::Ok)
                    .build(),
            ))
            .expect("Failed to send to factory");
    }

    // give some time to process all the messages
    crate::concurrency::sleep(Duration::from_millis(500)).await;

    // wait for factory termination
    factory.stop(None);
    factory_handle.await.unwrap();

    println!(
        "Counters: [{}] [{}] [{}]",
        worker_counters[0].load(Ordering::Relaxed),
        worker_counters[1].load(Ordering::Relaxed),
        worker_counters[2].load(Ordering::Relaxed)
    );

    // assert

    // each worker only got 1 message, then "slept"
    assert!(worker_counters
        .iter()
        .map(|c| c.load(Ordering::Relaxed))
        .all(|count| count > 1));
}

#[crate::concurrency::test]
async fn test_discarding_new_records_on_queuer() {
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
        fn discard(&self, _reason: DiscardReason, job: &mut Job<TestKey, TestMessage>) {
            if let TestMessage::Count(count) = job.msg {
                let _ = self.counter.fetch_add(count, Ordering::Relaxed);
            }
        }
    }

    let worker_builder = InsanelySlowWorkerBuilder {
        counters: worker_counters.clone(),
    };
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
            discard_handler: Some(Arc::new(TestDiscarder {
                counter: discard_counter.clone(),
            })),
            discard_settings: DiscardSettings::Static {
                limit: 5,
                mode: DiscardMode::Newest,
            },
            lifecycle_hooks: None,
            worker_builder: Box::new(worker_builder),
            stats: None,
        },
    )
    .await
    .expect("Failed to spawn factory");

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

    let active_requests = factory
        .call(FactoryMessage::GetNumActiveWorkers, None)
        .await
        .expect("Failed to send query to factory")
        .expect("Failed to get result from factory");
    assert!(active_requests > 0);

    // give some time to process all the messages
    crate::concurrency::sleep(Duration::from_millis(250)).await;

    println!(
        "Counters: [{}] [{}] [{}]",
        worker_counters[0].load(Ordering::Relaxed),
        worker_counters[1].load(Ordering::Relaxed),
        worker_counters[2].load(Ordering::Relaxed)
    );

    // assert

    // each worker only got 1 message, then "slept"
    assert!(worker_counters
        .iter()
        .map(|c| c.load(Ordering::Relaxed))
        .all(|count| count == 1));

    // 5 messages should be left in the factory's queue, while the remaining should get "discarded"
    //
    // The "newest" messages had values (8) and (9), respectively, which together should mean
    // the discard counter is 17
    assert_eq!(17, discard_counter.load(Ordering::Relaxed));

    // wait for factory termination
    factory.stop(None);
    factory_handle.await.unwrap();
}
