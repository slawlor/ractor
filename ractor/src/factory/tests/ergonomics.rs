// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::concurrency::Duration;
use crate::factory::*;
use crate::rpc::CallResult;
use crate::{Actor, ActorProcessingErr, ActorRef, MessagingErr, RpcReplyPort};

#[derive(Debug)]
enum TestMessage {
    Increment,
    Echo(usize, RpcReplyPort<usize>),
}

#[cfg(feature = "cluster")]
impl crate::Message for TestMessage {}

struct TestWorker {
    handled: Arc<AtomicUsize>,
    generation: usize,
}

#[cfg_attr(feature = "async-trait", crate::async_trait)]
impl Worker for TestWorker {
    type Key = ();
    type Message = TestMessage;
    type State = ();
    type Arguments = ();

    async fn pre_start(
        &self,
        _wid: WorkerId,
        _factory: &FactoryRef<Self::Key, Self::Message>,
        arguments: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(arguments)
    }

    async fn handle(
        &self,
        _wid: WorkerId,
        _factory: &ActorRef<FactoryMessage<Self::Key, Self::Message>>,
        Job { key, msg, .. }: Job<Self::Key, Self::Message>,
        _state: &mut Self::State,
    ) -> Result<Self::Key, ActorProcessingErr> {
        self.handled.fetch_add(self.generation, Ordering::Relaxed);
        if let TestMessage::Echo(value, reply) = msg {
            let _ = reply.send(value);
        }
        Ok(key)
    }
}

#[derive(Default)]
struct NewJobStats {
    count: AtomicUsize,
}

// All callbacks other than the one this test needs use the trait's no-op defaults.
impl FactoryStatsLayer for NewJobStats {
    fn new_job(&self, _factory: &str) {
        self.count.fetch_add(1, Ordering::Relaxed);
    }
}

#[crate::concurrency::test]
async fn factory_ref_and_closure_builder_cover_common_operations() {
    let handled = Arc::new(AtomicUsize::new(0));
    let builds = Arc::new(AtomicUsize::new(0));
    let stats = Arc::new(NewJobStats::default());
    let worker_handled = handled.clone();
    let worker_builds = builds.clone();
    let mut generation = 0;
    let builder = worker_builder(move |_wid| {
        generation += 1;
        worker_builds.store(generation, Ordering::Relaxed);
        (
            TestWorker {
                handled: worker_handled.clone(),
                generation,
            },
            (),
        )
    });

    let definition = Factory::<
        (),
        TestMessage,
        (),
        TestWorker,
        routing::QueuerRouting<(), TestMessage>,
        queues::DefaultQueue<(), TestMessage>,
    >::default();
    let arguments = FactoryArguments::builder()
        .worker_builder(Box::new(builder))
        .num_initial_workers(1)
        .router(routing::QueuerRouting::default())
        .queue(queues::DefaultQueue::default())
        .stats(stats.clone())
        .build();

    let (factory, handle) = Actor::spawn(None, definition, arguments)
        .await
        .expect("factory should start");
    let factory: FactoryRef<(), TestMessage> = factory;
    let timeout = Some(Duration::from_secs(1));

    assert_eq!(
        CallResult::Success(1),
        factory
            .available_capacity(timeout)
            .await
            .expect("capacity query should be sent")
    );

    factory
        .dispatch((), TestMessage::Increment)
        .expect("message should be dispatched");
    factory
        .dispatch_with_options(
            (),
            TestMessage::Increment,
            JobOptions::new(Some(Duration::from_secs(1))),
        )
        .expect("message with options should be dispatched");
    factory
        .dispatch_job(Job::new((), TestMessage::Increment))
        .expect("configured job should be dispatched");

    assert_eq!(
        CallResult::Success(42),
        factory
            .call_job((), |reply| TestMessage::Echo(42, reply), timeout)
            .await
            .expect("worker call should be sent")
    );
    assert_eq!(
        CallResult::Success(84),
        factory
            .call_job_with_options(
                (),
                |reply| TestMessage::Echo(84, reply),
                JobOptions::default(),
                timeout,
            )
            .await
            .expect("worker call with options should be sent")
    );

    assert!(factory
        .queue_depth(timeout)
        .await
        .expect("queue depth query should be sent")
        .is_success());
    assert!(factory
        .active_workers(timeout)
        .await
        .expect("active worker query should be sent")
        .is_success());

    factory
        .adjust_worker_pool(2)
        .expect("pool adjustment should be sent");
    assert!(factory
        .available_capacity(timeout)
        .await
        .expect("capacity query should be sent")
        .is_success());
    assert_eq!(2, builds.load(Ordering::Relaxed));
    factory
        .update_settings(UpdateSettingsRequest::builder().worker_count(1).build())
        .expect("settings update should be sent");

    factory
        .drain_requests()
        .expect("drain request should be sent");
    crate::concurrency::timeout(Duration::from_secs(1), handle)
        .await
        .expect("factory should drain within the timeout")
        .expect("factory should exit cleanly");

    assert_eq!(5, handled.load(Ordering::Relaxed));
    // Both the factory and worker layers observe each new job.
    assert_eq!(10, stats.count.load(Ordering::Relaxed));
}

#[crate::concurrency::test]
async fn one_way_helper_recovers_job_when_factory_is_stopped() {
    let handled = Arc::new(AtomicUsize::new(0));
    let definition = Factory::<
        (),
        TestMessage,
        (),
        TestWorker,
        routing::QueuerRouting<(), TestMessage>,
        queues::DefaultQueue<(), TestMessage>,
    >::default();
    let arguments = FactoryArguments::builder()
        .worker_builder(Box::new(worker_builder(move |_wid| {
            (
                TestWorker {
                    handled: handled.clone(),
                    generation: 1,
                },
                (),
            )
        })))
        .num_initial_workers(1)
        .router(routing::QueuerRouting::default())
        .queue(queues::DefaultQueue::default())
        .build();

    let (factory, handle) = Actor::spawn(None, definition, arguments)
        .await
        .expect("factory should start");
    factory.stop(None);
    handle.await.expect("factory should stop cleanly");

    let error = factory
        .dispatch((), TestMessage::Increment)
        .expect_err("dispatch to a stopped factory should fail");
    match *error {
        MessagingErr::SendErr(FactoryMessage::Dispatch(Job {
            key: (),
            msg: TestMessage::Increment,
            accepted: None,
            ..
        })) => {}
        other => panic!("expected the failed job to be returned, got {other:?}"),
    }
}
