// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Tests of timers

use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};

use tokio::time::Duration;

use crate::{Actor, ActorHandler, ActorRef};

#[tokio::test]
async fn test_intervals() {
    let counter = Arc::new(AtomicU8::new(0u8));

    struct TestActor {
        counter: Arc<AtomicU8>,
    }

    #[async_trait::async_trait]
    impl ActorHandler for TestActor {
        type Msg = ();
        type State = Arc<AtomicU8>;
        async fn pre_start(&self, _this_actor: ActorRef<Self>) -> Self::State {
            self.counter.clone()
        }
        async fn handle(
            &self,
            _this_actor: ActorRef<Self>,
            _message: Self::Msg,
            state: &Self::State,
        ) -> Option<Self::State> {
            // stop the supervisor, which starts the supervision shutdown of children
            state.fetch_add(1, Ordering::Relaxed);
            None
        }
    }

    let (actor_ref, actor_handle) = Actor::spawn(
        None,
        TestActor {
            counter: counter.clone(),
        },
    )
    .await
    .expect("Failed to create test actor");

    let interval_handle = crate::time::send_interval::<TestActor, _>(
        Duration::from_millis(10),
        actor_ref.clone().into(),
        || (),
    );

    // even though the timer is started, we should be in a "pause" state for 10ms,
    // therefore the counter should be empty
    assert_eq!(0, counter.load(Ordering::Relaxed));

    tokio::time::sleep(Duration::from_millis(120)).await;
    // kill the actor
    actor_ref.stop(None);

    tokio::time::sleep(Duration::from_millis(20)).await;
    // make sure the actor is dead + the interval handle doesn't send again
    assert!(interval_handle.is_finished());
    assert!(actor_handle.is_finished());

    // the counter was incremented at least this many times
    println!("Counter: {}", counter.load(Ordering::Relaxed));
    assert!(counter.load(Ordering::Relaxed) >= 9);
}

#[tokio::test]
async fn test_send_after() {
    let counter = Arc::new(AtomicU8::new(0u8));

    struct TestActor {
        counter: Arc<AtomicU8>,
    }

    #[async_trait::async_trait]
    impl ActorHandler for TestActor {
        type Msg = ();
        type State = Arc<AtomicU8>;
        async fn pre_start(&self, _this_actor: ActorRef<Self>) -> Self::State {
            self.counter.clone()
        }
        async fn handle(
            &self,
            _this_actor: ActorRef<Self>,
            _message: Self::Msg,
            state: &Self::State,
        ) -> Option<Self::State> {
            // stop the supervisor, which starts the supervision shutdown of children
            state.fetch_add(1, Ordering::Relaxed);
            None
        }
    }

    let (actor_ref, actor_handle) = Actor::spawn(
        None,
        TestActor {
            counter: counter.clone(),
        },
    )
    .await
    .expect("Failed to create test actor");

    let send_after_handle = crate::time::send_after::<TestActor, _>(
        Duration::from_millis(10),
        actor_ref.clone().into(),
        || (),
    );

    // even though the timer is started, we should be in a "pause" state for 10ms,
    // therefore the counter should be empty
    assert_eq!(0, counter.load(Ordering::Relaxed));

    tokio::time::sleep(Duration::from_millis(20)).await;
    // kill the actor
    actor_ref.stop(None);

    tokio::time::sleep(Duration::from_millis(20)).await;
    // make sure the actor is dead + the interval handle doesn't send again
    assert!(send_after_handle.is_finished());
    assert!(actor_handle.is_finished());

    // the counter was incremented at least this many times
    println!("Counter: {}", counter.load(Ordering::Relaxed));
    assert_eq!(1, counter.load(Ordering::Relaxed));
}

#[tokio::test]
async fn test_exit_after() {
    struct TestActor;

    #[async_trait::async_trait]
    impl ActorHandler for TestActor {
        type Msg = ();
        type State = ();
        async fn pre_start(&self, _this_actor: ActorRef<Self>) -> Self::State {}
    }

    let (actor_ref, actor_handle) = Actor::spawn(None, TestActor)
        .await
        .expect("Failed to create test actor");

    let exit_handle = crate::time::exit_after(Duration::from_millis(10), actor_ref.into());

    tokio::time::sleep(Duration::from_millis(20)).await;
    // make sure the actor is dead + the interval handle doesn't send again
    assert!(exit_handle.is_finished());
    assert!(actor_handle.is_finished());
}

#[tokio::test]
async fn test_kill_after() {
    struct TestActor;

    #[async_trait::async_trait]
    impl ActorHandler for TestActor {
        type Msg = ();
        type State = ();
        async fn pre_start(&self, _this_actor: ActorRef<Self>) -> Self::State {}
        async fn handle(
            &self,
            _myself: ActorRef<Self>,
            _message: Self::Msg,
            _state: &Self::State,
        ) -> Option<Self::State> {
            tokio::time::sleep(Duration::from_millis(100)).await;
            None
        }
    }

    let (actor_ref, actor_handle) = Actor::spawn(None, TestActor)
        .await
        .expect("Failed to create test actor");

    // put the actor into it's event processing sleep
    actor_ref
        .send_message(())
        .expect("Failed to send message to actor");
    tokio::time::sleep(Duration::from_millis(10)).await;

    // kill the actor
    let kill_handle = crate::time::kill_after(Duration::from_millis(10), actor_ref.into());

    tokio::time::sleep(Duration::from_millis(20)).await;
    // make sure the actor is dead + the interval handle doesn't send again
    assert!(kill_handle.is_finished());
    assert!(actor_handle.is_finished());
}
