// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Rate limiting tests for factories

use std::sync::atomic::AtomicU16;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::concurrency::sleep;
use crate::concurrency::Duration;
use crate::factory::routing::*;
use crate::factory::*;
use crate::*;

struct TestWorker;

#[cfg_attr(
    all(
        feature = "async-trait",
        not(all(target_arch = "wasm32", target_os = "unknown"))
    ),
    crate::async_trait
)]
#[cfg_attr(
    all(
        feature = "async-trait",
       all(target_arch = "wasm32", target_os = "unknown")
    ),
    crate::async_trait(?Send)
)]
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

struct BasicRateLimiter {
    hard_cap: usize,
    count: usize,
}

impl RateLimiter for BasicRateLimiter {
    fn bump(&mut self) {
        self.count += 1;
    }

    fn check(&mut self) -> bool {
        self.count < self.hard_cap
    }
}

async fn test_factory_rate_limiting_common<TRouter: Router<(), ()>>(router: TRouter) {
    // Setup
    let discard_counter = Arc::new(AtomicU16::new(0));

    struct TestDiscarder {
        counter: Arc<AtomicU16>,
    }
    impl DiscardHandler<(), ()> for TestDiscarder {
        fn discard(&self, reason: DiscardReason, _job: &mut Job<(), ()>) {
            tracing::debug!("Discarding job, reason {reason:?}");
            if reason == DiscardReason::RateLimited {
                let _ = self.counter.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    let worker_builder = TestWorkerBuilder;
    let arguments = FactoryArguments::builder()
        .num_initial_workers(1)
        .queue(Default::default())
        .router(
            // create a rate-limited message router which routes up to a hard cap, then
            // "rate limits" everything else
            RateLimitedRouter::builder()
                .router(router)
                .rate_limiter(BasicRateLimiter {
                    count: 0,
                    hard_cap: 5,
                })
                .build(),
        )
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
        RateLimitedRouter<TRouter, BasicRateLimiter>,
        queues::DefaultQueue<(), ()>,
    >::default();
    let (factory, factory_handle) = Actor::spawn(None, factory_definition, arguments)
        .await
        .expect("Failed to spawn factory");

    // Test
    for _ in 0..9 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                accepted: None,
                key: (),
                msg: (),
                options: JobOptions::default(),
            }))
            .expect("Failed to send message to factory");
    }

    // sleep a little to let the worker be in the "ratelim" state
    // as it'll have routed the max allowable number of requests
    sleep(Duration::from_millis(100)).await;
    // send an additional request, which should be marked ratelimiting before
    // even being queued
    factory
        .cast(FactoryMessage::Dispatch(Job {
            accepted: None,
            key: (),
            msg: (),
            options: JobOptions::default(),
        }))
        .expect("Failed to send message to factory");

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

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_factory_rate_limiting_queuer() {
    test_factory_rate_limiting_common::<QueuerRouting<(), ()>>(Default::default()).await
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_factory_rate_limiting_sticky_queuer() {
    test_factory_rate_limiting_common::<StickyQueuerRouting<(), ()>>(Default::default()).await
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_factory_rate_limiting_key_persistent() {
    test_factory_rate_limiting_common::<KeyPersistentRouting<(), ()>>(Default::default()).await
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_factory_rate_limiting_round_robin() {
    test_factory_rate_limiting_common::<RoundRobinRouting<(), ()>>(Default::default()).await
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_factory_rate_limiting_custom_hash() {
    struct MyHasher;

    impl CustomHashFunction<()> for MyHasher {
        fn hash(&self, _key: &(), _worker_count: usize) -> usize {
            0
        }
    }

    let router = CustomRouting::new(MyHasher);

    test_factory_rate_limiting_common(router).await
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_leaky_bucket_rate_limiting() {
    // Setup

    use crate::concurrency::sleep;
    let discard_counter = Arc::new(AtomicU16::new(0));

    struct TestDiscarder {
        counter: Arc<AtomicU16>,
    }
    impl DiscardHandler<(), ()> for TestDiscarder {
        fn discard(&self, reason: DiscardReason, _job: &mut Job<(), ()>) {
            tracing::debug!("Discarding job, reason {reason:?}");
            if reason == DiscardReason::RateLimited {
                let _ = self.counter.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    let worker_builder = TestWorkerBuilder;

    // Setup rate limited router
    let limiter = LeakyBucketRateLimiter::builder()
        .max(5)
        .initial(5)
        .refill(1)
        .interval(Duration::from_millis(100))
        .build();

    let router = RateLimitedRouter::builder()
        .router(routing::QueuerRouting::<(), ()>::default())
        .rate_limiter(limiter)
        .build();
    let arguments = FactoryArguments::builder()
        .num_initial_workers(1)
        .queue(Default::default())
        .router(router)
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
        RateLimitedRouter<routing::QueuerRouting<(), ()>, _>,
        queues::DefaultQueue<(), ()>,
    >::default();
    let (factory, factory_handle) = Actor::spawn(None, factory_definition, arguments)
        .await
        .expect("Failed to spawn factory");

    // Test
    for _ in 0..6 {
        factory
            .cast(FactoryMessage::Dispatch(Job {
                accepted: None,
                key: (),
                msg: (),
                options: JobOptions::default(),
            }))
            .expect("Failed to send message to factory");
    }

    // sleep >100ms and we should be able to push another job
    sleep(crate::concurrency::Duration::from_millis(200)).await;
    factory
        .cast(FactoryMessage::Dispatch(Job {
            accepted: None,
            key: (),
            msg: (),
            options: JobOptions::default(),
        }))
        .expect("Failed to send message to factory");

    // Drain factory
    factory
        .cast(FactoryMessage::DrainRequests)
        .expect("Failed to message factory");

    // once the factory is stopped, the shutdown handler should have been called
    crate::concurrency::timeout(Duration::from_secs(1), factory_handle)
        .await
        .expect("Failed to drain requests in 1s")
        .expect("Failed to join factory handle");

    // Check that we rate-limited only 1 message, from the first batch
    assert_eq!(1, discard_counter.load(Ordering::SeqCst));
}
