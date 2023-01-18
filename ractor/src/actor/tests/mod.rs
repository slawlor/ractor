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

use crate::{Actor, ActorCell, ActorRef, ActorStatus, SpawnErr, SupervisionEvent};

mod supervisor;

#[tokio::test]
async fn test_panic_on_start_captured() {
    struct TestActor;

    #[async_trait::async_trait]
    impl Actor for TestActor {
        type Msg = ();

        type State = ();

        async fn pre_start(&self, _this_actor: crate::ActorRef<Self>) -> Self::State {
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
    impl Actor for TestActor {
        type Msg = ();

        type State = ();

        async fn pre_start(&self, _this_actor: crate::ActorRef<Self>) -> Self::State {}

        async fn handle(
            &self,
            _myself: ActorRef<Self>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) {
            self.counter.fetch_add(1, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(100)).await;
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
            .send_message(())
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
    impl Actor for TestActor {
        type Msg = ();

        type State = ();

        async fn pre_start(&self, _this_actor: crate::ActorRef<Self>) -> Self::State {}

        async fn handle(
            &self,
            _myself: ActorRef<Self>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) {
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    }

    let (actor, handle) = Actor::spawn(None, TestActor)
        .await
        .expect("Actor failed to start");

    actor
        .send_message(())
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
    impl Actor for TestActor {
        type Msg = ();

        type State = ();

        async fn pre_start(&self, _this_actor: crate::ActorRef<Self>) -> Self::State {}

        async fn handle(
            &self,
            _myself: ActorRef<Self>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    let (actor, handle) = Actor::spawn(None, TestActor)
        .await
        .expect("Actor failed to start");

    // send a work message followed by a stop message
    actor
        .send_message(())
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
    impl Actor for TestActor {
        type Msg = ();

        type State = ();

        async fn pre_start(&self, _this_actor: crate::ActorRef<Self>) -> Self::State {}

        async fn handle_supervisor_evt(
            &self,
            _myself: ActorRef<Self>,
            _message: SupervisionEvent,
            _state: &mut Self::State,
        ) {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    let (actor, handle) = Actor::spawn(None, TestActor)
        .await
        .expect("Actor failed to start");

    // send some dummy event to cause the supervision stuff to hang
    let actor_cell: ActorCell = actor.clone().into();
    actor
        .send_supervisor_evt(SupervisionEvent::ActorStarted(actor_cell))
        .expect("Failed to send message to actor");
    tokio::time::sleep(Duration::from_millis(10)).await;

    actor.kill();
    tokio::time::sleep(Duration::from_millis(10)).await;

    assert_eq!(ActorStatus::Stopped, actor.get_status());
    assert!(handle.is_finished());
}

#[tokio::test]
async fn test_sending_message_to_invalid_actor_type() {
    struct TestActor1;
    struct TestMessage1;
    #[async_trait::async_trait]
    impl Actor for TestActor1 {
        type Msg = TestMessage1;
        type State = ();
        async fn pre_start(&self, _myself: ActorRef<Self>) -> Self::State {}
    }
    struct TestActor2;
    struct TestMessage2;
    #[async_trait::async_trait]
    impl Actor for TestActor2 {
        type Msg = TestMessage2;
        type State = ();
        async fn pre_start(&self, _myself: ActorRef<Self>) -> Self::State {}
    }

    let (actor1, handle1) = Actor::spawn(None, TestActor1)
        .await
        .expect("Failed to start test actor 1");
    let (actor2, handle2) = Actor::spawn(None, TestActor2)
        .await
        .expect("Failed to start test actor 2");

    // check that actor 2 can't receive a message for actor 1 type
    assert!(actor2
        .get_cell()
        .send_message::<TestActor1>(TestMessage1)
        .is_err());

    actor1.stop(None);
    actor2.stop(None);

    handle1.await.unwrap();
    handle2.await.unwrap();
}
