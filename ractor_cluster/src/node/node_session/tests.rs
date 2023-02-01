// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};

use ractor::concurrency::sleep;

use super::*;

struct DummyNodeServer;
#[async_trait::async_trait]
impl Actor for DummyNodeServer {
    type Msg = crate::node::NodeServerMessage;
    type State = ();
    async fn pre_start(&self, _myself: ActorRef<Self>) -> Result<Self::State, ActorProcessingErr> {
        Ok(())
    }
    async fn handle(
        &self,
        _myself: ActorRef<Self>,
        message: Self::Msg,
        _state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        if let crate::node::NodeServerMessage::CheckSession { reply, peer_name } = message {
            match peer_name.name.as_str() {
                "other_continues" => {
                    let _ = reply.send(crate::node::SessionCheckReply::OtherConnectionContinues);
                }
                "this_continues" => {
                    let _ = reply.send(crate::node::SessionCheckReply::ThisConnectionContinues);
                }
                "duplicate" => {
                    let _ = reply.send(crate::node::SessionCheckReply::DuplicateConnection);
                }
                _ => {
                    let _ = reply.send(crate::node::SessionCheckReply::NoOtherConnection);
                }
            }
        }
        Ok(())
    }
}

struct DummyNodeSession;
#[async_trait::async_trait]
impl Actor for DummyNodeSession {
    type Msg = crate::node::NodeSessionMessage;
    type State = ();
    async fn pre_start(&self, _myself: ActorRef<Self>) -> Result<Self::State, ActorProcessingErr> {
        Ok(())
    }
    async fn handle(
        &self,
        _myself: ActorRef<Self>,
        _message: Self::Msg,
        _state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        Ok(())
    }
}

#[ractor::concurrency::test]
async fn node_session_server_auth_success() {
    let (dummy_server, dummy_shandle) = Actor::spawn(None, DummyNodeServer)
        .await
        .expect("Failed to start dummy node server");
    let (dummy_session, dummy_chandle) = Actor::spawn(None, DummyNodeSession)
        .await
        .expect("Failed to start dummy node session");

    let server_ref: ActorRef<super::NodeServer> = dummy_server.get_cell().into();
    let session_ref: ActorRef<super::NodeSession> = dummy_session.get_cell().into();

    // Do NOT do what we do here, converting the ActorRef -> ActorCell -> ActorRef on wrong struct but with correct message type. This will work
    // but is very dangerous outside of tests
    let session = NodeSession {
        cookie: "cookie".to_string(),
        is_server: true,
        node_id: 1,
        node_name: auth_protocol::NameMessage {
            name: "myself".to_string(),
            flags: Some(auth_protocol::NodeFlags { version: 1 }),
        },
        node_server: server_ref.clone(),
    };

    let mut state = NodeSessionState {
        auth: AuthenticationState::AsServer(auth::ServerAuthenticationProcess::init()),
        local_addr: None,
        peer_addr: None,
        name: None,
        remote_actors: HashMap::new(),
        tcp: None,
    };

    // Client sends their name
    let message = super::auth_protocol::AuthenticationMessage {
        msg: Some(auth_protocol::authentication_message::Msg::Name(
            auth_protocol::NameMessage {
                name: "peer".to_string(),
                flags: Some(auth_protocol::NodeFlags { version: 1 }),
            },
        )),
    };
    session
        .handle_auth(&mut state, message, session_ref.clone())
        .await;

    // we should have sent back BOTH the status + the challenge from the server (if no match already there)
    assert!(matches!(
        state.auth,
        AuthenticationState::AsServer(
            auth::ServerAuthenticationProcess::WaitingOnClientChallengeReply(_, _)
        )
    ));

    let digest = if let AuthenticationState::AsServer(
        auth::ServerAuthenticationProcess::WaitingOnClientChallengeReply(_, d),
    ) = &state.auth
    {
        *d
    } else {
        panic!("C'est impossible!");
    };

    // Client sends their reply to the challenge
    let message = super::auth_protocol::AuthenticationMessage {
        msg: Some(auth_protocol::authentication_message::Msg::ClientChallenge(
            auth_protocol::ChallengeReply {
                challenge: 123,
                digest: digest.to_vec(),
            },
        )),
    };
    session
        .handle_auth(&mut state, message, session_ref.clone())
        .await;

    // All is well
    assert!(matches!(
        state.auth,
        AuthenticationState::AsServer(auth::ServerAuthenticationProcess::Ok(_))
    ));

    // cleanup
    dummy_server.stop(None);
    dummy_session.stop(None);
    dummy_shandle.await.unwrap();
    dummy_chandle.await.unwrap();
}

