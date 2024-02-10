// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Tests on the actor registry

use crate::concurrency::Duration;

use crate::{Actor, ActorProcessingErr, SpawnErr};

#[crate::concurrency::test]
#[tracing_test::traced_test]
async fn test_basic_registation() {
    struct EmptyActor;

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for EmptyActor {
        type Msg = ();
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

    let (actor, handle) = Actor::spawn(Some("my_actor".to_string()), EmptyActor, ())
        .await
        .expect("Actor failed to start");

    assert!(crate::registry::where_is("my_actor".to_string()).is_some());

    // Coverage for Issue #70
    assert!(crate::ActorRef::<()>::where_is("my_actor".to_string()).is_some());

    actor.stop(None);
    handle.await.expect("Failed to clean stop the actor");
}

#[crate::concurrency::test]
#[tracing_test::traced_test]
async fn test_duplicate_registration() {
    struct EmptyActor;

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for EmptyActor {
        type Msg = ();
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

    tracing::debug!(
        "{:?}",
        crate::registry::ActorRegistryErr::AlreadyRegistered("Some name".to_string())
    );

    let (actor, handle) = Actor::spawn(Some("my_second_actor".to_string()), EmptyActor, ())
        .await
        .expect("Actor failed to start");

    assert!(crate::registry::where_is("my_second_actor".to_string()).is_some());
    assert!(crate::registry::registered()
        .iter()
        .any(|name| name.as_str() == "my_second_actor"));

    let second_actor = Actor::spawn(Some("my_second_actor".to_string()), EmptyActor, ()).await;
    // fails to spawn the second actor due to name err
    assert!(matches!(
        second_actor,
        Err(SpawnErr::ActorAlreadyRegistered(_))
    ));

    // make sure the first actor is still registered
    assert!(crate::registry::where_is("my_second_actor".to_string()).is_some());

    actor.stop(None);
    handle.await.expect("Failed to clean stop the actor");
}

#[crate::concurrency::test]
#[tracing_test::traced_test]
async fn test_actor_registry_unenrollment() {
    struct EmptyActor;

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for EmptyActor {
        type Msg = ();
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

    let (actor, handle) = Actor::spawn(Some("unenrollment".to_string()), EmptyActor, ())
        .await
        .expect("Actor failed to start");

    assert!(crate::registry::where_is("unenrollment".to_string()).is_some());

    // stop the actor and wait for its death
    actor.stop(None);
    handle.await.expect("Failed to wait for agent stop");

    // drop the actor ref's
    drop(actor);

    // unenrollment is a cast operation, so it's not immediate. wait for cleanup
    crate::concurrency::sleep(Duration::from_millis(100)).await;

    // the actor was automatically removed
    assert!(crate::registry::where_is("unenrollment".to_string()).is_none());
}

#[cfg(feature = "cluster")]
mod pid_registry_tests {
    use std::sync::Arc;

    use dashmap::DashMap;

    use super::super::pid_registry::*;
    use crate::{
        common_test::periodic_check, concurrency::Duration, Actor, ActorId, ActorProcessingErr,
        SupervisionEvent,
    };

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

    #[crate::concurrency::test]
    #[tracing_test::traced_test]
    async fn try_enroll_remote_actor() {
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
        let remote_pid = ActorId::Remote { node_id: 1, pid: 1 };

        let (actor, handle) = Actor::spawn(None, EmptyActor, ())
            .await
            .expect("Actor failed to start");

        let (remote_actor, remote_handle) = crate::ActorRuntime::spawn_linked_remote(
            None,
            RemoteActor,
            remote_pid,
            (),
            actor.get_cell(),
        )
        .await
        .expect("Failed to start remote actor");

        // try and force the enrollment?
        crate::registry::pid_registry::register_pid(remote_actor.get_id(), remote_actor.get_cell())
            .expect("Enrollment of a remote actor should always be `Ok()`");

        assert!(crate::registry::where_is_pid(remote_actor.get_id()).is_none());
        assert!(crate::registry::where_is_pid(actor.get_id()).is_some());

        remote_actor.stop(None);
        actor.stop(None);
        handle.await.expect("Failed to clean stop the actor");
        remote_handle.await.expect("Failed to stop remote actor");
    }

    #[crate::concurrency::test]
    #[tracing_test::traced_test]
    async fn test_basic_registation() {
        struct EmptyActor;

        #[cfg_attr(feature = "async-trait", crate::async_trait)]
        impl Actor for EmptyActor {
            type Msg = ();
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

        let (actor, handle) = Actor::spawn(None, EmptyActor, ())
            .await
            .expect("Actor failed to start");

        assert!(crate::registry::where_is_pid(actor.get_id()).is_some());
        // check it's in the all pids collection too
        assert!(crate::registry::get_all_pids()
            .iter()
            .any(|cell| cell.get_id() == actor.get_id()));

        actor.stop(None);
        handle.await.expect("Failed to clean stop the actor");
    }

    #[crate::concurrency::test]
    #[tracing_test::traced_test]
    async fn test_actor_registry_unenrollment() {
        struct EmptyActor;

        #[cfg_attr(feature = "async-trait", crate::async_trait)]
        impl Actor for EmptyActor {
            type Msg = ();
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

        let (actor, handle) = Actor::spawn(None, EmptyActor, ())
            .await
            .expect("Actor failed to start");

        assert!(crate::registry::where_is_pid(actor.get_id()).is_some());

        // stop the actor and wait for its death
        actor.stop(None);
        handle.await.expect("Failed to wait for agent stop");

        let id = actor.get_id();

        // drop the actor ref's
        drop(actor);

        // unenrollment is a cast operation, so it's not immediate. wait for cleanup
        crate::concurrency::sleep(Duration::from_millis(100)).await;

        // the actor was automatically removed
        assert!(crate::registry::where_is_pid(id).is_none());
    }

    #[crate::concurrency::test]
    #[tracing_test::traced_test]
    async fn test_pid_lifecycle_monitoring() {
        let counter = Arc::new(DashMap::new());

        struct AutoJoinActor;

        #[cfg_attr(feature = "async-trait", crate::async_trait)]
        impl Actor for AutoJoinActor {
            type Msg = ();
            type Arguments = ();
            type State = ();

            async fn pre_start(
                &self,
                _myself: crate::ActorRef<Self::Msg>,
                _: (),
            ) -> Result<Self::State, ActorProcessingErr> {
                Ok(())
            }
        }

        struct NotificationMonitor {
            counter: Arc<DashMap<ActorId, u8>>,
        }

        #[cfg_attr(feature = "async-trait", crate::async_trait)]
        impl Actor for NotificationMonitor {
            type Msg = ();
            type Arguments = ();
            type State = ();

            async fn pre_start(
                &self,
                myself: crate::ActorRef<Self::Msg>,
                _: (),
            ) -> Result<Self::State, ActorProcessingErr> {
                monitor(myself.into());
                Ok(())
            }

            async fn handle_supervisor_evt(
                &self,
                _myself: crate::ActorRef<Self::Msg>,
                message: SupervisionEvent,
                _state: &mut Self::State,
            ) -> Result<(), ActorProcessingErr> {
                if let SupervisionEvent::PidLifecycleEvent(change) = message {
                    match change {
                        PidLifecycleEvent::Spawn(who) => {
                            self.counter.insert(who.get_id(), 1);
                            // self.counter.get_mut(&who.get_id())
                            // self.counter.fetch_add(1, Ordering::Relaxed);
                        }
                        PidLifecycleEvent::Terminate(who) => {
                            // self.counter.fetch_sub(1, Ordering::Relaxed);
                            self.counter.insert(who.get_id(), 0);
                        }
                    }
                }
                Ok(())
            }
        }
        let (monitor_actor, monitor_handle) = Actor::spawn(
            None,
            NotificationMonitor {
                counter: counter.clone(),
            },
            (),
        )
        .await
        .expect("Failed to start monitor actor");

        // this actor's startup should "monitor" for PG changes
        let (test_actor, test_handle) = Actor::spawn(None, AutoJoinActor, ())
            .await
            .expect("Failed to start test actor");

        // DUE to the static nature of the PID monitors, we're creating a LOT of actors
        // across the tests and there's a counting race here. So we use a map to check
        // this specific test actor
        periodic_check(
            || matches!(counter.get(&test_actor.get_id()).map(|v| *v), Some(1)),
            Duration::from_millis(500),
        )
        .await;

        // kill the pg member
        test_actor.stop(None);
        test_handle.await.expect("Actor cleanup failed");

        // should have decremented
        periodic_check(
            || matches!(counter.get(&test_actor.get_id()).map(|v| *v), Some(0)),
            Duration::from_millis(500),
        )
        .await;

        // cleanup
        monitor_actor.stop(None);
        monitor_handle.await.expect("Actor cleanup failed");

        let ev = PidLifecycleEvent::Spawn(test_actor.get_cell());
        tracing::debug!("{:?}", ev);
        tracing::debug!("{:?}", PidLifecycleEvent::Terminate(test_actor.get_cell()));
    }
}
