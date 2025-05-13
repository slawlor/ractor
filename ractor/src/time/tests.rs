// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Tests of timers

use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::common_test::periodic_check;
use crate::concurrency::Duration;
use crate::Actor;
use crate::ActorProcessingErr;
use crate::ActorRef;

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_intervals() {
    let counter = Arc::new(AtomicU8::new(0u8));

    struct TestActor {
        counter: Arc<AtomicU8>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
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

    periodic_check(
        || interval_handle.is_finished() && actor_handle.is_finished(),
        Duration::from_millis(500),
    )
    .await;

    // the counter was incremented at least this many times
    println!("Counter: {}", counter.load(Ordering::Relaxed));
    assert!(counter.load(Ordering::Relaxed) >= 7);
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_send_after() {
    let counter = Arc::new(AtomicU8::new(0u8));

    struct TestActor {
        counter: Arc<AtomicU8>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
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

    periodic_check(
        || send_after_handle.is_finished() && actor_handle.is_finished(),
        Duration::from_millis(500),
    )
    .await;

    // the counter was incremented at least this many times
    println!("Counter: {}", counter.load(Ordering::Relaxed));
    assert_eq!(1, counter.load(Ordering::Relaxed));
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_exit_after() {
    struct TestActor;

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
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

    periodic_check(
        || exit_handle.is_finished() && actor_handle.is_finished(),
        Duration::from_millis(500),
    )
    .await;
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_kill_after() {
    struct TestActor;

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
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

    periodic_check(
        || kill_handle.is_finished() && actor_handle.is_finished(),
        Duration::from_millis(500),
    )
    .await;
}
