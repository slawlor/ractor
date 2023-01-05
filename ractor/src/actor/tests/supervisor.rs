// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Supervisor tests

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use crate::{Actor, ActorCell, ActorHandler, SupervisionEvent};

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
                this_actor.stop().await;
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

    let (child, child_ports) = Actor::new(None, Child);
    let (child_ref, c_handle) = child
        .start(child_ports, Some(supervisor_ref))
        .await
        .expect("Child panicked on startup");

    let (_, _) = tokio::join!(s_handle, c_handle);

    assert_eq!(child_ref.get_id(), flag.load(Ordering::Relaxed));

    // supervisor relationship cleaned up correctly
    assert_eq!(0, supervisor_tree.get_num_children().await);
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
                this_actor.stop().await;
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

    let (child, child_ports) = Actor::new(None, Child);
    let (child_ref, c_handle) = child
        .start(child_ports, Some(supervisor_ref))
        .await
        .expect("Child panicked on startup");

    // check that the supervision is wired up correctly
    assert_eq!(1, supervisor_tree.get_num_children().await);
    assert_eq!(1, child_ref.get_tree().get_num_parents().await);

    // trigger the child failure
    child_ref
        .send_message::<Child, _>(())
        .expect("Failed to send message");

    let (_, _) = tokio::join!(s_handle, c_handle);

    assert_eq!(child_ref.get_id(), flag.load(Ordering::Relaxed));

    // supervisor relationship cleaned up correctly
    assert_eq!(0, supervisor_tree.get_num_children().await);
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
            this_actor.stop().await;
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
                this_actor.stop().await;
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

    let (child, child_ports) = Actor::new(None, Child);
    let (child_ref, c_handle) = child
        .start(child_ports, Some(supervisor_ref))
        .await
        .expect("Child panicked on startup");

    let (_, _) = tokio::join!(s_handle, c_handle);

    assert_eq!(child_ref.get_id(), flag.load(Ordering::Relaxed));

    // supervisor relationship cleaned up correctly
    assert_eq!(0, supervisor_tree.get_num_children().await);
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
                this_actor.stop().await;
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

    let (midpoint, midpoint_ports) = Actor::new(None, Midpoint);
    let (midpoint_ref, m_handle) = midpoint
        .start(midpoint_ports, Some(supervisor_ref))
        .await
        .expect("Midpoint actor failed to startup");

    let midpoint_tree = midpoint_ref.get_tree();
    let midpoint_ref_clone = midpoint_ref.clone();

    let (child, child_ports) = Actor::new(None, Child);
    let (child_ref, c_handle) = child
        .start(child_ports, Some(midpoint_ref))
        .await
        .expect("Child panicked on startup");

    // check that the supervision is wired up correctly
    assert_eq!(1, supervisor_tree.get_num_children().await);
    assert_eq!(1, midpoint_tree.get_num_children().await);
    assert_eq!(1, midpoint_tree.get_num_parents().await);
    assert_eq!(1, child_ref.get_tree().get_num_parents().await);

    // trigger the child failure
    child_ref
        .send_message::<Child, _>(())
        .expect("Failed to send message");

    // which triggers the handler in the midpoint, which panic's in supervision

    // wait for all actors to die in the test
    let _ = s_handle.await;
    let _ = c_handle.await;
    let _ = m_handle.await;

    // check that we got the midpoint's ref id
    assert_eq!(midpoint_ref_clone.get_id(), flag.load(Ordering::Relaxed));

    // supervisor relationship cleaned up correctly
    assert_eq!(0, supervisor_tree.get_num_children().await);
}
