// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};

use ractor::concurrency::sleep;

use crate::node::NodeConnectionMode;
use crate::NodeSessionMessage;

use super::*;

struct DummyNodeServer;
#[ractor::async_trait]
impl Actor for DummyNodeServer {
    type Msg = crate::node::NodeServerMessage;
    type State = ();
    type Arguments = ();
    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        _: (),
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(())
    }
    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
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
#[ractor::async_trait]
impl Actor for DummyNodeSession {
    type Msg = crate::node::NodeSessionMessage;
    type State = ();
    type Arguments = ();
    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        _: (),
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(())
    }
    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        _message: Self::Msg,
        _state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        Ok(())
    }
}

#[ractor::concurrency::test]
async fn node_sesison_client_auth_success() {
    let (dummy_server, dummy_shandle) = Actor::spawn(None, DummyNodeServer, ())
        .await
        .expect("Failed to start dummy node server");
    let (dummy_session, dummy_chandle) = Actor::spawn(None, DummyNodeSession, ())
        .await
        .expect("Failed to start dummy node session");

    let server_ref: ActorRef<super::NodeServerMessage> = dummy_server.get_cell().into();
    let session_ref: ActorRef<NodeSessionMessage> = dummy_session.get_cell().into();

    // Do NOT do what we do here, converting the ActorRef -> ActorCell -> ActorRef on wrong struct but with correct message type. This will work
    // but is very dangerous outside of tests
    let session = NodeSession {
        cookie: "cookie".to_string(),
        is_server: true,
        node_id: 1,
        this_node_name: auth_protocol::NameMessage {
            name: "myself".to_string(),
            flags: Some(auth_protocol::NodeFlags { version: 1 }),
            connection_string: "localhost:123".to_string(),
        },
        node_server: server_ref.clone(),
        connection_mode: NodeConnectionMode::Isolated,
    };

    let mut state = NodeSessionState {
        auth: AuthenticationState::AsClient(auth::ClientAuthenticationProcess::init()),
        local_addr: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
        peer_addr: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
        name: None,
        remote_actors: HashMap::new(),
        tcp: None,
    };

    // Client sends their name, Server responds with Ok
    let message = super::auth_protocol::AuthenticationMessage {
        msg: Some(auth_protocol::authentication_message::Msg::ServerStatus(
            auth_protocol::ServerStatus {
                status: auth_protocol::server_status::Status::Ok as i32,
            },
        )),
    };
    session
        .handle_auth(&mut state, message, session_ref.clone())
        .await;
    assert!(matches!(
        state.auth,
        AuthenticationState::AsClient(
            auth::ClientAuthenticationProcess::WaitingForServerChallenge(_)
        )
    ));

    // Client sends their name, Server responds with OkSimultaneous
    state.auth = AuthenticationState::AsClient(auth::ClientAuthenticationProcess::init());
    let message = super::auth_protocol::AuthenticationMessage {
        msg: Some(auth_protocol::authentication_message::Msg::ServerStatus(
            auth_protocol::ServerStatus {
                status: auth_protocol::server_status::Status::OkSimultaneous as i32,
            },
        )),
    };
    session
        .handle_auth(&mut state, message, session_ref.clone())
        .await;
    assert!(matches!(
        state.auth,
        AuthenticationState::AsClient(
            auth::ClientAuthenticationProcess::WaitingForServerChallenge(_)
        )
    ));

    state.auth = AuthenticationState::AsClient(auth::ClientAuthenticationProcess::init());
    let message = super::auth_protocol::AuthenticationMessage {
        msg: Some(auth_protocol::authentication_message::Msg::ServerStatus(
            auth_protocol::ServerStatus {
                status: auth_protocol::server_status::Status::Alive as i32,
            },
        )),
    };
    session
        .handle_auth(&mut state, message, session_ref.clone())
        .await;
    assert!(matches!(
        state.auth,
        AuthenticationState::AsClient(
            auth::ClientAuthenticationProcess::WaitingForServerChallenge(_)
        )
    ));

    // Server sends it's challenge in response to our reply
    let message = super::auth_protocol::AuthenticationMessage {
        msg: Some(auth_protocol::authentication_message::Msg::ServerChallenge(
            auth_protocol::Challenge {
                name: "Something".to_string(),
                flags: Some(auth_protocol::NodeFlags { version: 1 }),
                connection_string: "localhost:123".to_string(),
                challenge: 123,
            },
        )),
    };
    session
        .handle_auth(&mut state, message, session_ref.clone())
        .await;
    let digest = if let AuthenticationState::AsClient(
        auth::ClientAuthenticationProcess::WaitingForServerChallengeAck(
            _challenge_msg,
            _server_digest,
            _challenge,
            expected_digest,
        ),
    ) = &state.auth
    {
        *expected_digest
    } else {
        panic!("C'est impossible!");
    };

    // Server replies to the client's challenge
    let message = super::auth_protocol::AuthenticationMessage {
        msg: Some(auth_protocol::authentication_message::Msg::ServerAck(
            auth_protocol::ChallengeAck {
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
        AuthenticationState::AsClient(auth::ClientAuthenticationProcess::Ok)
    ));

    // cleanup
    dummy_server.stop(None);
    dummy_session.stop(None);
    dummy_shandle.await.unwrap();
    dummy_chandle.await.unwrap();
}

#[ractor::concurrency::test]
async fn node_session_client_auth_session_state_failures() {
    let (dummy_server, dummy_shandle) = Actor::spawn(None, DummyNodeServer, ())
        .await
        .expect("Failed to start dummy node server");
    let (dummy_session, dummy_chandle) = Actor::spawn(None, DummyNodeSession, ())
        .await
        .expect("Failed to start dummy node session");

    let server_ref: ActorRef<super::NodeServerMessage> = dummy_server.get_cell().into();
    let session_ref: ActorRef<NodeSessionMessage> = dummy_session.get_cell().into();

    // Do NOT do what we do here, converting the ActorRef -> ActorCell -> ActorRef on wrong struct but with correct message type. This will work
    // but is very dangerous outside of tests
    let session = NodeSession {
        cookie: "cookie".to_string(),
        is_server: true,
        node_id: 1,
        this_node_name: auth_protocol::NameMessage {
            name: "myself".to_string(),
            flags: Some(auth_protocol::NodeFlags { version: 1 }),
            connection_string: "localhost:123".to_string(),
        },
        node_server: server_ref.clone(),
        connection_mode: NodeConnectionMode::Isolated,
    };

    let mut state = NodeSessionState {
        auth: AuthenticationState::AsClient(auth::ClientAuthenticationProcess::init()),
        local_addr: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
        peer_addr: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
        name: None,
        remote_actors: HashMap::new(),
        tcp: None,
    };

    // Client sends their name, Server responds with Ok
    state.auth = AuthenticationState::AsClient(auth::ClientAuthenticationProcess::init());
    let message = super::auth_protocol::AuthenticationMessage {
        msg: Some(auth_protocol::authentication_message::Msg::ServerStatus(
            auth_protocol::ServerStatus {
                status: auth_protocol::server_status::Status::NotOk as i32,
            },
        )),
    };
    session
        .handle_auth(&mut state, message, session_ref.clone())
        .await;
    assert!(matches!(
        state.auth,
        AuthenticationState::AsClient(auth::ClientAuthenticationProcess::Close)
    ));

    state.auth = AuthenticationState::AsClient(auth::ClientAuthenticationProcess::init());
    let message = super::auth_protocol::AuthenticationMessage {
        msg: Some(auth_protocol::authentication_message::Msg::ServerStatus(
            auth_protocol::ServerStatus {
                status: auth_protocol::server_status::Status::NotAllowed as i32,
            },
        )),
    };
    session
        .handle_auth(&mut state, message, session_ref.clone())
        .await;
    assert!(matches!(
        state.auth,
        AuthenticationState::AsClient(auth::ClientAuthenticationProcess::Close)
    ));

    // start from server status
    state.auth = AuthenticationState::AsClient(
        auth::ClientAuthenticationProcess::WaitingForServerChallenge(auth_protocol::ServerStatus {
            status: auth_protocol::server_status::Status::Ok as i32,
        }),
    );
    // invalid out-of-order msg
    let message = super::auth_protocol::AuthenticationMessage {
        msg: Some(auth_protocol::authentication_message::Msg::ServerStatus(
            auth_protocol::ServerStatus {
                status: auth_protocol::server_status::Status::OkSimultaneous as i32,
            },
        )),
    };
    session
        .handle_auth(&mut state, message, session_ref.clone())
        .await;
    assert!(matches!(
        state.auth,
        AuthenticationState::AsClient(auth::ClientAuthenticationProcess::Close)
    ));

    // start from waiting on server ack
    state.auth = AuthenticationState::AsClient(
        auth::ClientAuthenticationProcess::WaitingForServerChallengeAck(
            auth_protocol::Challenge {
                name: "something".to_string(),
                flags: Some(auth_protocol::NodeFlags { version: 1 }),
                connection_string: "localhost:123".to_string(),
                challenge: 123,
            },
            [0u8; 32],
            123,
            [0u8; 32],
        ),
    );
    // invalid out-of-order msg
    let message = super::auth_protocol::AuthenticationMessage {
        msg: Some(auth_protocol::authentication_message::Msg::ServerStatus(
            auth_protocol::ServerStatus {
                status: auth_protocol::server_status::Status::OkSimultaneous as i32,
            },
        )),
    };
    session
        .handle_auth(&mut state, message, session_ref.clone())
        .await;
    assert!(matches!(
        state.auth,
        AuthenticationState::AsClient(auth::ClientAuthenticationProcess::Close)
    ));

    // cleanup
    dummy_server.stop(None);
    dummy_session.stop(None);
    dummy_shandle.await.unwrap();
    dummy_chandle.await.unwrap();
}

#[ractor::concurrency::test]
async fn node_session_server_auth_success() {
    let (dummy_server, dummy_shandle) = Actor::spawn(None, DummyNodeServer, ())
        .await
        .expect("Failed to start dummy node server");
    let (dummy_session, dummy_chandle) = Actor::spawn(None, DummyNodeSession, ())
        .await
        .expect("Failed to start dummy node session");

    let server_ref: ActorRef<super::NodeServerMessage> = dummy_server.get_cell().into();
    let session_ref: ActorRef<NodeSessionMessage> = dummy_session.get_cell().into();

    // Do NOT do what we do here, converting the ActorRef -> ActorCell -> ActorRef on wrong struct but with correct message type. This will work
    // but is very dangerous outside of tests
    let session = NodeSession {
        cookie: "cookie".to_string(),
        is_server: true,
        node_id: 1,
        this_node_name: auth_protocol::NameMessage {
            name: "myself".to_string(),
            flags: Some(auth_protocol::NodeFlags { version: 1 }),
            connection_string: "localhost:123".to_string(),
        },
        node_server: server_ref.clone(),
        connection_mode: NodeConnectionMode::Isolated,
    };

    // let addr = SocketAddr::
    let mut state = NodeSessionState {
        auth: AuthenticationState::AsServer(auth::ServerAuthenticationProcess::init()),
        local_addr: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
        peer_addr: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
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
                connection_string: "localhost:123".to_string(),
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
    let (dummy_server, dummy_shandle) = Actor::spawn(None, DummyNodeServer, ())
        .await
        .expect("Failed to start dummy node server");
    let (dummy_session, dummy_chandle) = Actor::spawn(None, DummyNodeSession, ())
        .await
        .expect("Failed to start dummy node session");

    let server_ref: ActorRef<super::NodeServerMessage> = dummy_server.get_cell().into();
    let session_ref: ActorRef<NodeSessionMessage> = dummy_session.get_cell().into();

    // Do NOT do what we do here, converting the ActorRef -> ActorCell -> ActorRef on wrong struct but with correct message type. This will work
    // but is very dangerous outside of tests
    let session = NodeSession {
        cookie: "cookie".to_string(),
        is_server: true,
        node_id: 1,
        this_node_name: auth_protocol::NameMessage {
            name: "myself".to_string(),
            flags: Some(auth_protocol::NodeFlags { version: 1 }),
            connection_string: "localhost:123".to_string(),
        },
        node_server: server_ref.clone(),
        connection_mode: NodeConnectionMode::Isolated,
    };

    let mut state = NodeSessionState {
        auth: AuthenticationState::AsServer(auth::ServerAuthenticationProcess::init()),
        local_addr: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
        peer_addr: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
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
                connection_string: "localhost:123".to_string(),
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
                connection_string: "localhost:123".to_string(),
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

    #[ractor::async_trait]
    impl Actor for DummyRemoteActor {
        type Msg = crate::remote_actor::RemoteActorMessage;
        type State = ();
        type Arguments = ();
        async fn pre_start(
            &self,
            _myself: ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }

        async fn handle_serialized(
            &self,
            _myself: ActorRef<Self::Msg>,
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

    let (dummy_server, dummy_shandle) = Actor::spawn(None, DummyNodeServer, ())
        .await
        .expect("Failed to start dummy node server");
    let (dummy_session, dummy_chandle) = Actor::spawn(None, DummyNodeSession, ())
        .await
        .expect("Failed to start dummy node session");

    let server_ref: ActorRef<super::NodeServerMessage> = dummy_server.get_cell().into();
    let session_ref: ActorRef<NodeSessionMessage> = dummy_session.get_cell().into();

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
        (),
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
        this_node_name: auth_protocol::NameMessage {
            name: "myself".to_string(),
            flags: Some(auth_protocol::NodeFlags { version: 1 }),
            connection_string: "localhost:123".to_string(),
        },
        node_server: server_ref.clone(),
        connection_mode: NodeConnectionMode::Isolated,
    };

    let mut state = NodeSessionState {
        auth: AuthenticationState::AsServer(auth::ServerAuthenticationProcess::Ok([0u8; 32])),
        local_addr: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
        peer_addr: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
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

#[ractor::concurrency::test]
async fn node_session_handle_control() {
    let (dummy_server, dummy_shandle) = Actor::spawn(None, DummyNodeServer, ())
        .await
        .expect("Failed to start dummy node server");
    let (dummy_session, dummy_chandle) = Actor::spawn(None, DummyNodeSession, ())
        .await
        .expect("Failed to start dummy node session");

    let server_ref: ActorRef<super::NodeServerMessage> = dummy_server.get_cell().into();
    let session_ref: ActorRef<NodeSessionMessage> = dummy_session.get_cell().into();

    // Do NOT do what we do here, converting the ActorRef -> ActorCell -> ActorRef on wrong struct but with correct message type. This will work
    // but is very dangerous outside of tests
    let session = NodeSession {
        cookie: "cookie".to_string(),
        is_server: true,
        node_id: 1,
        this_node_name: auth_protocol::NameMessage {
            name: "myself".to_string(),
            flags: Some(auth_protocol::NodeFlags { version: 1 }),
            connection_string: "localhost:123".to_string(),
        },
        node_server: server_ref.clone(),
        connection_mode: NodeConnectionMode::Isolated,
    };

    let mut state = NodeSessionState {
        auth: AuthenticationState::AsClient(auth::ClientAuthenticationProcess::Ok),
        local_addr: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
        peer_addr: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
        name: None,
        remote_actors: HashMap::new(),
        tcp: None,
    };

    // check spawn creates a remote actor
    session
        .handle_control(
            &mut state,
            control_protocol::ControlMessage {
                msg: Some(control_protocol::control_message::Msg::Spawn(
                    control_protocol::Spawn {
                        actors: vec![control_protocol::Actor {
                            name: None,
                            pid: 42,
                        }],
                    },
                )),
            },
            session_ref.clone(),
        )
        .await
        .expect("Failed to process control message");
    assert_eq!(1, state.remote_actors.len());

    // check terminate cleans up a remote actor
    session
        .handle_control(
            &mut state,
            control_protocol::ControlMessage {
                msg: Some(control_protocol::control_message::Msg::Terminate(
                    control_protocol::Terminate { ids: vec![42] },
                )),
            },
            session_ref.clone(),
        )
        .await
        .expect("Failed to process control message");
    assert_eq!(0, state.remote_actors.len());

    let scope_name = "node_session_test_scope";
    let group_name = "node_session_handle_control";

    // check pg join spawns + joins to a pg group
    session
        .handle_control(
            &mut state,
            control_protocol::ControlMessage {
                msg: Some(control_protocol::control_message::Msg::PgJoin(
                    control_protocol::PgJoin {
                        scope: scope_name.to_string(),
                        group: group_name.to_string(),
                        actors: vec![control_protocol::Actor {
                            name: None,
                            pid: 43,
                        }],
                    },
                )),
            },
            session_ref.clone(),
        )
        .await
        .expect("Failed to process control message");
    assert_eq!(1, state.remote_actors.len());
    let id_set = ractor::pg::get_scoped_members(&scope_name.to_string(), &group_name.to_string())
        .into_iter()
        .map(|a| a.get_id())
        .collect::<HashSet<_>>();
    assert!(id_set.contains(&ActorId::Remote {
        node_id: 1,
        pid: 43
    }));

    let id_set = ractor::pg::get_members(&group_name.to_string())
        .into_iter()
        .map(|a| a.get_id())
        .collect::<HashSet<_>>();
    assert!(!id_set.contains(&ActorId::Remote {
        node_id: 1,
        pid: 43
    }));

    // check pg leave leaves the pg group
    session
        .handle_control(
            &mut state,
            control_protocol::ControlMessage {
                msg: Some(control_protocol::control_message::Msg::PgLeave(
                    control_protocol::PgLeave {
                        scope: scope_name.to_string(),
                        group: group_name.to_string(),
                        actors: vec![control_protocol::Actor {
                            name: None,
                            pid: 43,
                        }],
                    },
                )),
            },
            session_ref.clone(),
        )
        .await
        .expect("Failed to process control message");
    assert_eq!(1, state.remote_actors.len());
    let id_set = ractor::pg::get_scoped_members(&scope_name.to_string(), &group_name.to_string())
        .into_iter()
        .map(|a| a.get_id())
        .collect::<HashSet<_>>();
    assert!(!id_set.contains(&ActorId::Remote {
        node_id: 1,
        pid: 43
    }));
    let id_set = ractor::pg::get_members(&group_name.to_string())
        .into_iter()
        .map(|a| a.get_id())
        .collect::<HashSet<_>>();
    assert!(!id_set.contains(&ActorId::Remote {
        node_id: 1,
        pid: 43
    }));
    // cleanup that test actor
    session
        .handle_control(
            &mut state,
            control_protocol::ControlMessage {
                msg: Some(control_protocol::control_message::Msg::Terminate(
                    control_protocol::Terminate { ids: vec![43] },
                )),
            },
            session_ref.clone(),
        )
        .await
        .expect("Failed to process control message");

    // TODO: ping? for healthchecks

    // cleanup
    dummy_server.stop(None);
    dummy_session.stop(None);
    dummy_shandle.await.unwrap();
    dummy_chandle.await.unwrap();
}
