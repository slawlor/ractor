// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Tests for remote procedure calls

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use tokio::time::Duration;

use crate::rpc;
use crate::{Actor, ActorCell, ActorHandler};

#[tokio::test]
async fn test_rpc_cast() {
    let counter = Arc::new(AtomicU8::new(0u8));

    struct TestActor {
        counter: Arc<AtomicU8>,
    }

    #[async_trait::async_trait]
    impl ActorHandler for TestActor {
        type Msg = ();

        type State = ();

        async fn pre_start(&self, _this_actor: ActorCell) -> Self::State {}

        async fn handle(
            &self,
            _this_actor: ActorCell,
            _message: Self::Msg,
            _state: &Self::State,
        ) -> Option<Self::State> {
            self.counter.fetch_add(1u8, Ordering::Relaxed);
            None
        }
    }

    let (actor_ref, handle) = Actor::spawn(
        None,
        TestActor {
            counter: counter.clone(),
        },
    )
    .await
    .expect("Failed to start test actor");

    rpc::cast::<TestActor, _>(&actor_ref, ()).expect("Failed to send message");
    actor_ref
        .cast::<TestActor, _>(())
        .expect("Failed to send message");

    // make sure they have time to process
    tokio::time::sleep(Duration::from_millis(100)).await;

    // assert the actor received 2 cast requests
    assert_eq!(2, counter.load(Ordering::Relaxed));

    // cleanup
    actor_ref.stop();
    handle.await.expect("Actor stopped with err");
}

#[tokio::test]
async fn test_rpc_call() {
    struct TestActor;

    enum MessageFormat {
        TestRpc(rpc::RpcReplyPort<String>),
    }

    #[async_trait::async_trait]
    impl ActorHandler for TestActor {
        type Msg = MessageFormat;

        type State = ();

        async fn pre_start(&self, _this_actor: ActorCell) -> Self::State {}

        async fn handle(
            &self,
            _this_actor: ActorCell,
            message: Self::Msg,
            _state: &Self::State,
        ) -> Option<Self::State> {
            match message {
                Self::Msg::TestRpc(reply) => {
                    // An error sending means no one is listening anymore (the receiver was dropped),
                    // so we should shortcut the processing of this message probably!
                    if !reply.is_closed() {
                        let _ = reply.send("howdy".to_string());
                    }
                }
            }
            None
        }
    }

    let (actor_ref, handle) = Actor::spawn(None, TestActor)
        .await
        .expect("Failed to start test actor");

    let rpc_result = rpc::call::<TestActor, _, _, _>(
        &actor_ref,
        MessageFormat::TestRpc,
        Some(Duration::from_millis(100)),
    )
    .await
    .expect("Failed to send message to actor")
    .expect("RPC didn't succeed");
    assert_eq!("howdy".to_string(), rpc_result);

    let rpc_result = actor_ref
        .call::<TestActor, _, _, _>(MessageFormat::TestRpc, Some(Duration::from_millis(100)))
        .await
        .expect("Failed to send message to actor")
        .expect("RPC didn't succeed");
    assert_eq!("howdy".to_string(), rpc_result);

    // cleanup
    actor_ref.stop();
    handle.await.expect("Actor stopped with err");
}

#[tokio::test]
async fn test_rpc_call_forwarding() {
    struct Worker;

    enum WorkerMessage {
        TestRpc(rpc::RpcReplyPort<String>),
    }

    #[async_trait::async_trait]
    impl ActorHandler for Worker {
        type Msg = WorkerMessage;

        type State = ();

        async fn pre_start(&self, _this_actor: ActorCell) -> Self::State {}

        async fn handle(
            &self,
            _this_actor: ActorCell,
            message: Self::Msg,
            _state: &Self::State,
        ) -> Option<Self::State> {
            match message {
                Self::Msg::TestRpc(reply) => {
                    // An error sending means no one is listening anymore (the receiver was dropped),
                    // so we should shortcut the processing of this message probably!
                    if !reply.is_closed() {
                        let _ = reply.send("howdy".to_string());
                    }
                }
            }
            None
        }
    }

    let counter = Arc::new(AtomicU8::new(0u8));
    struct Forwarder {
        counter: Arc<AtomicU8>,
    }

    enum ForwarderMessage {
        ForwardResult(String),
    }

    #[async_trait::async_trait]
    impl ActorHandler for Forwarder {
        type Msg = ForwarderMessage;

        type State = ();

        async fn pre_start(&self, _this_actor: ActorCell) -> Self::State {}

        async fn handle(
            &self,
            _this_actor: ActorCell,
            message: Self::Msg,
            _state: &Self::State,
        ) -> Option<Self::State> {
            match message {
                Self::Msg::ForwardResult(s) if s == *"howdy" => {
                    self.counter.fetch_add(1, Ordering::Relaxed);
                }
                _ => {}
            }
            None
        }
    }

    let (worker_ref, worker_handle) = Actor::spawn(None, Worker)
        .await
        .expect("Failed to start worker actor");

    let (forwarder_ref, forwarder_handle) = Actor::spawn(
        None,
        Forwarder {
            counter: counter.clone(),
        },
    )
    .await
    .expect("Failed to start forwarder actor");

    let forward_handle = rpc::call_and_forward::<Worker, Forwarder, _, _, _, _, _>(
        &worker_ref,
        WorkerMessage::TestRpc,
        forwarder_ref.clone(),
        ForwarderMessage::ForwardResult,
        Some(Duration::from_millis(100)),
    );

    forward_handle
        .expect("Failed to send message to actor")
        .await
        .expect("Forwarding task cancelled or panicked")
        .expect("Call result didn't return success")
        .expect("Failed to forward message");

    let forward_handle = worker_ref.call_and_forward::<Worker, Forwarder, _, _, _, _, _>(
        WorkerMessage::TestRpc,
        forwarder_ref.clone(),
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
    assert_eq!(2, counter.load(Ordering::Relaxed));

    // cleanup
    forwarder_ref.stop();
    worker_ref.stop();

    forwarder_handle.await.expect("Actor stopped with err");
    worker_handle.await.expect("Actor stopped with err");
}
