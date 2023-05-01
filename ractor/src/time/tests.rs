// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Tests of timers

use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};

use crate::{concurrency::Duration, ActorProcessingErr};

use crate::{Actor, ActorRef};

#[crate::concurrency::test]
async fn test_intervals() {
    let counter = Arc::new(AtomicU8::new(0u8));

    struct TestActor {
        counter: Arc<AtomicU8>,
    }

    #[async_trait::async_trait]
    impl Actor for TestActor {
        type Msg = ();
        type State = Arc<AtomicU8>;
        type Arguments = ();
        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(self.counter.clone())
        }
        async fn handle(
            &self,
            _this_actor: ActorRef<Self::Msg>,
            _message: Self::Msg,
            state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            // stop the supervisor, which starts the supervision shutdown of children
            state.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    let (actor_ref, actor_handle) = Actor::spawn(
        None,
        TestActor {
            counter: counter.clone(),
        },
        (),
    )
    .await
    .expect("Failed to create test actor");

    let interval_handle = actor_ref.send_interval(Duration::from_millis(10), || ());

    // even though the timer is started, we should be in a "pause" state for 10ms,
    // therefore the counter should be empty
    assert_eq!(0, counter.load(Ordering::Relaxed));

    crate::concurrency::sleep(Duration::from_millis(150)).await;
    // kill the actor
    actor_ref.stop(None);

    crate::concurrency::sleep(Duration::from_millis(50)).await;
    // make sure the actor is dead + the interval handle doesn't send again
    assert!(interval_handle.is_finished());
    assert!(actor_handle.is_finished());

    // the counter was incremented at least this many times
    println!("Counter: {}", counter.load(Ordering::Relaxed));
    assert!(counter.load(Ordering::Relaxed) >= 9);
}

#[crate::concurrency::test]
async fn test_send_after() {
    let counter = Arc::new(AtomicU8::new(0u8));

    struct TestActor {
        counter: Arc<AtomicU8>,
    }

    #[async_trait::async_trait]
    impl Actor for TestActor {
        type Msg = ();
        type State = Arc<AtomicU8>;
        type Arguments = ();
        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(self.counter.clone())
        }
        async fn handle(
            &self,
            _this_actor: ActorRef<Self::Msg>,
            _message: Self::Msg,
            state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            // stop the supervisor, which starts the supervision shutdown of children
            state.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    let (actor_ref, actor_handle) = Actor::spawn(
        None,
        TestActor {
            counter: counter.clone(),
        },
        (),
    )
    .await
    .expect("Failed to create test actor");

    let send_after_handle = actor_ref.send_after(Duration::from_millis(10), || ());

    // even though the timer is started, we should be in a "pause" state for 10ms,
    // therefore the counter should be empty
    assert_eq!(0, counter.load(Ordering::Relaxed));

    crate::concurrency::sleep(Duration::from_millis(20)).await;
    // kill the actor
    actor_ref.stop(None);

    crate::concurrency::sleep(Duration::from_millis(50)).await;
    // make sure the actor is dead + the interval handle doesn't send again
    assert!(send_after_handle.is_finished());
    assert!(actor_handle.is_finished());

    // the counter was incremented at least this many times
    println!("Counter: {}", counter.load(Ordering::Relaxed));
    assert_eq!(1, counter.load(Ordering::Relaxed));
}

#[crate::concurrency::test]
async fn test_exit_after() {
    struct TestActor;

    #[async_trait::async_trait]
    impl Actor for TestActor {
        type Msg = ();
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
    }

    let (actor_ref, actor_handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to create test actor");

    let exit_handle = actor_ref.exit_after(Duration::from_millis(10));

    crate::concurrency::sleep(Duration::from_millis(20)).await;
    // make sure the actor is dead + the interval handle doesn't send again
    assert!(exit_handle.is_finished());
    assert!(actor_handle.is_finished());
}

#[crate::concurrency::test]
async fn test_kill_after() {
    struct TestActor;

    #[async_trait::async_trait]
    impl Actor for TestActor {
        type Msg = ();
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self::Msg>,
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

    let (actor_ref, actor_handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to create test actor");

    // put the actor into it's event processing sleep
    actor_ref
        .send_message(())
        .expect("Failed to send message to actor");
    crate::concurrency::sleep(Duration::from_millis(10)).await;

    // kill the actor
    let kill_handle = actor_ref.kill_after(Duration::from_millis(10));

    crate::concurrency::sleep(Duration::from_millis(20)).await;
    // make sure the actor is dead + the interval handle doesn't send again
    assert!(kill_handle.is_finished());
    assert!(actor_handle.is_finished());
}
