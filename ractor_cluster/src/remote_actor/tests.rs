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

#[cfg_attr(feature = "async-trait", ractor::async_trait)]
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

struct TestContext {
    session: ActorRef<crate::node::NodeSessionMessage>,
    session_handle: JoinHandle<()>,
    remote_actor: ActorRef<RemoteActorMessage>,
    remote_actor_handle: JoinHandle<()>,
}

impl TestContext {
    async fn new() -> Self {
        let (session, session_handle) = FakeNodeSession::get_node_session().await;
        let (remote_actor, remote_actor_handle) = Actor::spawn(None, RemoteActor, session.clone())
            .await
            .expect("Failed to spawn remote actor");
        Self {
            session,
            session_handle,
            remote_actor,
            remote_actor_handle,
        }
    }

    fn state(&self) -> RemoteActorState {
        RemoteActorState::new(self.session.clone())
    }

    async fn shutdown(self) {
        self.remote_actor.stop(None);
        self.session.stop(None);
        self.remote_actor_handle.await.unwrap();
        self.session_handle.await.unwrap();
    }
}

fn serialized_call(reply: RpcReplyPort<Vec<u8>>) -> SerializedMessage {
    SerializedMessage::Call {
        variant: "Call".to_string(),
        args: vec![1, 2, 3],
        reply,
        metadata: None,
    }
}

fn serialized_cast() -> SerializedMessage {
    SerializedMessage::Cast {
        variant: "Cast".to_string(),
        args: vec![1, 2, 3],
        metadata: None,
    }
}

#[ractor::concurrency::test]
async fn remote_actor_serialized_message_handling() {
    // setup
    let context = TestContext::new().await;

    let remote_actor_instance = RemoteActor;
    let mut remote_actor_state = context.state();

    // act & verify
    let bad_handler = remote_actor_instance
        .handle(
            context.remote_actor.clone(),
            RemoteActorMessage,
            &mut remote_actor_state,
        )
        .await;
    assert!(bad_handler.is_err());

    let cast_output = remote_actor_instance
        .handle_serialized(
            context.remote_actor.clone(),
            serialized_cast(),
            &mut remote_actor_state,
        )
        .await;
    assert!(cast_output.is_ok());
    // cast's don't have pending requests
    assert_eq!(0, remote_actor_state.message_tag);

    let (tx, _rx) = ractor::concurrency::oneshot();
    let call_output = remote_actor_instance
        .handle_serialized(
            context.remote_actor.clone(),
            serialized_call(tx.into()),
            &mut remote_actor_state,
        )
        .await;
    assert!(call_output.is_ok());
    assert_eq!(1, remote_actor_state.message_tag);
    assert!(remote_actor_state.pending_requests.contains_key(&1));

    let reply = SerializedMessage::CallReply(1, vec![3, 4, 5]);
    let reply_output = remote_actor_instance
        .handle_serialized(context.remote_actor.clone(), reply, &mut remote_actor_state)
        .await;
    assert!(reply_output.is_ok());
    assert!(!remote_actor_state.pending_requests.contains_key(&1));

    // cleanup
    context.shutdown().await;
}

#[ractor::concurrency::test]
async fn cancelled_and_timed_out_requests_are_reclaimed_amortized() {
    let context = TestContext::new().await;
    let remote_actor_instance = RemoteActor;
    let mut state = context.state();
    let live_request_count = PENDING_REQUEST_CLEANUP_BUDGET * 2;
    let closed_request_count = PENDING_REQUEST_CLEANUP_BUDGET * 8;
    let mut live_receivers = Vec::with_capacity(live_request_count);

    for _ in 0..live_request_count {
        let (sender, receiver) = ractor::concurrency::oneshot::<Vec<u8>>();
        remote_actor_instance
            .handle_serialized(
                context.remote_actor.clone(),
                serialized_call(sender.into()),
                &mut state,
            )
            .await
            .unwrap();
        live_receivers.push(receiver);
    }

    for request in 0..closed_request_count {
        let (sender, receiver) = ractor::concurrency::oneshot::<Vec<u8>>();
        let reply: RpcReplyPort<Vec<u8>> = if request % 2 == 0 {
            drop(receiver);
            sender.into()
        } else {
            let reply = (sender, ractor::concurrency::Duration::ZERO).into();
            assert!(
                ractor::concurrency::timeout(ractor::concurrency::Duration::ZERO, receiver)
                    .await
                    .is_err()
            );
            reply
        };
        remote_actor_instance
            .handle_serialized(
                context.remote_actor.clone(),
                serialized_call(reply),
                &mut state,
            )
            .await
            .unwrap();
    }

    let cleanup_messages =
        (live_request_count + closed_request_count + PENDING_REQUEST_CLEANUP_BUDGET - 1)
            / PENDING_REQUEST_CLEANUP_BUDGET;
    for _ in 0..cleanup_messages {
        remote_actor_instance
            .handle_serialized(context.remote_actor.clone(), serialized_cast(), &mut state)
            .await
            .unwrap();
    }

    assert_eq!(state.pending_requests.len(), live_request_count);
    for tag in 1..=live_request_count as u64 {
        assert!(state.pending_requests.contains_key(&tag));
    }

    for (index, receiver) in live_receivers.into_iter().enumerate() {
        let tag = index as u64 + 1;
        let response = tag.to_be_bytes().to_vec();
        remote_actor_instance
            .handle_serialized(
                context.remote_actor.clone(),
                SerializedMessage::CallReply(tag, response.clone()),
                &mut state,
            )
            .await
            .unwrap();
        assert_eq!(receiver.await.unwrap(), response);
    }
    assert!(state.pending_requests.is_empty());

    context.shutdown().await;
}

