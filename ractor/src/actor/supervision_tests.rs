// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Supervisor tests

use std::sync::atomic::AtomicU64;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::concurrency::Duration;
use crate::message::DowncastErr;
use crate::periodic_check;
use crate::Actor;
use crate::ActorCell;
use crate::ActorProcessingErr;
use crate::ActorRef;
use crate::ActorStatus;
use crate::SupervisionEvent;

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
async fn test_supervision_panic_in_post_startup() {
    struct Child;
    struct Supervisor {
        flag: Arc<AtomicU64>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Child {
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
        async fn post_start(
            &self,
            _this_actor: ActorRef<Self::Msg>,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            panic!("Boom");
        }
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Supervisor {
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
            _this_actor: ActorRef<Self::Msg>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            this_actor: ActorRef<Self::Msg>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            println!("Supervisor event received {message:?}");

            // check that the panic was captured
            if let SupervisionEvent::ActorFailed(dead_actor, _panic_msg) = message {
                self.flag.store(dead_actor.get_id().pid(), Ordering::SeqCst);
                this_actor.stop(None);
            }
            Ok(())
        }
    }

    let flag = Arc::new(AtomicU64::new(0));

    let (supervisor_ref, s_handle) = Actor::spawn(None, Supervisor { flag: flag.clone() }, ())
        .await
        .expect("Supervisor panicked on startup");

    let (child_ref, c_handle) = supervisor_ref
        .spawn_linked(None, Child, ())
        .await
        .expect("Child panicked on startup");

    let maybe_sup = child_ref.try_get_supervisor();
    assert!(maybe_sup.is_some());
    assert_eq!(maybe_sup.map(|a| a.get_id()), Some(supervisor_ref.get_id()));

    let (_, _) = tokio::join!(s_handle, c_handle);

    assert_eq!(child_ref.get_id().pid(), flag.load(Ordering::SeqCst));

    // supervisor relationship cleaned up correctly
    assert_eq!(0, supervisor_ref.get_num_children());
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_supervision_error_in_post_startup() {
    struct Child;
    struct Supervisor {
        flag: Arc<AtomicU64>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Child {
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
        async fn post_start(
            &self,
            _this_actor: ActorRef<Self::Msg>,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            Err(From::from("boom"))
        }
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Supervisor {
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
            _this_actor: ActorRef<Self::Msg>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            this_actor: ActorRef<Self::Msg>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            println!("Supervisor event received {message:?}");

            // check that the panic was captured
            if let SupervisionEvent::ActorFailed(dead_actor, _panic_msg) = message {
                self.flag.store(dead_actor.get_id().pid(), Ordering::SeqCst);
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

    assert_eq!(child_ref.get_id().pid(), flag.load(Ordering::SeqCst));

    // supervisor relationship cleaned up correctly
    assert_eq!(0, supervisor_ref.get_num_children());
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
async fn test_supervision_panic_in_handle() {
    struct Child;
    struct Supervisor {
        flag: Arc<AtomicU64>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Child {
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
            _this_actor: ActorRef<Self::Msg>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            panic!("Boom");
        }
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Supervisor {
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
            _this_actor: ActorRef<Self::Msg>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            this_actor: ActorRef<Self::Msg>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            println!("Supervisor event received {message:?}");

            // check that the panic was captured
            if let SupervisionEvent::ActorFailed(dead_actor, _panic_msg) = message {
                self.flag.store(dead_actor.get_id().pid(), Ordering::SeqCst);
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

    assert_eq!(child_ref.get_id().pid(), flag.load(Ordering::SeqCst));

    // supervisor relationship cleaned up correctly
    assert_eq!(0, supervisor_ref.get_num_children());
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_supervision_error_in_handle() {
    struct Child;
    struct Supervisor {
        flag: Arc<AtomicU64>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Child {
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
            _this_actor: ActorRef<Self::Msg>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            Err(From::from("boom"))
        }
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Supervisor {
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
            _this_actor: ActorRef<Self::Msg>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            this_actor: ActorRef<Self::Msg>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            println!("Supervisor event received {message:?}");

            // check that the panic was captured
            if let SupervisionEvent::ActorFailed(dead_actor, _panic_msg) = message {
                self.flag.store(dead_actor.get_id().pid(), Ordering::SeqCst);
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

    assert_eq!(child_ref.get_id().pid(), flag.load(Ordering::SeqCst));

    // supervisor relationship cleaned up correctly
    assert_eq!(0, supervisor_ref.get_num_children());
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
async fn test_supervision_panic_in_post_stop() {
    struct Child;
    struct Supervisor {
        flag: Arc<AtomicU64>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Child {
        type Msg = ();
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            myself: ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            // trigger stop, which starts shutdown
            myself.stop(None);
            Ok(())
        }
        async fn post_stop(
            &self,
            _this_actor: ActorRef<Self::Msg>,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            panic!("Boom");
        }
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Supervisor {
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
        async fn handle_supervisor_evt(
            &self,
            this_actor: ActorRef<Self::Msg>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            println!("Supervisor event received {message:?}");

            // check that the panic was captured
            if let SupervisionEvent::ActorFailed(dead_actor, _panic_msg) = message {
                self.flag.store(dead_actor.get_id().pid(), Ordering::SeqCst);
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

    assert_eq!(child_ref.get_id().pid(), flag.load(Ordering::SeqCst));

    // supervisor relationship cleaned up correctly
    assert_eq!(0, supervisor_ref.get_num_children());
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_supervision_error_in_post_stop() {
    struct Child;
    struct Supervisor {
        flag: Arc<AtomicU64>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Child {
        type Msg = ();
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            myself: ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            // trigger stop, which starts shutdown
            myself.stop(None);
            Ok(())
        }
        async fn post_stop(
            &self,
            _this_actor: ActorRef<Self::Msg>,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            Err(From::from("boom"))
        }
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Supervisor {
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
        async fn handle_supervisor_evt(
            &self,
            this_actor: ActorRef<Self::Msg>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            println!("Supervisor event received {message:?}");

            // check that the panic was captured
            if let SupervisionEvent::ActorFailed(dead_actor, _panic_msg) = message {
                self.flag.store(dead_actor.get_id().pid(), Ordering::SeqCst);
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

    assert_eq!(child_ref.get_id().pid(), flag.load(Ordering::SeqCst));

    // supervisor relationship cleaned up correctly
    assert_eq!(0, supervisor_ref.get_num_children());
}

/// Test that a panic in the supervisor's handling propagates to
/// the supervisor's supervisor
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
async fn test_supervision_panic_in_supervisor_handle() {
    struct Child;
    struct Midpoint;
    struct Supervisor {
        flag: Arc<AtomicU64>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Child {
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
            _this_actor: ActorRef<Self::Msg>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            panic!("Boom!");
        }
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Midpoint {
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
        async fn handle_supervisor_evt(
            &self,
            _this_actor: ActorRef<Self::Msg>,
            _message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            if let SupervisionEvent::ActorFailed(_child, _msg) = _message {
                panic!("Boom again!");
            }
            Ok(())
        }
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Supervisor {
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
            _this_actor: ActorRef<Self::Msg>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            this_actor: ActorRef<Self::Msg>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            println!("Supervisor event received {message:?}");

            // check that the panic was captured
            if let SupervisionEvent::ActorFailed(dead_actor, _panic_msg) = message {
                self.flag.store(dead_actor.get_id().pid(), Ordering::SeqCst);
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
        flag.load(Ordering::SeqCst)
    );

    // supervisor relationship cleaned up correctly
    assert_eq!(0, supervisor_ref.get_num_children());
}

/// Test that a panic in the supervisor's handling propagates to
/// the supervisor's supervisor
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_supervision_error_in_supervisor_handle() {
    struct Child;
    struct Midpoint;
    struct Supervisor {
        flag: Arc<AtomicU64>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Child {
        type Msg = ();
        type Arguments = ();
        type State = ();
        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
        async fn handle(
            &self,
            _this_actor: ActorRef<Self::Msg>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            Err(From::from("boom"))
        }
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Midpoint {
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
        async fn handle_supervisor_evt(
            &self,
            _this_actor: ActorRef<Self::Msg>,
            _message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            if let SupervisionEvent::ActorFailed(_child, _msg) = _message {
                return Err(From::from("boom again!"));
            }
            Ok(())
        }
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Supervisor {
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
            _this_actor: ActorRef<Self::Msg>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            this_actor: ActorRef<Self::Msg>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            println!("Supervisor event received {message:?}");

            // check that the panic was captured
            if let SupervisionEvent::ActorFailed(dead_actor, _panic_msg) = message {
                self.flag.store(dead_actor.get_id().pid(), Ordering::SeqCst);
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
        flag.load(Ordering::SeqCst)
    );

    // supervisor relationship cleaned up correctly
    assert_eq!(0, supervisor_ref.get_num_children());
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_killing_a_supervisor_terminates_children() {
    struct Child;
    struct Supervisor;

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Child {
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

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Supervisor {
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
            _this_actor: ActorRef<Self::Msg>,
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
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn instant_supervised_spawns() {
    let counter = Arc::new(AtomicU8::new(0));

    struct EmptySupervisor;
    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for EmptySupervisor {
        type Msg = ();
        type State = ();
        type Arguments = ();
        async fn pre_start(&self, _: ActorRef<Self::Msg>, _: ()) -> Result<(), ActorProcessingErr> {
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            _: ActorRef<Self::Msg>,
            evt: SupervisionEvent,
            _: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            if let SupervisionEvent::ActorStarted(_) = evt {
                return Ok(());
            }
            Err(From::from(
                "Supervision event received when it shouldn't have been!",
            ))
        }
    }

    struct EmptyActor;
    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for EmptyActor {
        type Msg = String;
        type State = Arc<AtomicU8>;
        type Arguments = Arc<AtomicU8>;
        async fn pre_start(
            &self,
            _this_actor: crate::ActorRef<Self::Msg>,
            counter: Arc<AtomicU8>,
        ) -> Result<Self::State, ActorProcessingErr> {
            // delay startup by some amount
            crate::concurrency::sleep(Duration::from_millis(100)).await;
            Ok(counter)
        }

        async fn handle(
            &self,
            _this_actor: crate::ActorRef<Self::Msg>,
            _message: String,
            state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            state.fetch_add(1, Ordering::SeqCst);
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
    assert_eq!(0, counter.load(Ordering::SeqCst));

    // wait for everything processed
    periodic_check(
        || actor.get_status() == ActorStatus::Running,
        Duration::from_secs(5),
    )
    .await;
    actor
        .drain_and_wait(None)
        .await
        .expect("Failed to drain actor");
    assert_eq!(counter.load(Ordering::SeqCst), 10);

    // Cleanup
    supervisor.stop(None);
    shandle.await.unwrap();

    // actor.stop(None);
    handles
        .await
        .unwrap()
        .expect("Actor's pre_start routine panicked")
        .await
        .unwrap();
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_supervisor_captures_dead_childs_state() {
    struct Child;
    struct Supervisor {
        flag: Arc<AtomicU64>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Child {
        type Msg = ();
        type State = u64;
        type Arguments = ();
        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(1)
        }
        async fn post_start(
            &self,
            myself: ActorRef<Self::Msg>,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            myself.stop(None);
            Ok(())
        }
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Supervisor {
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
            _this_actor: ActorRef<Self::Msg>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            this_actor: ActorRef<Self::Msg>,
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
                    self.flag.store(dead_actor.get_id().pid(), Ordering::SeqCst);
                    this_actor.stop(None);
                }
            }
            Ok(())
        }
    }

    let mut temp_state = super::messages::BoxedState::new(123u64);
    assert_eq!(Err(DowncastErr), temp_state.take::<u32>());
    let mut temp_state = super::messages::BoxedState::new(123u64);
    assert_eq!(Ok(123u64), temp_state.take::<u64>());
    assert_eq!(Err(DowncastErr), temp_state.take::<u64>());

    let flag = Arc::new(AtomicU64::new(0));

    let (supervisor_ref, s_handle) = Actor::spawn(None, Supervisor { flag: flag.clone() }, ())
        .await
        .expect("Supervisor panicked on startup");

    let (child_ref, c_handle) = supervisor_ref
        .spawn_linked(None, Child, ())
        .await
        .expect("Child panicked on startup");

    let (_, _) = tokio::join!(s_handle, c_handle);

    assert_eq!(child_ref.get_id().pid(), flag.load(Ordering::SeqCst));

    // supervisor relationship cleaned up correctly
    assert_eq!(0, supervisor_ref.get_num_children());
}

// TODO: Still to be tested
// 1. terminate_children_after()

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_supervisor_double_link() {
    struct Who;

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Who {
        type Msg = ();
        type State = ();
        type Arguments = ();

        async fn pre_start(
            &self,
            _: ActorRef<Self::Msg>,
            _: Self::Arguments,
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
    }

    let (a, ah) = Actor::spawn(None, Who, ())
        .await
        .expect("Failed to start a");
    let (b, bh) = Actor::spawn(None, Who, ())
        .await
        .expect("Failed to start b");

    a.link(b.get_cell());
    b.link(a.get_cell());

    crate::concurrency::sleep(Duration::from_millis(100)).await;

    // stopping b should kill a since they're linked (or vice-versa)
    b.stop(None);

    // This panics if double-link, unlinking fails
    crate::concurrency::sleep(Duration::from_millis(100)).await;

    ah.await.unwrap();
    bh.await.unwrap();
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
// Ignored on wasm32-unknown-unknown, since panic can't be catched on this platform
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
async fn test_supervisor_exit_doesnt_call_child_post_stop() {
    struct Child {
        post_stop_calls: Arc<AtomicU8>,
    }
    struct Supervisor;

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Child {
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
        async fn post_stop(
            &self,
            _this_actor: ActorRef<Self::Msg>,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            self.post_stop_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Supervisor {
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
            _this_actor: ActorRef<Self::Msg>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            panic!("Boom")
        }
    }

    let flag = Arc::new(AtomicU8::new(0));

    let (supervisor_ref, s_handle) = Actor::spawn(None, Supervisor, ())
        .await
        .expect("Supervisor panicked on startup");

    let supervisor_cell: ActorCell = supervisor_ref.clone().into();

    let (_child_ref, c_handle) = Actor::spawn_linked(
        None,
        Child {
            post_stop_calls: flag.clone(),
        },
        (),
        supervisor_cell,
    )
    .await
    .expect("Child panicked on startup");

    // Send signal to blow-up the supervisor
    supervisor_ref
        .cast(())
        .expect("Failed to send message to supervisor");

    // Wait for exit
    s_handle.await.unwrap();
    c_handle.await.unwrap();

    // Child's post-stop should NOT have been called.
    assert_eq!(0, flag.load(Ordering::SeqCst));
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn stopping_children_and_wait_during_parent_shutdown() {
    struct Child {
        post_stop_calls: Arc<AtomicU8>,
    }
    struct Supervisor;

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Child {
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
        async fn post_stop(
            &self,
            _this_actor: ActorRef<Self::Msg>,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            self.post_stop_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Supervisor {
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
        async fn post_stop(
            &self,
            this_actor: ActorRef<Self::Msg>,
            _: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            this_actor.stop_children_and_wait(None, None).await;
            Ok(())
        }
    }

    let flag = Arc::new(AtomicU8::new(0));

    let (supervisor_ref, s_handle) = Actor::spawn(None, Supervisor, ())
        .await
        .expect("Supervisor panicked on startup");

    let supervisor_cell: ActorCell = supervisor_ref.clone().into();

    let (_child_ref, c_handle) = Actor::spawn_linked(
        None,
        Child {
            post_stop_calls: flag.clone(),
        },
        (),
        supervisor_cell,
    )
    .await
    .expect("Child panicked on startup");

    // Send signal to blow-up the supervisor
    supervisor_ref.stop(None);

    // Wait for exit
    s_handle.await.unwrap();
    c_handle.await.unwrap();

    // Child's post-stop should have been called.
    assert_eq!(1, flag.load(Ordering::SeqCst));
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn stopping_children_will_shutdown_parent_too() {
    struct Child {
        post_stop_calls: Arc<AtomicU8>,
    }
    struct Supervisor;

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Child {
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
        async fn post_stop(
            &self,
            _this_actor: ActorRef<Self::Msg>,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            self.post_stop_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Supervisor {
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
        async fn post_stop(
            &self,
            this_actor: ActorRef<Self::Msg>,
            _: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            this_actor.stop_children_and_wait(None, None).await;
            Ok(())
        }
    }

    let flag = Arc::new(AtomicU8::new(0));

    let (supervisor_ref, s_handle) = Actor::spawn(None, Supervisor, ())
        .await
        .expect("Supervisor panicked on startup");

    let supervisor_cell: ActorCell = supervisor_ref.clone().into();

    let (_child_ref, c_handle) = Actor::spawn_linked(
        None,
        Child {
            post_stop_calls: flag.clone(),
        },
        (),
        supervisor_cell,
    )
    .await
    .expect("Child panicked on startup");

    // Send signal to stop the actor's children, which will
    // notify the parent (supervisor) and take that down too
    supervisor_ref.stop_children(None);

    // Wait for exit
    s_handle.await.unwrap();
    c_handle.await.unwrap();

    // Child's post-stop should have been called.
    assert_eq!(1, flag.load(Ordering::SeqCst));
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn draining_children_and_wait_during_parent_shutdown() {
    struct Child {
        post_stop_calls: Arc<AtomicU8>,
    }
    struct Supervisor;

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Child {
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
        async fn post_stop(
            &self,
            _this_actor: ActorRef<Self::Msg>,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            self.post_stop_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Supervisor {
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
        async fn post_stop(
            &self,
            this_actor: ActorRef<Self::Msg>,
            _: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            this_actor.drain_children_and_wait(None).await;
            Ok(())
        }
    }

    let flag = Arc::new(AtomicU8::new(0));

    let (supervisor_ref, s_handle) = Actor::spawn(None, Supervisor, ())
        .await
        .expect("Supervisor panicked on startup");

    let supervisor_cell: ActorCell = supervisor_ref.clone().into();

    let (_child_ref, c_handle) = Actor::spawn_linked(
        None,
        Child {
            post_stop_calls: flag.clone(),
        },
        (),
        supervisor_cell,
    )
    .await
    .expect("Child panicked on startup");

    // Send signal to blow-up the supervisor
    supervisor_ref.stop(None);

    // Wait for exit
    s_handle.await.unwrap();
    c_handle.await.unwrap();

    // Child's post-stop should have been called.
    assert_eq!(1, flag.load(Ordering::SeqCst));
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn draining_children_will_shutdown_parent_too() {
    struct Child {
        post_stop_calls: Arc<AtomicU8>,
    }
    struct Supervisor;

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Child {
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
        async fn post_stop(
            &self,
            _this_actor: ActorRef<Self::Msg>,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            self.post_stop_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Supervisor {
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
        async fn post_stop(
            &self,
            this_actor: ActorRef<Self::Msg>,
            _: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            this_actor.drain_children_and_wait(None).await;
            Ok(())
        }
    }

    let flag = Arc::new(AtomicU8::new(0));

    let (supervisor_ref, s_handle) = Actor::spawn(None, Supervisor, ())
        .await
        .expect("Supervisor panicked on startup");

    let supervisor_cell: ActorCell = supervisor_ref.clone().into();

    let (_child_ref, c_handle) = Actor::spawn_linked(
        None,
        Child {
            post_stop_calls: flag.clone(),
        },
        (),
        supervisor_cell,
    )
    .await
    .expect("Child panicked on startup");

    // Send signal to drain the actor's children, which will
    // notify the parent (supervisor) and take that down too
    supervisor_ref.drain_children();

    // children were not unlinked by draining them (fixed from #310)
    assert!(!supervisor_ref.get_children().is_empty());

    // Wait for exit
    s_handle.await.unwrap();
    c_handle.await.unwrap();

    // Child's post-stop should have been called.
    assert_eq!(1, flag.load(Ordering::SeqCst));
}

#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
#[cfg(feature = "monitors")]
async fn test_simple_monitor() {
    struct Peer;
    struct Monitor {
        counter: Arc<AtomicU8>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Peer {
        type Msg = ();
        type State = ();
        type Arguments = ();

        async fn pre_start(
            &self,
            _: ActorRef<Self::Msg>,
            _: Self::Arguments,
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }

        async fn handle(
            &self,
            myself: ActorRef<Self::Msg>,
            _: Self::Msg,
            _: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            myself.stop(Some("oh no!".to_string()));
            Ok(())
        }
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for Monitor {
        type Msg = ();
        type State = ();
        type Arguments = ();

        async fn pre_start(
            &self,
            _: ActorRef<Self::Msg>,
            _: Self::Arguments,
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            _: ActorRef<Self::Msg>,
            evt: SupervisionEvent,
            _: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            if let SupervisionEvent::ActorTerminated(_who, _state, Some(msg)) = evt {
                if msg.as_str() == "oh no!" {
                    self.counter.fetch_add(1, Ordering::SeqCst);
                }
            }
            Ok(())
        }
    }

    let count = Arc::new(AtomicU8::new(0));

    let (p, ph) = Actor::spawn(None, Peer, ())
        .await
        .expect("Failed to start peer");
    let (m, mh) = Actor::spawn(
        None,
        Monitor {
            counter: count.clone(),
        },
        (),
    )
    .await
    .expect("Faield to start monitor");

    m.monitor(p.get_cell());

    // stopping the peer should notify the monitor, who can capture the state
    p.cast(()).expect("Failed to contact peer");
    periodic_check(|| count.load(Ordering::SeqCst) == 1, Duration::from_secs(1)).await;
    ph.await.unwrap();

    let (p, ph) = Actor::spawn(None, Peer, ())
        .await
        .expect("Failed to start peer");
    m.monitor(p.get_cell());
    m.unmonitor(p.get_cell());

    p.cast(()).expect("Failed to contact peer");
    ph.await.unwrap();

    // The count doesn't increment when the peer exits (we give some time
    // to schedule the supervision evt)
    crate::concurrency::sleep(Duration::from_millis(100)).await;
    assert_eq!(1, count.load(Ordering::SeqCst));

    m.stop(None);
    mh.await.unwrap();
}
