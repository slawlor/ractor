// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use std::sync::atomic::AtomicU16;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::common_test::periodic_check;
use crate::concurrency::sleep;
use crate::concurrency::Duration;
use crate::Actor;
use crate::ActorProcessingErr;
use crate::ActorRef;

use crate::factory::*;

struct MyWorker {
    counter: Arc<AtomicU16>,
}

#[derive(Debug)]
enum MyWorkerMessage {
    Busy,
    Increment,
    Boom,
}

#[cfg(feature = "cluster")]
impl Message for MyWorkerMessage {}

#[cfg_attr(feature = "async-trait", crate::async_trait)]
impl Actor for MyWorker {
    type State = Self::Arguments;
    type Msg = WorkerMessage<(), MyWorkerMessage>;
    type Arguments = WorkerStartContext<(), MyWorkerMessage, ()>;

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
                tracing::warn!("Worker received {:?}", job.msg);

                match job.msg {
                    MyWorkerMessage::Boom => {
                        panic!("Boom!");
                    }
                    MyWorkerMessage::Busy => {
                        sleep(Duration::from_millis(50)).await;
                    }
                    MyWorkerMessage::Increment => {
                        self.counter.fetch_add(1, Ordering::Relaxed);
                    }
                }

                // job finished, on success or err we report back to the factory
                state
                    .factory
                    .cast(FactoryMessage::Finished(state.wid, ()))?;
            }
        }
        Ok(())
    }
}

struct MyWorkerBuilder {
    counter: Arc<AtomicU16>,
}

impl WorkerBuilder<MyWorker, ()> for MyWorkerBuilder {
    fn build(&self, _wid: WorkerId) -> (MyWorker, ()) {
        (
            MyWorker {
                counter: self.counter.clone(),
            },
            (),
        )
    }
}

#[crate::concurrency::test]
#[tracing_test::traced_test]
async fn test_worker_death_restarts_and_gets_next_message() {
    let counter = Arc::new(AtomicU16::new(0));
    let worker_builder = MyWorkerBuilder {
        counter: counter.clone(),
    };
    let factory_definition = Factory::<(), MyWorkerMessage, (), MyWorker> {
        worker_count: 1,
        routing_mode: RoutingMode::Queuer,
        discard_threshold: Some(10),
        ..Default::default()
    };

    let (factory, factory_handle) =
        Actor::spawn(None, factory_definition, Box::new(worker_builder))
            .await
            .expect("Failed to spawn factory");

    // Now we need to send a specific sequence of events
    // first make the worker busy so we can be sure the "boom" job is enqueued
    factory
        .cast(FactoryMessage::Dispatch(Job {
            key: (),
            msg: MyWorkerMessage::Busy,
            options: JobOptions::default(),
        }))
        .expect("Failed to send message to factory");
    // After it's done being "busy" have it blow up, but we need to push some jobs into the queue asap so that
    // there's a backlog of work
    factory
        .cast(FactoryMessage::Dispatch(Job {
            key: (),
            msg: MyWorkerMessage::Boom,
            options: JobOptions::default(),
        }))
        .expect("Failed to send message to factory");

    // After the worker "boom"'s, it should be restarted and SHOULD dequeue following work
    // automatically without a new message needing to go to the factory
    for _i in 0..5 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                key: (),
                msg: MyWorkerMessage::Increment,
                options: JobOptions::default(),
            }))
            .expect("Failed to send message to factory");
    }

    periodic_check(
        || counter.load(Ordering::Relaxed) == 5,
        Duration::from_secs(2),
    )
    .await;

    // Cleanup
    factory.stop(None);
    factory_handle.await.unwrap();
}
