// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! General tests, more logic-specific tests are contained in sub-modules

use std::sync::atomic::AtomicU32;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::actor::derived_actor::DerivedActorRef;
use crate::common_test::periodic_check;
use crate::concurrency::sleep;
use crate::concurrency::Duration;
use crate::Actor;
use crate::ActorCell;
use crate::ActorProcessingErr;
use crate::ActorRef;
use crate::ActorStatus;
use crate::MessagingErr;
use crate::RactorErr;
use crate::SpawnErr;
use crate::SupervisionEvent;

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

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
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

    let actor_output = crate::spawn::<TestActor>(()).await;
    assert!(matches!(actor_output, Err(SpawnErr::StartupFailed(_))));
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_error_on_start_captured() {
    struct TestActor;

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
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

    let actor_output = Actor::spawn(None, TestActor, ()).await;
    assert!(matches!(actor_output, Err(SpawnErr::StartupFailed(_))));
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_stop_higher_priority_over_messages() {
    let message_counter = Arc::new(AtomicU8::new(0u8));

    struct TestActor {
        counter: Arc<AtomicU8>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
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
            self.counter.fetch_add(1, Ordering::Relaxed);
            crate::concurrency::sleep(Duration::from_millis(100)).await;
            Ok(())
        }
    }

    let (actor, handle) = Actor::spawn(
        None,
        TestActor {
            counter: message_counter.clone(),
        },
        (),
    )
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
    struct TestActor;

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
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

    let (actor, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Actor failed to start");

    actor
        .send_message(EmptyMessage)
        .expect("Failed to send message to actor");
    crate::concurrency::sleep(Duration::from_millis(10)).await;

    actor.kill();
    crate::concurrency::sleep(Duration::from_millis(10)).await;

    assert_eq!(ActorStatus::Stopped, actor.get_status());

    assert!(actor.send_message(EmptyMessage).is_err());

    assert!(handle.is_finished());
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_stop_does_not_terminate_async_work() {
    struct TestActor;

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
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

    let (actor, handle) = Actor::spawn(None, TestActor, ())
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
    struct TestActor;

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
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

    let (actor, handle) = Actor::spawn(None, TestActor, ())
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
    struct TestActor1;
    struct TestMessage1;
    #[cfg(feature = "cluster")]
    impl crate::Message for TestMessage1 {}
    #[cfg_attr(feature = "async-trait", crate::async_trait)]
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
    struct TestActor2;
    struct TestMessage2;
    #[cfg(feature = "cluster")]
    impl crate::Message for TestMessage2 {}
    #[cfg_attr(feature = "async-trait", crate::async_trait)]
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

    let (actor1, handle1) = Actor::spawn(None, TestActor1, ())
        .await
        .expect("Failed to start test actor 1");
    let (actor2, handle2) = Actor::spawn(None, TestActor2, ())
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

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
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

    let (actor, _) = crate::spawn::<TestActor>(())
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

#[cfg(feature = "cluster")]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_serialized_cast() {
    use crate::message::BoxedDowncastErr;
    use crate::message::SerializedMessage;
    use crate::Message;

    let counter = Arc::new(AtomicU8::new(0));

    struct TestActor {
        counter: Arc<AtomicU8>,
    }
    struct TestMessage;
    // a serializable basic no-op message
    impl Message for TestMessage {
        fn serializable() -> bool {
            true
        }
        fn deserialize(_bytes: SerializedMessage) -> Result<Self, BoxedDowncastErr> {
            Ok(TestMessage)
        }
        fn serialize(self) -> Result<SerializedMessage, BoxedDowncastErr> {
            Ok(crate::message::SerializedMessage::Cast {
                variant: "Cast".to_string(),
                args: vec![],
                metadata: None,
            })
        }
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for TestActor {
        type Msg = TestMessage;
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _myself: ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }

        async fn handle(
            &self,
            _myself: ActorRef<Self::Msg>,
            _message: TestMessage,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            self.counter.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    let (actor, handle) = Actor::spawn(
        None,
        TestActor {
            counter: counter.clone(),
        },
        (),
    )
    .await
    .expect("Failed to spawn test actor");

    assert!(actor.supports_remoting());

    let serialized = (TestMessage).serialize().unwrap();
    actor
        .send_serialized(serialized)
        .expect("Serialized message send failed!");

    periodic_check(
        || counter.load(Ordering::Relaxed) == 1,
        Duration::from_secs(2),
    )
    .await;

    // cleanup
    actor.stop(None);
    handle.await.unwrap();

    let serialized = (TestMessage).serialize().unwrap();
    assert!(actor.send_serialized(serialized).is_err());
}

#[cfg(feature = "cluster")]
fn port_forward<Tin, Tout, F>(
    typed_port: crate::RpcReplyPort<Tout>,
    converter: F,
) -> crate::RpcReplyPort<Tin>
where
    Tin: Send + 'static,
    Tout: crate::Message,
    F: Fn(Tin) -> Tout + Send + 'static,
{
    let (tx, rx) = crate::concurrency::oneshot();
    let timeout = typed_port.get_timeout();
    crate::concurrency::spawn(async move {
        match typed_port.get_timeout() {
            Some(timeout) => {
                if let Ok(Ok(result)) = crate::concurrency::timeout(timeout, rx).await {
                    let _ = typed_port.send(converter(result));
                }
            }
            None => {
                if let Ok(result) = rx.await {
                    let _ = typed_port.send(converter(result));
                }
            }
        }
    });
    if let Some(to) = timeout {
        crate::RpcReplyPort::<_>::from((tx, to))
    } else {
        crate::RpcReplyPort::<_>::from(tx)
    }
}

#[cfg(feature = "cluster")]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_serialized_rpc() {
    use crate::message::BoxedDowncastErr;
    use crate::message::SerializedMessage;
    use crate::Message;
    use crate::RpcReplyPort;

    let counter = Arc::new(AtomicU8::new(0));

    struct TestActor {
        counter: Arc<AtomicU8>,
    }
    enum TestMessage {
        Rpc(RpcReplyPort<String>),
    }

    // a serializable basic no-op message
    impl Message for TestMessage {
        fn serializable() -> bool {
            true
        }
        fn deserialize(bytes: SerializedMessage) -> Result<Self, BoxedDowncastErr> {
            match bytes {
                SerializedMessage::Call { reply, .. } => {
                    let tx = port_forward(reply, |data: String| data.into_bytes());
                    Ok(Self::Rpc(tx))
                }
                _ => panic!("whoopsie"),
            }
        }
        fn serialize(self) -> Result<SerializedMessage, BoxedDowncastErr> {
            match self {
                Self::Rpc(port) => {
                    let tx = port_forward(port, |data| String::from_utf8(data).unwrap());
                    Ok(SerializedMessage::Call {
                        args: vec![],
                        reply: tx,
                        variant: "Call".to_string(),
                        metadata: None,
                    })
                }
            }
        }
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for TestActor {
        type Msg = TestMessage;
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _myself: ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }

        async fn handle(
            &self,
            _myself: ActorRef<Self::Msg>,
            message: TestMessage,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            self.counter.fetch_add(1, Ordering::Relaxed);
            match message {
                TestMessage::Rpc(port) => {
                    let _ = port.send("hello".to_string());
                }
            }
            Ok(())
        }
    }

    let (actor, handle) = Actor::spawn(
        None,
        TestActor {
            counter: counter.clone(),
        },
        (),
    )
    .await
    .expect("Failed to spawn test actor");

    let (tx, rx) = crate::concurrency::oneshot();
    let msg = TestMessage::Rpc((tx, Duration::from_millis(100)).into())
        .serialize()
        .unwrap();
    actor
        .send_serialized(msg)
        .expect("Serialized message send failed!");

    assert!(actor.supports_remoting());

    let data = rx
        .await
        .expect("Failed to get reply from actor (within 100ms)");
    assert_eq!(data, "hello".to_string());

    periodic_check(
        || counter.load(Ordering::Relaxed) == 1,
        Duration::from_secs(2),
    )
    .await;

    // cleanup
    actor.stop(None);
    handle.await.unwrap();
}

#[cfg(feature = "cluster")]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_remote_actor() {
    use crate::message::BoxedDowncastErr;
    use crate::message::SerializedMessage;
    use crate::ActorId;
    use crate::ActorRuntime;
    use crate::Message;

    let counter = Arc::new(AtomicU8::new(0));

    struct DummySupervisor;
    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for DummySupervisor {
        type Msg = ();
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

    struct TestRemoteActor {
        counter: Arc<AtomicU8>,
    }
    struct TestRemoteMessage;
    // a serializable basic no-op message
    impl Message for TestRemoteMessage {
        fn serializable() -> bool {
            true
        }
        fn deserialize(_bytes: SerializedMessage) -> Result<Self, BoxedDowncastErr> {
            Ok(TestRemoteMessage)
        }
        fn serialize(self) -> Result<SerializedMessage, BoxedDowncastErr> {
            Ok(crate::message::SerializedMessage::Cast {
                args: vec![],
                variant: "Cast".to_string(),
                metadata: None,
            })
        }
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for TestRemoteActor {
        type Msg = TestRemoteMessage;
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _myself: ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }

        async fn handle(
            &self,
            _myself: ActorRef<Self::Msg>,
            _message: TestRemoteMessage,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            panic!("Remote actor's don't handle anything");
        }

        async fn handle_serialized(
            &self,
            _myself: ActorRef<Self::Msg>,
            _message: SerializedMessage,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            self.counter.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    let (sup, sup_handle) = Actor::spawn(None, DummySupervisor, ()).await.unwrap();

    let (actor, handle) = ActorRuntime::spawn_linked_remote(
        None,
        TestRemoteActor {
            counter: counter.clone(),
        },
        ActorId::Remote { node_id: 1, pid: 1 },
        (),
        sup.get_cell(),
    )
    .await
    .expect("Failed to spawn RemoteTestActor");

    actor
        .send_message(TestRemoteMessage)
        .expect("Failed to send non-serialized message to RemoteActor");

    periodic_check(
        || counter.load(Ordering::Relaxed) == 1,
        Duration::from_secs(2),
    )
    .await;

    // cleanup
    actor.stop(None);
    sup.stop(None);
    handle.await.unwrap();
    sup_handle.await.unwrap();
}

#[cfg(feature = "cluster")]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn spawning_local_actor_as_remote_fails() {
    use crate::ActorProcessingErr;

    struct RemoteActor;
    struct RemoteActorMessage;
    impl crate::Message for RemoteActorMessage {}
    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for RemoteActor {
        type Msg = RemoteActorMessage;
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
    }

    struct EmptyActor;
    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for EmptyActor {
        type Msg = ();
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
    }
    let remote_pid = crate::ActorId::Local(1);

    let (actor, handle) = Actor::spawn(None, EmptyActor, ())
        .await
        .expect("Actor failed to start");

    let remote_spawn_result = crate::ActorRuntime::spawn_linked_remote(
        None,
        RemoteActor,
        remote_pid,
        (),
        actor.get_cell(),
    )
    .await;

    assert!(remote_spawn_result.is_err());

    actor.stop(None);
    handle.await.expect("Failed to clean stop the actor");
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn instant_spawns() {
    let counter = Arc::new(AtomicU8::new(0));

    struct EmptyActor;
    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for EmptyActor {
        type Msg = String;
        type State = Arc<AtomicU8>;
        type Arguments = Arc<AtomicU8>;
        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self::Msg>,
            counter: Arc<AtomicU8>,
        ) -> Result<Self::State, ActorProcessingErr> {
            // delay startup by some amount
            crate::concurrency::sleep(Duration::from_millis(200)).await;
            Ok(counter)
        }

        async fn handle(
            &self,
            _this_actor: crate::ActorRef<Self::Msg>,
            _message: String,
            state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            state.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    let (actor, handles) = crate::ActorRuntime::spawn_instant(None, EmptyActor, counter.clone())
        .expect("Failed to instant spawn");

    for i in 0..10 {
        actor
            .cast(format!("I = {i}"))
            .expect("Actor couldn't receive message!");
    }

    // actor is still starting up
    assert_eq!(0, counter.load(Ordering::Relaxed));

    // check messages processed
    periodic_check(
        || counter.load(Ordering::Relaxed) == 10,
        Duration::from_secs(2),
    )
    .await;

    // Cleanup
    actor.stop(None);
    handles
        .await
        .unwrap()
        .expect("Actor's pre_start routine panicked")
        .await
        .unwrap();
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn stop_and_wait() {
    struct SlowActor;
    #[cfg_attr(feature = "async-trait", crate::async_trait)]
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
    let (actor, handle) = Actor::spawn(None, SlowActor, ())
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
    struct SlowActor;
    #[cfg_attr(feature = "async-trait", crate::async_trait)]
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
    let (actor, handle) = Actor::spawn(None, SlowActor, ())
        .await
        .expect("Failed to spawn actor");
    actor
        .kill_and_wait(None)
        .await
        .expect("Failed to wait for actor death");
    // the handle should be done and completed (after some brief scheduling delay)
    periodic_check(|| handle.is_finished(), Duration::from_millis(500)).await;
}

#[test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
fn test_err_map() {
    let err: RactorErr<i32> = RactorErr::Messaging(MessagingErr::SendErr(123));

    let _: RactorErr<()> = err.map(|_| ());
}

#[test]
fn returns_actor_references() {
    fn dummy_actor_cell() -> ActorCell {
        ActorCell::new::<TestActor>(None).unwrap().0
    }

    struct TestActor;

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
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

    let tests = [
        (true, SupervisionEvent::ActorStarted(dummy_actor_cell())),
        (
            true,
            SupervisionEvent::ActorFailed(dummy_actor_cell(), "Bang!".into()),
        ),
        (
            true,
            SupervisionEvent::ActorTerminated(dummy_actor_cell(), None, Some("Foo!".to_owned())),
        ),
        (
            false,
            SupervisionEvent::ProcessGroupChanged(crate::pg::GroupChangeMessage::Leave(
                "Foo".into(),
                "Bar".into(),
                vec![dummy_actor_cell()],
            )),
        ),
    ];

    for (want, event) in tests {
        // Cloned cells are "equal" since they point to the same actor id
        assert_eq!(event.actor_cell(), event.actor_cell().clone());
        assert_eq!(event.actor_cell().is_some(), want);
        assert_eq!(event.actor_id().is_some(), want);
    }
}

/// https://github.com/slawlor/ractor/issues/240
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn actor_failing_in_spawn_err_doesnt_poison_registries() {
    #[derive(Default)]
    struct Test;

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Test {
        type Msg = ();
        type State = ();
        type Arguments = ();

        async fn pre_start(&self, _: ActorRef<Self::Msg>, _: ()) -> Result<(), ActorProcessingErr> {
            Err("something".into())
        }
    }

    #[derive(Default)]
    struct Test2;

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Test2 {
        type Msg = ();
        type State = ();
        type Arguments = ();

        async fn pre_start(&self, _: ActorRef<Self::Msg>, _: ()) -> Result<(), ActorProcessingErr> {
            Ok(())
        }
    }

    let a = crate::spawn_named::<Test>("test".to_owned(), ()).await;
    assert!(a.is_err());
    drop(a);

    let (a, h) = Actor::spawn(Some("test".to_owned()), Test2, ())
        .await
        .expect("Failed to spawn second actor with name clash");

    // startup ok, we were able to reuse the name

    a.stop(None);
    h.await.unwrap();
}

/// https://github.com/slawlor/ractor/issues/254
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn actor_post_stop_executed_before_stop_and_wait_returns() {
    struct TestActor {
        signal: Arc<AtomicU8>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
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
            _: ActorRef<Self::Msg>,
            _: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            sleep(Duration::from_millis(1000)).await;
            self.signal.store(1, Ordering::SeqCst);
            Ok(())
        }
    }

    let signal = Arc::new(AtomicU8::new(0));
    let (actor, handle) = Actor::spawn(
        None,
        TestActor {
            signal: signal.clone(),
        },
        (),
    )
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
    struct TestActor {
        signal: Arc<AtomicU32>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
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
            _: ActorRef<Self::Msg>,
            _: Self::Msg,
            _: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            sleep(Duration::from_millis(10)).await;
            let _ = self.signal.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    let signal = Arc::new(AtomicU32::new(0));
    let (actor, handle) = Actor::spawn(
        None,
        TestActor {
            signal: signal.clone(),
        },
        (),
    )
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
async fn runtime_message_typing() {
    struct TestActor;

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
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

    let (actor, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to start test actor");
    // lose the strong typing info
    let actor: ActorCell = actor.into();
    assert_eq!(Some(true), actor.is_message_type_of::<EmptyMessage>());
    assert_eq!(Some(false), actor.is_message_type_of::<i64>());

    // cleanup
    actor.stop(None);
    handle.await.unwrap();
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn wait_for_death() {
    struct TestActor;

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
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

    let (actor, handle) = Actor::spawn(None, TestActor, ())
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

    struct TestActor {
        counter: Arc<AtomicU32>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for TestActor {
        type Msg = u32;
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
            message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            self.counter.fetch_add(message, Ordering::Relaxed);
            Ok(())
        }
    }

    let (actor, handle) = Actor::spawn(
        None,
        TestActor {
            counter: result_counter.clone(),
        },
        (),
    )
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

    // timer
    actor
        .get_derived()
        .send_after(Duration::from_millis(10), move || u16_message)
        .await
        .expect("Failed to await the join handle")
        .expect("Failed to send message to actor");

    sum += u16_message as u32;

    // Make sure delayed send is received
    crate::concurrency::sleep(Duration::from_millis(50)).await;
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

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn can_use_call_in_actor() {
    enum TestActorMessage {
        Call(crate::RpcReplyPort<u32>),
    }

    #[cfg(feature = "cluster")]
    impl crate::Message for TestActorMessage {}

    struct TestActor1;

    struct TestActor2;

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for TestActor1 {
        type Msg = TestActorMessage;
        type Arguments = ();
        type State = ActorRef<TestActorMessage>;

        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            let child_actor = Actor::spawn(None, TestActor2, ())
                .await
                .expect("Failed to spawn child actor");
            Ok(child_actor.0)
        }

        async fn handle(
            &self,
            _myself: ActorRef<Self::Msg>,
            message: Self::Msg,
            state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            match message {
                TestActorMessage::Call(reply_port) => {
                    let result = crate::call!(state, TestActorMessage::Call)?;
                    reply_port.send(result).expect("Failed to send response");
                }
            }

            Ok(())
        }
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for TestActor2 {
        type Msg = TestActorMessage;
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
            message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            match message {
                TestActorMessage::Call(reply_port) => {
                    reply_port.send(42).expect("Failed to send response");
                }
            }

            Ok(())
        }
    }

    // spawn the first actor, which will internally spawn the second actor
    let (actor1, _) = Actor::spawn(None, TestActor1, ())
        .await
        .expect("Failed to spawn actor");

    let result = crate::call!(actor1, TestActorMessage::Call).expect("Failed to call actor");

    assert!(result == 42);
}
