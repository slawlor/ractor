// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! General tests, more logic-specific tests are contained in sub-modules

use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};

use tokio::time::Duration;

use crate::{Actor, ActorCell, ActorHandler, ActorStatus, SpawnErr, SupervisionEvent};

mod supervisor;

#[tokio::test]
async fn test_panic_on_start_captured() {
    struct TestActor;

    #[async_trait::async_trait]
    impl ActorHandler for TestActor {
        type Msg = ();

        type State = ();

        async fn pre_start(&self, _this_actor: crate::ActorCell) -> Self::State {
            panic!("Boom!");
        }
    }

    let actor_output = Actor::spawn(None, TestActor).await;
    assert!(matches!(actor_output, Err(SpawnErr::StartupPanic(_))));
}

#[tokio::test]
async fn test_stop_higher_priority_over_messages() {
    let message_counter = Arc::new(AtomicU8::new(0u8));

    struct TestActor {
        counter: Arc<AtomicU8>,
    }

    #[async_trait::async_trait]
    impl ActorHandler for TestActor {
        type Msg = ();

        type State = ();

        async fn pre_start(&self, _this_actor: crate::ActorCell) -> Self::State {}

        async fn handle(
            &self,
            _myself: ActorCell,
            _message: Self::Msg,
            _state: &Self::State,
        ) -> Option<Self::State> {
            self.counter.fetch_add(1, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(100)).await;
            None
        }
    }

    let (actor, handle) = Actor::spawn(
        None,
        TestActor {
            counter: message_counter.clone(),
        },
    )
    .await
    .expect("Actor failed to start");

    // pump 10 messages on the queue
    for _i in 0..10 {
        actor
            .send_message::<TestActor>(())
            .expect("Failed to send message to actor");
    }

    // give some time to process the first message and start sleeping
    tokio::time::sleep(Duration::from_millis(10)).await;

    // followed by the "stop" signal
    actor.stop(None);

    // current async work should complete, so we're still "running" sleeping
    // on the first message
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(ActorStatus::Running, actor.get_status());
    assert!(!handle.is_finished());

    // now wait enough time for the first iteration to complete
    tokio::time::sleep(Duration::from_millis(150)).await;

    println!("Counter: {}", message_counter.load(Ordering::Relaxed));

    // actor should have "stopped"
    assert_eq!(ActorStatus::Stopped, actor.get_status());
    assert!(handle.is_finished());
    // counter should have bumped only a single time
    assert_eq!(1, message_counter.load(Ordering::Relaxed));
}

#[tokio::test]
async fn test_kill_terminates_work() {
    struct TestActor;

    #[async_trait::async_trait]
    impl ActorHandler for TestActor {
        type Msg = ();

        type State = ();

        async fn pre_start(&self, _this_actor: crate::ActorCell) -> Self::State {}

        async fn handle(
            &self,
            _myself: ActorCell,
            _message: Self::Msg,
            _state: &Self::State,
        ) -> Option<Self::State> {
            tokio::time::sleep(Duration::from_secs(10)).await;
            None
        }
    }

    let (actor, handle) = Actor::spawn(None, TestActor)
        .await
        .expect("Actor failed to start");

    actor
        .send_message::<TestActor>(())
        .expect("Failed to send message to actor");
    tokio::time::sleep(Duration::from_millis(10)).await;

    actor.kill();
    tokio::time::sleep(Duration::from_millis(10)).await;

    assert_eq!(ActorStatus::Stopped, actor.get_status());
    assert!(handle.is_finished());
}

#[tokio::test]
async fn test_stop_does_not_terminate_async_work() {
    struct TestActor;

    #[async_trait::async_trait]
    impl ActorHandler for TestActor {
        type Msg = ();

        type State = ();

        async fn pre_start(&self, _this_actor: crate::ActorCell) -> Self::State {}

        async fn handle(
            &self,
            _myself: ActorCell,
            _message: Self::Msg,
            _state: &Self::State,
        ) -> Option<Self::State> {
            tokio::time::sleep(Duration::from_millis(100)).await;
            None
        }
    }

    let (actor, handle) = Actor::spawn(None, TestActor)
        .await
        .expect("Actor failed to start");

    // send a work message followed by a stop message
    actor
        .send_message::<TestActor>(())
        .expect("Failed to send message to actor");
    tokio::time::sleep(Duration::from_millis(2)).await;
    actor.stop(None);

    // async work should complete, so we're still "running" sleeping
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(ActorStatus::Running, actor.get_status());
    assert!(!handle.is_finished());

    tokio::time::sleep(Duration::from_millis(100)).await;
    // now enouch time has passed, so we have proceesed the next message, which is stop
    // so we should be dead now
    assert_eq!(ActorStatus::Stopped, actor.get_status());
    assert!(handle.is_finished());
}

#[tokio::test]
async fn test_kill_terminates_supervision_work() {
    struct TestActor;

    #[async_trait::async_trait]
    impl ActorHandler for TestActor {
        type Msg = ();

        type State = ();

        async fn pre_start(&self, _this_actor: crate::ActorCell) -> Self::State {}

        async fn handle_supervisor_evt(
            &self,
            _myself: ActorCell,
            _message: SupervisionEvent,
            _state: &Self::State,
        ) -> Option<Self::State> {
            tokio::time::sleep(Duration::from_millis(100)).await;
            None
        }
    }

    let (actor, handle) = Actor::spawn(None, TestActor)
        .await
        .expect("Actor failed to start");

    // send some dummy event to cause the supervision stuff to hang
    actor
        .send_supervisor_evt(SupervisionEvent::ActorStarted(actor.clone()))
        .expect("Failed to send message to actor");
    tokio::time::sleep(Duration::from_millis(10)).await;

    actor.kill();
    tokio::time::sleep(Duration::from_millis(10)).await;

    assert_eq!(ActorStatus::Stopped, actor.get_status());
    assert!(handle.is_finished());
}
