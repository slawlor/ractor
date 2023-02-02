// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Tests on output ports

use std::time::Duration;

use crate::{concurrency::timeout, ActorProcessingErr};
use futures::future::join_all;

use crate::{Actor, ActorRef};

use super::*;

#[crate::concurrency::test]
async fn test_single_forward() {
    struct TestActor;
    enum TestActorMessage {
        Stop,
    }
    #[cfg(feature = "cluster")]
    impl crate::Message for TestActorMessage {}
    #[async_trait::async_trait]
    impl Actor for TestActor {
        type Msg = TestActorMessage;
        type Arguments = ();
        type State = u8;

        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(0u8)
        }

        async fn handle(
            &self,
            myself: ActorRef<Self>,
            message: Self::Msg,
            state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            println!("Test actor received a message");
            match message {
                Self::Msg::Stop => {
                    if *state > 3 {
                        myself.stop(None);
                    }
                }
            }
            *state += 1;
            Ok(())
        }
    }

    let (actor, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("failed to start test actor");

    let output = OutputPort::<()>::default();
    output.subscribe(actor, |_| Some(TestActorMessage::Stop));

    // send 3 sends, should not exit
    for _ in 0..4 {
        output.send(());
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!handle.is_finished());

    // last send should trigger the exit condition
    output.send(());
    timeout(Duration::from_millis(100), handle)
        .await
        .expect("Test actor failed in exit")
        .unwrap();
}

#[crate::concurrency::test]
async fn test_50_receivers() {
    struct TestActor;
    enum TestActorMessage {
        Stop,
    }
    #[cfg(feature = "cluster")]
    impl crate::Message for TestActorMessage {}
    #[async_trait::async_trait]
    impl Actor for TestActor {
        type Msg = TestActorMessage;
        type Arguments = ();
        type State = u8;

        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(0u8)
        }

        async fn handle(
            &self,
            myself: ActorRef<Self>,
            message: Self::Msg,
            state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            println!("Test actor received a message");
            match message {
                Self::Msg::Stop => {
                    if *state > 3 {
                        myself.stop(None);
                    }
                }
            }
            *state += 1;
            Ok(())
        }
    }

    let handles: Vec<(ActorRef<TestActor>, JoinHandle<()>)> =
        join_all((0..50).map(|_| async move {
            Actor::spawn(None, TestActor, ())
                .await
                .expect("Failed to start test actor")
        }))
        .await;

    let mut actor_refs = vec![];
    let mut actor_handles = vec![];
    for item in handles.into_iter() {
        let (a, b) = item;
        actor_refs.push(a);
        actor_handles.push(b);
    }

    let output = OutputPort::<()>::default();
    for actor in actor_refs.into_iter() {
        output.subscribe(actor, |_| Some(TestActorMessage::Stop));
    }

    let all_handle = crate::concurrency::spawn(async move { join_all(actor_handles).await });

    // send 3 sends, should not exit
    for _ in 0..4 {
        output.send(());
    }
    crate::concurrency::sleep(Duration::from_millis(50)).await;
    assert!(!all_handle.is_finished());

    // last send should trigger the exit condition
    output.send(());
    timeout(Duration::from_millis(100), all_handle)
        .await
        .expect("Test actor failed in exit")
        .unwrap();
}
