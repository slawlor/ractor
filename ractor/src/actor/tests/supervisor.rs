// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Supervisor tests

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use crate::concurrency::Duration;

use crate::{Actor, ActorCell, ActorRef, ActorStatus, SupervisionEvent};

#[cfg(feature = "cluster")]
impl crate::Message for () {}

#[crate::concurrency::test]
async fn test_supervision_panic_in_post_startup() {
    struct Child;
    struct Supervisor {
        flag: Arc<AtomicU64>,
    }

    #[async_trait::async_trait]
    impl Actor for Child {
        type Msg = ();
        type State = ();
        async fn pre_start(&self, _this_actor: ActorRef<Self>) -> Self::State {}
        async fn post_start(&self, _this_actor: ActorRef<Self>, _state: &mut Self::State) {
            panic!("Boom");
        }
    }

    #[async_trait::async_trait]
    impl Actor for Supervisor {
        type Msg = ();
        type State = ();
        async fn pre_start(&self, _this_actor: ActorRef<Self>) -> Self::State {}
        async fn handle(
            &self,
            _this_actor: ActorRef<Self>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) {
        }

        async fn handle_supervisor_evt(
            &self,
            this_actor: ActorRef<Self>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) {
            println!("Supervisor event received {:?}", message);

            // check that the panic was captured
            if let SupervisionEvent::ActorPanicked(dead_actor, _panic_msg) = message {
                self.flag
                    .store(dead_actor.get_id().get_pid(), Ordering::Relaxed);
                this_actor.stop(None);
            }
        }
    }

    let flag = Arc::new(AtomicU64::new(0));

    let (supervisor_ref, s_handle) = Actor::spawn(None, Supervisor { flag: flag.clone() })
        .await
        .expect("Supervisor panicked on startup");

    let supervisor_cell: ActorCell = supervisor_ref.clone().into();

    let (child_ref, c_handle) = Actor::spawn_linked(None, Child, supervisor_cell)
        .await
        .expect("Child panicked on startup");

    let (_, _) = tokio::join!(s_handle, c_handle);

    assert_eq!(child_ref.get_id().get_pid(), flag.load(Ordering::Relaxed));

    // supervisor relationship cleaned up correctly
    assert_eq!(0, supervisor_ref.get_num_children());
}

#[crate::concurrency::test]
async fn test_supervision_panic_in_handle() {
    struct Child;
    struct Supervisor {
        flag: Arc<AtomicU64>,
    }

    #[async_trait::async_trait]
    impl Actor for Child {
        type Msg = ();
        type State = ();
        async fn pre_start(&self, _this_actor: ActorRef<Self>) -> Self::State {}
        async fn handle(
            &self,
            _this_actor: ActorRef<Self>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) {
            panic!("Boom");
        }
    }

    #[async_trait::async_trait]
    impl Actor for Supervisor {
        type Msg = ();
        type State = ();
        async fn pre_start(&self, _this_actor: ActorRef<Self>) -> Self::State {}
        async fn handle(
            &self,
            _this_actor: ActorRef<Self>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) {
        }

        async fn handle_supervisor_evt(
            &self,
            this_actor: ActorRef<Self>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) {
            println!("Supervisor event received {:?}", message);

            // check that the panic was captured
            if let SupervisionEvent::ActorPanicked(dead_actor, _panic_msg) = message {
                self.flag
                    .store(dead_actor.get_id().get_pid(), Ordering::Relaxed);
                this_actor.stop(None);
            }
        }
    }

    let flag = Arc::new(AtomicU64::new(0));

    let (supervisor_ref, s_handle) = Actor::spawn(None, Supervisor { flag: flag.clone() })
        .await
        .expect("Supervisor panicked on startup");

    let supervisor_cell: ActorCell = supervisor_ref.clone().into();

    let (child_ref, c_handle) = Actor::spawn_linked(None, Child, supervisor_cell)
        .await
        .expect("Child panicked on startup");

    // check that the supervision is wired up correctly
    assert_eq!(1, supervisor_ref.get_num_children());
    assert_eq!(1, child_ref.get_num_parents());

    // trigger the child failure
    child_ref.send_message(()).expect("Failed to send message");

    let _ = s_handle.await;
    let _ = c_handle.await;

    assert_eq!(child_ref.get_id().get_pid(), flag.load(Ordering::Relaxed));

    // supervisor relationship cleaned up correctly
    assert_eq!(0, supervisor_ref.get_num_children());
}

#[crate::concurrency::test]
async fn test_supervision_panic_in_post_stop() {
    struct Child;
    struct Supervisor {
        flag: Arc<AtomicU64>,
    }

    #[async_trait::async_trait]
    impl Actor for Child {
        type Msg = ();
        type State = ();
        async fn pre_start(&self, myself: ActorRef<Self>) -> Self::State {
            // trigger stop, which starts shutdown
            myself.stop(None);
        }
        async fn post_stop(&self, _this_actor: ActorRef<Self>, _state: &mut Self::State) {
            panic!("Boom");
        }
    }

    #[async_trait::async_trait]
    impl Actor for Supervisor {
        type Msg = ();
        type State = ();
        async fn pre_start(&self, _this_actor: ActorRef<Self>) -> Self::State {}
        async fn handle_supervisor_evt(
            &self,
            this_actor: ActorRef<Self>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) {
            println!("Supervisor event received {:?}", message);

            // check that the panic was captured
            if let SupervisionEvent::ActorPanicked(dead_actor, _panic_msg) = message {
                self.flag
                    .store(dead_actor.get_id().get_pid(), Ordering::Relaxed);
                this_actor.stop(None);
            }
        }
    }

    let flag = Arc::new(AtomicU64::new(0));

    let (supervisor_ref, s_handle) = Actor::spawn(None, Supervisor { flag: flag.clone() })
        .await
        .expect("Supervisor panicked on startup");

    let (child_ref, c_handle) = Actor::spawn_linked(None, Child, supervisor_ref.clone().into())
        .await
        .expect("Child panicked on startup");

    let _ = s_handle.await;
    let _ = c_handle.await;

    assert_eq!(child_ref.get_id().get_pid(), flag.load(Ordering::Relaxed));

    // supervisor relationship cleaned up correctly
    assert_eq!(0, supervisor_ref.get_num_children());
}

