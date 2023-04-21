// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Supervisor tests

use std::sync::{
    atomic::{AtomicU64, AtomicU8, Ordering},
    Arc,
};

use crate::{concurrency::Duration, message::BoxedDowncastErr, ActorProcessingErr};

use crate::{Actor, ActorCell, ActorRef, ActorStatus, SupervisionEvent};

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
        type Arguments = ();
        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
        async fn post_start(
            &self,
            _this_actor: ActorRef<Self>,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            panic!("Boom");
        }
    }

    #[async_trait::async_trait]
    impl Actor for Supervisor {
        type Msg = ();
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
        async fn handle(
            &self,
            _this_actor: ActorRef<Self>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            this_actor: ActorRef<Self>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            println!("Supervisor event received {message:?}");

            // check that the panic was captured
            if let SupervisionEvent::ActorPanicked(dead_actor, _panic_msg) = message {
                self.flag
                    .store(dead_actor.get_id().pid(), Ordering::Relaxed);
                this_actor.stop(None);
            }
            Ok(())
        }
    }

    let flag = Arc::new(AtomicU64::new(0));

    let (supervisor_ref, s_handle) = Actor::spawn(None, Supervisor { flag: flag.clone() }, ())
        .await
        .expect("Supervisor panicked on startup");

    let supervisor_cell: ActorCell = supervisor_ref.clone().into();

    let (child_ref, c_handle) = Actor::spawn_linked(None, Child, (), supervisor_cell)
        .await
        .expect("Child panicked on startup");

    let (_, _) = tokio::join!(s_handle, c_handle);

    assert_eq!(child_ref.get_id().pid(), flag.load(Ordering::Relaxed));

    // supervisor relationship cleaned up correctly
    assert_eq!(0, supervisor_ref.get_num_children());
}

#[crate::concurrency::test]
async fn test_supervision_error_in_post_startup() {
    struct Child;
    struct Supervisor {
        flag: Arc<AtomicU64>,
    }

    #[async_trait::async_trait]
    impl Actor for Child {
        type Msg = ();
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
        async fn post_start(
            &self,
            _this_actor: ActorRef<Self>,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            Err(From::from("boom"))
        }
    }

    #[async_trait::async_trait]
    impl Actor for Supervisor {
        type Msg = ();
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
        async fn handle(
            &self,
            _this_actor: ActorRef<Self>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            this_actor: ActorRef<Self>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            println!("Supervisor event received {message:?}");

            // check that the panic was captured
            if let SupervisionEvent::ActorPanicked(dead_actor, _panic_msg) = message {
                self.flag
                    .store(dead_actor.get_id().pid(), Ordering::Relaxed);
                this_actor.stop(None);
            }
            Ok(())
        }
    }

    let flag = Arc::new(AtomicU64::new(0));

    let (supervisor_ref, s_handle) = Actor::spawn(None, Supervisor { flag: flag.clone() }, ())
        .await
        .expect("Supervisor panicked on startup");

    let supervisor_cell: ActorCell = supervisor_ref.clone().into();

    let (child_ref, c_handle) = Actor::spawn_linked(None, Child, (), supervisor_cell)
        .await
        .expect("Child panicked on startup");

    let (_, _) = tokio::join!(s_handle, c_handle);

    assert_eq!(child_ref.get_id().pid(), flag.load(Ordering::Relaxed));

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
        type Arguments = ();
        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
        async fn handle(
            &self,
            _this_actor: ActorRef<Self>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            panic!("Boom");
        }
    }

    #[async_trait::async_trait]
    impl Actor for Supervisor {
        type Msg = ();
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
        async fn handle(
            &self,
            _this_actor: ActorRef<Self>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            this_actor: ActorRef<Self>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            println!("Supervisor event received {message:?}");

            // check that the panic was captured
            if let SupervisionEvent::ActorPanicked(dead_actor, _panic_msg) = message {
                self.flag
                    .store(dead_actor.get_id().pid(), Ordering::Relaxed);
                this_actor.stop(None);
            }
            Ok(())
        }
    }

    let flag = Arc::new(AtomicU64::new(0));

    let (supervisor_ref, s_handle) = Actor::spawn(None, Supervisor { flag: flag.clone() }, ())
        .await
        .expect("Supervisor panicked on startup");

    let supervisor_cell: ActorCell = supervisor_ref.clone().into();

    let (child_ref, c_handle) = Actor::spawn_linked(None, Child, (), supervisor_cell)
        .await
        .expect("Child panicked on startup");

    // check that the supervision is wired up correctly
    assert_eq!(1, supervisor_ref.get_num_children());
    assert_eq!(1, child_ref.get_num_parents());

    // trigger the child failure
    child_ref.send_message(()).expect("Failed to send message");

    let _ = s_handle.await;
    let _ = c_handle.await;

    assert_eq!(child_ref.get_id().pid(), flag.load(Ordering::Relaxed));

    // supervisor relationship cleaned up correctly
    assert_eq!(0, supervisor_ref.get_num_children());
}

