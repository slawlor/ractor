// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Tests around draining a factory of current work, making sure all jobs execute before the factory exits

use std::sync::atomic::AtomicU16;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::concurrency::sleep;
use crate::concurrency::Duration;
use crate::factory::*;
use crate::Actor;
use crate::ActorProcessingErr;
use crate::ActorRef;

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

struct TestWorker {
    counter: Arc<AtomicU16>,
}

#[cfg_attr(feature = "async-trait", crate::async_trait)]
impl Actor for TestWorker {
    type Msg = WorkerMessage<TestKey, TestMessage>;
    type State = Self::Arguments;
    type Arguments = WorkerStartContext<TestKey, TestMessage, ()>;

    async fn pre_start(
        &self,
        _: ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(args)
    }

    async fn handle(
        &self,
        _: ActorRef<Self::Msg>,
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
                self.counter.fetch_add(1, Ordering::Relaxed);

                sleep(Duration::from_millis(5)).await;

                // job finished, on success or err we report back to the factory
                state
                    .factory
                    .cast(FactoryMessage::Finished(state.wid, job.key))?;
            }
        }
        Ok(())
    }
}

struct SlowWorkerBuilder {
    counter: Arc<AtomicU16>,
}

impl WorkerBuilder<TestWorker, ()> for SlowWorkerBuilder {
    fn build(&mut self, _wid: usize) -> (TestWorker, ()) {
        (
            TestWorker {
                counter: self.counter.clone(),
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
async fn test_request_draining() {
    let counter = Arc::new(AtomicU16::new(0));

    let worker_builder = SlowWorkerBuilder {
        counter: counter.clone(),
    };
    let factory_definition = Factory::<
        TestKey,
        TestMessage,
        (),
        TestWorker,
        routing::QueuerRouting<TestKey, TestMessage>,
        queues::DefaultQueue<TestKey, TestMessage>,
    >::default();
    let args = FactoryArguments::builder()
        .num_initial_workers(2)
        .queue(Default::default())
        .router(Default::default())
        .worker_builder(Box::new(worker_builder))
        .build();
    let (factory, factory_handle) = Actor::spawn(None, factory_definition, args)
        .await
        .expect("Failed to spawn factory");

    for id in 0..999 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                key: TestKey { id },
                msg: TestMessage::Ok,
                options: JobOptions::default(),
                accepted: None,
            }))
            .expect("Failed to send to factory");
    }

    // start draining requests
    factory
        .cast(FactoryMessage::DrainRequests)
        .expect("Failed to contact factory");

    // try and push a new message, but it should be rejected since we're now draining
    let (tx, rx) = crate::concurrency::oneshot();
    factory
        .cast(FactoryMessage::Dispatch(Job {
            key: TestKey { id: 1000 },
            msg: TestMessage::Ok,
            options: JobOptions::default(),
            accepted: Some(tx.into()),
        }))
        .expect("Failed to send to factory");

    assert!(matches!(rx.await, Ok(Some(_))));

    // wait for factory to exit (it should once drained)
    factory_handle.await.unwrap();

    // check the counter
    assert_eq!(999, counter.load(Ordering::Relaxed));
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_worker_drains_when_worker_queueing() {
    let counter = Arc::new(AtomicU16::new(0));

    let worker_builder = SlowWorkerBuilder {
        counter: counter.clone(),
    };
    let factory_definition = Factory::<
        TestKey,
        TestMessage,
        (),
        TestWorker,
        routing::KeyPersistentRouting<TestKey, TestMessage>,
        queues::DefaultQueue<TestKey, TestMessage>,
    >::default();
    let args = FactoryArguments::builder()
        .num_initial_workers(1)
        .queue(Default::default())
        .router(Default::default())
        .worker_builder(Box::new(worker_builder))
        .build();
    let (factory, factory_handle) = Actor::spawn(None, factory_definition, args)
        .await
        .expect("Failed to spawn factory");

    for id in 0..999 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                key: TestKey { id },
                msg: TestMessage::Ok,
                options: JobOptions::default(),
                accepted: None,
            }))
            .expect("Failed to send to factory");
    }

    // start draining requests
    factory
        .cast(FactoryMessage::DrainRequests)
        .expect("Failed to contact factory");

    // wait for factory to exit (it should once drained)
    factory_handle.await.unwrap();

    // check the counter, make sure all messages processed
    assert_eq!(999, counter.load(Ordering::Relaxed));
}