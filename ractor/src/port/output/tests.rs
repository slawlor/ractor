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
#[tracing_test::traced_test]
async fn test_single_forward() {
    struct TestActor;
    enum TestActorMessage {
        Stop,
    }
    #[cfg(feature = "cluster")]
    impl crate::Message for TestActorMessage {}
    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for TestActor {
        type Msg = TestActorMessage;
        type Arguments = ();
        type State = u8;

        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(0u8)
        }

        async fn handle(
            &self,
            myself: ActorRef<Self::Msg>,
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
    crate::concurrency::sleep(Duration::from_millis(50)).await;
    assert!(!handle.is_finished());

    // last send should trigger the exit condition
    output.send(());
    timeout(Duration::from_millis(100), handle)
        .await
        .expect("Test actor failed in exit")
        .unwrap();
}

#[crate::concurrency::test]
#[tracing_test::traced_test]
async fn test_50_receivers() {
    struct TestActor;
    enum TestActorMessage {
        Stop,
    }
    #[cfg(feature = "cluster")]
    impl crate::Message for TestActorMessage {}
    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for TestActor {
        type Msg = TestActorMessage;
        type Arguments = ();
        type State = u8;

        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(0u8)
        }

        async fn handle(
            &self,
            myself: ActorRef<Self::Msg>,
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

    let handles: Vec<(ActorRef<TestActorMessage>, JoinHandle<()>)> =
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

#[allow(unused_imports)]
use output_port_subscriber_tests::*;

mod output_port_subscriber_tests {
    use super::*;
    use crate::{call_t, cast, Actor, ActorRef, RpcReplyPort};

    enum NumberPublisherMessage {
        Publish(u8),
        Subscribe(OutputPortSubscriber<u8>),
    }
    impl Message for NumberPublisherMessage {
        fn serializable() -> bool {
            false
        }
    }

    struct NumberPublisher;

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for NumberPublisher {
        type State = OutputPort<u8>;
        type Msg = NumberPublisherMessage;
        type Arguments = ();

        async fn pre_start(
            &self,
            _myself: ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(OutputPort::default())
        }

        async fn handle(
            &self,
            _myself: ActorRef<Self::Msg>,
            message: Self::Msg,
            state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            match message {
                NumberPublisherMessage::Subscribe(subscriber) => {
                    subscriber.subscribe_to_port(state);
                }
                NumberPublisherMessage::Publish(value) => {
                    state.send(value);
                }
            }
            Ok(())
        }
    }

    #[derive(Debug)]
    enum PlusSubscriberMessage {
        Plus(u8),
        Result(RpcReplyPort<u8>),
    }

    impl From<u8> for PlusSubscriberMessage {
        fn from(value: u8) -> Self {
            PlusSubscriberMessage::Plus(value)
        }
    }
    impl Message for PlusSubscriberMessage {
        fn serializable() -> bool {
            false
        }
    }

    struct PlusSubscriber;
    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for PlusSubscriber {
        type State = u8;
        type Msg = PlusSubscriberMessage;
        type Arguments = ();

        async fn pre_start(
            &self,
            _myself: ActorRef<Self::Msg>,
            _arguments: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(0)
        }

        async fn handle(
            &self,
            _myself: ActorRef<Self::Msg>,
            message: Self::Msg,
            state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            match message {
                PlusSubscriberMessage::Plus(value) => {
                    *state += value;
                }
                PlusSubscriberMessage::Result(reply) => {
                    if !reply.is_closed() {
                        reply.send(*state).unwrap();
                    }
                }
            }
            Ok(())
        }
    }

    #[derive(Debug)]
    enum MulSubscriberMessage {
        Mul(u8),
        Result(RpcReplyPort<u8>),
    }
    impl Message for MulSubscriberMessage {
        fn serializable() -> bool {
            false
        }
    }
    impl From<u8> for MulSubscriberMessage {
        fn from(value: u8) -> Self {
            MulSubscriberMessage::Mul(value)
        }
    }

    struct MulSubscriber;
    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for MulSubscriber {
        type State = u8;
        type Msg = MulSubscriberMessage;
        type Arguments = ();

        async fn pre_start(
            &self,
            _myself: ActorRef<Self::Msg>,
            _arguments: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(1)
        }

        async fn handle(
            &self,
            _myself: ActorRef<Self::Msg>,
            message: Self::Msg,
            state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            match message {
                MulSubscriberMessage::Mul(value) => {
                    *state *= value;
                }
                MulSubscriberMessage::Result(reply) => {
                    if !reply.is_closed() {
                        reply.send(*state).unwrap();
                    }
                }
            }
            Ok(())
        }
    }

    #[crate::concurrency::test]
    #[tracing_test::traced_test]
    async fn test_output_port_subscriber() {
        let (number_publisher_ref, number_publisher_handler) =
            Actor::spawn(None, NumberPublisher, ()).await.unwrap();

        let (plus_subcriber_ref, plus_subscriber_handler) =
            Actor::spawn(None, PlusSubscriber, ()).await.unwrap();

        let (mul_subcriber_ref, mul_subscriber_handler) =
            Actor::spawn(None, MulSubscriber, ()).await.unwrap();

        cast!(
            number_publisher_ref,
            NumberPublisherMessage::Subscribe(Box::new(plus_subcriber_ref.clone()))
        )
        .unwrap();
        cast!(
            number_publisher_ref,
            NumberPublisherMessage::Subscribe(Box::new(mul_subcriber_ref.clone()))
        )
        .unwrap();

        cast!(number_publisher_ref, NumberPublisherMessage::Publish(2)).unwrap();
        cast!(number_publisher_ref, NumberPublisherMessage::Publish(3)).unwrap();

        crate::concurrency::sleep(Duration::from_millis(50)).await;

        let plus_result = call_t!(plus_subcriber_ref, PlusSubscriberMessage::Result, 10).unwrap();
        let mul_result = call_t!(mul_subcriber_ref, MulSubscriberMessage::Result, 10).unwrap();
        assert_eq!(2 + 3, plus_result);
        assert_eq!(2 * 3, mul_result);

        number_publisher_ref.stop(None);
        plus_subcriber_ref.stop(None);
        mul_subcriber_ref.stop(None);

        number_publisher_handler.await.unwrap();
        plus_subscriber_handler.await.unwrap();
        mul_subscriber_handler.await.unwrap();
    }
}