#[crate::concurrency::test]
async fn test_supervision_error_in_handle() {
    struct Child;
    struct Supervisor {
        flag: Arc<AtomicU64>,
    }

    #[async_trait::async_trait]
    impl Actor for Child {
        type Msg = ();
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
        async fn handle(
            &self,
            _this_actor: ActorRef<Self>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            Err(From::from("boom"))
        }
    }

    #[async_trait::async_trait]
    impl Actor for Supervisor {
        type Msg = ();
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
        async fn handle(
            &self,
            _this_actor: ActorRef<Self>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            this_actor: ActorRef<Self>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            println!("Supervisor event received {message:?}");

            // check that the panic was captured
            if let SupervisionEvent::ActorPanicked(dead_actor, _panic_msg) = message {
                self.flag
                    .store(dead_actor.get_id().pid(), Ordering::Relaxed);
                this_actor.stop(None);
            }
            Ok(())
        }
    }

    let flag = Arc::new(AtomicU64::new(0));

    let (supervisor_ref, s_handle) = Actor::spawn(None, Supervisor { flag: flag.clone() }, ())
        .await
        .expect("Supervisor panicked on startup");

    let supervisor_cell: ActorCell = supervisor_ref.clone().into();

    let (child_ref, c_handle) = Actor::spawn_linked(None, Child, (), supervisor_cell)
        .await
        .expect("Child panicked on startup");

    // check that the supervision is wired up correctly
    assert_eq!(1, supervisor_ref.get_num_children());
    assert_eq!(1, child_ref.get_num_parents());

    // trigger the child failure
    child_ref.send_message(()).expect("Failed to send message");

    let _ = s_handle.await;
    let _ = c_handle.await;

    assert_eq!(child_ref.get_id().pid(), flag.load(Ordering::Relaxed));

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
        type Arguments = ();
        async fn pre_start(
            &self,
            myself: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            // trigger stop, which starts shutdown
            myself.stop(None);
            Ok(())
        }
        async fn post_stop(
            &self,
            _this_actor: ActorRef<Self>,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            panic!("Boom");
        }
    }

    #[async_trait::async_trait]
    impl Actor for Supervisor {
        type Msg = ();
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
        async fn handle_supervisor_evt(
            &self,
            this_actor: ActorRef<Self>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            println!("Supervisor event received {message:?}");

            // check that the panic was captured
            if let SupervisionEvent::ActorPanicked(dead_actor, _panic_msg) = message {
                self.flag
                    .store(dead_actor.get_id().pid(), Ordering::Relaxed);
                this_actor.stop(None);
            }

            Ok(())
        }
    }

    let flag = Arc::new(AtomicU64::new(0));

    let (supervisor_ref, s_handle) = Actor::spawn(None, Supervisor { flag: flag.clone() }, ())
        .await
        .expect("Supervisor panicked on startup");

    let (child_ref, c_handle) = Actor::spawn_linked(None, Child, (), supervisor_ref.clone().into())
        .await
        .expect("Child panicked on startup");

    let _ = s_handle.await;
    let _ = c_handle.await;

    assert_eq!(child_ref.get_id().pid(), flag.load(Ordering::Relaxed));

    // supervisor relationship cleaned up correctly
    assert_eq!(0, supervisor_ref.get_num_children());
}

