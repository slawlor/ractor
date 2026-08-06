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
    assert_eq!(4, scopes_and_groups.len());
    let scopes = pg::which_scopes();
    assert_eq!(1, scopes.iter().filter(|scope| *scope == &scope_a).count());
    assert_eq!(1, scopes.iter().filter(|scope| *scope == &scope_b).count());

    // Cleanup
    actor.stop(None);
    handle.await.expect("Actor cleanup failed");

    let scopes_and_groups = pg::which_scopes_and_groups();
    assert!(scopes_and_groups.is_empty());
}

#[named]
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
            pg::monitor(self.pg_group.clone(), myself.clone().into());
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
    test_actor.stop(None);
    test_handle.await.expect("Actor cleanup failed");
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
            pg::monitor_scope(self.scope.clone(), myself.clone().into());
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

    let remote_pid = crate::ActorId::Remote { node_id: 1, pid: 2 };

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

#[named]
#[serial]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_monitor_scope_no_duplicate_notifications() {
    let scope = function_name!().to_string();
    let group = function_name!().to_string();

    let counter = Arc::new(AtomicU8::new(0u8));

    struct NotificationCounter {
        scope: ScopeName,
        counter: Arc<AtomicU8>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for NotificationCounter {
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
                if let pg::GroupChangeMessage::Join(scope_name, _, _) = change {
                    if scope_name == self.scope {
                        self.counter.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            Ok(())
        }
    }

    let (monitor_actor, monitor_handle) = Actor::spawn(
        None,
        NotificationCounter {
            scope: scope.clone(),
            counter: counter.clone(),
        },
        (),
    )
    .await
    .expect("Failed to start monitor actor");

    // First actor joins the group BEFORE scope monitoring is set up
    let (actor1, handle1) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn test actor");
    pg::join_scoped(scope.clone(), group.clone(), vec![actor1.clone().into()]);

    // Now set up scope monitoring (after group already has members)
    pg::monitor_scope(scope.clone(), monitor_actor.clone().into());

    // Second actor joins - should produce exactly 1 notification, not 2
    let (actor2, handle2) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn test actor");
    pg::join_scoped(scope.clone(), group.clone(), vec![actor2.clone().into()]);

    periodic_check(
        || counter.load(Ordering::Relaxed) == 1,
        Duration::from_secs(5),
    )
    .await;

    // Wait briefly and verify no duplicate notification arrived
    crate::concurrency::sleep(Duration::from_millis(200)).await;
    assert_eq!(1, counter.load(Ordering::Relaxed));

    // Cleanup
    actor1.stop(None);
    handle1.await.expect("Actor cleanup failed");
    actor2.stop(None);
    handle2.await.expect("Actor cleanup failed");
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
async fn test_demonitor_scope_fully_unsubscribes() {
    let scope = function_name!().to_string();
    let group = function_name!().to_string();

    let counter = Arc::new(AtomicU8::new(0u8));

    struct NotificationCounter {
        scope: ScopeName,
        counter: Arc<AtomicU8>,
    }

    #[cfg_attr(feature = "async-trait", crate::async_trait)]
    impl Actor for NotificationCounter {
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
                    pg::GroupChangeMessage::Join(scope_name, _, _) if scope_name == self.scope => {
                        self.counter.fetch_add(1, Ordering::Relaxed);
                    }
                    _ => {}
                }
            }
            Ok(())
        }
    }

    let (monitor_actor, monitor_handle) = Actor::spawn(
        None,
        NotificationCounter {
            scope: scope.clone(),
            counter: counter.clone(),
        },
        (),
    )
    .await
    .expect("Failed to start monitor actor");

    // Monitor the scope, then immediately demonitor
    pg::monitor_scope(scope.clone(), monitor_actor.clone().into());
    pg::demonitor_scope(scope.clone(), monitor_actor.get_id());

    // Join a group in the scope - should NOT produce any notification
    let (actor, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn test actor");
    pg::join_scoped(scope.clone(), group.clone(), vec![actor.clone().into()]);

    // Wait and verify no notification was received
    crate::concurrency::sleep(Duration::from_millis(200)).await;
    assert_eq!(0, counter.load(Ordering::Relaxed));

    // Cleanup
    actor.stop(None);
    handle.await.expect("Actor cleanup failed");
    monitor_actor.stop(None);
    monitor_handle.await.expect("Actor cleanup failed");
}

#[named]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_stopped_actor_not_inserted() {
    let group = function_name!().to_string();

    let (actor, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn test actor");
    let cell = actor.clone().get_cell();

    // Stop the actor first
    actor.stop(None);
    handle.await.expect("Actor cleanup failed");

    // Try to join after stop - should NOT be inserted
    pg::join(group.clone(), vec![cell]);

    let members = pg::get_members(&group);
    assert_eq!(0, members.len());
}

