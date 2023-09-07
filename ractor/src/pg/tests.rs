// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use crate::common_test::periodic_check;
use crate::concurrency::Duration;
use ::function_name::named;

use crate::{Actor, ActorProcessingErr, GroupName, SupervisionEvent};

use crate::pg;

struct TestActor;

#[async_trait::async_trait]
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
#[tracing_test::traced_test]
async fn test_basic_group() {
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
#[tracing_test::traced_test]
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
#[tracing_test::traced_test]
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
#[tracing_test::traced_test]
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
#[tracing_test::traced_test]
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

    // members comes back empty
    let members = pg::get_members(&group);
    assert_eq!(0, members.len());

    // Cleanup
    actor.stop(None);
    handle.await.expect("Actor cleanup failed");
}

#[named]
#[crate::concurrency::test]
#[tracing_test::traced_test]
async fn test_pg_monitoring() {
    let group = function_name!().to_string();

    let counter = Arc::new(AtomicU8::new(0u8));

    struct AutoJoinActor {
        pg_group: GroupName,
    }

    #[async_trait::async_trait]
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

    #[async_trait::async_trait]
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
                    pg::GroupChangeMessage::Join(_which, who) => {
                        self.counter.fetch_add(who.len() as u8, Ordering::Relaxed);
                    }
                    pg::GroupChangeMessage::Leave(_which, who) => {
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

    // this actor's startup should "monitor" for PG changes
    let (test_actor, test_handle) = Actor::spawn(None, AutoJoinActor { pg_group: group }, ())
        .await
        .expect("Failed to start test actor");

    // the monitor is notified async, so we need to wait a tiny bit
    periodic_check(
        || counter.load(Ordering::Relaxed) == 1,
        Duration::from_millis(500),
    )
    .await;

    // kill the pg member
    test_actor.stop(None);
    test_handle.await.expect("Actor cleanup failed");
    // it should have notified that it's unsubscribed
    assert_eq!(0, counter.load(Ordering::Relaxed));

    // cleanup
    monitor_actor.stop(None);
    monitor_handle.await.expect("Actor cleanup failed");
}

#[named]
#[cfg(feature = "cluster")]
#[crate::concurrency::test]
#[tracing_test::traced_test]
async fn local_vs_remote_pg_members() {
    use crate::ActorRuntime;

    let group = function_name!().to_string();

    struct TestRemoteActor;
    struct TestRemoteActorMessage;
    impl crate::Message for TestRemoteActorMessage {}
    #[async_trait::async_trait]
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