#[ractor::concurrency::test]
async fn node_session_server_auth_session_state_failures() {
    let (dummy_server, dummy_shandle) = Actor::spawn(None, DummyNodeServer)
        .await
        .expect("Failed to start dummy node server");
    let (dummy_session, dummy_chandle) = Actor::spawn(None, DummyNodeSession)
        .await
        .expect("Failed to start dummy node session");

    let server_ref: ActorRef<super::NodeServer> = dummy_server.get_cell().into();
    let session_ref: ActorRef<super::NodeSession> = dummy_session.get_cell().into();

    // Do NOT do what we do here, converting the ActorRef -> ActorCell -> ActorRef on wrong struct but with correct message type. This will work
    // but is very dangerous outside of tests
    let session = NodeSession {
        cookie: "cookie".to_string(),
        is_server: true,
        node_id: 1,
        node_name: auth_protocol::NameMessage {
            name: "myself".to_string(),
            flags: Some(auth_protocol::NodeFlags { version: 1 }),
        },
        node_server: server_ref.clone(),
    };

    let mut state = NodeSessionState {
        auth: AuthenticationState::AsServer(auth::ServerAuthenticationProcess::init()),
        local_addr: None,
        peer_addr: None,
        name: None,
        remote_actors: HashMap::new(),
        tcp: None,
    };

    // Other session continues, this one dies
    let message = super::auth_protocol::AuthenticationMessage {
        msg: Some(auth_protocol::authentication_message::Msg::Name(
            auth_protocol::NameMessage {
                name: "other_continues".to_string(),
                flags: Some(auth_protocol::NodeFlags { version: 1 }),
            },
        )),
    };
    session
        .handle_auth(&mut state, message, session_ref.clone())
        .await;
    assert!(matches!(
        state.auth,
        AuthenticationState::AsServer(auth::ServerAuthenticationProcess::Close)
    ));

    // Other session dies, this one continues
    state.auth = AuthenticationState::AsServer(auth::ServerAuthenticationProcess::init());
    let message = super::auth_protocol::AuthenticationMessage {
        msg: Some(auth_protocol::authentication_message::Msg::Name(
            auth_protocol::NameMessage {
                name: "this_continues".to_string(),
                flags: Some(auth_protocol::NodeFlags { version: 1 }),
            },
        )),
    };
    session
        .handle_auth(&mut state, message, session_ref.clone())
        .await;
    assert!(matches!(
        state.auth,
        AuthenticationState::AsServer(
            auth::ServerAuthenticationProcess::WaitingOnClientChallengeReply(_, _)
        )
    ));

    // TODO: The duplicate session handling. The client needs to figure out what it's "state" is in this case

    // cleanup
    dummy_server.stop(None);
    dummy_session.stop(None);
    dummy_shandle.await.unwrap();
    dummy_chandle.await.unwrap();
}

