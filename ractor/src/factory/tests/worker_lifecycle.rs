// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use std::marker::PhantomData;
use std::sync::atomic::AtomicU16;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::concurrency::sleep;
use crate::concurrency::Duration;
use crate::concurrency::JoinHandle;
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
impl crate::Message for MyWorkerMessage {}

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
    fn build(&mut self, _wid: crate::factory::WorkerId) -> (MyWorker, ()) {
        (
            MyWorker {
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
async fn test_worker_death_restarts_and_gets_next_message() {
    let counter = Arc::new(AtomicU16::new(0));
    let worker_builder = MyWorkerBuilder {
        counter: counter.clone(),
    };
    let factory_definition = Factory::<
        (),
        MyWorkerMessage,
        (),
        MyWorker,
        routing::RoundRobinRouting<(), MyWorkerMessage>,
        queues::DefaultQueue<(), MyWorkerMessage>,
    >::default();
    let (factory, factory_handle) = Actor::spawn(
        None,
        factory_definition,
        FactoryArguments {
            num_initial_workers: 1,
            queue: Default::default(),
            router: Default::default(),
            capacity_controller: None,
            dead_mans_switch: None,
            discard_handler: None,
            discard_settings: DiscardSettings::Static {
                limit: 10,
                mode: DiscardMode::Newest,
            },
            lifecycle_hooks: None,
            worker_builder: Box::new(worker_builder),
            stats: None,
        },
    )
    .await
    .expect("Failed to spawn factory");

    // Now we need to send a specific sequence of events
    // first make the worker busy so we can be sure the "boom" job is enqueued
    factory
        .cast(FactoryMessage::Dispatch(Job {
            key: (),
            msg: MyWorkerMessage::Busy,
            options: JobOptions::default(),
            accepted: None,
        }))
        .expect("Failed to send message to factory");
    // After it's done being "busy" have it blow up, but we need to push some jobs into the queue asap so that
    // there's a backlog of work
    factory
        .cast(FactoryMessage::Dispatch(Job {
            key: (),
            msg: MyWorkerMessage::Boom,
            options: JobOptions::default(),
            accepted: None,
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
                accepted: None,
            }))
            .expect("Failed to send message to factory");
    }
    // now wait for everything to propogate
    crate::periodic_check(
        || counter.load(Ordering::Relaxed) == 5,
        Duration::from_secs(1),
    )
    .await;

    // Cleanup
    factory.stop(None);
    factory_handle.await.unwrap();
}

enum MockFactoryMessage {
    Boom(bool, crate::concurrency::MpscUnboundedSender<()>),
}

#[cfg(feature = "cluster")]
impl Message for MockFactoryMessage {}

/// Create a mock factory with specific, defined logic
async fn make_mock_factory<F, TKey, TMessage>(
    mock_logic: F,
) -> (
    ActorRef<FactoryMessage<TKey, RetriableMessage<TKey, TMessage>>>,
    JoinHandle<()>,
)
where
    TKey: JobKey,
    TMessage: Message,
    F: Fn(&mut Job<TKey, RetriableMessage<TKey, TMessage>>) + Send + Sync + 'static,
{
    struct MockFactory<F2, TKey2, TMessage2>
    where
        TKey2: JobKey,
        TMessage2: Message,
        F2: Fn(&mut Job<TKey2, RetriableMessage<TKey2, TMessage2>>) + Send + Sync + 'static,
    {
        message_handler_logic: F2,
        _k: PhantomData<fn() -> TKey2>,
        _m: PhantomData<fn() -> TMessage2>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl<F2, TKey2, TMessage2> Actor for MockFactory<F2, TKey2, TMessage2>
    where
        TKey2: JobKey,
        TMessage2: Message,
        F2: Fn(&mut Job<TKey2, RetriableMessage<TKey2, TMessage2>>) + Send + Sync + 'static,
    {
        type Msg = FactoryMessage<TKey2, RetriableMessage<TKey2, TMessage2>>;
        type State = ();
        type Arguments = ();

        async fn pre_start(
            &self,
            _: ActorRef<Self::Msg>,
            _: Self::Arguments,
        ) -> Result<Self::State, ActorProcessingErr> {
            tracing::info!("pre-starting the mock factory");
            Ok(())
        }

        async fn handle(
            &self,
            _myself: ActorRef<Self::Msg>,
            message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            tracing::info!("message handler called on the mock factory");
            if let FactoryMessage::Dispatch(mut job) = message {
                (self.message_handler_logic)(&mut job);
            }
            Ok(())
        }
    }

    Actor::spawn(
        None,
        MockFactory {
            message_handler_logic: mock_logic,
            _k: PhantomData,
            _m: PhantomData,
        },
        (),
    )
    .await
    .expect("Failed to spawn mock factory")
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_factory_can_silent_retry() {
    let num_retries = Arc::new(AtomicU8::new(0));

    let (factory, handle) = make_mock_factory(move |msg| {
        match msg.msg.message.as_mut() {
            Some(MockFactoryMessage::Boom(should_panic, response)) => {
                if *should_panic {
                    *should_panic = false;
                    // Simulate a panic by dropping the message without sending a reply or marking it completed
                    tracing::info!("Dropping the request without marking it completed!");
                    return;
                }
                tracing::info!("Sending response");
                _ = response.send(());
            }
            _ => {
                tracing::info!("Got handler with no message payload");
            }
        }
        msg.msg.completed();
    })
    .await;
    let (tx, mut rx) = crate::concurrency::mpsc_unbounded();

    let message = MockFactoryMessage::Boom(true, tx);
    let mut job = RetriableMessage::from_job(
        Job {
            accepted: None,
            options: JobOptions::default(),
            key: (),
            msg: message,
        },
        MessageRetryStrategy::Count(1),
        factory.clone(),
    );
    let counter = num_retries.clone();
    job.msg.set_retry_hook(move |_| {
        tracing::info!("Job is being retried");
        counter.fetch_add(1, Ordering::SeqCst);
    });
    factory
        .cast(FactoryMessage::Dispatch(job))
        .expect("Failed to dispatch job");

    // wait for RPC to complete
    let result = rx.recv().await;
    assert!(result.is_some());

    assert_eq!(1, num_retries.load(Ordering::SeqCst));
    factory.stop(None);
    handle.await.unwrap();
}
