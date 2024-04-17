// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::concurrency::sleep;
use crate::concurrency::Duration;
use crate::Actor;
use crate::ActorProcessingErr;
use crate::ActorRef;

use crate::common_test::periodic_check;
use crate::factory::Factory;
use crate::factory::FactoryLifecycleHooks;
use crate::factory::FactoryMessage;
use crate::factory::RoutingMode;
use crate::factory::WorkerBuilder;
use crate::factory::WorkerMessage;
use crate::factory::WorkerStartContext;

#[derive(Clone)]
struct AtomicHooks {
    state: Arc<AtomicU8>,
}

#[crate::async_trait]
impl FactoryLifecycleHooks<(), ()> for AtomicHooks {
    async fn on_factory_started(
        &self,
        _factory_ref: ActorRef<FactoryMessage<(), ()>>,
    ) -> Result<(), ActorProcessingErr> {
        self.state.store(1, Ordering::SeqCst);
        Ok(())
    }

    async fn on_factory_stopped(&self) -> Result<(), ActorProcessingErr> {
        self.state.store(3, Ordering::SeqCst);
        Ok(())
    }

    async fn on_factory_draining(
        &self,
        _factory_ref: ActorRef<FactoryMessage<(), ()>>,
    ) -> Result<(), ActorProcessingErr> {
        self.state.store(2, Ordering::SeqCst);
        Ok(())
    }
}

struct TestWorker;

#[async_trait::async_trait]
impl Actor for TestWorker {
    type State = Self::Arguments;
    type Msg = WorkerMessage<(), ()>;
    type Arguments = WorkerStartContext<(), (), ()>;

    async fn pre_start(
        &self,
        _: ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        // slow down factory startup waiting for workers to spawn
        sleep(Duration::from_millis(100)).await;
        Ok(args)
    }

    async fn post_stop(
        &self,
        _: ActorRef<Self::Msg>,
        _: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        // slow down factory shutdown waiting for workers to die
        sleep(Duration::from_millis(100)).await;
        Ok(())
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
                tracing::warn!("Worker received message");
                // job finished, on success or err we report back to the factory
                state.factory.cast({
                    job.key;
                    FactoryMessage::Finished(state.wid, ())
                })?;
            }
        }
        Ok(())
    }
}

struct TestWorkerBuilder;

impl WorkerBuilder<TestWorker, ()> for TestWorkerBuilder {
    fn build(&self, _wid: crate::factory::WorkerId) -> (TestWorker, ()) {
        (TestWorker, ())
    }
}

#[crate::concurrency::test]
async fn test_lifecycle_hooks() {
    let hooks = AtomicHooks {
        state: Arc::new(AtomicU8::new(0)),
    };

    let worker_builder = TestWorkerBuilder;
    let factory_definition = Factory::<(), (), (), TestWorker> {
        worker_count: 1,
        routing_mode: RoutingMode::Queuer,
        lifecycle_hooks: Some(Box::new(hooks.clone())),
        ..Default::default()
    };

    let (factory, factory_handle) =
        Actor::spawn(None, factory_definition, Box::new(worker_builder))
            .await
            .expect("Failed to spawn factory");

    // startup has some delay creating workers, so we shouldn't see on_started called immediately
    assert_eq!(0, hooks.state.load(Ordering::SeqCst));
    periodic_check(
        || hooks.state.load(Ordering::SeqCst) == 1,
        Duration::from_millis(500),
    )
    .await;

    assert_eq!(1, hooks.state.load(Ordering::SeqCst));
    factory
        .cast(FactoryMessage::DrainRequests)
        .expect("Failed to message factory");

    // give a little time to see if the factory moved to the draining state
    periodic_check(
        || hooks.state.load(Ordering::SeqCst) == 2,
        Duration::from_millis(500),
    )
    .await;
    // once the factory is stopped, the shutdown handler should have been called
    factory_handle.await.unwrap();
    assert_eq!(3, hooks.state.load(Ordering::SeqCst));
}
