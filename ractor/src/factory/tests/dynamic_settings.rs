// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Tests around dynamic setting changes in factories

use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};

use crate::concurrency::sleep;
use crate::factory::*;
use crate::{Actor, ActorProcessingErr, ActorRef};

struct TestWorker;

#[cfg_attr(feature = "async-trait", crate::async_trait)]
impl Worker for TestWorker {
    type State = ();
    type Key = ();
    type Message = ();
    type Arguments = ();

    async fn pre_start(
        &self,
        _wid: WorkerId,
        _factory: &ActorRef<FactoryMessage<Self::Key, Self::Message>>,
        _args: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        tracing::debug!("Worker started");
        Ok(())
    }

    async fn handle(
        &self,
        _wid: WorkerId,
        _factory: &ActorRef<FactoryMessage<Self::Key, Self::Message>>,
        _job: Job<Self::Key, Self::Message>,
        _state: &mut Self::State,
    ) -> Result<Self::Key, ActorProcessingErr> {
        tracing::debug!("Worker received dispatch");
        sleep(Duration::from_millis(10)).await;
        Ok(())
    }
}

struct TestWorkerBuilder;

impl WorkerBuilder<TestWorker, ()> for TestWorkerBuilder {
    fn build(&mut self, _wid: crate::factory::WorkerId) -> (TestWorker, ()) {
        (TestWorker, ())
    }
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_dynamic_settings() {
    let counter_one = Arc::new(AtomicU8::new(0));
    let counter_two = Arc::new(AtomicU8::new(0));

    struct TestDiscardHandler {
        counter: Arc<AtomicU8>,
    }

    impl DiscardHandler<(), ()> for TestDiscardHandler {
        fn discard(&self, _reason: DiscardReason, _job: &mut Job<(), ()>) {
            self.counter.fetch_add(1, Ordering::SeqCst);
        }
    }

    let factory_definition = Factory::<
        (),
        (),
        (),
        TestWorker,
        routing::QueuerRouting<(), ()>,
        queues::DefaultQueue<(), ()>,
    >::default();
    let args = FactoryArguments::builder()
        .num_initial_workers(1)
        .queue(Default::default())
        .router(Default::default())
        .worker_builder(Box::new(TestWorkerBuilder))
        .discard_handler(Arc::new(TestDiscardHandler {
            counter: counter_one.clone(),
        }))
        .discard_settings(DiscardSettings::Static {
            limit: 0,
            mode: DiscardMode::Newest,
        })
        .build();
    let (factory, factory_handle) = Actor::spawn(None, factory_definition, args)
        .await
        .expect("Failed to spawn factory");

    // check that there's 1 worker running
    let worker_count = factory
        .call(
            FactoryMessage::GetAvailableCapacity,
            Some(Duration::from_secs(1)),
        )
        .await
        .expect("Failed to message factory")
        .expect("Failed to get reply from factory");
    assert_eq!(1, worker_count);

    // send 2 messages, making sure we discard one.
    for _ in 0..2 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                accepted: None,
                key: (),
                msg: (),
                options: JobOptions::default(),
            }))
            .expect("Failed to message factory");
    }

    // fetch the worker count, to make sure the factory has processed all
    // the messages from above
    _ = factory
        .call(
            FactoryMessage::GetAvailableCapacity,
            Some(Duration::from_secs(1)),
        )
        .await
        .expect("Failed to message factory")
        .expect("Failed to get reply from factory");

    // wait for worker to finish
    sleep(Duration::from_millis(100)).await;

    // update factory logic
    factory
        .cast(FactoryMessage::UpdateSettings(
            UpdateSettingsRequest::builder()
                .worker_count(2)
                .discard_handler(Some(Arc::new(TestDiscardHandler {
                    counter: counter_two.clone(),
                })))
                // these are the defaults, but exercise the path for sanity.
                .capacity_controller(None)
                .dead_mans_switch(None)
                .discard_settings(DiscardSettings::Static {
                    limit: 0,
                    mode: DiscardMode::Newest,
                })
                .lifecycle_hooks(None)
                .stats(None)
                .build(),
        ))
        .expect("Failed to send request to update factory");

    // Check updated worker count
    let worker_count = factory
        .call(
            FactoryMessage::GetAvailableCapacity,
            Some(Duration::from_secs(1)),
        )
        .await
        .expect("Failed to message factory")
        .expect("Failed to get reply from factory");
    assert_eq!(2, worker_count);

    // send 3 messages, making sure we discard one with the
    // new handler
    for _ in 0..3 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                accepted: None,
                key: (),
                msg: (),
                options: JobOptions::default(),
            }))
            .expect("Failed to message factory");
    }

    // Make sure messages processed
    _ = factory
        .call(
            FactoryMessage::GetAvailableCapacity,
            Some(Duration::from_secs(1)),
        )
        .await
        .expect("Failed to message factory")
        .expect("Failed to get reply from factory");

    // Check both discard handler counters
    assert_eq!(counter_one.load(Ordering::SeqCst), 1);
    assert_eq!(counter_two.load(Ordering::SeqCst), 1);

    // Cleanup
    factory.stop(None);
    factory_handle.await.unwrap();
}
