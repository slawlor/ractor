// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Tests for remote procedure calls

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use crate::concurrency::Duration;

use crate::{call, call_t, cast, forward, Actor, ActorRef};
use crate::{rpc, ActorProcessingErr};

#[crate::concurrency::test]
async fn test_rpc_cast() {
    let counter = Arc::new(AtomicU8::new(0u8));

    struct TestActor {
        counter: Arc<AtomicU8>,
    }

    #[async_trait::async_trait]
    impl Actor for TestActor {
        type Msg = ();
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }

        async fn handle(
            &self,
            _this_actor: ActorRef<Self>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            self.counter.fetch_add(1u8, Ordering::Relaxed);
            Ok(())
        }
    }

    let (actor_ref, handle) = Actor::spawn(
        None,
        TestActor {
            counter: counter.clone(),
        },
        (),
    )
    .await
    .expect("Failed to start test actor");

    actor_ref.cast(()).expect("Failed to send message");
    actor_ref.cast(()).expect("Failed to send message");
    cast!(actor_ref, ()).unwrap();

    // make sure they have time to process
    crate::concurrency::sleep(Duration::from_millis(100)).await;

    // assert the actor received 2 cast requests
    assert_eq!(3, counter.load(Ordering::Relaxed));

    // cleanup
    actor_ref.stop(None);
    handle.await.expect("Actor stopped with err");
}

#[crate::concurrency::test]
async fn test_rpc_call() {
    struct TestActor;
    enum MessageFormat {
        Rpc(rpc::RpcReplyPort<String>),
        Timeout(rpc::RpcReplyPort<String>),
        MultiArg(String, u32, rpc::RpcReplyPort<String>),
    }
    #[cfg(feature = "cluster")]
    impl crate::Message for MessageFormat {}
    #[async_trait::async_trait]
    impl Actor for TestActor {
        type Msg = MessageFormat;
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }

        async fn handle(
            &self,
            _this_actor: ActorRef<Self>,
            message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            match message {
                Self::Msg::Rpc(reply) => {
                    // An error sending means no one is listening anymore (the receiver was dropped),
                    // so we should shortcut the processing of this message probably!
                    if !reply.is_closed() {
                        let _ = reply.send("howdy".to_string());
                    }
                }
                Self::Msg::Timeout(reply) => {
                    crate::concurrency::sleep(Duration::from_millis(100)).await;
                    let _ = reply.send("howdy".to_string());
                }
                Self::Msg::MultiArg(message, count, reply) => {
                    let _ = reply.send(format!("{message}-{count}"));
                }
            }
            Ok(())
        }
    }

    let (actor_ref, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to start test actor");

    let rpc_result = call_t!(actor_ref, MessageFormat::Rpc, 100).unwrap();
    assert_eq!("howdy".to_string(), rpc_result);

    let rpc_result = call!(actor_ref, MessageFormat::Rpc).unwrap();
    assert_eq!("howdy".to_string(), rpc_result);

    let rpc_result = actor_ref
        .call(MessageFormat::Rpc, Some(Duration::from_millis(100)))
        .await
        .expect("Failed to send message to actor")
        .expect("RPC didn't succeed");
    assert_eq!("howdy".to_string(), rpc_result);

    let rpc_timeout = call_t!(actor_ref, MessageFormat::Timeout, 10);
    assert!(rpc_timeout.is_err());
    println!("RPC Error {rpc_timeout:?}");

    let rpc_value = call!(actor_ref, MessageFormat::MultiArg, "Msg".to_string(), 32).unwrap();
    assert_eq!("Msg-32".to_string(), rpc_value);

    let rpc_value = call_t!(
        actor_ref,
        MessageFormat::MultiArg,
        100,
        "Msg".to_string(),
        32
    )
    .unwrap();
    assert_eq!("Msg-32".to_string(), rpc_value);

    // cleanup
    actor_ref.stop(None);

    crate::concurrency::sleep(Duration::from_millis(200)).await;

    let rpc_send_fail = call!(actor_ref, MessageFormat::Rpc);
    assert!(rpc_send_fail.is_err());
    handle.await.expect("Actor stopped with err");
}

#[crate::concurrency::test]
async fn test_rpc_call_forwarding() {
    struct Worker;

    enum WorkerMessage {
        TestRpc(rpc::RpcReplyPort<String>),
    }
    #[cfg(feature = "cluster")]
    impl crate::Message for WorkerMessage {}
    #[async_trait::async_trait]
    impl Actor for Worker {
        type Msg = WorkerMessage;
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }

        async fn handle(
            &self,
            _this_actor: ActorRef<Self>,
            message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            match message {
                Self::Msg::TestRpc(reply) => {
                    // An error sending means no one is listening anymore (the receiver was dropped),
                    // so we should shortcut the processing of this message probably!
                    if !reply.is_closed() {
                        let _ = reply.send("howdy".to_string());
                    }
                }
            }
            Ok(())
        }
    }

    let counter = Arc::new(AtomicU8::new(0u8));
    struct Forwarder {
        counter: Arc<AtomicU8>,
    }

    enum ForwarderMessage {
        ForwardResult(String),
    }
    #[cfg(feature = "cluster")]
    impl crate::Message for ForwarderMessage {}

    #[async_trait::async_trait]
    impl Actor for Forwarder {
        type Msg = ForwarderMessage;
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }

        async fn handle(
            &self,
            _this_actor: ActorRef<Self>,
            message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            match message {
                Self::Msg::ForwardResult(s) if s == *"howdy" => {
                    self.counter.fetch_add(1, Ordering::Relaxed);
                }
                _ => {}
            }
            Ok(())
        }
    }

    let (worker_ref, worker_handle) = Actor::spawn(None, Worker, ())
        .await
        .expect("Failed to start worker actor");

    let (forwarder_ref, forwarder_handle) = Actor::spawn(
        None,
        Forwarder {
            counter: counter.clone(),
        },
        (),
    )
    .await
    .expect("Failed to start forwarder actor");

    let forward_handle = worker_ref.call_and_forward(
        WorkerMessage::TestRpc,
        &forwarder_ref,
        ForwarderMessage::ForwardResult,
        Some(Duration::from_millis(100)),
    );

    forward_handle
        .expect("Failed to send message to actor")
        .await
        .expect("Forwarding task cancelled or panicked")
        .expect("Call result didn't return success")
        .expect("Failed to forward message");

    forward!(
        worker_ref,
        WorkerMessage::TestRpc,
        forwarder_ref,
        ForwarderMessage::ForwardResult
    )
    .expect("Failed to foward message");

    forward!(
        worker_ref,
        WorkerMessage::TestRpc,
        forwarder_ref,
        ForwarderMessage::ForwardResult,
        Duration::from_millis(100)
    )
    .expect("Failed to forward message");

    let forward_handle = worker_ref.call_and_forward(
        WorkerMessage::TestRpc,
        &forwarder_ref,
        ForwarderMessage::ForwardResult,
        Some(Duration::from_millis(100)),
    );

    forward_handle
        .expect("Failed to send message to actor")
        .await
        .expect("Forwarding task cancelled or panicked")
        .expect("Call result didn't return success")
        .expect("Failed to forward message");

    // make sure the counter was bumped to say the message was forwarded
    assert_eq!(4, counter.load(Ordering::Relaxed));

    // cleanup
    forwarder_ref.stop(None);
    worker_ref.stop(None);

    forwarder_handle.await.expect("Actor stopped with err");
    worker_handle.await.expect("Actor stopped with err");
}

// TODO: test multi_call
