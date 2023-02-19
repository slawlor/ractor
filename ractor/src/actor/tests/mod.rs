// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! General tests, more logic-specific tests are contained in sub-modules

use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};

use crate::concurrency::Duration;

use crate::{
    Actor, ActorCell, ActorProcessingErr, ActorRef, ActorStatus, SpawnErr, SupervisionEvent,
};

mod supervisor;

struct EmptyMessage;
#[cfg(feature = "cluster")]
impl crate::Message for EmptyMessage {}

#[crate::concurrency::test]
async fn test_panic_on_start_captured() {
    struct TestActor;

    #[async_trait::async_trait]
    impl Actor for TestActor {
        type Msg = EmptyMessage;
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            panic!("Boom!");
        }
    }

    let actor_output = Actor::spawn(None, TestActor, ()).await;
    assert!(matches!(actor_output, Err(SpawnErr::StartupPanic(_))));
}

#[crate::concurrency::test]
async fn test_error_on_start_captured() {
    struct TestActor;

    #[async_trait::async_trait]
    impl Actor for TestActor {
        type Msg = EmptyMessage;
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Err(From::from("boom"))
        }
    }

    let actor_output = Actor::spawn(None, TestActor, ()).await;
    assert!(matches!(actor_output, Err(SpawnErr::StartupPanic(_))));
}