/// Test that a panic in the supervisor's handling propogates to
/// the supervisor's supervisor
#[crate::concurrency::test]
async fn test_supervision_panic_in_supervisor_handle() {
    struct Child;
    struct Midpoint;
    struct Supervisor {
        flag: Arc<AtomicU64>,
    }

    #[async_trait::async_trait]
    impl Actor for Child {
        type Msg = ();
        type State = ();
        async fn pre_start(&self, _this_actor: ActorRef<Self>) -> Self::State {}
        async fn handle(
            &self,
            _this_actor: ActorRef<Self>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) {
            panic!("Boom!");
        }
    }

    #[async_trait::async_trait]
    impl Actor for Midpoint {
        type Msg = ();
        type State = ();
        async fn pre_start(&self, _this_actor: ActorRef<Self>) -> Self::State {}
        async fn handle_supervisor_evt(
            &self,
            _this_actor: ActorRef<Self>,
            _message: SupervisionEvent,
            _state: &mut Self::State,
        ) {
            if let SupervisionEvent::ActorPanicked(_child, _msg) = _message {
                panic!("Boom again!");
            }
        }
    }

    #[async_trait::async_trait]
    impl Actor for Supervisor {
        type Msg = ();
        type State = ();
        async fn pre_start(&self, _this_actor: ActorRef<Self>) -> Self::State {}
        async fn handle(
            &self,
            _this_actor: ActorRef<Self>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) {
        }

        async fn handle_supervisor_evt(
            &self,
            this_actor: ActorRef<Self>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) {
            println!("Supervisor event received {:?}", message);

            // check that the panic was captured
            if let SupervisionEvent::ActorPanicked(dead_actor, _panic_msg) = message {
                self.flag
                    .store(dead_actor.get_id().get_pid(), Ordering::Relaxed);
                this_actor.stop(None);
            }
        }
    }

    let flag = Arc::new(AtomicU64::new(0));

    let (supervisor_ref, s_handle) = Actor::spawn(None, Supervisor { flag: flag.clone() })
        .await
        .expect("Supervisor panicked on startup");

    let supervisor_cell: ActorCell = supervisor_ref.clone().into();

    let (midpoint_ref, m_handle) = Actor::spawn_linked(None, Midpoint, supervisor_cell)
        .await
        .expect("Midpoint actor failed to startup");

    let midpoint_cell: ActorCell = midpoint_ref.clone().into();
    let midpoint_ref_clone = midpoint_ref.clone();

    let (child_ref, c_handle) = Actor::spawn_linked(None, Child, midpoint_cell)
        .await
        .expect("Child panicked on startup");

    // check that the supervision is wired up correctly
    assert_eq!(1, supervisor_ref.get_num_children());
    assert_eq!(1, midpoint_ref.get_num_children());
    assert_eq!(1, midpoint_ref.get_num_parents());
    assert_eq!(1, child_ref.get_num_parents());

    // trigger the child failure
    child_ref.send_message(()).expect("Failed to send message");

    // which triggers the handler in the midpoint, which panic's in supervision

    // wait for all actors to die in the test
    let _ = s_handle.await;
    let _ = c_handle.await;
    let _ = m_handle.await;

    // check that we got the midpoint's ref id
    assert_eq!(
        midpoint_ref_clone.get_id().get_pid(),
        flag.load(Ordering::Relaxed)
    );

    // supervisor relationship cleaned up correctly
    assert_eq!(0, supervisor_ref.get_num_children());
}

#[crate::concurrency::test]
async fn test_killing_a_supervisor_terminates_children() {
    struct Child;
    struct Supervisor;

    #[async_trait::async_trait]
    impl Actor for Child {
        type Msg = ();
        type State = ();
        async fn pre_start(&self, _this_actor: ActorRef<Self>) -> Self::State {}
    }

    #[async_trait::async_trait]
    impl Actor for Supervisor {
        type Msg = ();
        type State = ();
        async fn pre_start(&self, _this_actor: ActorRef<Self>) -> Self::State {}
        async fn handle(
            &self,
            _this_actor: ActorRef<Self>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) {
            // stop the supervisor, which starts the supervision shutdown of children
            _this_actor.stop(None);
        }
    }

    let (supervisor_ref, s_handle) = Actor::spawn(None, Supervisor)
        .await
        .expect("Supervisor panicked on startup");

    let (child_ref, c_handle) = Actor::spawn_linked(None, Child, supervisor_ref.clone().into())
        .await
        .expect("Child panicked on startup");

    // check that the supervision is wired up correctly
    assert_eq!(1, supervisor_ref.get_num_children());
    assert_eq!(1, child_ref.get_num_parents());

    // initate the shutdown of the supervisor
    supervisor_ref
        .cast(())
        .expect("Sending message to supervisor failed");

    s_handle
        .await
        .expect("Failed to wait for supervisor to shutdown");

    // wait for async shutdown
    crate::concurrency::sleep(Duration::from_millis(50)).await;

    assert_eq!(ActorStatus::Stopped, child_ref.get_status());

    c_handle.await.expect("Failed to wait for child to die");
}

// TODO: Still to be tested
// 1. terminate_children_after()