#[crate::concurrency::test]
async fn test_supervision_error_in_post_stop() {
    struct Child;
    struct Supervisor {
        flag: Arc<AtomicU64>,
    }

    #[async_trait::async_trait]
    impl Actor for Child {
        type Msg = ();
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            myself: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            // trigger stop, which starts shutdown
            myself.stop(None);
            Ok(())
        }
        async fn post_stop(
            &self,
            _this_actor: ActorRef<Self>,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            Err(From::from("boom"))
        }
    }

    #[async_trait::async_trait]
    impl Actor for Supervisor {
        type Msg = ();
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
        async fn handle_supervisor_evt(
            &self,
            this_actor: ActorRef<Self>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            println!("Supervisor event received {message:?}");

            // check that the panic was captured
            if let SupervisionEvent::ActorPanicked(dead_actor, _panic_msg) = message {
                self.flag
                    .store(dead_actor.get_id().pid(), Ordering::Relaxed);
                this_actor.stop(None);
            }

            Ok(())
        }
    }

    let flag = Arc::new(AtomicU64::new(0));

    let (supervisor_ref, s_handle) = Actor::spawn(None, Supervisor { flag: flag.clone() }, ())
        .await
        .expect("Supervisor panicked on startup");

    let (child_ref, c_handle) = Actor::spawn_linked(None, Child, (), supervisor_ref.clone().into())
        .await
        .expect("Child panicked on startup");

    let _ = s_handle.await;
    let _ = c_handle.await;

    assert_eq!(child_ref.get_id().pid(), flag.load(Ordering::Relaxed));

    // supervisor relationship cleaned up correctly
    assert_eq!(0, supervisor_ref.get_num_children());
}

/// Test that a panic in the supervisor's handling propagates to
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
        type Arguments = ();
        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
        async fn handle(
            &self,
            _this_actor: ActorRef<Self>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            panic!("Boom!");
        }
    }

    #[async_trait::async_trait]
    impl Actor for Midpoint {
        type Msg = ();
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
        async fn handle_supervisor_evt(
            &self,
            _this_actor: ActorRef<Self>,
            _message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            if let SupervisionEvent::ActorPanicked(_child, _msg) = _message {
                panic!("Boom again!");
            }
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl Actor for Supervisor {
        type Msg = ();
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
        async fn handle(
            &self,
            _this_actor: ActorRef<Self>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            this_actor: ActorRef<Self>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            println!("Supervisor event received {message:?}");

            // check that the panic was captured
            if let SupervisionEvent::ActorPanicked(dead_actor, _panic_msg) = message {
                self.flag
                    .store(dead_actor.get_id().pid(), Ordering::Relaxed);
                this_actor.stop(None);
            }
            Ok(())
        }
    }

    let flag = Arc::new(AtomicU64::new(0));

    let (supervisor_ref, s_handle) = Actor::spawn(None, Supervisor { flag: flag.clone() }, ())
        .await
        .expect("Supervisor panicked on startup");

    let supervisor_cell: ActorCell = supervisor_ref.clone().into();

    let (midpoint_ref, m_handle) = Actor::spawn_linked(None, Midpoint, (), supervisor_cell)
        .await
        .expect("Midpoint actor failed to startup");

    let midpoint_cell: ActorCell = midpoint_ref.clone().into();
    let midpoint_ref_clone = midpoint_ref.clone();

    let (child_ref, c_handle) = Actor::spawn_linked(None, Child, (), midpoint_cell)
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
        midpoint_ref_clone.get_id().pid(),
        flag.load(Ordering::Relaxed)
    );

    // supervisor relationship cleaned up correctly
    assert_eq!(0, supervisor_ref.get_num_children());
}

