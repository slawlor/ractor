// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! General tests, more logic-specific tests are contained in sub-modules

use std::sync::atomic::AtomicU32;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use once_cell::sync::OnceCell;

use super::ThreadLocalActorSpawner;
use crate::actor::derived_actor::DerivedActorRef;
use crate::common_test::periodic_check;
use crate::concurrency::sleep;
use crate::concurrency::Duration;
use crate::thread_local::ThreadLocalActor as Actor;
use crate::ActorCell;
use crate::ActorProcessingErr;
use crate::ActorRef;
use crate::ActorStatus;
use crate::MessagingErr;
use crate::SpawnErr;
use crate::SupervisionEvent;

static LOCAL_SPAWNER: OnceCell<ThreadLocalActorSpawner> = OnceCell::new();

fn get_spawner() -> ThreadLocalActorSpawner {
    LOCAL_SPAWNER
        .get_or_init(ThreadLocalActorSpawner::new)
        .clone()
}

struct EmptyMessage;
#[cfg(feature = "cluster")]
impl crate::Message for EmptyMessage {}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
async fn test_panic_on_start_captured() {
    #[derive(Default)]
    struct TestActor;

    impl Actor for TestActor {
        type Msg = EmptyMessage;
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            panic!("Boom!");
        }
    }

    let actor_output = crate::spawn_local::<TestActor>((), get_spawner()).await;
    assert!(matches!(actor_output, Err(SpawnErr::StartupFailed(_))));
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_error_on_start_captured() {
    #[derive(Default)]
    struct TestActor;

    impl Actor for TestActor {
        type Msg = EmptyMessage;
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Err(From::from("boom"))
        }
    }

    let actor_output = crate::spawn_local::<TestActor>((), get_spawner()).await;
    assert!(matches!(actor_output, Err(SpawnErr::StartupFailed(_))));
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_stop_higher_priority_over_messages() {
    let message_counter = Arc::new(AtomicU8::new(0u8));

    #[derive(Default)]
    struct TestActor;

    impl Actor for TestActor {
        type Msg = EmptyMessage;
        type Arguments = Arc<AtomicU8>;
        type State = Arc<AtomicU8>;

        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self::Msg>,
            args: Arc<AtomicU8>,
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(args)
        }

        async fn handle(
            &self,
            _myself: ActorRef<Self::Msg>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            _state.fetch_add(1, Ordering::Relaxed);
            crate::concurrency::sleep(Duration::from_millis(100)).await;
            Ok(())
        }
    }

    let (actor, handle) = crate::spawn_local::<TestActor>(message_counter.clone(), get_spawner())
        .await
        .expect("Actor failed to start");

    #[cfg(feature = "cluster")]
    assert!(!actor.supports_remoting());

    // pump 10 messages on the queue
    for _i in 0..10 {
        actor
            .send_message(EmptyMessage)
            .expect("Failed to send message to actor");
    }

    // give some time to process the first message and start sleeping
    crate::concurrency::sleep(Duration::from_millis(10)).await;

    // followed by the "stop" signal
    actor.stop(None);

    // current async work should complete, so we're still "running" sleeping
    // on the first message
    crate::concurrency::sleep(Duration::from_millis(10)).await;
    assert_eq!(ActorStatus::Running, actor.get_status());
    assert!(!handle.is_finished());

    // now wait enough time for the first iteration to complete
    crate::concurrency::sleep(Duration::from_millis(150)).await;

    tracing::info!("Counter: {}", message_counter.load(Ordering::Relaxed));

    // actor should have "stopped"
    assert_eq!(ActorStatus::Stopped, actor.get_status());
    assert!(handle.is_finished());
    // counter should have bumped only a single time
    assert_eq!(1, message_counter.load(Ordering::Relaxed));
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_kill_terminates_work() {
    #[derive(Default)]
    struct TestActor;

    impl Actor for TestActor {
        type Msg = EmptyMessage;
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }

        async fn handle(
            &self,
            _myself: ActorRef<Self::Msg>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            crate::concurrency::sleep(Duration::from_secs(10)).await;
            Ok(())
        }
    }

    let (actor, handle) = crate::spawn_local::<TestActor>((), get_spawner())
        .await
        .expect("Actor failed to start");

    actor
        .send_message(EmptyMessage)
        .expect("Failed to send message to actor");
    crate::concurrency::sleep(Duration::from_millis(10)).await;

    actor.kill();
    crate::concurrency::sleep(Duration::from_millis(10)).await;

    assert_eq!(ActorStatus::Stopped, actor.get_status());
    assert!(handle.is_finished());
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_stop_does_not_terminate_async_work() {
    #[derive(Default)]
    struct TestActor;

    impl Actor for TestActor {
        type Msg = EmptyMessage;
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }

        async fn handle(
            &self,
            _myself: ActorRef<Self::Msg>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            crate::concurrency::sleep(Duration::from_millis(100)).await;
            Ok(())
        }
    }

    let (actor, handle) = crate::spawn_local::<TestActor>((), get_spawner())
        .await
        .expect("Actor failed to start");

    // send a work message followed by a stop message
    actor
        .send_message(EmptyMessage)
        .expect("Failed to send message to actor");
    crate::concurrency::sleep(Duration::from_millis(10)).await;
    actor.stop(None);

    // async work should complete, so we're still "running" sleeping
    crate::concurrency::sleep(Duration::from_millis(10)).await;
    assert_eq!(ActorStatus::Running, actor.get_status());
    assert!(!handle.is_finished());

    periodic_check(
        || ActorStatus::Stopped == actor.get_status(),
        Duration::from_millis(500),
    )
    .await;

    assert!(handle.is_finished());
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_kill_terminates_supervision_work() {
    #[derive(Default)]
    struct TestActor;

    impl Actor for TestActor {
        type Msg = EmptyMessage;
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            _myself: ActorRef<Self::Msg>,
            _message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            crate::concurrency::sleep(Duration::from_millis(100)).await;
            Ok(())
        }
    }

    let (actor, handle) = crate::spawn_local::<TestActor>((), get_spawner())
        .await
        .expect("Actor failed to start");

    // send some dummy event to cause the supervision stuff to hang
    let actor_cell: ActorCell = actor.clone().into();
    actor
        .send_supervisor_evt(SupervisionEvent::ActorStarted(actor_cell))
        .expect("Failed to send message to actor");
    crate::concurrency::sleep(Duration::from_millis(10)).await;

    actor.kill();
    crate::concurrency::sleep(Duration::from_millis(10)).await;

    assert_eq!(ActorStatus::Stopped, actor.get_status());
    assert!(handle.is_finished());
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_sending_message_to_invalid_actor_type() {
    #[derive(Default)]
    struct TestActor1;
    struct TestMessage1;
    #[cfg(feature = "cluster")]
    impl crate::Message for TestMessage1 {}

    impl Actor for TestActor1 {
        type Msg = TestMessage1;
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _myself: ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
    }
    #[derive(Default)]
    struct TestActor2;
    struct TestMessage2;
    #[cfg(feature = "cluster")]
    impl crate::Message for TestMessage2 {}

    impl Actor for TestActor2 {
        type Msg = TestMessage2;
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _myself: ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
    }

    let (actor1, handle1) = crate::spawn_local::<TestActor1>((), get_spawner())
        .await
        .expect("Failed to start test actor 1");
    let (actor2, handle2) = crate::spawn_local::<TestActor2>((), get_spawner())
        .await
        .expect("Failed to start test actor 2");

    // check that actor 2 can't receive a message for actor 1 type
    assert!(actor2
        .get_cell()
        .send_message::<TestMessage1>(TestMessage1)
        .is_err());

    actor1.stop(None);
    actor2.stop(None);

    handle1.await.unwrap();
    handle2.await.unwrap();
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_sending_message_to_dead_actor() {
    #[derive(Default)]
    struct TestActor;

    impl Actor for TestActor {
        type Msg = EmptyMessage;
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
    }

    let (actor, _) = crate::spawn_local::<TestActor>((), get_spawner())
        .await
        .expect("Actor failed to start");

    // Stop the actor and wait for it to die
    actor
        .stop_and_wait(None, None)
        .await
        .expect("Failed to stop");

    // assert that if we send a message, it doesn't transmit since
    // the receiver is dropped
    assert!(actor.cast(EmptyMessage).is_err());
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn stop_and_wait() {
    #[derive(Default)]
    struct SlowActor;

    impl Actor for SlowActor {
        type Msg = ();
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _: ActorRef<Self::Msg>,
            _: Self::Arguments,
        ) -> Result<Self::State, ActorProcessingErr> {
            crate::concurrency::sleep(Duration::from_millis(200)).await;
            Ok(())
        }
    }
    let (actor, handle) = crate::spawn_local::<SlowActor>((), get_spawner())
        .await
        .expect("Failed to spawn actor");
    actor
        .stop_and_wait(None, None)
        .await
        .expect("Failed to wait for actor death");
    // the handle should be done and completed (after some brief scheduling delay)
    periodic_check(|| handle.is_finished(), Duration::from_millis(500)).await;
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn kill_and_wait() {
    #[derive(Default)]
    struct SlowActor;

    impl Actor for SlowActor {
        type Msg = ();
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _: ActorRef<Self::Msg>,
            _: Self::Arguments,
        ) -> Result<Self::State, ActorProcessingErr> {
            crate::concurrency::sleep(Duration::from_millis(200)).await;
            Ok(())
        }
    }
    let (actor, handle) = crate::spawn_local::<SlowActor>((), get_spawner())
        .await
        .expect("Failed to spawn actor");
    actor
        .kill_and_wait(None)
        .await
        .expect("Failed to wait for actor death");
    // the handle should be done and completed (after some brief scheduling delay)
    periodic_check(|| handle.is_finished(), Duration::from_millis(500)).await;
}

/// https://github.com/slawlor/ractor/issues/254
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn actor_post_stop_executed_before_stop_and_wait_returns() {
    #[derive(Default)]
    struct TestActor;

    impl Actor for TestActor {
        type Msg = EmptyMessage;
        type Arguments = Arc<AtomicU8>;
        type State = Arc<AtomicU8>;

        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self::Msg>,
            args: Arc<AtomicU8>,
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(args)
        }

        async fn post_stop(
            &self,
            _: ActorRef<Self::Msg>,
            state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            sleep(Duration::from_millis(1000)).await;
            state.store(1, Ordering::SeqCst);
            Ok(())
        }
    }

    let signal = Arc::new(AtomicU8::new(0));
    let (actor, handle) = crate::spawn_local::<TestActor>(signal.clone(), get_spawner())
        .await
        .expect("Failed to spawn test actor");

    actor
        .stop_and_wait(None, None)
        .await
        .expect("Failed to stop and wait");

    assert_eq!(1, signal.load(Ordering::SeqCst));

    handle.await.unwrap();
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn actor_drain_messages() {
    #[derive(Default)]
    struct TestActor;

    impl Actor for TestActor {
        type Msg = EmptyMessage;
        type Arguments = Arc<AtomicU32>;
        type State = Arc<AtomicU32>;

        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self::Msg>,
            args: Arc<AtomicU32>,
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(args)
        }

        async fn handle(
            &self,
            _: ActorRef<Self::Msg>,
            _: Self::Msg,
            state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            sleep(Duration::from_millis(10)).await;
            state.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    let signal = Arc::new(AtomicU32::new(0));
    let (actor, handle) = crate::spawn_local::<TestActor>(signal.clone(), get_spawner())
        .await
        .expect("Failed to spawn test actor");

    for _ in 0..1000 {
        actor
            .cast(EmptyMessage)
            .expect("Failed to send message to actor");
    }

    assert!(signal.load(Ordering::SeqCst) < 1000);

    actor.drain().expect("Failed to trigger actor draining");

    // future cast's fail after draining triggered
    assert!(actor.cast(EmptyMessage).is_err());

    // wait for drain complete
    handle.await.unwrap();

    assert_eq!(1000, signal.load(Ordering::SeqCst));
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn wait_for_death() {
    #[derive(Default)]
    struct TestActor;

    impl Actor for TestActor {
        type Msg = EmptyMessage;
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }

        async fn post_stop(
            &self,
            _myself: ActorRef<Self::Msg>,
            _: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            crate::concurrency::sleep(Duration::from_millis(10)).await;
            Ok(())
        }
    }

    let (actor, handle) = crate::spawn_local::<TestActor>((), get_spawner())
        .await
        .expect("Failed to start test actor");

    actor.stop(None);
    assert!(actor.wait(Some(Duration::from_millis(100))).await.is_ok());

    // cleanup
    actor.stop(None);
    handle.await.unwrap();
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn derived_actor_ref() {
    let result_counter = Arc::new(AtomicU32::new(0));

    #[derive(Default)]
    struct TestActor;

    impl Actor for TestActor {
        type Msg = u32;
        type Arguments = Arc<AtomicU32>;
        type State = Arc<AtomicU32>;

        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self::Msg>,
            args: Arc<AtomicU32>,
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(args)
        }

        async fn handle(
            &self,
            _myself: ActorRef<Self::Msg>,
            message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            _state.fetch_add(message, Ordering::Relaxed);
            Ok(())
        }
    }

    let (actor, handle) = crate::spawn_local::<TestActor>(result_counter.clone(), get_spawner())
        .await
        .expect("Actor failed to start");

    let mut sum: u32 = 0;

    let from_u8: DerivedActorRef<u8> = actor.clone().get_derived();
    let u8_message: u8 = 1;
    sum += u8_message as u32;
    from_u8
        .send_message(u8_message)
        .expect("Failed to send message to actor");

    periodic_check(
        || result_counter.load(Ordering::Relaxed) == sum,
        Duration::from_millis(500),
    )
    .await;

    let from_u16: DerivedActorRef<u16> = actor.get_derived();
    let u16_message: u16 = 2;
    sum += u16_message as u32;
    from_u16
        .send_message(u16_message)
        .expect("Failed to send message to actor");

    periodic_check(
        || result_counter.load(Ordering::Relaxed) == sum,
        Duration::from_millis(500),
    )
    .await;

    actor
        .drain_and_wait(None)
        .await
        .expect("Failed to drain actor");
    handle.await.unwrap();

    assert_eq!(result_counter.load(Ordering::Relaxed), sum);

    // trying to send the message to a dead actor to verify reverse conversion in SendError handling
    let message: u16 = 3;
    let res = from_u16.send_message(message);
    assert!(res.is_err());
    if let Err(MessagingErr::SendErr(failed_message)) = res {
        assert_eq!(failed_message, message);
    } else {
        panic!("Invalid error type");
    }
}
