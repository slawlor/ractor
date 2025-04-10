// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use std::sync::atomic::AtomicU16;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::Actor;
use crate::ActorProcessingErr;
use crate::ActorRef;
use tokio::sync::Notify;

use crate::factory::queues::Priority;
use crate::factory::queues::PriorityManager;
use crate::factory::queues::StandardPriority;
use crate::factory::*;

type TestKey = StandardPriority;

#[derive(Debug)]
enum TestMessage {
    /// Doh'k
    #[allow(dead_code)]
    Count(u16),
}
#[cfg(feature = "cluster")]
impl crate::Message for TestMessage {}

struct TestWorker {
    counters: [Arc<AtomicU16>; 5],
    signal: Arc<Notify>,
}

struct TestPriorityManager;

impl PriorityManager<StandardPriority, StandardPriority> for TestPriorityManager {
    fn get_priority(&self, job: &StandardPriority) -> Option<StandardPriority> {
        Some(*job)
    }
    fn is_discardable(&self, _job: &StandardPriority) -> bool {
        true
    }
}

#[cfg_attr(feature = "async-trait", crate::async_trait)]
impl Actor for TestWorker {
    type Msg = WorkerMessage<TestKey, TestMessage>;
    type State = (Self::Arguments, u16);
    type Arguments = WorkerStartContext<TestKey, TestMessage, ()>;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        startup_context: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok((startup_context, 0))
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
                    .0
                    .factory
                    .cast(FactoryMessage::WorkerPong(state.0.wid, time.elapsed()))?;
            }
            WorkerMessage::Dispatch(Job {
                key,
                msg: TestMessage::Count(count),
                ..
            }) => {
                self.counters[key.get_index()].fetch_add(count, Ordering::Relaxed);

                // job finished, on success or err we report back to the factory
                state
                    .0
                    .factory
                    .cast(FactoryMessage::Finished(state.0.wid, key))?;

                state.1 += 1;
                if state.1 == 5 {
                    self.signal.notify_one();
                    // wait to be notified back
                    self.signal.notified().await;
                    // reset the counter
                    state.1 = 0;
                }
            }
        }
        Ok(())
    }
}

struct TestWorkerBuilder {
    counters: [Arc<AtomicU16>; 5],
    signal: Arc<Notify>,
}

impl WorkerBuilder<TestWorker, ()> for TestWorkerBuilder {
    fn build(&mut self, _wid: usize) -> (TestWorker, ()) {
        (
            TestWorker {
                counters: self.counters.clone(),
                signal: self.signal.clone(),
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
async fn test_basic_priority_queueing() {
    // Setup
    // a counter for each priority
    let counters = [
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
        Arc::new(AtomicU16::new(0)),
    ];
    let signal = Arc::new(Notify::new());

    let factory_definition = Factory::<
        TestKey,
        TestMessage,
        (),
        TestWorker,
        routing::QueuerRouting<TestKey, TestMessage>,
        queues::PriorityQueue<
            TestKey,
            TestMessage,
            StandardPriority,
            TestPriorityManager,
            { StandardPriority::size() },
        >,
    >::default();
    let args = FactoryArguments::builder()
        .queue(queues::PriorityQueue::new(TestPriorityManager))
        .router(Default::default())
        .worker_builder(Box::new(TestWorkerBuilder {
            counters: counters.clone(),
            signal: signal.clone(),
        }))
        .build();
    let (factory, factory_handle) = Actor::spawn(None, factory_definition, args)
        .await
        .expect("Failed to spawn factory");

    // Act
    // Send 5 high pri and 5 low pri messages to the factory. Only the high pri should
    // be serviced before the notifier is triggered
    let pri = StandardPriority::Highest;
    for _i in 0..5 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                key: pri,
                msg: TestMessage::Count(1),
                options: JobOptions::default(),
                accepted: None,
            }))
            .expect("Failed to send to factory");
    }
    let pri = StandardPriority::BestEffort;
    for _i in 0..5 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                key: pri,
                msg: TestMessage::Count(1),
                options: JobOptions::default(),
                accepted: None,
            }))
            .expect("Failed to send to factory");
    }

    // wait for the factory to signal
    signal.notified().await;

    // check the counters
    let hpc = counters[0].load(Ordering::Relaxed);
    let lpc = counters[4].load(Ordering::Relaxed);
    assert_eq!(hpc, 5);
    assert_eq!(lpc, 0);

    // tell the factory to continue
    signal.notify_one();

    // wait for the next batch to complete
    signal.notified().await;
    signal.notify_one();

    let hpc = counters[0].load(Ordering::Relaxed);
    let lpc = counters[4].load(Ordering::Relaxed);
    assert_eq!(hpc, 5);
    assert_eq!(lpc, 5);

    // Cleanup
    // wait for factory termination
    factory.stop(None);
    factory_handle.await.unwrap();
}