#[ractor::concurrency::test]
async fn node_session_handle_node_msg() {
    let casts = Arc::new(AtomicU8::new(0));
    let calls = Arc::new(AtomicU8::new(0));
    let call_replies = Arc::new(AtomicU8::new(0));
    struct DummyRemoteActor {
        casts: Arc<AtomicU8>,
        calls: Arc<AtomicU8>,
        call_replies: Arc<AtomicU8>,
    }

    #[async_trait::async_trait]
    impl Actor for DummyRemoteActor {
        type Msg = crate::remote_actor::RemoteActorMessage;
        type State = ();
        async fn pre_start(
            &self,
            _myself: ActorRef<Self>,
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }

        async fn handle_serialized(
            &self,
            _myself: ActorRef<Self>,
            message: SerializedMessage,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            match message {
                SerializedMessage::Cast { .. } => {
                    println!("Received cast");
                    self.casts.fetch_add(1, Ordering::Relaxed);
                }
                SerializedMessage::Call { .. } => {
                    println!("Received call");
                    self.calls.fetch_add(1, Ordering::Relaxed);
                }
                SerializedMessage::CallReply(_, _) => {
                    println!("Received reply");
                    self.call_replies.fetch_add(1, Ordering::Relaxed);
                }
            }
            Ok(())
        }
    }

    let (dummy_server, dummy_shandle) = Actor::spawn(None, DummyNodeServer)
        .await
        .expect("Failed to start dummy node server");
    let (dummy_session, dummy_chandle) = Actor::spawn(None, DummyNodeSession)
        .await
        .expect("Failed to start dummy node session");

    let server_ref: ActorRef<super::NodeServer> = dummy_server.get_cell().into();
    let session_ref: ActorRef<super::NodeSession> = dummy_session.get_cell().into();

    let test_pid = ActorId::Remote {
        node_id: 1,
        pid: 123,
    };
    let (test_actor, _) = ractor::ActorRuntime::spawn_linked_remote(
        Some("dummy_remote_actor".to_string()),
        DummyRemoteActor {
            calls: calls.clone(),
            casts: casts.clone(),
            call_replies: call_replies.clone(),
        },
        test_pid,
        session_ref.get_cell(),
    )
    .await
    .expect("Failed to spawn test remote actor");

    // Do NOT do what we do here, converting the ActorRef -> ActorCell -> ActorRef on wrong struct but with correct message type. This will work
    // but is very dangerous outside of tests
    let session = NodeSession {
        cookie: "cookie".to_string(),
        is_server: true,
        node_id: 1,
        node_name: auth_protocol::NameMessage {
            name: "myself".to_string(),
            flags: Some(auth_protocol::NodeFlags { version: 1 }),
        },
        node_server: server_ref.clone(),
    };

    let mut state = NodeSessionState {
        auth: AuthenticationState::AsServer(auth::ServerAuthenticationProcess::Ok([0u8; 32])),
        local_addr: None,
        peer_addr: None,
        name: None,
        remote_actors: HashMap::new(),
        tcp: None,
    };
    // add the "remote" actor
    state
        .remote_actors
        .insert(test_pid.pid(), test_actor.get_cell().into());

    // ***** The following doesn't work due to remote actor id's vs local ids. We can't spawn a specific local id
    // so we can only spawn a remote one, which at that point the actor won't be in the PID registry and therefore
    // won't be able to be looked up by its pid

    // session.handle_node(
    //     &mut state,
    //     super::node_protocol::NodeMessage {
    //         msg: Some(node_protocol::node_message::Msg::Cast(
    //             node_protocol::Cast {
    //                 to: test_pid,
    //                 what: vec![],
    //                 variant: "Call".to_string(),
    //             },
    //         )),
    //     },
    //     session_ref.clone(),
    // );

    // sleep(Duration::from_millis(100)).await;
    // assert_eq!(1, casts.load(Ordering::Relaxed));

    // session.handle_node(
    //     &mut state,
    //     super::node_protocol::NodeMessage {
    //         msg: Some(node_protocol::node_message::Msg::Call(
    //             node_protocol::Call { to: test_pid, what: vec![], tag: 1, timeout_ms: None, variant: "Call".to_string() }
    //         )),
    //     },
    //     session_ref.clone(),
    // );
    // sleep(Duration::from_millis(100)).await;
    // assert_eq!(1, calls.load(Ordering::Relaxed));

    session.handle_node(
        &mut state,
        super::node_protocol::NodeMessage {
            msg: Some(node_protocol::node_message::Msg::Reply(
                node_protocol::CallReply {
                    to: test_pid.pid(),
                    tag: 1,
                    what: vec![],
                },
            )),
        },
        session_ref.clone(),
    );
    sleep(Duration::from_millis(100)).await;
    assert_eq!(1, call_replies.load(Ordering::Relaxed));

    // cleanup
    dummy_server.stop(None);
    dummy_session.stop(None);
    dummy_shandle.await.unwrap();
    dummy_chandle.await.unwrap();
}
