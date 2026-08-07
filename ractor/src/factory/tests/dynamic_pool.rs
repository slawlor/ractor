// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Tests around dynamic worker pool configuration. These require a more complex setup than the basic tests
//! and therefore are separated out

use std::sync::Arc;

#[cfg(not(feature = "async-trait"))]
use futures::future::BoxFuture;
#[cfg(not(feature = "async-trait"))]
use futures::FutureExt;

use crate::concurrency::Duration;
use crate::factory::*;
use crate::Actor;
use crate::ActorProcessingErr;
use crate::ActorRef;

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
impl Worker for TestWorker {
    type Key = TestKey;
    type Message = TestMessage;
    type State = Self::Arguments;
    type Arguments = ();

    async fn pre_start(
        &self,
        _wid: WorkerId,
        _factory: &ActorRef<FactoryMessage<Self::Key, Self::Message>>,
        startup_context: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(startup_context)
    }

    async fn handle(
        &self,
        wid: WorkerId,
        _factory: &ActorRef<FactoryMessage<Self::Key, Self::Message>>,
        Job { msg, key, .. }: Job<Self::Key, Self::Message>,
        _state: &mut Self::State,
    ) -> Result<Self::Key, ActorProcessingErr> {
        tracing::debug!("Worker received {:?}", msg);

        self.id_map.insert(wid);
        Ok(key)
    }
}

struct TestWorkerBuilder {
    id_map: Arc<dashmap::DashSet<usize>>,
}

#[derive(Debug)]
enum OrderedMessage {
    Block,
    Record,
}

#[cfg(feature = "cluster")]
impl crate::Message for OrderedMessage {}

struct OrderedWorker {
    events: crate::concurrency::MpscUnboundedSender<(WorkerId, &'static str)>,
    release: Arc<crate::concurrency::Notify>,
}

#[cfg_attr(feature = "async-trait", crate::async_trait)]
impl Worker for OrderedWorker {
    type Key = TestKey;
    type Message = OrderedMessage;
    type State = ();
    type Arguments = ();

    async fn pre_start(
        &self,
        _wid: WorkerId,
        _factory: &ActorRef<FactoryMessage<Self::Key, Self::Message>>,
        _: (),
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(())
    }

    async fn handle(
        &self,
        wid: WorkerId,
        _factory: &ActorRef<FactoryMessage<Self::Key, Self::Message>>,
        Job { key, msg, .. }: Job<Self::Key, Self::Message>,
        _state: &mut Self::State,
    ) -> Result<Self::Key, ActorProcessingErr> {
        match msg {
            OrderedMessage::Block => {
                let _ = self.events.send((wid, "block"));
                self.release.notified().await;
            }
            OrderedMessage::Record => {
                let _ = self.events.send((wid, "record"));
            }
        }
        Ok(key)
    }
}

impl WorkerBuilder<TestWorker, ()> for TestWorkerBuilder {
    fn build(&mut self, _wid: usize) -> (TestWorker, ()) {
        (
            TestWorker {
                id_map: self.id_map.clone(),
            },
            (),
        )
    }
}

async fn assert_key_persistent_resize_keeps_pending_key(
    initial_workers: usize,
    resized_workers: usize,
) {
    let key = (0..10_000)
        .map(|id| TestKey { id })
        .find(|key| {
            crate::factory::hash::hash_with_max(key, initial_workers)
                != crate::factory::hash::hash_with_max(key, resized_workers)
        })
        .expect("a key should map differently after resize");
    let (events, mut event_rx) = crate::concurrency::mpsc_unbounded();
    let release = Arc::new(crate::concurrency::Notify::new());
    let worker_events = events.clone();
    let worker_release = release.clone();
    let definition = Factory::<
        TestKey,
        OrderedMessage,
        (),
        OrderedWorker,
        routing::KeyPersistentRouting<TestKey, OrderedMessage>,
        queues::DefaultQueue<TestKey, OrderedMessage>,
    >::default();
    let arguments = FactoryArguments::builder()
        .worker_builder(Box::new(worker_builder(move |_wid| {
            (
                OrderedWorker {
                    events: worker_events.clone(),
                    release: worker_release.clone(),
                },
                (),
            )
        })))
        .num_initial_workers(initial_workers)
        .router(routing::KeyPersistentRouting::default())
        .queue(queues::DefaultQueue::default())
        .build();
    let (factory, handle) = Actor::spawn(None, definition, arguments).await.unwrap();

    factory
        .cast(FactoryMessage::Dispatch(Job::new(
            key.clone(),
            OrderedMessage::Block,
        )))
        .unwrap();
    let (original_worker, phase) =
        crate::concurrency::timeout(Duration::from_secs(1), event_rx.recv())
            .await
            .unwrap()
            .unwrap();
    assert_eq!(phase, "block");

    factory
        .cast(FactoryMessage::AdjustWorkerPool(resized_workers))
        .unwrap();
    let _ = factory
        .call(
            FactoryMessage::GetAvailableCapacity,
            Some(Duration::from_secs(1)),
        )
        .await
        .unwrap();
    factory
        .cast(FactoryMessage::Dispatch(Job::new(
            key,
            OrderedMessage::Record,
        )))
        .unwrap();

    assert!(
        crate::concurrency::timeout(Duration::from_millis(50), event_rx.recv())
            .await
            .is_err()
    );
    release.notify_one();

    let (next_worker, phase) = crate::concurrency::timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(phase, "record");
    assert_eq!(next_worker, original_worker);

    factory.stop(None);
    handle.await.unwrap();
}

#[crate::concurrency::test]
async fn key_persistent_grow_keeps_pending_key_on_the_original_worker() {
    assert_key_persistent_resize_keeps_pending_key(2, 3).await;
}

#[crate::concurrency::test]
async fn key_persistent_shrink_keeps_pending_key_on_the_original_worker() {
    assert_key_persistent_resize_keeps_pending_key(3, 2).await;
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
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
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
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