#[crate::concurrency::test]
async fn test_stop_higher_priority_over_messages() {
    let message_counter = Arc::new(AtomicU8::new(0u8));

    struct TestActor {
        counter: Arc<AtomicU8>,
    }

    #[async_trait::async_trait]
    impl Actor for TestActor {
        type Msg = EmptyMessage;
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }

        async fn handle(
            &self,
            _myself: ActorRef<Self>,
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

    println!("Counter: {}", message_counter.load(Ordering::Relaxed));

    // actor should have "stopped"
    assert_eq!(ActorStatus::Stopped, actor.get_status());
    assert!(handle.is_finished());
    // counter should have bumped only a single time
    assert_eq!(1, message_counter.load(Ordering::Relaxed));
}

#[crate::concurrency::test]
async fn test_kill_terminates_work() {
    struct TestActor;

    #[async_trait::async_trait]
    impl Actor for TestActor {
        type Msg = EmptyMessage;
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }

        async fn handle(
            &self,
            _myself: ActorRef<Self>,
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
    assert!(handle.is_finished());
}

#[crate::concurrency::test]
async fn test_stop_does_not_terminate_async_work() {
    struct TestActor;

    #[async_trait::async_trait]
    impl Actor for TestActor {
        type Msg = EmptyMessage;
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }

        async fn handle(
            &self,
            _myself: ActorRef<Self>,
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
    crate::concurrency::sleep(Duration::from_millis(2)).await;
    actor.stop(None);

    // async work should complete, so we're still "running" sleeping
    crate::concurrency::sleep(Duration::from_millis(10)).await;
    assert_eq!(ActorStatus::Running, actor.get_status());
    assert!(!handle.is_finished());

    crate::concurrency::sleep(Duration::from_millis(100)).await;
    // now enough time has passed, so we have processed the next message, which is stop
    // so we should be dead now
    assert_eq!(ActorStatus::Stopped, actor.get_status());
    assert!(handle.is_finished());
}

#[crate::concurrency::test]
async fn test_kill_terminates_supervision_work() {
    struct TestActor;

    #[async_trait::async_trait]
    impl Actor for TestActor {
        type Msg = EmptyMessage;
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            _myself: ActorRef<Self>,
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
async fn test_sending_message_to_invalid_actor_type() {
    struct TestActor1;
    struct TestMessage1;
    #[cfg(feature = "cluster")]
    impl crate::Message for TestMessage1 {}
    #[async_trait::async_trait]
    impl Actor for TestActor1 {
        type Msg = TestMessage1;
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _myself: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
    }
    struct TestActor2;
    struct TestMessage2;
    #[cfg(feature = "cluster")]
    impl crate::Message for TestMessage2 {}
    #[async_trait::async_trait]
    impl Actor for TestActor2 {
        type Msg = TestMessage2;
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _myself: ActorRef<Self>,
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
        .send_message::<TestActor1>(TestMessage1)
        .is_err());

    actor1.stop(None);
    actor2.stop(None);

    handle1.await.unwrap();
    handle2.await.unwrap();
}

#[crate::concurrency::test]
async fn test_sending_message_to_dead_actor() {
    struct TestActor;

    #[async_trait::async_trait]
    impl Actor for TestActor {
        type Msg = EmptyMessage;
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
    }

    let (actor, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Actor failed to start");

    // Stop the actor and wait for it to die
    actor.stop(None);
    handle.await.unwrap();

    // assert that if we send a message, it doesn't transmit since
    // the receiver is dropped
    assert!(actor.cast(EmptyMessage).is_err());
}

#[cfg(feature = "cluster")]
#[crate::concurrency::test]
async fn test_serialized_cast() {
    use crate::message::{BoxedDowncastErr, SerializedMessage};
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

    #[async_trait::async_trait]
    impl Actor for TestActor {
        type Msg = TestMessage;
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _myself: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }

        async fn handle(
            &self,
            _myself: ActorRef<Self>,
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

    crate::concurrency::sleep(Duration::from_millis(100)).await;

    assert_eq!(1, counter.load(Ordering::Relaxed));

    // cleanup
    actor.stop(None);
    handle.await.unwrap();
}

#[cfg(feature = "cluster")]
fn port_forward<Tin, Tout, F>(
    typed_port: crate::RpcReplyPort<Tout>,
    converter: F,
) -> crate::RpcReplyPort<Tin>
where
    Tin: Send + 'static,
    Tout: Send + 'static,
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
async fn test_serialized_rpc() {
    use crate::message::{BoxedDowncastErr, SerializedMessage};
    use crate::{Message, RpcReplyPort};

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

    #[async_trait::async_trait]
    impl Actor for TestActor {
        type Msg = TestMessage;
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _myself: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }

        async fn handle(
            &self,
            _myself: ActorRef<Self>,
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

    crate::concurrency::sleep(Duration::from_millis(100)).await;
    assert_eq!(1, counter.load(Ordering::Relaxed));

    // cleanup
    actor.stop(None);
    handle.await.unwrap();
}

#[cfg(feature = "cluster")]
#[crate::concurrency::test]
async fn test_remote_actor() {
    use crate::message::{BoxedDowncastErr, SerializedMessage};
    use crate::{ActorId, ActorRuntime, Message};

    let counter = Arc::new(AtomicU8::new(0));

    struct DummySupervisor;
    #[async_trait::async_trait]
    impl Actor for DummySupervisor {
        type Msg = ();
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _myself: ActorRef<Self>,
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

    #[async_trait::async_trait]
    impl Actor for TestRemoteActor {
        type Msg = TestRemoteMessage;
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _myself: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }

        async fn handle(
            &self,
            _myself: ActorRef<Self>,
            _message: TestRemoteMessage,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            panic!("Remote actor's don't handle anything");
        }

        async fn handle_serialized(
            &self,
            _myself: ActorRef<Self>,
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

    crate::concurrency::sleep(Duration::from_millis(100)).await;

    assert_eq!(1, counter.load(Ordering::Relaxed));

    // cleanup
    actor.stop(None);
    sup.stop(None);
    handle.await.unwrap();
    sup_handle.await.unwrap();
}

#[cfg(feature = "cluster")]
#[crate::concurrency::test]
async fn spawning_local_actor_as_remote_fails() {
    use crate::ActorProcessingErr;

    struct RemoteActor;
    struct RemoteActorMessage;
    impl crate::Message for RemoteActorMessage {}
    #[async_trait::async_trait]
    impl Actor for RemoteActor {
        type Msg = RemoteActorMessage;
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
    }

    struct EmptyActor;
    #[async_trait::async_trait]
    impl Actor for EmptyActor {
        type Msg = ();
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self>,
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
async fn instant_spawns() {
    let counter = Arc::new(AtomicU8::new(0));

    struct EmptyActor;
    #[async_trait::async_trait]
    impl Actor for EmptyActor {
        type Msg = String;
        type State = Arc<AtomicU8>;
        type Arguments = Arc<AtomicU8>;
        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self>,
            counter: Arc<AtomicU8>,
        ) -> Result<Self::State, ActorProcessingErr> {
            // delay startup by some amount
            crate::concurrency::sleep(Duration::from_millis(200)).await;
            Ok(counter)
        }

        async fn handle(
            &self,
            _this_actor: crate::ActorRef<Self>,
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

    crate::concurrency::sleep(Duration::from_millis(250)).await;
    // actor is started now and processing messages
    assert_eq!(10, counter.load(Ordering::Relaxed));

    // Cleanup
    actor.stop(None);
    handles
        .await
        .unwrap()
        .expect("Actor's pre_start routine panicked")
        .await
        .unwrap();
}
