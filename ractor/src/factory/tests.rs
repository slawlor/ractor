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
    concurrency::{Duration, Instant},
    Actor, ActorProcessingErr, ActorRef,
};

use super::{
    CustomHashFunction, Factory, FactoryMessage, Job, JobOptions, RoutingMode, WorkerStartContext,
};

const NUM_TEST_WORKERS: usize = 3;

#[derive(Hash)]
struct TestKey {
    id: u64,
}
#[cfg(feature = "cluster")]
impl crate::Message for TestKey {}

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
}

#[async_trait::async_trait]
impl Actor for TestWorker {
    type Msg = super::Job<TestKey, TestMessage>;
    type State = Self::Arguments;
    type Arguments = WorkerStartContext<TestKey, TestMessage, Self>;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self>,
        startup_context: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(startup_context)
    }

    async fn handle(
        &self,
        myself: ActorRef<Self>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        log::debug!("Worker received {:?}", message.msg);

        self.counter.fetch_add(1, Ordering::Relaxed);

        // job finished, on success or err we report back to the factory
        state
            .factory
            .cast(FactoryMessage::Finished(myself.get_id(), message.key))?;
        Ok(())
    }
}

struct TestWorkerBuilder {
    counters: [Arc<AtomicU16>; NUM_TEST_WORKERS],
}

impl super::WorkerBuilder<TestWorker> for TestWorkerBuilder {
    fn build(&self, wid: usize) -> TestWorker {
        TestWorker {
            counter: self.counters[wid].clone(),
        }
    }
}

#[crate::concurrency::test]
async fn test_dispatch_key_persistent() {
    let worker_counters: [_; NUM_TEST_WORKERS] = [
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
    ];

    let worker_builder = TestWorkerBuilder {
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
                options: JobOptions {
                    factory_time: Instant::now(),
                    submit_time: Instant::now(),
                    ttl: Duration::from_millis(100),
                },
            }))
            .expect("Failed to send to factory");
    }

    // give some time to process all the messages
    crate::concurrency::sleep(Duration::from_millis(1000)).await;

    factory.stop(None);
    factory_handle.await.unwrap();

    // assert
    let all_counter = worker_counters[0].load(Ordering::Relaxed);
    assert_eq!(999, all_counter);
}

// TODO: This test should probably use like a slow queuer or something to check we move to the next item, etc
#[crate::concurrency::test]
async fn test_dispatch_queuer() {
    let worker_counters: [_; NUM_TEST_WORKERS] = [
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
    ];

    let worker_builder = TestWorkerBuilder {
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
                options: JobOptions {
                    factory_time: Instant::now(),
                    submit_time: Instant::now(),
                    ttl: Duration::from_millis(100),
                },
            }))
            .expect("Failed to send to factory");
    }

    // give some time to process all the messages
    crate::concurrency::sleep(Duration::from_millis(1000)).await;

    factory.stop(None);
    factory_handle.await.unwrap();

    // assert
    let all_counter: u16 = worker_counters
        .iter()
        .map(|w| w.load(Ordering::Relaxed))
        .sum();
    assert_eq!(999, all_counter);
}

#[crate::concurrency::test]
async fn test_dispatch_round_robin() {
    let worker_counters: [_; NUM_TEST_WORKERS] = [
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
    ];

    let worker_builder = TestWorkerBuilder {
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
                options: JobOptions {
                    factory_time: Instant::now(),
                    submit_time: Instant::now(),
                    ttl: Duration::from_millis(100),
                },
            }))
            .expect("Failed to send to factory");
    }

    // give some time to process all the messages
    crate::concurrency::sleep(Duration::from_millis(1000)).await;

    factory.stop(None);
    factory_handle.await.unwrap();

    // assert
    for counter in worker_counters {
        assert_eq!(333, counter.load(Ordering::Relaxed));
    }
}

#[crate::concurrency::test]
async fn test_dispatch_random() {
    let worker_counters: [_; NUM_TEST_WORKERS] = [
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
    ];

    let worker_builder = TestWorkerBuilder {
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
                options: JobOptions {
                    factory_time: Instant::now(),
                    submit_time: Instant::now(),
                    ttl: Duration::from_millis(100),
                },
            }))
            .expect("Failed to send to factory");
    }

    // give some time to process all the messages
    crate::concurrency::sleep(Duration::from_millis(1000)).await;

    factory.stop(None);
    factory_handle.await.unwrap();

    // assert
    let all_counter: u16 = worker_counters
        .iter()
        .map(|w| w.load(Ordering::Relaxed))
        .sum();
    assert_eq!(999, all_counter);
}

#[crate::concurrency::test]
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

    let worker_builder = TestWorkerBuilder {
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
                options: JobOptions {
                    factory_time: Instant::now(),
                    submit_time: Instant::now(),
                    ttl: Duration::from_millis(100),
                },
            }))
            .expect("Failed to send to factory");
    }

    // give some time to process all the messages
    crate::concurrency::sleep(Duration::from_millis(1000)).await;

    factory.stop(None);
    factory_handle.await.unwrap();

    // assert
    let all_counter = worker_counters[2].load(Ordering::Relaxed);
    assert_eq!(999, all_counter);
}