#[ractor::concurrency::test]
async fn forwarding_failure_drops_the_pending_reply_port() {
    let TestContext {
        session,
        session_handle,
        remote_actor,
        remote_actor_handle,
    } = TestContext::new().await;
    session.stop(None);
    session_handle.await.unwrap();

    let remote_actor_instance = RemoteActor;
    let mut state = RemoteActorState::new(session);
    let (sender, receiver) = ractor::concurrency::oneshot::<Vec<u8>>();
    remote_actor_instance
        .handle_serialized(
            remote_actor.clone(),
            serialized_call(sender.into()),
            &mut state,
        )
        .await
        .unwrap();

    assert_eq!(state.message_tag, 1);
    assert!(state.pending_requests.is_empty());
    assert!(receiver.await.is_err());

    remote_actor.stop(None);
    remote_actor_handle.await.unwrap();
}

#[ractor::concurrency::test]
async fn live_concurrent_requests_survive_cleanup_and_reply_out_of_order() {
    let context = TestContext::new().await;
    let remote_actor_instance = RemoteActor;
    let mut state = context.state();
    let request_count = PENDING_REQUEST_CLEANUP_BUDGET * 3 + 1;
    let mut receivers = Vec::with_capacity(request_count);

    for _ in 0..request_count {
        let (sender, receiver) = ractor::concurrency::oneshot::<Vec<u8>>();
        remote_actor_instance
            .handle_serialized(
                context.remote_actor.clone(),
                serialized_call(sender.into()),
                &mut state,
            )
            .await
            .unwrap();
        receivers.push(receiver);
    }
    assert_eq!(state.pending_requests.len(), request_count);

    for tag in (1..=request_count as u64).rev() {
        remote_actor_instance
            .handle_serialized(
                context.remote_actor.clone(),
                SerializedMessage::CallReply(tag, tag.to_be_bytes().to_vec()),
                &mut state,
            )
            .await
            .unwrap();
    }

    for (index, receiver) in receivers.into_iter().enumerate() {
        assert_eq!(
            receiver.await.unwrap(),
            (index as u64 + 1).to_be_bytes().to_vec()
        );
    }
    assert!(state.pending_requests.is_empty());

    context.shutdown().await;
}

#[ractor::concurrency::test]
async fn late_reply_after_request_cleanup_is_ignored() {
    let context = TestContext::new().await;
    let remote_actor_instance = RemoteActor;
    let mut state = context.state();
    let (sender, receiver) = ractor::concurrency::oneshot::<Vec<u8>>();
    drop(receiver);

    remote_actor_instance
        .handle_serialized(
            context.remote_actor.clone(),
            serialized_call(sender.into()),
            &mut state,
        )
        .await
        .unwrap();
    assert!(state.pending_requests.contains_key(&1));

    remote_actor_instance
        .handle_serialized(context.remote_actor.clone(), serialized_cast(), &mut state)
        .await
        .unwrap();
    assert!(state.pending_requests.is_empty());

    remote_actor_instance
        .handle_serialized(
            context.remote_actor.clone(),
            SerializedMessage::CallReply(1, vec![9, 9, 9]),
            &mut state,
        )
        .await
        .unwrap();
    assert!(state.pending_requests.is_empty());

    context.shutdown().await;
}