#[named]
#[test]
fn test_idempotent_membership_calls_preserve_notification_payloads() {
    let group = function_name!().to_owned();
    let (listener, mut listener_ports) =
        crate::ActorCell::new::<TestActor>(None).expect("Failed to create listener");
    let (member, _member_ports) =
        crate::ActorCell::new::<TestActor>(None).expect("Failed to create member");

    pg::monitor(group.clone(), listener.clone());
    pg::join(group.clone(), vec![member.clone(), member.clone()]);
    pg::join(group.clone(), vec![member.clone()]);

    let first_join = listener_ports
        .supervisor_rx
        .try_recv()
        .expect("join notification missing");
    assert!(matches!(
        first_join,
        SupervisionEvent::ProcessGroupChanged(pg::GroupChangeMessage::Join(_, _, actors))
            if actors.len() == 2 && actors.iter().all(|actor| actor.get_id() == member.get_id())
    ));
    let second_join = listener_ports
        .supervisor_rx
        .try_recv()
        .expect("idempotent join notification missing");
    assert!(matches!(
        second_join,
        SupervisionEvent::ProcessGroupChanged(pg::GroupChangeMessage::Join(_, _, actors))
            if actors.len() == 1 && actors[0].get_id() == member.get_id()
    ));

    pg::leave(group.clone(), vec![member.clone(), member.clone()]);
    pg::leave(group, vec![member.clone()]);

    let first_leave = listener_ports
        .supervisor_rx
        .try_recv()
        .expect("leave notification missing");
    assert!(matches!(
        first_leave,
        SupervisionEvent::ProcessGroupChanged(pg::GroupChangeMessage::Leave(_, _, actors))
            if actors.len() == 2 && actors.iter().all(|actor| actor.get_id() == member.get_id())
    ));
    let second_leave = listener_ports
        .supervisor_rx
        .try_recv()
        .expect("idempotent leave notification missing");
    assert!(matches!(
        second_leave,
        SupervisionEvent::ProcessGroupChanged(pg::GroupChangeMessage::Leave(_, _, actors))
            if actors.len() == 1 && actors[0].get_id() == member.get_id()
    ));
    assert!(listener_ports.supervisor_rx.try_recv().is_err());

    listener.set_status(crate::ActorStatus::Stopped);
    member.set_status(crate::ActorStatus::Stopped);
}

