// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use super::*;

struct FakeNodeSession;

impl FakeNodeSession {
    async fn get_node_session() -> (ActorRef<crate::node::NodeSessionMessage>, JoinHandle<()>) {
        let (r, h) = Actor::spawn(None, FakeNodeSession, ())
            .await
            .expect("Failed to start fake node session");
        let cell: ractor::ActorCell = r.into();
        (cell.into(), h)
    }
}

#[cfg_attr(
    all(
        feature = "async-trait",
        not(all(target_arch = "wasm32", target_os = "unknown"))
    ),
    ractor::async_trait
)]
#[cfg_attr(all(feature = "async-trait", all(target_arch = "wasm32", target_os = "unknown")), ractor::async_trait(?Send))]
impl Actor for FakeNodeSession {
    type Msg = crate::node::NodeSessionMessage;
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

#[ractor::concurrency::test]
async fn remote_actor_serialized_message_handling() {
    // setup
    let (actor, handle) = FakeNodeSession::get_node_session().await;
    let (remote_actor_ref, remote_actor_handle) = Actor::spawn(None, RemoteActor, actor.clone())
        .await
        .expect("Failed to spawn remote actor");

    let remote_actor_instance = RemoteActor;
    let mut remote_actor_state = RemoteActorState {
        message_tag: 0,
        pending_requests: HashMap::new(),
        session: actor.clone(),
    };

    // act & verify
    let bad_handler = remote_actor_instance
        .handle(
            remote_actor_ref.clone(),
            RemoteActorMessage,
            &mut remote_actor_state,
        )
        .await;
    assert!(bad_handler.is_err());

    let cast = SerializedMessage::Cast {
        variant: "A".to_string(),
        args: vec![1, 2, 3],
        metadata: None,
    };
    let cast_output = remote_actor_instance
        .handle_serialized(remote_actor_ref.clone(), cast, &mut remote_actor_state)
        .await;
    assert!(cast_output.is_ok());
    // cast's don't have pending requests
    assert_eq!(0, remote_actor_state.message_tag);

    let (tx, _rx) = ractor::concurrency::oneshot();
    let call = SerializedMessage::Call {
        variant: "B".to_string(),
        args: vec![1, 2, 3],
        reply: tx.into(),
        metadata: None,
    };
    let call_output = remote_actor_instance
        .handle_serialized(remote_actor_ref.clone(), call, &mut remote_actor_state)
        .await;
    assert!(call_output.is_ok());
    assert_eq!(1, remote_actor_state.message_tag);
    assert!(remote_actor_state.pending_requests.contains_key(&1));

    let reply = SerializedMessage::CallReply(1, vec![3, 4, 5]);
    let reply_output = remote_actor_instance
        .handle_serialized(remote_actor_ref.clone(), reply, &mut remote_actor_state)
        .await;
    assert!(reply_output.is_ok());
    assert!(!remote_actor_state.pending_requests.contains_key(&1));

    // cleanup
    remote_actor_ref.stop(None);
    actor.stop(None);
    remote_actor_handle.await.unwrap();
    handle.await.unwrap();
}
