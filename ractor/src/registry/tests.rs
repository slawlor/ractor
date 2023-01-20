// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Tests on the actor registry

use crate::concurrency::Duration;

use crate::{Actor, SpawnErr};

#[crate::concurrency::test]
async fn test_basic_registation() {
    struct EmptyActor;

    #[async_trait::async_trait]
    impl Actor for EmptyActor {
        type Msg = ();

        type State = ();

        async fn pre_start(&self, _this_actor: crate::ActorRef<Self>) -> Self::State {}
    }

    let (actor, handle) = Actor::spawn(Some("my_actor"), EmptyActor)
        .await
        .expect("Actor failed to start");

    assert!(crate::registry::where_is("my_actor").is_some());

    actor.stop(None);
    handle.await.expect("Failed to clean stop the actor");
}

#[crate::concurrency::test]
async fn test_duplicate_registration() {
    struct EmptyActor;

    #[async_trait::async_trait]
    impl Actor for EmptyActor {
        type Msg = ();

        type State = ();

        async fn pre_start(&self, _this_actor: crate::ActorRef<Self>) -> Self::State {}
    }

    let (actor, handle) = Actor::spawn(Some("my_second_actor"), EmptyActor)
        .await
        .expect("Actor failed to start");

    assert!(crate::registry::where_is("my_second_actor").is_some());

    let second_actor = Actor::spawn(Some("my_second_actor"), EmptyActor).await;
    // fails to spawn the second actor due to name err
    assert!(matches!(
        second_actor,
        Err(SpawnErr::ActorAlreadyRegistered(_))
    ));

    // make sure the first actor is still registered
    assert!(crate::registry::where_is("my_second_actor").is_some());

    actor.stop(None);
    handle.await.expect("Failed to clean stop the actor");
}

#[crate::concurrency::test]
async fn test_actor_registry_unenrollment() {
    struct EmptyActor;

    #[async_trait::async_trait]
    impl Actor for EmptyActor {
        type Msg = ();

        type State = ();

        async fn pre_start(&self, _this_actor: crate::ActorRef<Self>) -> Self::State {}
    }

    let (actor, handle) = Actor::spawn(Some("unenrollment"), EmptyActor)
        .await
        .expect("Actor failed to start");

    assert!(crate::registry::where_is("unenrollment").is_some());

    // stop the actor and wait for its death
    actor.stop(None);
    handle.await.expect("Failed to wait for agent stop");

    // drop the actor ref's
    drop(actor);

    // unenrollment is a cast operation, so it's not immediate. wait for cleanup
    crate::concurrency::sleep(Duration::from_millis(100)).await;

    // the actor was automatically removed
    assert!(crate::registry::where_is("unenrollment").is_none());
}
