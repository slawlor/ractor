// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use ::function_name::named;
use serial_test::serial;

use crate::common_test::periodic_check;
use crate::concurrency::Duration;
use crate::pg::{self};
use crate::Actor;
use crate::ActorProcessingErr;
use crate::GroupName;
use crate::ScopeName;
use crate::SupervisionEvent;

struct TestActor;

#[cfg_attr(feature = "async-trait", crate::async_trait)]
impl Actor for TestActor {
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

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_basic_group_in_default_scope() {
    let (actor, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn test actor");

    let group = function_name!().to_string();

    // join the group
    pg::join(group.clone(), vec![actor.clone().into()]);

    let members = pg::get_members(&group);
    assert_eq!(1, members.len());

    // Cleanup
    actor.stop(None);
    handle.await.expect("Actor cleanup failed");
}

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_basic_group_in_named_scope() {
    let (actor, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn test actor");

    let scope = function_name!().to_string();
    let group = function_name!().to_string();

    // join the group
    pg::join_scoped(scope.clone(), group.clone(), vec![actor.clone().into()]);

    let members = pg::get_scoped_members(&scope, &group);
    assert_eq!(1, members.len());

    // Cleanup
    actor.stop(None);
    handle.await.expect("Actor cleanup failed");
}

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_which_scopes_and_groups() {
    let (actor, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn test actor");

    let scope_a = concat!(function_name!(), "_a").to_string();
    let scope_b = concat!(function_name!(), "_b").to_string();
    let group_a = concat!(function_name!(), "_a").to_string();
    let group_b = concat!(function_name!(), "_b").to_string();

    // join all scopes twice with each group
    let scope_group = [
        (scope_a.clone(), group_a.clone()),
        (scope_a.clone(), group_b.clone()),
        (scope_b.clone(), group_a.clone()),
        (scope_b.clone(), group_b.clone()),
    ];

    for (scope, group) in scope_group.iter() {
        pg::join_scoped(scope.clone(), group.clone(), vec![actor.clone().into()]);
        pg::join_scoped(scope.clone(), group.clone(), vec![actor.clone().into()]);
    }

    let scopes_and_groups = pg::which_scopes_and_groups();
    // println!("Scopes and groups are: {:#?}", scopes_and_groups);
    assert_eq!(
        4,
        scopes_and_groups.len(),
        "expected 4 scopes but got {:?}",
        scopes_and_groups
    );

    // Cleanup
    actor.stop(None);
    handle.await.expect("Actor cleanup failed");

    let scopes_and_groups = pg::which_scopes_and_groups();
    assert!(scopes_and_groups.is_empty());
}

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_multiple_members_in_group() {
    let group = function_name!().to_string();

    let mut actors = vec![];
    let mut handles = vec![];
    for _ in 0..10 {
        let (actor, handle) = Actor::spawn(None, TestActor, ())
            .await
            .expect("Failed to spawn test actor");
        actors.push(actor);
        handles.push(handle);
    }

    // join the group
    pg::join(
        group.clone(),
        actors
            .iter()
            .map(|aref| aref.clone().get_cell())
            .collect::<Vec<_>>(),
    );

    let members = pg::get_members(&group);
    assert_eq!(10, members.len());

    // Cleanup
    for actor in actors {
        actor.stop(None);
    }
    for handle in handles.into_iter() {
        handle.await.expect("Actor cleanup failed");
    }
}

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_multiple_members_in_scoped_group() {
    let scope = function_name!().to_string();
    let group = function_name!().to_string();

    let mut actors = vec![];
    let mut handles = vec![];
    for _ in 0..10 {
        let (actor, handle) = Actor::spawn(None, TestActor, ())
            .await
            .expect("Failed to spawn test actor");
        actors.push(actor);
        handles.push(handle);
    }

    // join the group
    pg::join_scoped(
        scope.clone(),
        group.clone(),
        actors
            .iter()
            .map(|aref| aref.clone().get_cell())
            .collect::<Vec<_>>(),
    );

    let members = pg::get_scoped_members(&scope, &group);
    assert_eq!(10, members.len());

    // Cleanup
    for actor in actors {
        actor.stop(None);
    }
    for handle in handles.into_iter() {
        handle.await.expect("Actor cleanup failed");
    }
}

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_which_scoped_groups() {
    let scope = function_name!().to_string();
    let group = function_name!().to_string();

    let mut actors = vec![];
    let mut handles = vec![];
    for _ in 0..10 {
        let (actor, handle) = Actor::spawn(None, TestActor, ())
            .await
            .expect("Failed to spawn test actor");
        actors.push(actor);
        handles.push(handle);
    }

    // join the group
    pg::join_scoped(
        scope.clone(),
        group.clone(),
        actors
            .iter()
            .map(|aref| aref.clone().get_cell())
            .collect::<Vec<_>>(),
    );

    let groups_in_scope = pg::which_scoped_groups(&scope);
    assert_eq!(vec![group.clone()], groups_in_scope);

    // Cleanup
    for actor in actors {
        actor.stop(None);
    }
    for handle in handles.into_iter() {
        handle.await.expect("Actor cleanup failed");
    }
}

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_multiple_groups() {
    let group_a = concat!(function_name!(), "_a").to_string();
    let group_b = concat!(function_name!(), "_b").to_string();

    let mut actors = vec![];
    let mut handles = vec![];
    for _ in 0..10 {
        let (actor, handle) = Actor::spawn(None, TestActor, ())
            .await
            .expect("Failed to spawn test actor");
        actors.push(actor);
        handles.push(handle);
    }

    // setup group_a and group_b
    let these_actors = actors[0..5]
        .iter()
        .map(|a| a.clone().get_cell())
        .collect::<Vec<_>>();
    pg::join(group_a.clone(), these_actors);

    let these_actors = actors[5..10]
        .iter()
        .map(|a| a.clone().get_cell())
        .collect::<Vec<_>>();
    pg::join(group_b.clone(), these_actors);

    let members = pg::get_members(&group_a);
    assert_eq!(5, members.len());

    let members = pg::get_members(&group_b);
    assert_eq!(5, members.len());

    // Cleanup
    for actor in actors {
        actor.stop(None);
    }
    for handle in handles.into_iter() {
        handle.await.expect("Actor cleanup failed");
    }
}

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_multiple_groups_in_multiple_scopes() {
    let scope_a = concat!(function_name!(), "_b").to_string();
    let scope_b = concat!(function_name!(), "_b").to_string();

    let group_a = concat!(function_name!(), "_a").to_string();
    let group_b = concat!(function_name!(), "_b").to_string();

    let mut actors = vec![];
    let mut handles = vec![];
    for _ in 0..10 {
        let (actor, handle) = Actor::spawn(None, TestActor, ())
            .await
            .expect("Failed to spawn test actor");
        actors.push(actor);
        handles.push(handle);
    }

    // setup scope_a and scope_b, and group_a and group_b
    let these_actors = actors[0..5]
        .iter()
        .map(|a| a.clone().get_cell())
        .collect::<Vec<_>>();
    pg::join_scoped(scope_a.clone(), group_a.clone(), these_actors);

    let these_actors = actors[5..10]
        .iter()
        .map(|a| a.clone().get_cell())
        .collect::<Vec<_>>();
    pg::join_scoped(scope_a.clone(), group_b.clone(), these_actors);

    let these_actors = actors[0..5]
        .iter()
        .map(|a| a.clone().get_cell())
        .collect::<Vec<_>>();
    pg::join_scoped(scope_b.clone(), group_a.clone(), these_actors);

    let these_actors = actors[5..10]
        .iter()
        .map(|a| a.clone().get_cell())
        .collect::<Vec<_>>();
    pg::join_scoped(scope_b.clone(), group_b.clone(), these_actors);

    let members = pg::get_scoped_members(&scope_a, &group_a);
    assert_eq!(5, members.len());

    let members = pg::get_scoped_members(&scope_a, &group_b);
    assert_eq!(5, members.len());

    let members = pg::get_scoped_members(&scope_b, &group_a);
    assert_eq!(5, members.len());

    let members = pg::get_scoped_members(&scope_b, &group_b);
    assert_eq!(5, members.len());

    // Cleanup
    for actor in actors {
        actor.stop(None);
    }
    for handle in handles.into_iter() {
        handle.await.expect("Actor cleanup failed");
    }
}

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_actor_leaves_pg_group_on_shutdown() {
    let (actor, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn test actor");

    let group = function_name!().to_string();

    // join the group
    pg::join(group.clone(), vec![actor.clone().into()]);

    let members = pg::get_members(&group);
    assert_eq!(1, members.len());

    // Cleanup
    actor.stop(None);
    handle.await.expect("Actor cleanup failed");
    drop(actor);

    let members = pg::get_members(&group);
    assert_eq!(0, members.len());
}

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_actor_leaves_scope_on_shupdown() {
    let (actor, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn test actor");

    let scope = function_name!().to_string();
    let group = function_name!().to_string();

    // join the scope and group
    pg::join_scoped(scope.clone(), group.clone(), vec![actor.clone().into()]);

    let members = pg::get_scoped_members(&scope, &group);
    assert_eq!(1, members.len());

    // Cleanup
    actor.stop(None);
    handle.await.expect("Actor cleanup failed");
    drop(actor);

    let members = pg::get_scoped_members(&scope, &group);
    assert_eq!(0, members.len());
}

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_actor_leaves_pg_group_manually() {
    let group = function_name!().to_string();

    let (actor, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn test actor");

    // join the group (create on first use)
    pg::join(group.clone(), vec![actor.clone().into()]);

    // the group was created and is present
    let groups = pg::which_groups();
    assert!(groups.contains(&group));

    let members = pg::get_members(&group);
    assert_eq!(1, members.len());

    // leave the group
    pg::leave(group.clone(), vec![actor.clone().into()]);

    // pif-paf-poof the group is gone!
    let groups = pg::which_groups();
    assert!(!groups.contains(&group));

    // pif-paf-poof the group is gone from the monitor's index!
    let scoped_groups = pg::which_scoped_groups(&group);
    assert!(!scoped_groups.contains(&group));

    // members comes back empty
    let members = pg::get_members(&group);
    assert_eq!(0, members.len());

    // Cleanup
    actor.stop(None);
    handle.await.expect("Actor cleanup failed");
}

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_actor_leaves_scope_manually() {
    let scope = function_name!().to_string();
    let group = function_name!().to_string();

    let (actor, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn test actor");

    // join the group in scope (create on first use)
    pg::join_scoped(scope.clone(), group.clone(), vec![actor.clone().into()]);

    // the scope was created and is present
    let scopes = pg::which_scopes();
    assert!(scopes.contains(&scope));

    // the group was created and is present
    let groups = pg::which_groups();
    assert!(groups.contains(&group));

    let members = pg::get_scoped_members(&scope, &group);
    assert_eq!(1, members.len());

    // leave the group
    pg::leave_scoped(scope.clone(), group.clone(), vec![actor.clone().into()]);

    // pif-paf-poof the scope is gone!
    let scopes = pg::which_scopes();
    assert!(!scopes.contains(&scope));

    // pif-paf-poof the group is gone!
    let groups = pg::which_groups();
    assert!(!groups.contains(&group));

    // pif-paf-poof the group is gone from the monitor's index!
    let scoped_groups = pg::which_scoped_groups(&group);
    assert!(!scoped_groups.contains(&group));

    // members comes back empty
    let members = pg::get_scoped_members(&scope, &group);
    assert_eq!(0, members.len());

    // Cleanup
    actor.stop(None);
    handle.await.expect("Actor cleanup failed");
}

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_pg_monitoring() {
    let group = function_name!().to_string();

    let counter = Arc::new(AtomicU8::new(0u8));

    struct AutoJoinActor {
        pg_group: GroupName,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for AutoJoinActor {
        type Msg = ();
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            myself: crate::ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            pg::join(self.pg_group.clone(), vec![myself.into()]);
            Ok(())
        }
    }

    struct NotificationMonitor {
        pg_group: GroupName,
        counter: Arc<AtomicU8>,
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
            pg::monitor(self.pg_group.clone(), myself.into());
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            _myself: crate::ActorRef<Self::Msg>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            if let SupervisionEvent::ProcessGroupChanged(change) = message {
                match change {
                    pg::GroupChangeMessage::Join(_scope, _which, who) => {
                        self.counter.fetch_add(who.len() as u8, Ordering::Relaxed);
                    }
                    pg::GroupChangeMessage::Leave(_scope, _which, who) => {
                        self.counter.fetch_sub(who.len() as u8, Ordering::Relaxed);
                    }
                }
            }
            Ok(())
        }
    }
    let (monitor_actor, monitor_handle) = Actor::spawn(
        None,
        NotificationMonitor {
            pg_group: group.clone(),
            counter: counter.clone(),
        },
        (),
    )
    .await
    .expect("Failed to start monitor actor");

    // this actor's startup should notify the "monitor" for PG changes
    let (test_actor, test_handle) = Actor::spawn(None, AutoJoinActor { pg_group: group }, ())
        .await
        .expect("Failed to start test actor");

    // the monitor is notified async, so we need to wait a bit
    periodic_check(
        || counter.load(Ordering::Relaxed) == 1,
        Duration::from_secs(5),
    )
    .await;

    // kill the pg member
    println!("close pg test actor");
    test_actor.stop(None);
    test_handle.await.expect("Actor cleanup failed");
    // it should have notified that it's unsubscribed
    periodic_check(
        || counter.load(Ordering::Relaxed) == 0,
        Duration::from_secs(5),
    )
    .await;

    // cleanup
    println!("close pg monitor");
    monitor_actor.stop(None);
    monitor_handle.await.expect("Actor cleanup failed");
}

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_scope_monitoring() {
    let scope = function_name!().to_string();
    let group = function_name!().to_string();

    let counter = Arc::new(AtomicU8::new(0u8));

    struct AutoJoinActor {
        scope: ScopeName,
        pg_group: GroupName,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for AutoJoinActor {
        type Msg = ();
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            myself: crate::ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            pg::join_scoped(
                self.scope.clone(),
                self.pg_group.clone(),
                vec![myself.into()],
            );
            Ok(())
        }
    }

    struct NotificationMonitor {
        scope: ScopeName,
        counter: Arc<AtomicU8>,
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
            pg::monitor_scope(self.scope.clone(), myself.into());
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            _myself: crate::ActorRef<Self::Msg>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            if let SupervisionEvent::ProcessGroupChanged(change) = message {
                match change {
                    pg::GroupChangeMessage::Join(scope_name, _which, who) => {
                        // ensure this test can run concurrently to others
                        if scope_name == function_name!() {
                            self.counter.fetch_add(who.len() as u8, Ordering::Relaxed);
                        }
                    }
                    pg::GroupChangeMessage::Leave(scope_name, _which, who) => {
                        // ensure this test can run concurrently to others
                        if scope_name == function_name!() {
                            self.counter.fetch_sub(who.len() as u8, Ordering::Relaxed);
                        }
                    }
                }
            }
            Ok(())
        }
    }

    let (monitor_actor, monitor_handle) = Actor::spawn(
        None,
        NotificationMonitor {
            scope: scope.clone(),
            counter: counter.clone(),
        },
        (),
    )
    .await
    .expect("Failed to start monitor actor");

    // this actor's startup should notify the "monitor" for scope changes
    let (test_actor, test_handle) = Actor::spawn(
        None,
        AutoJoinActor {
            scope: scope.clone(),
            pg_group: group.clone(),
        },
        (),
    )
    .await
    .expect("Failed to start test actor");

    // start a second actor in the same scope to test if we multiply messages exponentially
    let (test_actor1, test_handle1) = Actor::spawn(
        None,
        AutoJoinActor {
            scope: scope.clone(),
            pg_group: group.clone(),
        },
        (),
    )
    .await
    .expect("Failed to start test actor");

    // the monitor is notified async, so we need to wait a bit
    periodic_check(
        || counter.load(Ordering::Relaxed) == 2,
        Duration::from_secs(5),
    )
    .await;

    // kill the scope members
    test_actor.stop(None);
    test_handle.await.expect("Actor cleanup failed");
    test_actor1.stop(None);
    test_handle1.await.expect("Actor cleanup failed");

    // it should have notified that it's unsubscribed
    periodic_check(
        || counter.load(Ordering::Relaxed) == 0,
        Duration::from_secs(5),
    )
    .await;

    // cleanup
    monitor_actor.stop(None);
    monitor_handle.await.expect("Actor cleanup failed");
}

#[named]
#[cfg(feature = "cluster")]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn local_vs_remote_pg_members() {
    use crate::ActorRuntime;

    let group = function_name!().to_string();

    struct TestRemoteActor;
    struct TestRemoteActorMessage;
    impl crate::Message for TestRemoteActorMessage {}
    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for TestRemoteActor {
        type Msg = TestRemoteActorMessage;
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

    let remote_pid = crate::ActorId::Remote { node_id: 1, pid: 1 };

    let mut actors: Vec<crate::ActorCell> = vec![];
    let mut handles = vec![];
    for _ in 0..10 {
        let (actor, handle) = Actor::spawn(None, TestActor, ())
            .await
            .expect("Failed to spawn test actor");
        actors.push(actor.into());
        handles.push(handle);
    }
    let (actor, handle) = ActorRuntime::spawn_linked_remote(
        None,
        TestRemoteActor,
        remote_pid,
        (),
        actors.first().unwrap().clone(),
    )
    .await
    .expect("Failed to spawn remote actor");
    println!("Spawned {}", actor.get_id());

    actors.push(actor.into());
    handles.push(handle);

    // join the group
    pg::join(group.clone(), actors.to_vec());

    // assert
    let members = pg::get_local_members(&group);
    assert_eq!(10, members.len());

    let members = pg::get_members(&group);
    assert_eq!(11, members.len());

    // Cleanup
    for actor in actors {
        actor.stop(None);
    }
    for handle in handles.into_iter() {
        handle.await.expect("Actor cleanup failed");
    }
}

#[named]
#[cfg(feature = "cluster")]
#[crate::concurrency::test]
#[serial]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn local_vs_remote_pg_members_in_named_scopes() {
    use crate::ActorRuntime;

    let scope = function_name!().to_string();
    let group = function_name!().to_string();

    struct TestRemoteActor;
    struct TestRemoteActorMessage;
    impl crate::Message for TestRemoteActorMessage {}
    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for TestRemoteActor {
        type Msg = TestRemoteActorMessage;
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

    let remote_pid = crate::ActorId::Remote { node_id: 1, pid: 1 };

    let mut actors: Vec<crate::ActorCell> = vec![];
    let mut handles = vec![];
    for _ in 0..10 {
        let (actor, handle) = Actor::spawn(None, TestActor, ())
            .await
            .expect("Failed to spawn test actor");
        actors.push(actor.into());
        handles.push(handle);
    }
    let (actor, handle) = ActorRuntime::spawn_linked_remote(
        None,
        TestRemoteActor,
        remote_pid,
        (),
        actors.first().unwrap().clone(),
    )
    .await
    .expect("Failed to spawn remote actor");
    println!("Spawned {}", actor.get_id());

    actors.push(actor.into());
    handles.push(handle);

    // join the group in scope
    pg::join_scoped(scope.clone(), group.clone(), actors.to_vec());

    // assert
    let members = pg::get_scoped_local_members(&scope, &group);
    assert_eq!(10, members.len());

    let members = pg::get_scoped_members(&scope, &group);
    assert_eq!(11, members.len());

    // Cleanup
    for actor in actors {
        actor.stop(None);
    }
    for handle in handles.into_iter() {
        handle.await.expect("Actor cleanup failed");
    }
}

// ============================================================================
// Tests for special constants (ALL_SCOPES_NOTIFICATION, ALL_GROUPS_NOTIFICATION)
// ============================================================================

#[named]
#[serial]
#[crate::concurrency::test]
#[serial]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_monitor_all_scopes_notification() {
    let scope1 = format!("{}_scope1", function_name!());
    let scope2 = format!("{}_scope2", function_name!());
    let group = format!("{}_group", function_name!());

    let counter = Arc::new(AtomicU8::new(0u8));

    struct AllScopesMonitor {
        group: GroupName,
        counter: Arc<AtomicU8>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for AllScopesMonitor {
        type Msg = ();
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            myself: crate::ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            // Monitor the group across ALL existing scopes
            pg::monitor_scoped(pg::ALL_SCOPES_NOTIFICATION, &self.group, myself.into());
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            _myself: crate::ActorRef<Self::Msg>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            if let SupervisionEvent::ProcessGroupChanged(change) = message {
                match change {
                    pg::GroupChangeMessage::Join(_, _, who) => {
                        self.counter.fetch_add(who.len() as u8, Ordering::Relaxed);
                    }
                    pg::GroupChangeMessage::Leave(_, _, who) => {
                        self.counter.fetch_sub(who.len() as u8, Ordering::Relaxed);
                    }
                }
            }
            Ok(())
        }
    }

    // Create some groups in different scopes first
    let (actor1, handle1) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");
    let (actor2, handle2) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");

    pg::join_scoped(&scope1, &group, vec![actor1.clone().into()]);
    pg::join_scoped(&scope2, &group, vec![actor2.clone().into()]);

    // Now start the monitor (should monitor existing groups)
    let (monitor_actor, monitor_handle) = Actor::spawn(
        None,
        AllScopesMonitor {
            group: group.clone(),
            counter: counter.clone(),
        },
        (),
    )
    .await
    .expect("Failed to start monitor");

    // Add another actor to one of the existing scopes
    let (actor3, handle3) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");
    pg::join_scoped(&scope1, &group, vec![actor3.clone().into()]);

    // Should have received 1 notification (for actor3 joining)
    periodic_check(
        || counter.load(Ordering::Relaxed) == 1,
        Duration::from_secs(5),
    )
    .await;

    // Cleanup
    actor1.stop(None);
    actor2.stop(None);
    actor3.stop(None);
    monitor_actor.stop(None);

    handle1.await.expect("Actor cleanup failed");
    handle2.await.expect("Actor cleanup failed");
    handle3.await.expect("Actor cleanup failed");
    monitor_handle.await.expect("Monitor cleanup failed");
}

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_monitor_all_groups_notification() {
    let scope = format!("{}_scope", function_name!());
    let group1 = format!("{}_group1", function_name!());
    let group2 = format!("{}_group2", function_name!());

    let counter = Arc::new(AtomicU8::new(0u8));

    struct AllGroupsMonitor {
        scope: ScopeName,
        counter: Arc<AtomicU8>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for AllGroupsMonitor {
        type Msg = ();
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            myself: crate::ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            // Monitor ALL existing groups in the scope
            pg::monitor_scoped(&self.scope, pg::ALL_GROUPS_NOTIFICATION, myself.into());
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            _myself: crate::ActorRef<Self::Msg>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            if let SupervisionEvent::ProcessGroupChanged(change) = message {
                match change {
                    pg::GroupChangeMessage::Join(_, _, who) => {
                        self.counter.fetch_add(who.len() as u8, Ordering::Relaxed);
                    }
                    pg::GroupChangeMessage::Leave(_, _, who) => {
                        self.counter.fetch_sub(who.len() as u8, Ordering::Relaxed);
                    }
                }
            }
            Ok(())
        }
    }

    // Create some groups in the scope first
    let (actor1, handle1) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");
    let (actor2, handle2) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");

    pg::join_scoped(&scope, &group1, vec![actor1.clone().into()]);
    pg::join_scoped(&scope, &group2, vec![actor2.clone().into()]);

    // Now start the monitor (should monitor existing groups)
    let (monitor_actor, monitor_handle) = Actor::spawn(
        None,
        AllGroupsMonitor {
            scope: scope.clone(),
            counter: counter.clone(),
        },
        (),
    )
    .await
    .expect("Failed to start monitor");

    // Add another actor to one of the existing groups
    let (actor3, handle3) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");
    pg::join_scoped(&scope, &group1, vec![actor3.clone().into()]);

    // Should have received 1 notification (for actor3 joining group1)
    periodic_check(
        || counter.load(Ordering::Relaxed) == 1,
        Duration::from_secs(5),
    )
    .await;

    // Cleanup
    actor1.stop(None);
    actor2.stop(None);
    actor3.stop(None);
    monitor_actor.stop(None);

    handle1.await.expect("Actor cleanup failed");
    handle2.await.expect("Actor cleanup failed");
    handle3.await.expect("Actor cleanup failed");
    monitor_handle.await.expect("Monitor cleanup failed");
}

// ============================================================================
// Tests for monitor_world and world monitoring functionality
// ============================================================================

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_monitor_world() {
    let scope1 = format!("{}_scope1", function_name!());
    let scope2 = format!("{}_scope2", function_name!());
    let group1 = format!("{}_group1", function_name!());
    let group2 = format!("{}_group2", function_name!());

    let counter = Arc::new(AtomicU8::new(0u8));

    struct WorldMonitor {
        counter: Arc<AtomicU8>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for WorldMonitor {
        type Msg = ();
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            myself: crate::ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            // Monitor ALL changes across ALL scopes and groups
            pg::monitor_world(&myself.into());
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            _myself: crate::ActorRef<Self::Msg>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            if let SupervisionEvent::ProcessGroupChanged(change) = message {
                match change {
                    pg::GroupChangeMessage::Join(_, _, who) => {
                        self.counter.fetch_add(who.len() as u8, Ordering::Relaxed);
                    }
                    pg::GroupChangeMessage::Leave(_, _, who) => {
                        self.counter.fetch_sub(who.len() as u8, Ordering::Relaxed);
                    }
                }
            }
            Ok(())
        }
    }

    // Start the world monitor
    let (monitor_actor, monitor_handle) = Actor::spawn(
        None,
        WorldMonitor {
            counter: counter.clone(),
        },
        (),
    )
    .await
    .expect("Failed to start monitor");

    // Create actors and join different groups in different scopes
    let (actor1, handle1) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");
    let (actor2, handle2) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");
    let (actor3, handle3) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");

    pg::join_scoped(&scope1, &group1, vec![actor1.clone().into()]);
    pg::join_scoped(&scope2, &group2, vec![actor2.clone().into()]);
    pg::join(&group1, vec![actor3.clone().into()]); // default scope

    // Should receive 3 notifications (all joins)
    periodic_check(
        || counter.load(Ordering::Relaxed) == 3,
        Duration::from_secs(5),
    )
    .await;

    // Test demonitor_world
    pg::demonitor_world(&monitor_actor.get_cell());

    // After demonitoring, no new notifications should be received
    let (actor4, handle4) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");
    pg::join_scoped(&scope1, &group2, vec![actor4.clone().into()]);

    // Wait a bit to ensure no new notifications
    crate::concurrency::sleep(Duration::from_millis(100)).await;
    assert_eq!(counter.load(Ordering::Relaxed), 3); // Should still be 3

    // Cleanup
    actor1.stop(None);
    actor2.stop(None);
    actor3.stop(None);
    actor4.stop(None);
    monitor_actor.stop(None);

    handle1.await.expect("Actor cleanup failed");
    handle2.await.expect("Actor cleanup failed");
    handle3.await.expect("Actor cleanup failed");
    handle4.await.expect("Actor cleanup failed");
    monitor_handle.await.expect("Monitor cleanup failed");
}

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_monitor_scope_with_future_groups() {
    let scope = format!("{}_scope", function_name!());
    let group1 = format!("{}_group1", function_name!());
    let group2 = format!("{}_group2", function_name!());

    let counter = Arc::new(AtomicU8::new(0u8));

    struct ScopeMonitor {
        scope: ScopeName,
        counter: Arc<AtomicU8>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for ScopeMonitor {
        type Msg = ();
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            myself: crate::ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            // Monitor the scope for ALL group changes (current and future)
            pg::monitor_scope(&self.scope, myself.into());
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            _myself: crate::ActorRef<Self::Msg>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            if let SupervisionEvent::ProcessGroupChanged(change) = message {
                match change {
                    pg::GroupChangeMessage::Join(_, _, who) => {
                        self.counter.fetch_add(who.len() as u8, Ordering::Relaxed);
                    }
                    pg::GroupChangeMessage::Leave(_, _, who) => {
                        self.counter.fetch_sub(who.len() as u8, Ordering::Relaxed);
                    }
                }
            }
            Ok(())
        }
    }

    // Start the scope monitor BEFORE creating any groups
    let (monitor_actor, monitor_handle) = Actor::spawn(
        None,
        ScopeMonitor {
            scope: scope.clone(),
            counter: counter.clone(),
        },
        (),
    )
    .await
    .expect("Failed to start monitor");

    // Create actors and join groups (should be detected by scope monitor)
    let (actor1, handle1) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");
    let (actor2, handle2) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");

    pg::join_scoped(&scope, &group1, vec![actor1.clone().into()]);
    pg::join_scoped(&scope, &group2, vec![actor2.clone().into()]);

    // Should receive 2 notifications (both joins detected)
    periodic_check(
        || counter.load(Ordering::Relaxed) == 2,
        Duration::from_secs(5),
    )
    .await;

    // Test demonitor_scope
    pg::demonitor_scope(&scope, monitor_actor.get_id());

    // After demonitoring, no new notifications should be received
    let (actor3, handle3) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");
    pg::join_scoped(&scope, &group1, vec![actor3.clone().into()]);

    // Wait a bit to ensure no new notifications
    crate::concurrency::sleep(Duration::from_millis(100)).await;
    assert_eq!(counter.load(Ordering::Relaxed), 2); // Should still be 2

    // Cleanup
    actor1.stop(None);
    actor2.stop(None);
    actor3.stop(None);
    monitor_actor.stop(None);

    handle1.await.expect("Actor cleanup failed");
    handle2.await.expect("Actor cleanup failed");
    handle3.await.expect("Actor cleanup failed");
    monitor_handle.await.expect("Monitor cleanup failed");
}

// ============================================================================
// Tests for edge cases and error scenarios
// ============================================================================

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_join_leave_nonexistent_groups() {
    let scope = format!("{}_scope", function_name!());
    let group = format!("{}_group", function_name!());

    let (actor, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");

    // Test getting members from nonexistent group/scope
    let members = pg::get_members(&group);
    assert_eq!(members.len(), 0);

    let scoped_members = pg::get_scoped_members(&scope, &group);
    assert_eq!(scoped_members.len(), 0);

    let local_members = pg::get_local_members(&group);
    assert_eq!(local_members.len(), 0);

    let scoped_local_members = pg::get_scoped_local_members(&scope, &group);
    assert_eq!(scoped_local_members.len(), 0);

    // Test leaving from nonexistent group (should not panic)
    pg::leave(&group, vec![actor.clone().into()]);
    pg::leave_scoped(&scope, &group, vec![actor.clone().into()]);

    // Test monitoring nonexistent group (should create the group structure)
    pg::monitor(&group, actor.get_cell());
    pg::monitor_scoped(&scope, &group, actor.get_cell());

    // Test demonitoring nonexistent group (should not panic)
    pg::demonitor(&group, actor.get_id());
    pg::demonitor_scoped(&scope, &group, actor.get_id());

    // Cleanup
    actor.stop(None);
    handle.await.expect("Actor cleanup failed");
}

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_double_join_and_leave() {
    let group = format!("{}_group", function_name!());

    let (actor, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");

    // Join the same actor twice (should only be in group once)
    pg::join(&group, vec![actor.clone().into()]);
    pg::join(&group, vec![actor.clone().into()]);

    let members = pg::get_members(&group);
    assert_eq!(members.len(), 1);

    // Leave once (should remove the actor)
    pg::leave(&group, vec![actor.clone().into()]);

    let members = pg::get_members(&group);
    assert_eq!(members.len(), 0);

    // Leave again (should not panic)
    pg::leave(&group, vec![actor.clone().into()]);

    let members = pg::get_members(&group);
    assert_eq!(members.len(), 0);

    // Cleanup
    actor.stop(None);
    handle.await.expect("Actor cleanup failed");
}

// ============================================================================
// Tests for utility structures (ScopeGroupKey, GroupChangeMessage)
// ============================================================================

#[serial]
#[crate::concurrency::test]
async fn test_scope_group_key_methods() {
    let scope = "test_scope".to_string();
    let group = "test_group".to_string();

    let key = pg::ScopeGroupKey::new(scope.clone(), group.clone());

    assert_eq!(key.get_scope(), scope);
    assert_eq!(key.get_group(), group);

    // Test Clone, PartialEq, etc.
    let key2 = key.clone();
    assert_eq!(key, key2);

    let key3 = pg::ScopeGroupKey::new("other_scope".to_string(), group.clone());
    assert_ne!(key, key3);
}

#[serial]
#[crate::concurrency::test]
async fn test_group_change_message_methods() {
    let scope = "test_scope".to_string();
    let group = "test_group".to_string();
    let actors = vec![];

    let join_msg = pg::GroupChangeMessage::Join(scope.clone(), group.clone(), actors.clone());
    assert_eq!(join_msg.get_scope(), scope);
    assert_eq!(join_msg.get_group(), group);

    let leave_msg = pg::GroupChangeMessage::Leave(scope.clone(), group.clone(), actors);
    assert_eq!(leave_msg.get_scope(), scope);
    assert_eq!(leave_msg.get_group(), group);

    // Test clone
    let join_msg2 = join_msg.clone();
    assert_eq!(join_msg.get_scope(), join_msg2.get_scope());
    assert_eq!(join_msg.get_group(), join_msg2.get_group());
}

// ============================================================================
// Tests for get_local_members vs get_members differences
// ============================================================================

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_get_local_vs_all_members() {
    let group = format!("{}_group", function_name!());
    let scope = format!("{}_scope", function_name!());

    let (actor1, handle1) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");
    let (actor2, handle2) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");

    // Join actors to groups
    pg::join(&group, vec![actor1.clone().into(), actor2.clone().into()]);
    pg::join_scoped(
        &scope,
        &group,
        vec![actor1.clone().into(), actor2.clone().into()],
    );

    // Test default scope
    let all_members = pg::get_members(&group);
    let local_members = pg::get_local_members(&group);

    assert_eq!(all_members.len(), 2);
    assert_eq!(local_members.len(), 2); // All actors are local in this test

    // Test scoped version
    let all_scoped_members = pg::get_scoped_members(&scope, &group);
    let local_scoped_members = pg::get_scoped_local_members(&scope, &group);

    assert_eq!(all_scoped_members.len(), 2);
    assert_eq!(local_scoped_members.len(), 2); // All actors are local in this test

    // Cleanup
    actor1.stop(None);
    actor2.stop(None);
    handle1.await.expect("Actor cleanup failed");
    handle2.await.expect("Actor cleanup failed");
}

// ============================================================================
// Tests for empty groups and scopes cleanup
// ============================================================================

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_empty_groups_and_scopes_cleanup() {
    let scope = format!("{}_scope", function_name!());
    let group = format!("{}_group", function_name!());

    let (actor, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");

    // Join then leave to trigger cleanup
    pg::join_scoped(&scope, &group, vec![actor.clone().into()]);

    // Verify the group and scope exist
    let scopes = pg::which_scopes();
    assert!(scopes.contains(&scope));

    let groups = pg::which_scoped_groups(&scope);
    assert!(groups.contains(&group));

    // Leave the group (should trigger cleanup)
    pg::leave_scoped(&scope, &group, vec![actor.clone().into()]);

    // Groups and scopes should be cleaned up when empty
    let groups_after = pg::which_scoped_groups(&scope);
    assert_eq!(groups_after.len(), 0);

    // Cleanup
    actor.stop(None);
    handle.await.expect("Actor cleanup failed");
}

// ============================================================================
// Tests for empty inputs and edge cases
// ============================================================================

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_empty_inputs_and_edge_cases() {
    let group = format!("{}_group", function_name!());
    let scope = format!("{}_scope", function_name!());

    // Test join with empty actor list (should not panic)
    pg::join(&group, vec![]);
    pg::join_scoped(&scope, &group, vec![]);

    // Test leave with empty actor list (should not panic)
    pg::leave(&group, vec![]);
    pg::leave_scoped(&scope, &group, vec![]);

    // Verify groups remain empty
    assert_eq!(pg::get_members(&group).len(), 0);
    assert_eq!(pg::get_scoped_members(&scope, &group).len(), 0);

    // Test with empty string names (should work but not recommended)
    let (actor, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");

    pg::join_scoped("", "", vec![actor.clone().into()]);
    let empty_scope_members = pg::get_scoped_members("", "");
    assert_eq!(empty_scope_members.len(), 1);

    // Cleanup
    actor.stop(None);
    handle.await.expect("Actor cleanup failed");
}

// ============================================================================
// Tests for demonitor functions
// ============================================================================

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_demonitor_functions() {
    let scope = format!("{}_scope", function_name!());
    let group = format!("{}_group", function_name!());
    let counter = Arc::new(AtomicU8::new(0u8));

    struct TestMonitor {
        scope: ScopeName,
        group: GroupName,
        counter: Arc<AtomicU8>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for TestMonitor {
        type Msg = ();
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            myself: crate::ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            let cell: crate::ActorCell = myself.into();
            // Set up multiple monitoring subscriptions
            pg::monitor(&self.group, cell.clone()); // default scope
            pg::monitor_scoped(&self.scope, &self.group, cell.clone());
            pg::monitor_scope(&self.scope, cell.clone());
            pg::monitor_world(&cell);
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            _myself: crate::ActorRef<Self::Msg>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            if let SupervisionEvent::ProcessGroupChanged(_) = message {
                self.counter.fetch_add(1, Ordering::Relaxed);
            }
            Ok(())
        }
    }

    // Start monitor with multiple subscriptions
    let (monitor_actor, monitor_handle) = Actor::spawn(
        None,
        TestMonitor {
            scope: scope.clone(),
            group: group.clone(),
            counter: counter.clone(),
        },
        (),
    )
    .await
    .expect("Failed to start monitor");

    // Create an actor and trigger notifications
    let (actor, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");
    pg::join_scoped(&scope, &group, vec![actor.clone().into()]);

    // Wait for notifications (should receive multiple due to multiple subscriptions)
    periodic_check(
        || counter.load(Ordering::Relaxed) > 0,
        Duration::from_secs(5),
    )
    .await;

    // Test individual demonitor functions
    pg::demonitor(&group, monitor_actor.get_id()); // Remove default scope monitoring
    pg::demonitor_scoped(&scope, &group, monitor_actor.get_id()); // Remove scoped group monitoring

    // Reset counter
    counter.store(0, Ordering::Relaxed);

    // Add another actor (should still get notifications from scope and world monitoring)
    let (actor2, handle2) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");
    pg::join_scoped(&scope, &group, vec![actor2.clone().into()]);

    // Wait for notifications
    crate::concurrency::sleep(Duration::from_millis(100)).await;
    let notifications_after_partial = counter.load(Ordering::Relaxed);
    assert!(notifications_after_partial > 0); // Should still get some notifications

    // Now demonitor scope and world
    pg::demonitor_scope(&scope, monitor_actor.get_id());
    pg::demonitor_world(&monitor_actor.get_cell());

    // Reset counter
    counter.store(0, Ordering::Relaxed);

    // Add another actor (should get NO notifications now)
    let (actor3, handle3) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");
    pg::join_scoped(&scope, &group, vec![actor3.clone().into()]);

    // Wait a bit and ensure no notifications
    crate::concurrency::sleep(Duration::from_millis(100)).await;
    assert_eq!(counter.load(Ordering::Relaxed), 0);

    // Cleanup
    actor.stop(None);
    actor2.stop(None);
    actor3.stop(None);
    monitor_actor.stop(None);

    handle.await.expect("Actor cleanup failed");
    handle2.await.expect("Actor2 cleanup failed");
    handle3.await.expect("Actor3 cleanup failed");
    monitor_handle.await.expect("Monitor cleanup failed");
}

// ============================================================================
// Tests for which_* functions comprehensive coverage
// ============================================================================

#[named]
#[crate::concurrency::test]
#[serial]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_which_functions_comprehensive() {
    let scope1 = format!("{}_scope1", function_name!());
    let scope2 = format!("{}_scope2", function_name!());
    let group1 = format!("{}_group1", function_name!());
    let group2 = format!("{}_group2", function_name!());
    let group3 = format!("{}_group3", function_name!());

    let (actor1, handle1) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");
    let (actor2, handle2) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");
    let (actor3, handle3) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");

    // Create a complex structure
    pg::join(&group1, vec![actor1.clone().into()]); // default scope
    pg::join_scoped(&scope1, &group1, vec![actor1.clone().into()]);
    pg::join_scoped(&scope1, &group2, vec![actor2.clone().into()]);
    pg::join_scoped(&scope2, &group1, vec![actor2.clone().into()]);
    pg::join_scoped(&scope2, &group3, vec![actor3.clone().into()]);

    // Test which_scopes
    let scopes = pg::which_scopes();
    assert!(scopes.contains(&pg::DEFAULT_SCOPE.to_string()));
    assert!(scopes.contains(&scope1));
    assert!(scopes.contains(&scope2));
    assert!(scopes.len() >= 3);

    // Test which_groups (should aggregate from all scopes)
    let all_groups = pg::which_groups();
    assert!(all_groups.contains(&group1));
    assert!(all_groups.contains(&group2));
    assert!(all_groups.contains(&group3));
    assert!(all_groups.len() >= 3);

    // Test which_scoped_groups
    let scope1_groups = pg::which_scoped_groups(&scope1);
    assert!(scope1_groups.contains(&group1));
    assert!(scope1_groups.contains(&group2));
    assert!(!scope1_groups.contains(&group3)); // group3 is not in scope1
    assert_eq!(scope1_groups.len(), 2);

    let scope2_groups = pg::which_scoped_groups(&scope2);
    assert!(scope2_groups.contains(&group1));
    assert!(scope2_groups.contains(&group3));
    assert!(!scope2_groups.contains(&group2)); // group2 is not in scope2
    assert_eq!(scope2_groups.len(), 2);

    // Test which_scopes_and_groups
    let scope_group_combinations = pg::which_scopes_and_groups();
    let expected_keys = vec![
        pg::ScopeGroupKey::new(pg::DEFAULT_SCOPE.to_string(), group1.clone()),
        pg::ScopeGroupKey::new(scope1.clone(), group1.clone()),
        pg::ScopeGroupKey::new(scope1.clone(), group2.clone()),
        pg::ScopeGroupKey::new(scope2.clone(), group1.clone()),
        pg::ScopeGroupKey::new(scope2.clone(), group3.clone()),
    ];

    for expected_key in expected_keys {
        assert!(scope_group_combinations.contains(&expected_key));
    }

    // Cleanup
    actor1.stop(None);
    actor2.stop(None);
    actor3.stop(None);
    handle1.await.expect("Actor cleanup failed");
    handle2.await.expect("Actor cleanup failed");
    handle3.await.expect("Actor cleanup failed");
}

// ============================================================================
// Tests for ALL_SCOPES_NOTIFICATION in demonitor_scope
// ============================================================================

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_demonitor_scope_all_scopes_special_case() {
    let scope1 = format!("{}_scope1", function_name!());
    let scope2 = format!("{}_scope2", function_name!());
    let group = format!("{}_group", function_name!());
    let counter = Arc::new(AtomicU8::new(0u8));

    struct AllScopesScopeMonitor {
        counter: Arc<AtomicU8>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for AllScopesScopeMonitor {
        type Msg = ();
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            myself: crate::ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            let cell = myself.into();
            // Monitor world and specific scopes
            pg::monitor_world(&cell);
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            _myself: crate::ActorRef<Self::Msg>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            if let SupervisionEvent::ProcessGroupChanged(change) = message {
                match change {
                    pg::GroupChangeMessage::Join(_, _, who) => {
                        self.counter.fetch_add(who.len() as u8, Ordering::Relaxed);
                    }
                    pg::GroupChangeMessage::Leave(_, _, who) => {
                        self.counter.fetch_sub(who.len() as u8, Ordering::Relaxed);
                    }
                }
            }
            Ok(())
        }
    }

    // Create some scopes first
    let (actor1, handle1) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");
    let (actor2, handle2) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");

    pg::join_scoped(&scope1, &group, vec![actor1.clone().into()]);
    pg::join_scoped(&scope2, &group, vec![actor2.clone().into()]);

    // Start monitor
    let (monitor_actor, monitor_handle) = Actor::spawn(
        None,
        AllScopesScopeMonitor {
            counter: counter.clone(),
        },
        (),
    )
    .await
    .expect("Failed to start monitor");

    // Add to existing scopes (should be detected)
    let (actor3, handle3) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");
    pg::join_scoped(&scope1, &group, vec![actor3.clone().into()]);

    // Should receive notification
    periodic_check(
        || counter.load(Ordering::Relaxed) > 0,
        Duration::from_secs(5),
    )
    .await;

    // Test demonitor_scope with ALL_SCOPES_NOTIFICATION (special case)
    pg::demonitor_scope(pg::ALL_SCOPES_NOTIFICATION, monitor_actor.get_id());

    // Reset counter
    counter.store(0, Ordering::Relaxed);

    // Add another actor (should get NO notifications now)
    let (actor4, handle4) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");
    pg::join_scoped(&scope1, &group, vec![actor4.clone().into()]);

    // Wait and verify no notifications
    crate::concurrency::sleep(Duration::from_millis(100)).await;
    assert_eq!(counter.load(Ordering::Relaxed), 0);

    // Cleanup
    actor1.stop(None);
    actor2.stop(None);
    actor3.stop(None);
    actor4.stop(None);
    monitor_actor.stop(None);

    handle1.await.expect("Actor cleanup failed");
    handle2.await.expect("Actor cleanup failed");
    handle3.await.expect("Actor cleanup failed");
    handle4.await.expect("Actor cleanup failed");
    monitor_handle.await.expect("Monitor cleanup failed");
}

// ============================================================================
// Tests for robustness with invalid operations
// ============================================================================

#[named]
#[crate::concurrency::test]
#[serial]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_robustness_with_invalid_operations() {
    let scope = format!("{}_scope", function_name!());
    let group = format!("{}_group", function_name!());

    let (actor, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");

    // Test operations on non-existent groups/scopes (should not panic)
    pg::demonitor("nonexistent_group", actor.get_id());
    pg::demonitor_scoped("nonexistent_scope", "nonexistent_group", actor.get_id());
    pg::demonitor_scope("nonexistent_scope", actor.get_id());

    // Monitor then immediately demonitor
    pg::monitor(&group, actor.get_cell());
    pg::demonitor(&group, actor.get_id());

    pg::monitor_scoped(&scope, &group, actor.get_cell());
    pg::demonitor_scoped(&scope, &group, actor.get_id());

    pg::monitor_scope(&scope, actor.get_cell());
    pg::demonitor_scope(&scope, actor.get_id());

    pg::monitor_world(&actor.get_cell());
    pg::demonitor_world(&actor.get_cell());

    // Multiple demonitor calls should not panic
    pg::demonitor(&group, actor.get_id());
    pg::demonitor_scoped(&scope, &group, actor.get_id());
    pg::demonitor_scope(&scope, actor.get_id());
    pg::demonitor_world(&actor.get_cell());

    // Operations should still work normally after invalid operations
    pg::join(&group, vec![actor.clone().into()]);
    let members = pg::get_members(&group);
    assert_eq!(members.len(), 1);

    // Cleanup
    actor.stop(None);
    handle.await.expect("Actor cleanup failed");
}

// ============================================================================
// Tests for automatic cleanup behavior
// ============================================================================

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_automatic_cleanup_on_actor_shutdown() {
    let scope1 = format!("{}_scope1", function_name!());
    let scope2 = format!("{}_scope2", function_name!());
    let group1 = format!("{}_group1", function_name!());
    let group2 = format!("{}_group2", function_name!());

    let counter = Arc::new(AtomicU8::new(0u8));

    struct CleanupMonitor {
        counter: Arc<AtomicU8>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for CleanupMonitor {
        type Msg = ();
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            myself: crate::ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            pg::monitor_world(&myself.into());
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            _myself: crate::ActorRef<Self::Msg>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            if let SupervisionEvent::ProcessGroupChanged(change) = message {
                match change {
                    pg::GroupChangeMessage::Leave(scope, group, who) => {
                        println!(
                            "Leave event: scope={}, group={}, who={:?}",
                            scope, group, who
                        );
                        self.counter.fetch_add(who.len() as u8, Ordering::Relaxed);
                    }
                    _ => {}
                }
            }
            Ok(())
        }
    }

    // Start monitor to track leave events
    let (monitor_actor, monitor_handle) = Actor::spawn(
        None,
        CleanupMonitor {
            counter: counter.clone(),
        },
        (),
    )
    .await
    .expect("Failed to start monitor");

    struct MultiGroupActor {
        scope1: ScopeName,
        scope2: ScopeName,
        group1: GroupName,
        group2: GroupName,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for MultiGroupActor {
        type Msg = ();
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            myself: crate::ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            let cell: crate::ActorCell = myself.into();
            // Join multiple groups in multiple scopes
            pg::join(&self.group1, vec![cell.clone()]);
            pg::join_scoped(&self.scope1, &self.group1, vec![cell.clone()]);
            pg::join_scoped(&self.scope1, &self.group2, vec![cell.clone()]);
            pg::join_scoped(&self.scope2, &self.group1, vec![cell.clone()]);
            Ok(())
        }
    }

    // Create actor that joins multiple groups
    let (multi_actor, multi_handle) = Actor::spawn(
        None,
        MultiGroupActor {
            scope1: scope1.clone(),
            scope2: scope2.clone(),
            group1: group1.clone(),
            group2: group2.clone(),
        },
        (),
    )
    .await
    .expect("Failed to spawn multi-group actor");

    // Verify the actor is in all expected groups
    assert!(pg::get_members(&group1).len() > 0);
    assert!(pg::get_scoped_members(&scope1, &group1).len() > 0);
    assert!(pg::get_scoped_members(&scope1, &group2).len() > 0);
    assert!(pg::get_scoped_members(&scope2, &group1).len() > 0);

    // Stop the actor (should trigger automatic cleanup via leave_and_demonitor_all)
    multi_actor.stop(None);
    multi_handle.await.expect("Multi-actor cleanup failed");

    // Should have received leave notifications for all groups
    periodic_check(
        || counter.load(Ordering::Relaxed) == 4, // 4 groups the actor was in
        Duration::from_secs(5),
    )
    .await;

    // Verify the actor was removed from all groups
    assert_eq!(pg::get_members(&group1).len(), 0);
    assert_eq!(pg::get_scoped_members(&scope1, &group1).len(), 0);
    assert_eq!(pg::get_scoped_members(&scope1, &group2).len(), 0);
    assert_eq!(pg::get_scoped_members(&scope2, &group1).len(), 0);

    // Cleanup
    monitor_actor.stop(None);
    monitor_handle.await.expect("Monitor cleanup failed");
}

// ============================================================================
// Tests for mixed default and named scopes
// ============================================================================

#[named]
#[crate::concurrency::test]
#[serial]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_mixed_default_and_named_scopes() {
    let scope = format!("{}_scope", function_name!());
    let group = format!("{}_group", function_name!());

    let (actor1, handle1) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");
    let (actor2, handle2) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");

    // Same group name in different scopes
    pg::join(&group, vec![actor1.clone().into()]); // default scope
    pg::join_scoped(&scope, &group, vec![actor2.clone().into()]); // named scope

    // Verify isolation between scopes
    let default_members = pg::get_members(&group);
    let scoped_members = pg::get_scoped_members(&scope, &group);

    assert_eq!(default_members.len(), 1);
    assert_eq!(scoped_members.len(), 1);

    // Verify they're different actors
    assert_ne!(default_members[0].get_id(), scoped_members[0].get_id());

    // Test which_groups aggregates from both scopes
    let all_groups = pg::which_groups();
    assert!(all_groups.contains(&group));

    let scope_combinations = pg::which_scopes_and_groups();
    assert!(scope_combinations.contains(&pg::ScopeGroupKey::new(
        pg::DEFAULT_SCOPE.to_string(),
        group.clone()
    )));
    assert!(scope_combinations.contains(&pg::ScopeGroupKey::new(scope.clone(), group.clone())));

    // Test which_scoped_groups shows only the groups in that specific scope
    let default_scope_groups = pg::which_scoped_groups(pg::DEFAULT_SCOPE);
    let named_scope_groups = pg::which_scoped_groups(&scope);

    assert!(default_scope_groups.contains(&group));
    assert!(named_scope_groups.contains(&group));

    // Cleanup
    actor1.stop(None);
    actor2.stop(None);
    handle1.await.expect("Actor cleanup failed");
    handle2.await.expect("Actor cleanup failed");
}

// ============================================================================
// Tests for ALL_GROUPS_NOTIFICATION with demonitor_scoped
// ============================================================================

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_demonitor_all_groups_in_scope() {
    let scope = format!("{}_scope", function_name!());
    let group1 = format!("{}_group1", function_name!());
    let group2 = format!("{}_group2", function_name!());
    let counter = Arc::new(AtomicU8::new(0u8));

    struct MultiGroupMonitor {
        scope: ScopeName,
        group1: GroupName,
        group2: GroupName,
        counter: Arc<AtomicU8>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for MultiGroupMonitor {
        type Msg = ();
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            myself: crate::ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            let cell: crate::ActorCell = myself.into();
            // Monitor specific groups
            pg::monitor_scoped(&self.scope, &self.group1, cell.clone());
            pg::monitor_scoped(&self.scope, &self.group2, cell.clone());
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            _myself: crate::ActorRef<Self::Msg>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            if let SupervisionEvent::ProcessGroupChanged(change) = message {
                match change {
                    pg::GroupChangeMessage::Join(_, _, who) => {
                        self.counter.fetch_add(who.len() as u8, Ordering::Relaxed);
                    }
                    pg::GroupChangeMessage::Leave(_, _, who) => {
                        self.counter.fetch_sub(who.len() as u8, Ordering::Relaxed);
                    }
                }
            }
            Ok(())
        }
    }

    // Create groups first
    let (actor1, handle1) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");
    let (actor2, handle2) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");

    pg::join_scoped(&scope, &group1, vec![actor1.clone().into()]);
    pg::join_scoped(&scope, &group2, vec![actor2.clone().into()]);

    // Start monitor
    let (monitor_actor, monitor_handle) = Actor::spawn(
        None,
        MultiGroupMonitor {
            scope: scope.clone(),
            group1: group1.clone(),
            group2: group2.clone(),
            counter: counter.clone(),
        },
        (),
    )
    .await
    .expect("Failed to start monitor");

    // Add more actors to trigger notifications
    let (actor3, handle3) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");
    let (actor4, handle4) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");

    pg::join_scoped(&scope, &group1, vec![actor3.clone().into()]);
    pg::join_scoped(&scope, &group2, vec![actor4.clone().into()]);

    // Should receive 2 notifications
    periodic_check(
        || counter.load(Ordering::Relaxed) == 2,
        Duration::from_secs(5),
    )
    .await;

    // Test demonitor with ALL_GROUPS_NOTIFICATION (should remove all group monitoring in scope)
    pg::demonitor_scoped(&scope, pg::ALL_GROUPS_NOTIFICATION, monitor_actor.get_id());

    // Reset counter
    counter.store(0, Ordering::Relaxed);

    // Add more actors (should get NO notifications now)
    let (actor5, handle5) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");
    pg::join_scoped(&scope, &group1, vec![actor5.clone().into()]);

    // Wait and verify no notifications
    crate::concurrency::sleep(Duration::from_millis(100)).await;
    assert_eq!(counter.load(Ordering::Relaxed), 0);

    // Cleanup
    actor1.stop(None);
    actor2.stop(None);
    actor3.stop(None);
    actor4.stop(None);
    actor5.stop(None);
    monitor_actor.stop(None);

    handle1.await.expect("Actor cleanup failed");
    handle2.await.expect("Actor cleanup failed");
    handle3.await.expect("Actor cleanup failed");
    handle4.await.expect("Actor cleanup failed");
    handle5.await.expect("Actor cleanup failed");
    monitor_handle.await.expect("Monitor cleanup failed");
}

// ============================================================================
// Tests for comprehensive monitoring scenarios with multiple layers
// ============================================================================

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_layered_monitoring_overlaps() {
    let scope = format!("{}_scope", function_name!());
    let group = format!("{}_group", function_name!());

    let world_counter = Arc::new(AtomicU8::new(0u8));
    let scope_counter = Arc::new(AtomicU8::new(0u8));
    let group_counter = Arc::new(AtomicU8::new(0u8));

    struct LayeredMonitor {
        monitor_type: String,
        scope: ScopeName,
        group: GroupName,
        counter: Arc<AtomicU8>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for LayeredMonitor {
        type Msg = ();
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            myself: crate::ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            let cell = myself.into();
            match self.monitor_type.as_str() {
                "world" => pg::monitor_world(&cell),
                "scope" => pg::monitor_scope(&self.scope, cell),
                "group" => pg::monitor_scoped(&self.scope, &self.group, cell),
                _ => panic!("Unknown monitor type"),
            }
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            _myself: crate::ActorRef<Self::Msg>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            if let SupervisionEvent::ProcessGroupChanged(change) = message {
                match change {
                    pg::GroupChangeMessage::Join(_, _, who) => {
                        self.counter.fetch_add(who.len() as u8, Ordering::Relaxed);
                    }
                    pg::GroupChangeMessage::Leave(_, _, who) => {
                        self.counter.fetch_sub(who.len() as u8, Ordering::Relaxed);
                    }
                }
            }
            Ok(())
        }
    }

    // Start three different types of monitors
    let (world_monitor, world_handle) = Actor::spawn(
        None,
        LayeredMonitor {
            monitor_type: "world".to_string(),
            scope: scope.clone(),
            group: group.clone(),
            counter: world_counter.clone(),
        },
        (),
    )
    .await
    .expect("Failed to start world monitor");

    let (scope_monitor, scope_handle) = Actor::spawn(
        None,
        LayeredMonitor {
            monitor_type: "scope".to_string(),
            scope: scope.clone(),
            group: group.clone(),
            counter: scope_counter.clone(),
        },
        (),
    )
    .await
    .expect("Failed to start scope monitor");

    let (group_monitor, group_handle) = Actor::spawn(
        None,
        LayeredMonitor {
            monitor_type: "group".to_string(),
            scope: scope.clone(),
            group: group.clone(),
            counter: group_counter.clone(),
        },
        (),
    )
    .await
    .expect("Failed to start group monitor");

    // Create an actor and join the monitored group
    let (actor, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");
    pg::join_scoped(&scope, &group, vec![actor.clone().into()]);

    // All three monitors should receive the notification
    periodic_check(
        || {
            world_counter.load(Ordering::Relaxed) == 1
                && scope_counter.load(Ordering::Relaxed) == 1
                && group_counter.load(Ordering::Relaxed) == 1
        },
        Duration::from_secs(5),
    )
    .await;

    // Create another group in the same scope (only world and scope monitors should see it)
    let other_group = format!("{}_other", function_name!());
    let (actor2, handle2) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");
    pg::join_scoped(&scope, &other_group, vec![actor2.clone().into()]);

    // Wait for notifications
    periodic_check(
        || world_counter.load(Ordering::Relaxed) == 2 && scope_counter.load(Ordering::Relaxed) == 2,
        Duration::from_secs(5),
    )
    .await;

    // Group monitor should still only have 1 notification
    assert_eq!(group_counter.load(Ordering::Relaxed), 1);

    // Cleanup
    actor.stop(None);
    actor2.stop(None);
    world_monitor.stop(None);
    scope_monitor.stop(None);
    group_monitor.stop(None);

    handle.await.expect("Actor cleanup failed");
    handle2.await.expect("Actor2 cleanup failed");
    world_handle.await.expect("World monitor cleanup failed");
    scope_handle.await.expect("Scope monitor cleanup failed");
    group_handle.await.expect("Group monitor cleanup failed");
}

// ============================================================================
// Tests for notification content validation
// ============================================================================

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_notification_content_validation() {
    let scope = format!("{}_scope", function_name!());
    let group = format!("{}_group", function_name!());

    let received_notifications = Arc::new(std::sync::Mutex::new(Vec::new()));

    struct NotificationCapture {
        scope: ScopeName,
        group: GroupName,
        notifications: Arc<std::sync::Mutex<Vec<pg::GroupChangeMessage>>>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for NotificationCapture {
        type Msg = ();
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            myself: crate::ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            pg::monitor_scoped(&self.scope, &self.group, myself.into());
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            _myself: crate::ActorRef<Self::Msg>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            if let SupervisionEvent::ProcessGroupChanged(change) = message {
                self.notifications.lock().unwrap().push(change);
            }
            Ok(())
        }
    }

    // Start notification capture
    let (monitor_actor, monitor_handle) = Actor::spawn(
        None,
        NotificationCapture {
            scope: scope.clone(),
            group: group.clone(),
            notifications: received_notifications.clone(),
        },
        (),
    )
    .await
    .expect("Failed to start monitor");

    // Create actors and perform operations
    let (actor1, handle1) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");
    let (actor2, handle2) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");

    // Join multiple actors
    pg::join_scoped(
        &scope,
        &group,
        vec![actor1.clone().into(), actor2.clone().into()],
    );

    // Wait for notification
    periodic_check(
        || received_notifications.lock().unwrap().len() == 1,
        Duration::from_secs(5),
    )
    .await;

    // Verify notification content
    {
        let notifications = received_notifications.lock().unwrap();
        let notification = &notifications[0];
        assert_eq!(notification.get_scope(), scope);
        assert_eq!(notification.get_group(), group);

        if let pg::GroupChangeMessage::Join(notif_scope, notif_group, actors) = notification {
            assert_eq!(notif_scope, &scope);
            assert_eq!(notif_group, &group);
            assert_eq!(actors.len(), 2);
        } else {
            panic!("Expected Join notification");
        }
    }

    // Test leave notification
    pg::leave_scoped(&scope, &group, vec![actor1.clone().into()]);

    // Wait for leave notification
    periodic_check(
        || received_notifications.lock().unwrap().len() == 2,
        Duration::from_secs(5),
    )
    .await;

    // Verify leave notification content
    {
        let notifications = received_notifications.lock().unwrap();
        let leave_notification = &notifications[1];

        if let pg::GroupChangeMessage::Leave(notif_scope, notif_group, actors) = leave_notification
        {
            assert_eq!(notif_scope, &scope);
            assert_eq!(notif_group, &group);
            assert_eq!(actors.len(), 1);
        } else {
            panic!("Expected Leave notification");
        }
    }

    // Cleanup
    actor1.stop(None);
    actor2.stop(None);
    monitor_actor.stop(None);

    handle1.await.expect("Actor cleanup failed");
    handle2.await.expect("Actor cleanup failed");
    monitor_handle.await.expect("Monitor cleanup failed");
}

// ============================================================================
// Tests for stress scenarios with many groups and scopes
// ============================================================================

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_many_groups_and_scopes() {
    let base_scope = format!("{}_scope", function_name!());
    let base_group = format!("{}_group", function_name!());

    let mut actors = Vec::new();
    let mut handles = Vec::new();

    // Create multiple scopes and groups
    for scope_idx in 0..3 {
        for group_idx in 0..3 {
            let scope = format!("{}_{}", base_scope, scope_idx);
            let group = format!("{}_{}", base_group, group_idx);

            let (actor, handle) = Actor::spawn(None, TestActor, ())
                .await
                .expect("Failed to spawn");
            actors.push(actor.clone());
            handles.push(handle);

            pg::join_scoped(&scope, &group, vec![actor.into()]);
        }
    }

    // Verify we have the expected number of scopes and groups
    let scopes = pg::which_scopes();
    let all_groups = pg::which_groups();
    let scope_group_combinations = pg::which_scopes_and_groups();

    assert!(scopes.len() >= 3);
    assert!(all_groups.len() >= 3);
    assert!(scope_group_combinations.len() >= 9); // 3 scopes × 3 groups

    // Test that each scope has the expected groups
    for scope_idx in 0..3 {
        let scope = format!("{}_{}", base_scope, scope_idx);
        let scope_groups = pg::which_scoped_groups(&scope);
        assert_eq!(scope_groups.len(), 3);

        for group_idx in 0..3 {
            let group = format!("{}_{}", base_group, group_idx);
            assert!(scope_groups.contains(&group));

            // Verify each group has exactly one member
            let members = pg::get_scoped_members(&scope, &group);
            assert_eq!(members.len(), 1);
        }
    }

    // Cleanup
    for actor in actors {
        actor.stop(None);
    }
    for handle in handles {
        handle.await.expect("Actor cleanup failed");
    }
}

// ============================================================================
// Tests for concurrent operations
// ============================================================================

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_concurrent_join_leave_operations() {
    let group = format!("{}_group", function_name!());
    let scope = format!("{}_scope", function_name!());

    let mut actors = Vec::new();
    let mut handles = Vec::new();

    // Create multiple actors
    for _ in 0..5 {
        let (actor, handle) = Actor::spawn(None, TestActor, ())
            .await
            .expect("Failed to spawn");
        actors.push(actor);
        handles.push(handle);
    }

    // Perform concurrent operations
    let join_tasks: Vec<_> = actors
        .iter()
        .enumerate()
        .map(|(i, actor)| {
            let group = group.clone();
            let scope = scope.clone();
            let actor = actor.clone();
            crate::concurrency::spawn(async move {
                if i % 2 == 0 {
                    pg::join(&group, vec![actor.into()]);
                } else {
                    pg::join_scoped(&scope, &group, vec![actor.into()]);
                }
            })
        })
        .collect();

    // Wait for all join operations to complete
    for task in join_tasks {
        task.await.expect("Join task failed");
    }

    // Verify memberships
    let default_members = pg::get_members(&group);
    let scoped_members = pg::get_scoped_members(&scope, &group);

    assert!(default_members.len() >= 2); // At least actors with even indices
    assert!(scoped_members.len() >= 2); // At least actors with odd indices

    // Perform concurrent leave operations
    let leave_tasks: Vec<_> = actors
        .iter()
        .enumerate()
        .map(|(i, actor)| {
            let group = group.clone();
            let scope = scope.clone();
            let actor = actor.clone();
            crate::concurrency::spawn(async move {
                if i % 2 == 0 {
                    pg::leave(&group, vec![actor.into()]);
                } else {
                    pg::leave_scoped(&scope, &group, vec![actor.into()]);
                }
            })
        })
        .collect();

    // Wait for all leave operations to complete
    for task in leave_tasks {
        task.await.expect("Leave task failed");
    }

    // Verify all actors left
    let final_default_members = pg::get_members(&group);
    let final_scoped_members = pg::get_scoped_members(&scope, &group);

    assert_eq!(final_default_members.len(), 0);
    assert_eq!(final_scoped_members.len(), 0);

    // Cleanup
    for actor in actors {
        actor.stop(None);
    }
    for handle in handles {
        handle.await.expect("Actor cleanup failed");
    }
}

// ============================================================================
// Tests for monitor/demonitor cycles and timing
// ============================================================================

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_monitor_then_demonitor_cycles() {
    let scope = format!("{}_scope", function_name!());
    let group = format!("{}_group", function_name!());
    let counter = Arc::new(AtomicU8::new(0u8));

    struct CycleMonitor {
        counter: Arc<AtomicU8>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for CycleMonitor {
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

        async fn handle_supervisor_evt(
            &self,
            _myself: crate::ActorRef<Self::Msg>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            if let SupervisionEvent::ProcessGroupChanged(change) = message {
                match change {
                    pg::GroupChangeMessage::Join(_, _, who) => {
                        self.counter.fetch_add(who.len() as u8, Ordering::Relaxed);
                    }
                    pg::GroupChangeMessage::Leave(_, _, who) => {
                        self.counter.fetch_sub(who.len() as u8, Ordering::Relaxed);
                    }
                }
            }
            Ok(())
        }
    }

    let (monitor_actor, monitor_handle) = Actor::spawn(
        None,
        CycleMonitor {
            counter: counter.clone(),
        },
        (),
    )
    .await
    .expect("Failed to start monitor");

    let (actor, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");

    // Cycle 1: Monitor -> Join -> Demonitor -> Join again
    pg::monitor_scoped(&scope, &group, monitor_actor.get_cell());
    pg::join_scoped(&scope, &group, vec![actor.clone().into()]);

    periodic_check(
        || counter.load(Ordering::Relaxed) == 1,
        Duration::from_secs(5),
    )
    .await;

    pg::demonitor_scoped(&scope, &group, monitor_actor.get_id());
    pg::leave_scoped(&scope, &group, vec![actor.clone().into()]);

    // Rejoin after demonitoring (should not receive notification)
    pg::join_scoped(&scope, &group, vec![actor.clone().into()]);
    crate::concurrency::sleep(Duration::from_millis(100)).await;
    assert_eq!(counter.load(Ordering::Relaxed), 1); // Should still be 1

    // Monitor again and verify notifications work
    pg::monitor_scoped(&scope, &group, monitor_actor.get_cell());
    pg::leave_scoped(&scope, &group, vec![actor.clone().into()]);

    periodic_check(
        || counter.load(Ordering::Relaxed) == 0,
        Duration::from_secs(5),
    )
    .await;

    // Cleanup
    actor.stop(None);
    monitor_actor.stop(None);
    handle.await.expect("Actor cleanup failed");
    monitor_handle.await.expect("Monitor cleanup failed");
}

// ============================================================================
// Tests for batch operations and notification batching
// ============================================================================

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_batch_operations_notifications() {
    let group = format!("{}_group", function_name!());
    let counter = Arc::new(AtomicU8::new(0u8));
    let notification_count = Arc::new(AtomicU8::new(0u8));

    struct BatchMonitor {
        group: GroupName,
        counter: Arc<AtomicU8>,
        notification_count: Arc<AtomicU8>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for BatchMonitor {
        type Msg = ();
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            myself: crate::ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            pg::monitor(&self.group, myself.into());
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            _myself: crate::ActorRef<Self::Msg>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            if let SupervisionEvent::ProcessGroupChanged(change) = message {
                self.notification_count.fetch_add(1, Ordering::Relaxed);
                match change {
                    pg::GroupChangeMessage::Join(_, _, who) => {
                        self.counter.fetch_add(who.len() as u8, Ordering::Relaxed);
                    }
                    pg::GroupChangeMessage::Leave(_, _, who) => {
                        self.counter.fetch_sub(who.len() as u8, Ordering::Relaxed);
                    }
                }
            }
            Ok(())
        }
    }

    // Start monitor
    let (monitor_actor, monitor_handle) = Actor::spawn(
        None,
        BatchMonitor {
            group: group.clone(),
            counter: counter.clone(),
            notification_count: notification_count.clone(),
        },
        (),
    )
    .await
    .expect("Failed to start monitor");

    // Test batch operations
    let mut actors = Vec::new();
    let mut handles = Vec::new();

    for _ in 0..5 {
        let (actor, handle) = Actor::spawn(None, TestActor, ())
            .await
            .expect("Failed to spawn");
        actors.push(actor);
        handles.push(handle);
    }

    // Join multiple actors at once (should be 1 notification with 5 actors)
    let actor_cells: Vec<_> = actors.iter().map(|a| a.clone().into()).collect();
    pg::join(&group, actor_cells.clone());

    // Should receive 1 notification with 5 actors
    periodic_check(
        || counter.load(Ordering::Relaxed) == 5 && notification_count.load(Ordering::Relaxed) == 1,
        Duration::from_secs(5),
    )
    .await;

    // Leave multiple actors at once (should be 1 notification with 5 actors)
    pg::leave(&group, actor_cells);

    // Should receive 1 leave notification with 5 actors
    periodic_check(
        || counter.load(Ordering::Relaxed) == 0 && notification_count.load(Ordering::Relaxed) == 2,
        Duration::from_secs(5),
    )
    .await;

    // Cleanup
    for actor in actors {
        actor.stop(None);
    }
    monitor_actor.stop(None);

    for handle in handles {
        handle.await.expect("Actor cleanup failed");
    }
    monitor_handle.await.expect("Monitor cleanup failed");
}

// ============================================================================
// Tests for monitoring setup with non-existent targets
// ============================================================================

#[named]
#[crate::concurrency::test]
#[serial]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_monitoring_nonexistent_targets() {
    let scope = format!("{}_scope", function_name!());
    let group = format!("{}_group", function_name!());
    let counter = Arc::new(AtomicU8::new(0u8));

    struct PreemptiveMonitor {
        scope: ScopeName,
        group: GroupName,
        counter: Arc<AtomicU8>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for PreemptiveMonitor {
        type Msg = ();
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            myself: crate::ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            let cell: crate::ActorCell = myself.into();
            // Monitor groups/scopes that don't exist yet
            pg::monitor_scoped(&self.scope, &self.group, cell.clone());
            pg::monitor_scope(&self.scope, cell);
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            _myself: crate::ActorRef<Self::Msg>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            if let SupervisionEvent::ProcessGroupChanged(change) = message {
                match change {
                    pg::GroupChangeMessage::Join(_, _, who) => {
                        self.counter.fetch_add(who.len() as u8, Ordering::Relaxed);
                    }
                    pg::GroupChangeMessage::Leave(_, _, who) => {
                        self.counter.fetch_sub(who.len() as u8, Ordering::Relaxed);
                    }
                }
            }
            Ok(())
        }
    }

    // Start monitor BEFORE creating any groups or scopes
    let (monitor_actor, monitor_handle) = Actor::spawn(
        None,
        PreemptiveMonitor {
            scope: scope.clone(),
            group: group.clone(),
            counter: counter.clone(),
        },
        (),
    )
    .await
    .expect("Failed to start monitor");

    // Now create the group/scope that was being monitored
    let (actor, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");
    pg::join_scoped(&scope, &group, vec![actor.clone().into()]);

    // Should receive notifications even though monitoring was set up before group existed
    periodic_check(
        || counter.load(Ordering::Relaxed) == 2, // Should get 2 notifications (scope + group monitors)
        Duration::from_secs(5),
    )
    .await;

    // Create another group in the same scope (scope monitor should detect it)
    let other_group = format!("{}_other", function_name!());
    let (actor2, handle2) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");
    pg::join_scoped(&scope, &other_group, vec![actor2.clone().into()]);

    // Should receive 1 more notification (only from scope monitor)
    periodic_check(
        || counter.load(Ordering::Relaxed) == 3,
        Duration::from_secs(5),
    )
    .await;

    // Cleanup
    actor.stop(None);
    actor2.stop(None);
    monitor_actor.stop(None);

    handle.await.expect("Actor cleanup failed");
    handle2.await.expect("Actor2 cleanup failed");
    monitor_handle.await.expect("Monitor cleanup failed");
}

// ============================================================================
// Tests for comprehensive cleanup verification
// ============================================================================

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_comprehensive_cleanup_verification() {
    let scope = format!("{}_scope", function_name!());
    let group = format!("{}_group", function_name!());

    // Verify initial state is clean
    assert_eq!(pg::which_scopes().len(), 0);
    assert_eq!(pg::which_groups().len(), 0);
    assert_eq!(pg::which_scopes_and_groups().len(), 0);

    let (actor, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");

    // Build up state
    pg::join_scoped(&scope, &group, vec![actor.clone().into()]);

    // Verify state exists
    assert!(pg::which_scopes().contains(&scope));
    assert!(pg::which_groups().contains(&group));
    assert!(pg::which_scoped_groups(&scope).contains(&group));
    assert_eq!(pg::get_scoped_members(&scope, &group).len(), 1);

    // Set up monitoring
    pg::monitor_scoped(&scope, &group, actor.get_cell());
    pg::monitor_scope(&scope, actor.get_cell());

    // Clean removal
    pg::leave_scoped(&scope, &group, vec![actor.clone().into()]);

    // Scope and group are not removed while there is a monitor
    assert_eq!(pg::which_scoped_groups(&scope).len(), 1);
    assert_eq!(pg::get_scoped_members(&scope, &group).len(), 0);

    // Manual monitoring cleanup
    pg::demonitor_scoped(&scope, &group, actor.get_id());
    pg::demonitor_scope(&scope, actor.get_id());

    // Scope and group are not removed while there is a monitor
    assert_eq!(pg::which_scoped_groups(&scope).len(), 0);
    assert_eq!(pg::get_scoped_members(&scope, &group).len(), 0);

    // Verify system can still operate normally after cleanup
    pg::join_scoped(&scope, &group, vec![actor.clone().into()]);
    assert_eq!(pg::get_scoped_members(&scope, &group).len(), 1);

    // Cleanup
    actor.stop(None);
    handle.await.expect("Actor cleanup failed");
}

// ============================================================================
// Tests for comprehensive scope and group name validation
// ============================================================================

#[crate::concurrency::test]
#[serial]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_scope_group_name_variations() {
    let (actor, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");

    // Test with various string types and cases
    let test_cases = vec![
        ("simple", "group"),
        ("scope_with_underscores", "group_with_underscores"),
        ("scope-with-dashes", "group-with-dashes"),
        ("scope.with.dots", "group.with.dots"),
        ("scope123", "group456"),
        ("UPPERCASE_SCOPE", "UPPERCASE_GROUP"),
        ("MixedCase_Scope", "MixedCase_Group"),
    ];

    for (scope, group) in test_cases {
        // Join and verify
        pg::join_scoped(scope, group, vec![actor.clone().into()]);
        assert_eq!(pg::get_scoped_members(scope, group).len(), 1);

        // Leave and verify cleanup
        pg::leave_scoped(scope, group, vec![actor.clone().into()]);
        assert_eq!(pg::get_scoped_members(scope, group).len(), 0);
    }

    // Test with Unicode characters (should work)
    let unicode_scope = "测试范围";
    let unicode_group = "测试组";

    pg::join_scoped(unicode_scope, unicode_group, vec![actor.clone().into()]);
    assert_eq!(
        pg::get_scoped_members(unicode_scope, unicode_group).len(),
        1
    );

    // Cleanup
    actor.stop(None);
    handle.await.expect("Actor cleanup failed");
}

// ============================================================================
// Tests for monitoring with immediate cleanup
// ============================================================================

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_monitoring_with_immediate_cleanup() {
    let scope = format!("{}_scope", function_name!());
    let group = format!("{}_group", function_name!());
    let counter = Arc::new(AtomicU8::new(0u8));

    struct ImmediateCleanupMonitor {
        scope: ScopeName,
        group: GroupName,
        counter: Arc<AtomicU8>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for ImmediateCleanupMonitor {
        type Msg = ();
        type Arguments = ();
        type State = ();

        async fn pre_start(
            &self,
            myself: crate::ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            let cell = myself.into();
            // Monitor a group that will have immediate cleanup
            pg::monitor_scoped(&self.scope, &self.group, cell);
            Ok(())
        }

        async fn handle_supervisor_evt(
            &self,
            _myself: crate::ActorRef<Self::Msg>,
            message: SupervisionEvent,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            if let SupervisionEvent::ProcessGroupChanged(change) = message {
                match change {
                    pg::GroupChangeMessage::Join(_, _, who) => {
                        self.counter.fetch_add(who.len() as u8, Ordering::Relaxed);
                    }
                    pg::GroupChangeMessage::Leave(_, _, who) => {
                        self.counter.fetch_sub(who.len() as u8, Ordering::Relaxed);
                    }
                }
            }
            Ok(())
        }
    }

    // Start monitor
    let (monitor_actor, monitor_handle) = Actor::spawn(
        None,
        ImmediateCleanupMonitor {
            scope: scope.clone(),
            group: group.clone(),
            counter: counter.clone(),
        },
        (),
    )
    .await
    .expect("Failed to start monitor");

    // Create actor, join, and immediately leave (rapid lifecycle)
    let (actor, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");

    pg::join_scoped(&scope, &group, vec![actor.clone().into()]);

    // Wait for join notification
    periodic_check(
        || counter.load(Ordering::Relaxed) == 1,
        Duration::from_secs(5),
    )
    .await;

    pg::leave_scoped(&scope, &group, vec![actor.clone().into()]);

    // Wait for leave notification
    periodic_check(
        || counter.load(Ordering::Relaxed) == 0,
        Duration::from_secs(5),
    )
    .await;

    // Verify group and scope still monitored
    assert_eq!(pg::get_scoped_members(&scope, &group).len(), 0);
    assert_eq!(pg::which_scoped_groups(&scope).len(), 1);

    monitor_actor.stop(None);
    monitor_handle.await.expect("Monitor cleanup failed");

    // No more monitor, group and scope are cleaned up
    assert_eq!(pg::get_scoped_members(&scope, &group).len(), 0);
    assert_eq!(pg::which_scoped_groups(&scope).len(), 0);

    // Cleanup
    actor.stop(None);
    handle.await.expect("Actor cleanup failed");
}

// ============================================================================
// Tests for error conditions during monitoring
// ============================================================================

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_monitor_error_conditions() {
    let scope = format!("{}_scope", function_name!());
    let group = format!("{}_group", function_name!());

    let (actor, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");

    // Test monitoring the same group multiple times (should not cause issues)
    pg::monitor(&group, actor.get_cell());
    pg::monitor(&group, actor.get_cell());
    pg::monitor(&group, actor.get_cell());

    // Test monitoring the same scoped group multiple times
    pg::monitor_scoped(&scope, &group, actor.get_cell());
    pg::monitor_scoped(&scope, &group, actor.get_cell());

    // Test monitoring the same scope multiple times
    pg::monitor_scope(&scope, actor.get_cell());
    pg::monitor_scope(&scope, actor.get_cell());

    // Test monitoring world multiple times
    pg::monitor_world(&actor.get_cell());
    pg::monitor_world(&actor.get_cell());

    // All operations should work normally despite multiple monitoring setups
    let (test_actor, test_handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn");

    pg::join(&group, vec![test_actor.clone().into()]);
    assert_eq!(pg::get_members(&group).len(), 1);

    pg::join_scoped(&scope, &group, vec![test_actor.clone().into()]);
    assert_eq!(pg::get_scoped_members(&scope, &group).len(), 1);

    // Cleanup monitoring (multiple demonitor calls should be safe)
    pg::demonitor(&group, actor.get_id());
    pg::demonitor(&group, actor.get_id());
    pg::demonitor_scoped(&scope, &group, actor.get_id());
    pg::demonitor_scoped(&scope, &group, actor.get_id());
    pg::demonitor_scope(&scope, actor.get_id());
    pg::demonitor_scope(&scope, actor.get_id());
    pg::demonitor_world(&actor.get_cell());
    pg::demonitor_world(&actor.get_cell());

    // System should still function normally
    pg::leave(&group, vec![test_actor.clone().into()]);
    assert_eq!(pg::get_members(&group).len(), 0);

    // Cleanup
    actor.stop(None);
    test_actor.stop(None);
    handle.await.expect("Actor cleanup failed");
    test_handle.await.expect("Test actor cleanup failed");
}
