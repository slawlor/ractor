// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Tests around dynamic worker pool configuration. These require a more complex setup than the basic tests
//! and therefore are separated out

use std::sync::Arc;

#[cfg(not(feature = "async-trait"))]
use futures::{future::BoxFuture, FutureExt};

use crate::concurrency::Duration;
use crate::Actor;
use crate::ActorProcessingErr;
use crate::ActorRef;

use crate::factory::*;

#[derive(Debug, Hash, Clone, Eq, PartialEq)]
struct TestKey {
    id: u64,
}

#[derive(Debug)]
enum TestMessage {
    /// Doh'k
    #[allow(dead_code)]
    Count(u16),
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

struct TestWorker {
    id_map: Arc<dashmap::DashSet<usize>>,
}
#[cfg(feature = "cluster")]
impl crate::Message for TestMessage {}

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

                self.id_map.insert(state.wid);

                // job finished, on success or err we report back to the factory
                state
                    .factory
                    .cast(FactoryMessage::Finished(state.wid, job.key))?;
            }
        }
        Ok(())
    }
}

struct TestWorkerBuilder {
    id_map: Arc<dashmap::DashSet<usize>>,
}

impl WorkerBuilder<TestWorker, ()> for TestWorkerBuilder {
    fn build(&self, _wid: usize) -> (TestWorker, ()) {
        (
            TestWorker {
                id_map: self.id_map.clone(),
            },
            (),
        )
    }
}

#[crate::concurrency::test]
#[tracing_test::traced_test]
async fn test_worker_pool_adjustment_manual() {
    // Setup

    let id_map = Arc::new(dashmap::DashSet::new());

    let worker_builder = TestWorkerBuilder {
        id_map: id_map.clone(),
    };
    let factory_definition = Factory::<
        TestKey,
        TestMessage,
        (),
        TestWorker,
        routing::RoundRobinRouting<TestKey, TestMessage>,
        queues::DefaultQueue<TestKey, TestMessage>,
    >::default();
    let (factory, factory_handle) = Actor::spawn(
        None,
        factory_definition,
        FactoryArguments {
            num_initial_workers: 4,
            queue: queues::DefaultQueue::default(),
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

    // Act
    for i in 0..50 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                key: TestKey { id: 1 },
                msg: TestMessage::Count(i),
                options: JobOptions::default(),
                accepted: None,
            }))
            .expect("Failed to send to factory");
    }

    crate::periodic_check(
        || {
            // The map should only have 4 entries, the id of each worker
            id_map.len() == 4
        },
        Duration::from_millis(200),
    )
    .await;

    // Setup new state
    id_map.clear();
    factory
        .cast(FactoryMessage::AdjustWorkerPool(25))
        .expect("Failed to send to factory");

    // Act again
    for i in 0..50 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                key: TestKey { id: 1 },
                msg: TestMessage::Count(i),
                options: JobOptions::default(),
                accepted: None,
            }))
            .expect("Failed to send to factory");
    }

    crate::periodic_check(
        || {
            // The map should have 25 entries, the id of each worker
            id_map.len() == 25
        },
        Duration::from_millis(200),
    )
    .await;

    // Cleanup
    // wait for factory termination
    factory.stop(None);
    factory_handle.await.unwrap();
}

#[crate::concurrency::test]
#[tracing_test::traced_test]
async fn test_worker_pool_adjustment_automatic() {
    // Setup

    struct DynamicWorkerController;

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl WorkerCapacityController for DynamicWorkerController {
        #[cfg(feature = "async-trait")]
        async fn get_pool_size(&mut self, _current: usize) -> usize {
            10
        }

        #[cfg(not(feature = "async-trait"))]
        fn get_pool_size(&mut self, _current: usize) -> BoxFuture<'_, usize> {
            async { 10 }.boxed()
        }
    }

    let id_map = Arc::new(dashmap::DashSet::new());

    let worker_builder = TestWorkerBuilder {
        id_map: id_map.clone(),
    };
    let factory_definition = Factory::<
        TestKey,
        TestMessage,
        (),
        TestWorker,
        routing::RoundRobinRouting<TestKey, TestMessage>,
        queues::DefaultQueue<TestKey, TestMessage>,
    >::default();
    let (factory, factory_handle) = Actor::spawn(
        None,
        factory_definition,
        FactoryArguments {
            num_initial_workers: 4,
            queue: queues::DefaultQueue::default(),
            router: Default::default(),
            capacity_controller: Some(Box::new(DynamicWorkerController)),
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

    // Act
    for i in 0..50 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                key: TestKey { id: 1 },
                msg: TestMessage::Count(i),
                options: JobOptions::default(),
                accepted: None,
            }))
            .expect("Failed to send to factory");
    }

    crate::periodic_check(
        || {
            // The map should only have 4 entries, the id of each worker
            id_map.len() == 4
        },
        Duration::from_millis(200),
    )
    .await;

    // Setup new state
    id_map.clear();
    // now we wait for the ping to change the worker pool to 10
    crate::concurrency::sleep(Duration::from_millis(300)).await;

    // Act again
    for i in 0..50 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                key: TestKey { id: 1 },
                msg: TestMessage::Count(i),
                options: JobOptions::default(),
                accepted: None,
            }))
            .expect("Failed to send to factory");
    }

    crate::periodic_check(
        || {
            // The map should have 10 entries, the id of each worker
            id_map.len() == 10
        },
        Duration::from_millis(200),
    )
    .await;

    // Cleanup
    // wait for factory termination
    factory.stop(None);
    factory_handle.await.unwrap();
}
