// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Rate limiting tests for factories

use std::collections::HashMap;
use std::sync::atomic::AtomicU16;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::factory::routing::{RouteResult, Router};
use crate::factory::*;
use crate::*;

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
        Ok(())
    }
}

struct TestWorkerBuilder;

impl WorkerBuilder<TestWorker, ()> for TestWorkerBuilder {
    fn build(&mut self, _wid: crate::factory::WorkerId) -> (TestWorker, ()) {
        (TestWorker, ())
    }
}

#[derive(Debug)]
struct BasicRateLimitWrapper<TRouter: State> {
    router: TRouter,
    hard_cap: usize,
    count: usize,
}

impl<TKey, TMsg, TRouter> Router<TKey, TMsg> for BasicRateLimitWrapper<TRouter>
where
    TKey: JobKey,
    TMsg: Message,
    TRouter: Router<TKey, TMsg>,
{
    fn route_message(
        &mut self,
        job: Job<TKey, TMsg>,
        pool_size: usize,
        worker_hint: Option<WorkerId>,
        worker_pool: &mut HashMap<WorkerId, WorkerProperties<TKey, TMsg>>,
    ) -> Result<RouteResult<TKey, TMsg>, ActorProcessingErr> {
        tracing::info!("Rate limit state: {}/{}", self.count, self.hard_cap);
        if self.count >= self.hard_cap {
            Ok(RouteResult::RateLimited(job))
        } else {
            let result = self
                .router
                .route_message(job, pool_size, worker_hint, worker_pool);
            if matches!(result, Ok(RouteResult::Handled)) {
                self.count += 1;
            }
            result
        }
    }

    fn choose_target_worker(
        &mut self,
        job: &Job<TKey, TMsg>,
        pool_size: usize,
        worker_hint: Option<WorkerId>,
        worker_pool: &HashMap<WorkerId, WorkerProperties<TKey, TMsg>>,
    ) -> Option<WorkerId> {
        self.router
            .choose_target_worker(job, pool_size, worker_hint, worker_pool)
    }

    fn is_factory_queueing(&self) -> bool {
        self.router.is_factory_queueing()
    }
}

#[crate::concurrency::test]
#[tracing_test::traced_test]
async fn test_factory_rate_limited() {
    // Setup
    let discard_counter = Arc::new(AtomicU16::new(0));

    struct TestDiscarder {
        counter: Arc<AtomicU16>,
    }
    impl DiscardHandler<(), ()> for TestDiscarder {
        fn discard(&self, reason: DiscardReason, _job: &mut Job<(), ()>) {
            tracing::debug!("Discarding message, reason {reason:?}");
            if reason == DiscardReason::RateLimited {
                let _ = self.counter.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    let worker_builder = TestWorkerBuilder;
    let arguments = FactoryArguments::builder()
        .num_initial_workers(1)
        .queue(Default::default())
        .router(BasicRateLimitWrapper {
            count: 0,
            hard_cap: 5,
            router: routing::QueuerRouting::<(), ()>::default(),
        })
        .worker_builder(Box::new(worker_builder))
        .discard_handler(Arc::new(TestDiscarder {
            counter: discard_counter.clone(),
        }))
        .build();

    let factory_definition = Factory::<
        (),
        (),
        (),
        TestWorker,
        BasicRateLimitWrapper<routing::QueuerRouting<(), ()>>,
        queues::DefaultQueue<(), ()>,
    >::default();
    let (factory, factory_handle) = Actor::spawn(None, factory_definition, arguments)
        .await
        .expect("Failed to spawn factory");

    // Test
    for _ in 0..10 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                accepted: None,
                key: (),
                msg: (),
                options: JobOptions::default(),
            }))
            .expect("Failed to send message to factory");
    }

    // Drain factory
    factory
        .cast(FactoryMessage::DrainRequests)
        .expect("Failed to message factory");

    // once the factory is stopped, the shutdown handler should have been called
    crate::concurrency::timeout(Duration::from_secs(1), factory_handle)
        .await
        .expect("Failed to drain requests in 1s")
        .expect("Failed to join factory handle");

    // Check that we rate-limited 5 messages, but allowed 5 through
    assert_eq!(5, discard_counter.load(Ordering::SeqCst));
}
