// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Supervisor tests

use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use once_cell::sync::OnceCell;

use crate::thread_local::ThreadLocalActor;
use crate::thread_local::ThreadLocalActorSpawner;
use crate::Actor;
use crate::ActorCell;
use crate::ActorProcessingErr;
use crate::ActorRef;
use crate::SupervisionEvent;

static LOCAL_SPAWNER: OnceCell<ThreadLocalActorSpawner> = OnceCell::new();

fn get_spawner() -> ThreadLocalActorSpawner {
    LOCAL_SPAWNER
        .get_or_init(ThreadLocalActorSpawner::new)
        .clone()
}

#[crate::concurrency::test]
#[tracing_test::traced_test]
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
async fn test_thread_local_child() {
    use crate::thread_local::ThreadLocalActor;

    #[derive(Default)]
    struct Child;

    struct Supervisor {
        flag: Arc<AtomicU64>,
    }

    impl ThreadLocalActor for Child {
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

    #[cfg_attr(
    all(
        feature = "async-trait",
        not(all(target_arch = "wasm32", target_os = "unknown"))
    ),
    crate::async_trait
)]
#[cfg_attr(
    all(
        feature = "async-trait",
       all(target_arch = "wasm32", target_os = "unknown")
    ),
    crate::async_trait(?Send)
)]
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
        .spawn_local_linked::<Child>(None, (), get_spawner())
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
#[tracing_test::traced_test]
async fn test_thread_local_supervisor() {
    struct Child;
    #[derive(Default)]
    struct Supervisor;

    #[cfg_attr(
    all(
        feature = "async-trait",
        not(all(target_arch = "wasm32", target_os = "unknown"))
    ),
    crate::async_trait
)]
#[cfg_attr(
    all(
        feature = "async-trait",
       all(target_arch = "wasm32", target_os = "unknown")
    ),
    crate::async_trait(?Send)
)]
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

    impl ThreadLocalActor for Supervisor {
        type Msg = ();
        type State = Arc<AtomicU64>;
        type Arguments = Arc<AtomicU64>;
        async fn pre_start(
            &self,
            _this_actor: ActorRef<Self::Msg>,
            args: Arc<AtomicU64>,
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(args)
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
                _state.store(dead_actor.get_id().pid(), Ordering::SeqCst);
                this_actor.stop(None);
            }
            Ok(())
        }
    }

    let flag = Arc::new(AtomicU64::new(0));

    let (supervisor_ref, s_handle) = Supervisor::spawn(None, flag.clone(), get_spawner())
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
#[tracing_test::traced_test]
async fn test_thread_local_child_panic_handle() {
    #[derive(Default)]
    struct Child;
    struct Supervisor {
        flag: Arc<AtomicU64>,
    }

    impl ThreadLocalActor for Child {
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

    #[cfg_attr(
    all(
        feature = "async-trait",
        not(all(target_arch = "wasm32", target_os = "unknown"))
    ),
    crate::async_trait
)]
#[cfg_attr(
    all(
        feature = "async-trait",
       all(target_arch = "wasm32", target_os = "unknown")
    ),
    crate::async_trait(?Send)
)]
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

    let (child_ref, c_handle) = supervisor_cell
        .spawn_local_linked::<Child>(None, (), get_spawner())
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
