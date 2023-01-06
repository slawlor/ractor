// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Supervisor tests

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use tokio::time::Duration;

use crate::{Actor, ActorCell, ActorHandler, ActorStatus, SupervisionEvent};

#[tokio::test]
async fn test_supervision_panic_in_post_startup() {
    struct Child;
    struct Supervisor {
        flag: Arc<AtomicU64>,
    }

    #[async_trait::async_trait]
    impl ActorHandler for Child {
        type Msg = ();
        type State = ();
        async fn pre_start(&self, _this_actor: ActorCell) -> Self::State {}
        async fn post_start(
            &self,
            _this_actor: ActorCell,
            _state: &Self::State,
        ) -> Option<Self::State> {
            panic!("Boom");
        }
    }

    #[async_trait::async_trait]
    impl ActorHandler for Supervisor {
        type Msg = ();
        type State = ();
        async fn pre_start(&self, _this_actor: ActorCell) -> Self::State {}
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
            message: SupervisionEvent,
            _state: &Self::State,
        ) -> Option<Self::State> {
            println!("Supervisor event received {:?}", message);

            // check that the panic was captured
            if let SupervisionEvent::ActorPanicked(dead_actor, _panic_msg) = message {
                self.flag.store(dead_actor.get_id(), Ordering::Relaxed);
                this_actor.stop(None);
            }

            None
        }
    }

    let flag = Arc::new(AtomicU64::new(0));

    let (supervisor_ref, s_handle) = Actor::spawn(None, Supervisor { flag: flag.clone() })
        .await
        .expect("Supervisor panicked on startup");

    let supervisor_tree = supervisor_ref.get_tree();

    let (child_ref, c_handle) = Actor::spawn_linked(None, Child, supervisor_ref)
        .await
        .expect("Child panicked on startup");

    let (_, _) = tokio::join!(s_handle, c_handle);

    assert_eq!(child_ref.get_id(), flag.load(Ordering::Relaxed));

    // supervisor relationship cleaned up correctly
    assert_eq!(0, supervisor_tree.get_num_children());
}

#[tokio::test]
async fn test_supervision_panic_in_handle() {
    struct Child;
    struct Supervisor {
        flag: Arc<AtomicU64>,
    }

    #[async_trait::async_trait]
    impl ActorHandler for Child {
        type Msg = ();
        type State = ();
        async fn pre_start(&self, _this_actor: ActorCell) -> Self::State {}
        async fn handle(
            &self,
            _this_actor: ActorCell,
            _message: Self::Msg,
            _state: &Self::State,
        ) -> Option<Self::State> {
            panic!("Boom");
        }
    }

    #[async_trait::async_trait]
    impl ActorHandler for Supervisor {
        type Msg = ();
        type State = ();
        async fn pre_start(&self, _this_actor: ActorCell) -> Self::State {}
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
            message: SupervisionEvent,
            _state: &Self::State,
        ) -> Option<Self::State> {
            println!("Supervisor event received {:?}", message);

            // check that the panic was captured
            if let SupervisionEvent::ActorPanicked(dead_actor, _panic_msg) = message {
                self.flag.store(dead_actor.get_id(), Ordering::Relaxed);
                this_actor.stop(None);
            }

            None
        }
    }

    let flag = Arc::new(AtomicU64::new(0));

    let (supervisor_ref, s_handle) = Actor::spawn(None, Supervisor { flag: flag.clone() })
        .await
        .expect("Supervisor panicked on startup");

    let supervisor_tree = supervisor_ref.get_tree();

    let (child_ref, c_handle) = Actor::spawn_linked(None, Child, supervisor_ref)
        .await
        .expect("Child panicked on startup");

    // check that the supervision is wired up correctly
    assert_eq!(1, supervisor_tree.get_num_children());
    assert_eq!(1, child_ref.get_tree().get_num_parents());

    // trigger the child failure
    child_ref
        .send_message::<Child>(())
        .expect("Failed to send message");

    let (_, _) = tokio::join!(s_handle, c_handle);

    assert_eq!(child_ref.get_id(), flag.load(Ordering::Relaxed));

    // supervisor relationship cleaned up correctly
    assert_eq!(0, supervisor_tree.get_num_children());
}

#[tokio::test]
async fn test_supervision_panic_in_post_stop() {
    struct Child;
    struct Supervisor {
        flag: Arc<AtomicU64>,
    }

    #[async_trait::async_trait]
    impl ActorHandler for Child {
        type Msg = ();
        type State = ();
        async fn pre_start(&self, this_actor: ActorCell) -> Self::State {
            // trigger stop, which starts shutdown
            this_actor.stop(None);
        }
        async fn post_stop(&self, _this_actor: ActorCell, _state: Self::State) -> Self::State {
            panic!("Boom");
        }
    }

    #[async_trait::async_trait]
    impl ActorHandler for Supervisor {
        type Msg = ();
        type State = ();
        async fn pre_start(&self, _this_actor: ActorCell) -> Self::State {}
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
            message: SupervisionEvent,
            _state: &Self::State,
        ) -> Option<Self::State> {
            println!("Supervisor event received {:?}", message);

            // check that the panic was captured
            if let SupervisionEvent::ActorPanicked(dead_actor, _panic_msg) = message {
                self.flag.store(dead_actor.get_id(), Ordering::Relaxed);
                this_actor.stop(None);
            }

            None
        }
    }

    let flag = Arc::new(AtomicU64::new(0));

    let (supervisor_ref, s_handle) = Actor::spawn(None, Supervisor { flag: flag.clone() })
        .await
        .expect("Supervisor panicked on startup");

    let supervisor_tree = supervisor_ref.get_tree();

    let (child_ref, c_handle) = Actor::spawn_linked(None, Child, supervisor_ref)
        .await
        .expect("Child panicked on startup");

    let (_, _) = tokio::join!(s_handle, c_handle);

    assert_eq!(child_ref.get_id(), flag.load(Ordering::Relaxed));

    // supervisor relationship cleaned up correctly
    assert_eq!(0, supervisor_tree.get_num_children());
}

