// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Actor tests

use std::sync::{atomic::{AtomicBool, Ordering}, Arc};

use crate::{ActorCell, ActorHandler, Actor, SpawnErr, SupervisionEvent};

#[tokio::test]
async fn test_panic_on_start_captured() {

    struct TestActor;

    impl ActorHandler for TestActor {
        type Msg = ();

        type State = ();

        fn pre_start(&self,_this_actor:crate::ActorCell) -> Self::State {
            panic!("Boom!");
        }
    }

    let (actor, ports) = Actor::new(None, TestActor);
    let actor_output = actor.start(ports, None).await;

    assert!(matches!(actor_output, Err(SpawnErr::StartupPanic(_))));
}

#[tokio::test]
async fn test_supervisor_notified_on_child_panic() {
    struct Child;
    struct Supervisor {
        flag: Arc<AtomicBool>
    }

    #[async_trait::async_trait]
    impl ActorHandler for Child {
        type Msg = ();
        type State = ();
        fn pre_start(&self, _this_actor: ActorCell) -> Self::State {
            ()
        }
        async fn handle(
            &self,
            _this_actor: ActorCell,
            _message: Self::Msg,
            _state: &Self::State,
        ) -> Option<Self::State> {
            panic!("Boom again!");
        }
    }

    #[async_trait::async_trait]
    impl ActorHandler for Supervisor {
        type Msg = ();
        type State = ();
        fn pre_start(&self, _this_actor: ActorCell) -> Self::State {
            ()
        }
        async fn handle(
            &self,
            _this_actor: ActorCell,
            _message: Self::Msg,
            _state: &Self::State,
        ) -> Option<Self::State> {
            None
        }

        async fn handle_supervisor_evt(
            &self,
            this_actor: ActorCell,
            _message: SupervisionEvent,
            _state: &Self::State,
        ) -> Option<Self::State> {
            self.flag.store(true, Ordering::Relaxed);
            let _ = this_actor.stop().await;
            None
        }
    }

    let flag = Arc::new(AtomicBool::new(false));

    let (supervisor, supervisor_ports) = Actor::new(None, Supervisor{ flag: flag.clone() });
    let (supervisor_ref, s_handle) = supervisor.start(supervisor_ports, None).await.expect("Supervisor panicked on startup");

    let (child, child_ports) = Actor::new(None, Child);
    let (child_ref, c_handle) = child.start(child_ports, Some(supervisor_ref)).await.expect("Child panicked on startup");

    child_ref.send_message_t(()).expect("Failed to send message to child");

    let (_, _) = tokio::join!(s_handle, c_handle);

    assert!(flag.load(Ordering::Relaxed));
}