/// Test that a panic in the supervisor's handling propagates to
/// the supervisor's supervisor
#[crate::concurrency::test]
async fn test_supervision_error_in_supervisor_handle() {
    struct Child;
    struct Midpoint;
    struct Supervisor {
        flag: Arc<AtomicU64>,
    }

    #[async_trait::async_trait]
    impl Actor for Child {
        type Msg = ();
        type Arguments = ();
        type State = ();
        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
        async fn handle(
            &self,
            _this_actor: ActorRef<Self>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            Err(From::from("boom"))
        }
    }

    #[async_trait::async_trait]
    impl Actor for Midpoint {
        type Msg = ();
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
        async fn handle_supervisor_evt(
            &self,
            _this_actor: ActorRef<Self>,
            _message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            if let SupervisionEvent::ActorPanicked(_child, _msg) = _message {
                return Err(From::from("boom again!"));
            }
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl Actor for Supervisor {
        type Msg = ();
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
        async fn handle(
            &self,
            _this_actor: ActorRef<Self>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            this_actor: ActorRef<Self>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            println!("Supervisor event received {message:?}");

            // check that the panic was captured
            if let SupervisionEvent::ActorPanicked(dead_actor, _panic_msg) = message {
                self.flag
                    .store(dead_actor.get_id().pid(), Ordering::Relaxed);
                this_actor.stop(None);
            }
            Ok(())
        }
    }

    let flag = Arc::new(AtomicU64::new(0));

    let (supervisor_ref, s_handle) = Actor::spawn(None, Supervisor { flag: flag.clone() }, ())
        .await
        .expect("Supervisor panicked on startup");

    let supervisor_cell: ActorCell = supervisor_ref.clone().into();

    let (midpoint_ref, m_handle) = Actor::spawn_linked(None, Midpoint, (), supervisor_cell)
        .await
        .expect("Midpoint actor failed to startup");

    let midpoint_cell: ActorCell = midpoint_ref.clone().into();
    let midpoint_ref_clone = midpoint_ref.clone();

    let (child_ref, c_handle) = Actor::spawn_linked(None, Child, (), midpoint_cell)
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
        midpoint_ref_clone.get_id().pid(),
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
        type Arguments = ();
        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl Actor for Supervisor {
        type Msg = ();
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
        async fn handle(
            &self,
            _this_actor: ActorRef<Self>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            // stop the supervisor, which starts the supervision shutdown of children
            _this_actor.stop(None);
            Ok(())
        }
    }

    let (supervisor_ref, s_handle) = Actor::spawn(None, Supervisor, ())
        .await
        .expect("Supervisor panicked on startup");

    let (child_ref, c_handle) = Actor::spawn_linked(None, Child, (), supervisor_ref.clone().into())
        .await
        .expect("Child panicked on startup");

    // check that the supervision is wired up correctly
    assert_eq!(1, supervisor_ref.get_num_children());
    assert_eq!(1, child_ref.get_num_parents());

    // initiate the shutdown of the supervisor
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

#[crate::concurrency::test]
async fn instant_supervised_spawns() {
    let counter = Arc::new(AtomicU8::new(0));

    struct EmptySupervisor;
    #[async_trait::async_trait]
    impl Actor for EmptySupervisor {
        type Msg = ();
        type State = ();
        type Arguments = ();
        async fn pre_start(&self, _: ActorRef<Self>, _: ()) -> Result<(), ActorProcessingErr> {
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            _: ActorRef<Self>,
            _: SupervisionEvent,
            _: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            Err(From::from(
                "Supervision event received when it shouldn't have been!",
            ))
        }
    }

    struct EmptyActor;
    #[async_trait::async_trait]
    impl Actor for EmptyActor {
        type Msg = String;
        type State = Arc<AtomicU8>;
        type Arguments = Arc<AtomicU8>;
        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self>,
            counter: Arc<AtomicU8>,
        ) -> Result<Self::State, ActorProcessingErr> {
            // delay startup by some amount
            crate::concurrency::sleep(Duration::from_millis(200)).await;
            Ok(counter)
        }

        async fn handle(
            &self,
            _this_actor: crate::ActorRef<Self>,
            _message: String,
            state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            state.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    let (supervisor, shandle) = Actor::spawn(None, EmptySupervisor, ())
        .await
        .expect("Failed to startup supervisor");
    let (actor, handles) = crate::ActorRuntime::spawn_linked_instant(
        None,
        EmptyActor,
        counter.clone(),
        supervisor.get_cell(),
    )
    .expect("Failed to instant spawn");

    for i in 0..10 {
        actor
            .cast(format!("I = {i}"))
            .expect("Actor couldn't receive message!");
    }

    // actor is still starting up
    assert_eq!(0, counter.load(Ordering::Relaxed));

    crate::concurrency::sleep(Duration::from_millis(250)).await;
    // actor is started now and processing messages
    assert_eq!(10, counter.load(Ordering::Relaxed));

    // Cleanup
    supervisor.stop(None);
    shandle.await.unwrap();

    actor.stop(None);
    handles
        .await
        .unwrap()
        .expect("Actor's pre_start routine panicked")
        .await
        .unwrap();
}

#[crate::concurrency::test]
async fn test_supervisor_captures_dead_childs_state() {
    struct Child;
    struct Supervisor {
        flag: Arc<AtomicU64>,
    }

    #[async_trait::async_trait]
    impl Actor for Child {
        type Msg = ();
        type State = u64;
        type Arguments = ();
        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(1)
        }
        async fn post_start(
            &self,
            myself: ActorRef<Self>,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            myself.stop(None);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl Actor for Supervisor {
        type Msg = ();
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
        async fn handle(
            &self,
            _this_actor: ActorRef<Self>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            this_actor: ActorRef<Self>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            println!("Supervisor event received {message:?}");

            // Check (1) the termination was captured and we got the actor's state
            if let SupervisionEvent::ActorTerminated(
                dead_actor,
                Some(mut boxed_state),
                _maybe_msg,
            ) = message
            {
                if let Ok(1) = boxed_state.take::<u64>() {
                    self.flag
                        .store(dead_actor.get_id().pid(), Ordering::Relaxed);
                    this_actor.stop(None);
                }
            }
            Ok(())
        }
    }

    let mut temp_state = super::super::messages::BoxedState::new(123u64);
    assert_eq!(Err(BoxedDowncastErr), temp_state.take::<u32>());
    let mut temp_state = super::super::messages::BoxedState::new(123u64);
    assert_eq!(Ok(123u64), temp_state.take::<u64>());
    assert_eq!(Err(BoxedDowncastErr), temp_state.take::<u64>());

    let flag = Arc::new(AtomicU64::new(0));

    let (supervisor_ref, s_handle) = Actor::spawn(None, Supervisor { flag: flag.clone() }, ())
        .await
        .expect("Supervisor panicked on startup");

    let supervisor_cell: ActorCell = supervisor_ref.clone().into();

    let (child_ref, c_handle) = Actor::spawn_linked(None, Child, (), supervisor_cell)
        .await
        .expect("Child panicked on startup");

    let (_, _) = tokio::join!(s_handle, c_handle);

    assert_eq!(child_ref.get_id().pid(), flag.load(Ordering::Relaxed));

    // supervisor relationship cleaned up correctly
    assert_eq!(0, supervisor_ref.get_num_children());
}

// TODO: Still to be tested
// 1. terminate_children_after()