/// Test that a panic in the supervisor's handling propogates to
/// the supervisor's supervisor
#[tokio::test]
async fn test_supervision_panic_in_supervisor_handle() {
    struct Child;
    struct Midpoint;
    struct Supervisor {
        flag: Arc<AtomicU64>,
    }

    #[async_trait::async_trait]
    impl ActorHandler for Child {
        type Msg = ();
        type State = ();
        async fn pre_start(&self, _this_actor: ActorCell) -> Self::State {}
        async fn handle(
            &self,
            _this_actor: ActorCell,
            _message: Self::Msg,
            _state: &Self::State,
        ) -> Option<Self::State> {
            panic!("Boom!");
        }
    }

    #[async_trait::async_trait]
    impl ActorHandler for Midpoint {
        type Msg = ();
        type State = ();
        async fn pre_start(&self, _this_actor: ActorCell) -> Self::State {}
        async fn handle_supervisor_evt(
            &self,
            _this_actor: ActorCell,
            _message: SupervisionEvent,
            _state: &Self::State,
        ) -> Option<Self::State> {
            if let SupervisionEvent::ActorPanicked(_child, _msg) = _message {
                panic!("Boom again!");
            }
            None
        }
    }

    #[async_trait::async_trait]
    impl ActorHandler for Supervisor {
        type Msg = ();
        type State = ();
        async fn pre_start(&self, _this_actor: ActorCell) -> Self::State {}
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
            message: SupervisionEvent,
            _state: &Self::State,
        ) -> Option<Self::State> {
            println!("Supervisor event received {:?}", message);

            // check that the panic was captured
            if let SupervisionEvent::ActorPanicked(dead_actor, _panic_msg) = message {
                self.flag.store(dead_actor.get_id(), Ordering::Relaxed);
                this_actor.stop(None);
            }

            None
        }
    }

    let flag = Arc::new(AtomicU64::new(0));

    let (supervisor, supervisor_ports) = Actor::new(None, Supervisor { flag: flag.clone() });
    let (supervisor_ref, s_handle) = supervisor
        .start(supervisor_ports, None)
        .await
        .expect("Supervisor panicked on startup");

    let supervisor_tree = supervisor_ref.get_tree();

    let (midpoint_ref, m_handle) = Actor::spawn_linked(None, Midpoint, supervisor_ref)
        .await
        .expect("Midpoint actor failed to startup");

    let midpoint_tree = midpoint_ref.get_tree();
    let midpoint_ref_clone = midpoint_ref.clone();

    let (child_ref, c_handle) = Actor::spawn_linked(None, Child, midpoint_ref)
        .await
        .expect("Child panicked on startup");

    // check that the supervision is wired up correctly
    assert_eq!(1, supervisor_tree.get_num_children());
    assert_eq!(1, midpoint_tree.get_num_children());
    assert_eq!(1, midpoint_tree.get_num_parents());
    assert_eq!(1, child_ref.get_tree().get_num_parents());

    // trigger the child failure
    child_ref
        .send_message::<Child>(())
        .expect("Failed to send message");

    // which triggers the handler in the midpoint, which panic's in supervision

    // wait for all actors to die in the test
    let _ = s_handle.await;
    let _ = c_handle.await;
    let _ = m_handle.await;

    // check that we got the midpoint's ref id
    assert_eq!(midpoint_ref_clone.get_id(), flag.load(Ordering::Relaxed));

    // supervisor relationship cleaned up correctly
    assert_eq!(0, supervisor_tree.get_num_children());
}

#[tokio::test]
async fn test_killing_a_supervisor_terminates_children() {
    struct Child;
    struct Supervisor;

    #[async_trait::async_trait]
    impl ActorHandler for Child {
        type Msg = ();
        type State = ();
        async fn pre_start(&self, _this_actor: ActorCell) -> Self::State {}
    }

    #[async_trait::async_trait]
    impl ActorHandler for Supervisor {
        type Msg = ();
        type State = ();
        async fn pre_start(&self, _this_actor: ActorCell) -> Self::State {}
        async fn handle(
            &self,
            _this_actor: ActorCell,
            _message: Self::Msg,
            _state: &Self::State,
        ) -> Option<Self::State> {
            // stop the supervisor, which starts the supervision shutdown of children
            _this_actor.stop(None);
            None
        }
    }

    let (supervisor_ref, s_handle) = Actor::spawn(None, Supervisor)
        .await
        .expect("Supervisor panicked on startup");

    let supervisor_tree = supervisor_ref.get_tree();

    let (child_ref, c_handle) = Actor::spawn_linked(None, Child, supervisor_ref.clone())
        .await
        .expect("Child panicked on startup");

    // check that the supervision is wired up correctly
    assert_eq!(1, supervisor_tree.get_num_children());
    assert_eq!(1, child_ref.get_tree().get_num_parents());

    // initate the shutdown of the supervisor
    supervisor_ref
        .cast::<Supervisor>(())
        .expect("Sending message to supervisor failed");

    s_handle
        .await
        .expect("Failed to wait for supervisor to shutdown");

    // wait for async shutdown
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(ActorStatus::Stopped, child_ref.get_status());

    c_handle.await.expect("Failed to wait for child to die");
}