#[named]
#[crate::concurrency::test]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    tracing_test::traced_test
)]
async fn test_shutdown_cleans_only_the_actors_pg_relationships() {
    let (departing, departing_handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn departing actor");
    let (remaining, remaining_handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to spawn remaining actor");

    let scope = format!("{}_scope", function_name!());
    let other_scope = format!("{}_other_scope", function_name!());
    let default_group = format!("{}_default", function_name!());
    let departing_group = format!("{}_departing", function_name!());
    let shared_group = format!("{}_shared", function_name!());
    let untouched_group = format!("{}_untouched", function_name!());
    let monitored_group = format!("{}_monitored", function_name!());
    let departing_monitored_group = format!("{}_departing_monitored", function_name!());

    pg::join(
        default_group.clone(),
        vec![departing.clone().into(), remaining.clone().into()],
    );
    pg::join_scoped(
        scope.clone(),
        departing_group.clone(),
        vec![departing.clone().into()],
    );
    pg::join_scoped(
        scope.clone(),
        shared_group.clone(),
        vec![departing.clone().into(), remaining.clone().into()],
    );
    pg::join_scoped(
        other_scope.clone(),
        untouched_group.clone(),
        vec![remaining.clone().into()],
    );
    pg::monitor(monitored_group.clone(), departing.clone().into());
    pg::monitor(monitored_group.clone(), remaining.clone().into());
    pg::monitor(departing_monitored_group.clone(), departing.clone().into());
    pg::monitor_scope(scope.clone(), departing.clone().into());
    pg::monitor_scope(scope.clone(), remaining.clone().into());
    pg::monitor_scope(
        pg::ALL_SCOPES_NOTIFICATION.to_owned(),
        departing.clone().into(),
    );
    pg::monitor_scope(
        pg::ALL_SCOPES_NOTIFICATION.to_owned(),
        remaining.clone().into(),
    );

    departing.stop(None);
    departing_handle.await.expect("Actor cleanup failed");

    let default_members = pg::get_members(&default_group);
    assert_eq!(1, default_members.len());
    assert_eq!(remaining.get_id(), default_members[0].get_id());
    assert!(pg::get_scoped_members(&scope, &departing_group).is_empty());

    let shared_members = pg::get_scoped_members(&scope, &shared_group);
    assert_eq!(1, shared_members.len());
    assert_eq!(remaining.get_id(), shared_members[0].get_id());

    let untouched_members = pg::get_scoped_members(&other_scope, &untouched_group);
    assert_eq!(1, untouched_members.len());
    assert_eq!(remaining.get_id(), untouched_members[0].get_id());
    assert!(!pg::which_scoped_groups(&scope).contains(&departing_group));
    let monitor = pg::get_monitor();
    let group_key = pg::ScopeGroupKey {
        scope: pg::DEFAULT_SCOPE.to_owned(),
        group: monitored_group.clone(),
    };
    let group_state = monitor.map.get(&group_key).expect("group monitor missing");
    assert!(group_state
        .listeners
        .iter()
        .any(|listener| listener.get_id() == remaining.get_id()));
    assert!(group_state
        .listeners
        .iter()
        .all(|listener| listener.get_id() != departing.get_id()));
    drop(group_state);

    let departing_group_key = pg::ScopeGroupKey {
        scope: pg::DEFAULT_SCOPE.to_owned(),
        group: departing_monitored_group,
    };
    assert!(monitor.map.get(&departing_group_key).is_none());

    for world_key in [
        pg::ScopeGroupKey {
            scope: scope.clone(),
            group: pg::ALL_GROUPS_NOTIFICATION.to_owned(),
        },
        pg::ScopeGroupKey {
            scope: pg::ALL_SCOPES_NOTIFICATION.to_owned(),
            group: pg::ALL_GROUPS_NOTIFICATION.to_owned(),
        },
    ] {
        let listeners = monitor
            .world_listeners
            .get(&world_key)
            .expect("world monitor missing");
        assert!(listeners
            .iter()
            .any(|listener| listener.get_id() == remaining.get_id()));
        assert!(listeners
            .iter()
            .all(|listener| listener.get_id() != departing.get_id()));
    }
    assert!(monitor.actor_relations.get(&departing.get_id()).is_none());

    {
        let remaining_relations = pg::get_actor_relations(monitor, remaining.get_id())
            .expect("remaining actor reverse index missing");
        let remaining_relations = pg::lock_relations(&remaining_relations);
        let expected_memberships = [
            pg::ScopeGroupKey {
                scope: pg::DEFAULT_SCOPE.to_owned(),
                group: default_group,
            },
            pg::ScopeGroupKey {
                scope: scope.clone(),
                group: shared_group,
            },
            pg::ScopeGroupKey {
                scope: other_scope,
                group: untouched_group,
            },
        ];
        assert_eq!(
            expected_memberships.len(),
            remaining_relations.memberships.len()
        );
        assert!(expected_memberships
            .iter()
            .all(|key| remaining_relations.memberships.contains(key)));
        assert_eq!(1, remaining_relations.group_monitors.len());
        assert!(remaining_relations
            .group_monitors
            .contains(&pg::ScopeGroupKey {
                scope: pg::DEFAULT_SCOPE.to_owned(),
                group: monitored_group,
            }));
        assert_eq!(2, remaining_relations.world_monitors.len());
        assert!(remaining_relations
            .world_monitors
            .contains(&pg::ScopeGroupKey {
                scope,
                group: pg::ALL_GROUPS_NOTIFICATION.to_owned(),
            }));
        assert!(remaining_relations
            .world_monitors
            .contains(&pg::ScopeGroupKey {
                scope: pg::ALL_SCOPES_NOTIFICATION.to_owned(),
                group: pg::ALL_GROUPS_NOTIFICATION.to_owned(),
            }));
    }

    remaining.stop(None);
    remaining_handle.await.expect("Actor cleanup failed");
}

#[named]
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
#[test]
#[tracing_test::traced_test]
fn test_stopping_actor_rejects_registration_blocked_on_relations() {
    let group = format!("{}_group", function_name!());
    let scope = format!("{}_scope", function_name!());
    let world_key = pg::ScopeGroupKey {
        scope: scope.clone(),
        group: pg::ALL_GROUPS_NOTIFICATION.to_owned(),
    };
    let (actor, _ports) = crate::ActorCell::new::<TestActor>(None).expect("Failed to create actor");
    pg::join(group.clone(), vec![actor.clone()]);

    let monitor = pg::get_monitor();
    let relations =
        pg::get_actor_relations(monitor, actor.get_id()).expect("actor reverse index missing");
    let relations_guard = pg::lock_relations(&relations);

    let stopping_actor = actor.clone();
    let stop_thread =
        std::thread::spawn(move || stopping_actor.set_status(crate::ActorStatus::Stopping));
    while actor.get_status() != crate::ActorStatus::Stopping {
        std::thread::yield_now();
    }

    let registering_actor = actor.clone();
    let registration = std::thread::spawn(move || {
        pg::monitor_scope(scope, registering_actor);
    });
    while !monitor.world_listeners.try_get(&world_key).is_locked() {
        std::thread::yield_now();
    }

    drop(relations_guard);
    let previous_status = stop_thread.join().expect("stop thread panicked");
    assert!(previous_status < crate::ActorStatus::Stopping);
    registration.join().expect("registration thread panicked");

    assert!(pg::get_members(&group).is_empty());
    assert!(monitor
        .world_listeners
        .get(&world_key)
        .map_or(true, |listeners| listeners
            .iter()
            .all(|listener| listener.get_id() != actor.get_id())));
    assert!(monitor.actor_relations.get(&actor.get_id()).is_none());
    actor.set_status(crate::ActorStatus::Stopped);
}
