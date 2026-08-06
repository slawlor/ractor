// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;
use std::time::SystemTime;

use ractor::concurrency::sleep;

use super::*;
use crate::node::NodeConnectionMode;
use crate::NodeSessionMessage;

struct DummyNodeServer;
#[cfg_attr(feature = "async-trait", ractor::async_trait)]
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
        match message {
            crate::node::NodeServerMessage::CheckSession { reply, peer_name } => {
                match peer_name.name.as_str() {
                    "other_continues" => {
                        let _ =
                            reply.send(crate::node::SessionCheckReply::OtherConnectionContinues);
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
            _ => {}
        }
        Ok(())
    }
}

struct DummyNodeSession;
#[cfg_attr(feature = "async-trait", ractor::async_trait)]
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

    async fn handle_supervisor_evt(
        &self,
        _myself: ActorRef<Self::Msg>,
        _message: ractor::SupervisionEvent,
        _state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        Ok(())
    }
}

struct PingCounter {
    pings: Arc<AtomicU8>,
}

#[cfg_attr(feature = "async-trait", ractor::async_trait)]
impl Actor for PingCounter {
    type Msg = SessionMessage;
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
        if matches!(
            message,
            SessionMessage::Send(crate::protocol::NetworkMessage {
                message: Some(crate::protocol::meta::network_message::Message::Control(
                    control_protocol::ControlMessage {
                        msg: Some(control_protocol::control_message::Msg::Ping(_)),
                    },
                )),
            })
        ) {
            self.pings.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }
}

fn authenticated_state(tcp: Option<ActorRef<SessionMessage>>) -> NodeSessionState {
    NodeSessionState {
        auth: AuthenticationState::AsClient(auth::ClientAuthenticationProcess::Ok),
        ready: ReadyState::Open,
        local_addr: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
        peer_addr: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
        name: None,
        connection_id: 0,
        remote_actors: HashMap::new(),
        advertised_local_pids: HashSet::new(),
        tcp,
        ping_task: None,
        epoch: Instant::now(),
        pong_warnings: PongWarnings::default(),
    }
}

#[test]
fn malformed_pongs_are_ignored() {
    let mut state = authenticated_state(None);
    let future_timestamp = prost_types::Timestamp::from(
        SystemTime::UNIX_EPOCH + state.epoch.elapsed() + Duration::from_secs(60),
    );

    for timestamp in [
        None,
        Some(prost_types::Timestamp {
            seconds: 0,
            nanos: 1_000_000_000,
        }),
        Some(prost_types::Timestamp {
            seconds: -1,
            nanos: 0,
        }),
        Some(future_timestamp),
    ] {
        assert!(state.pong_latency(timestamp.clone()).is_err());
        state.handle_pong(control_protocol::Pong { timestamp });
    }

    assert!(state.pong_warnings.invalid);
    assert!(state.ping_task.is_none());
}

#[ractor::concurrency::test]
async fn ping_loop_is_single_and_aborted_with_session_state() {
    let pings = Arc::new(AtomicU8::new(0));
    let (tcp, tcp_handle) = Actor::spawn(
        None,
        PingCounter {
            pings: pings.clone(),
        },
        (),
    )
    .await
    .unwrap();
    let mut state = authenticated_state(Some(tcp.clone()));
    let task_capture = Arc::new(AtomicBool::new(false));
    let weak_task_capture = Arc::downgrade(&task_capture);

    assert!(state.start_ping_loop_with_delay(move || {
        task_capture.store(true, Ordering::Relaxed);
        Duration::from_millis(5)
    }));
    assert!(!state.start_ping_loop_with_delay(|| Duration::from_millis(1)));

    ractor::concurrency::timeout(Duration::from_millis(250), async {
        while pings.load(Ordering::Relaxed) < 2 {
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();

    drop(state);
    ractor::concurrency::timeout(Duration::from_millis(250), async {
        while weak_task_capture.upgrade().is_some() {
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();

    let final_count = pings.load(Ordering::Relaxed);
    sleep(Duration::from_millis(20)).await;
    assert_eq!(final_count, pings.load(Ordering::Relaxed));

    tcp.stop(None);
    tcp_handle.await.unwrap();
}

struct RemotableMessage;

impl ractor::Message for RemotableMessage {
    fn serializable() -> bool {
        true
    }

    fn deserialize(message: SerializedMessage) -> Result<Self, ractor::message::BoxedDowncastErr> {
        match message {
            SerializedMessage::Cast { .. } | SerializedMessage::Call { .. } => Ok(Self),
            SerializedMessage::CallReply(_, _) => Err(ractor::message::BoxedDowncastErr),
        }
    }
}

struct RemotableCounter {
    received: Arc<AtomicU8>,
}

#[cfg_attr(feature = "async-trait", ractor::async_trait)]
impl Actor for RemotableCounter {
    type Msg = RemotableMessage;
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
        self.received.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

#[ractor::concurrency::test]
async fn inbound_messages_require_an_advertised_remotable_pid() {
    let (server, server_handle) = Actor::spawn(None, DummyNodeServer, ()).await.unwrap();
    let (session_actor, session_handle) = Actor::spawn(None, DummyNodeSession, ()).await.unwrap();
    let received = Arc::new(AtomicU8::new(0));
    let (target, target_handle) = Actor::spawn(
        None,
        RemotableCounter {
            received: received.clone(),
        },
        (),
    )
    .await
    .unwrap();
    assert!(target.supports_remoting());

    let session = NodeSession {
        cookie: "cookie".to_string(),
        is_server: true,
        node_id: 1,
        this_node_name: auth_protocol::NameMessage {
            name: "myself".to_string(),
            flags: Some(auth_protocol::NodeFlags { version: 1 }),
            connection_string: "localhost:123".to_string(),
            connection_id: 0,
        },
        node_server: server.get_cell().into(),
        connection_mode: NodeConnectionMode::Isolated,
        max_inbound_frame_size: super::super::DEFAULT_MAX_INBOUND_FRAME_SIZE,
        connection_id: 0,
    };
    let mut state = authenticated_state(None);
    let pid = target.get_id().pid();
    let session_ref: ActorRef<NodeSessionMessage> = session_actor.get_cell().into();

    session.handle_node(
        &mut state,
        node_protocol::NodeMessage {
            msg: Some(node_protocol::node_message::Msg::Cast(
                node_protocol::Cast {
                    to: pid,
                    what: vec![],
                    variant: "Cast".to_string(),
                    metadata: None,
                },
            )),
        },
        session_ref.clone(),
    );
    sleep(Duration::from_millis(20)).await;
    assert_eq!(received.load(Ordering::Relaxed), 0);

    state.advertised_local_pids.insert(pid);
    session.handle_node(
        &mut state,
        node_protocol::NodeMessage {
            msg: Some(node_protocol::node_message::Msg::Cast(
                node_protocol::Cast {
                    to: pid,
                    what: vec![],
                    variant: "Cast".to_string(),
                    metadata: None,
                },
            )),
        },
        session_ref.clone(),
    );
    sleep(Duration::from_millis(20)).await;
    assert_eq!(received.load(Ordering::Relaxed), 1);

    state.advertised_local_pids.remove(&pid);
    session.handle_node(
        &mut state,
        node_protocol::NodeMessage {
            msg: Some(node_protocol::node_message::Msg::Call(
                node_protocol::Call {
                    to: pid,
                    tag: 1,
                    what: vec![],
                    timeout_ms: Some(10),
                    variant: "Call".to_string(),
                    metadata: None,
                },
            )),
        },
        session_ref.clone(),
    );
    sleep(Duration::from_millis(20)).await;
    assert_eq!(received.load(Ordering::Relaxed), 1);

    target.stop(None);
    server.stop(None);
    session_actor.stop(None);
    target_handle.await.unwrap();
    server_handle.await.unwrap();
    session_handle.await.unwrap();
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
            connection_id: 0,
        },
        node_server: server_ref.clone(),
        connection_mode: NodeConnectionMode::Isolated,
        max_inbound_frame_size: super::super::DEFAULT_MAX_INBOUND_FRAME_SIZE,
        connection_id: 0,
    };

    let mut state = NodeSessionState {
        auth: AuthenticationState::AsClient(auth::ClientAuthenticationProcess::init()),
        ready: ReadyState::Open,
        local_addr: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
        peer_addr: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
        name: None,
        connection_id: 0,
        remote_actors: HashMap::new(),
        advertised_local_pids: HashSet::new(),
        tcp: None,
        ping_task: None,
        epoch: Instant::now(),
        pong_warnings: PongWarnings::default(),
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
            connection_id: 0,
        },
        node_server: server_ref.clone(),
        connection_mode: NodeConnectionMode::Isolated,
        max_inbound_frame_size: super::super::DEFAULT_MAX_INBOUND_FRAME_SIZE,
        connection_id: 0,
    };

    let mut state = NodeSessionState {
        auth: AuthenticationState::AsClient(auth::ClientAuthenticationProcess::init()),
        ready: ReadyState::Open,
        local_addr: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
        peer_addr: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
        name: None,
        connection_id: 0,
        remote_actors: HashMap::new(),
        advertised_local_pids: HashSet::new(),
        tcp: None,
        ping_task: None,
        epoch: Instant::now(),
        pong_warnings: PongWarnings::default(),
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
            connection_id: 0,
        },
        node_server: server_ref.clone(),
        connection_mode: NodeConnectionMode::Isolated,
        max_inbound_frame_size: super::super::DEFAULT_MAX_INBOUND_FRAME_SIZE,
        connection_id: 0,
    };

    // let addr = SocketAddr::
    let mut state = NodeSessionState {
        auth: AuthenticationState::AsServer(auth::ServerAuthenticationProcess::init()),
        ready: ReadyState::Open,
        local_addr: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
        peer_addr: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
        name: None,
        connection_id: 0,
        remote_actors: HashMap::new(),
        advertised_local_pids: HashSet::new(),
        tcp: None,
        ping_task: None,
        epoch: Instant::now(),
        pong_warnings: PongWarnings::default(),
    };

    // Client sends their name
    let message = super::auth_protocol::AuthenticationMessage {
        msg: Some(auth_protocol::authentication_message::Msg::Name(
            auth_protocol::NameMessage {
                name: "peer".to_string(),
                flags: Some(auth_protocol::NodeFlags { version: 1 }),
                connection_string: "localhost:123".to_string(),
                connection_id: 11,
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
            connection_id: 0,
        },
        node_server: server_ref.clone(),
        connection_mode: NodeConnectionMode::Isolated,
        max_inbound_frame_size: super::super::DEFAULT_MAX_INBOUND_FRAME_SIZE,
        connection_id: 0,
    };

    let mut state = NodeSessionState {
        auth: AuthenticationState::AsServer(auth::ServerAuthenticationProcess::init()),
        ready: ReadyState::Open,
        local_addr: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
        peer_addr: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
        name: None,
        connection_id: 0,
        remote_actors: HashMap::new(),
        advertised_local_pids: HashSet::new(),
        tcp: None,
        ping_task: None,
        epoch: Instant::now(),
        pong_warnings: PongWarnings::default(),
    };

    // Other session continues, this one dies
    let message = super::auth_protocol::AuthenticationMessage {
        msg: Some(auth_protocol::authentication_message::Msg::Name(
            auth_protocol::NameMessage {
                name: "other_continues".to_string(),
                flags: Some(auth_protocol::NodeFlags { version: 1 }),
                connection_string: "localhost:123".to_string(),
                connection_id: 12,
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
                connection_id: 13,
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

    #[cfg_attr(feature = "async-trait", ractor::async_trait)]
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
            connection_id: 0,
        },
        node_server: server_ref.clone(),
        connection_mode: NodeConnectionMode::Isolated,
        max_inbound_frame_size: super::super::DEFAULT_MAX_INBOUND_FRAME_SIZE,
        connection_id: 0,
    };

    let mut state = NodeSessionState {
        auth: AuthenticationState::AsServer(auth::ServerAuthenticationProcess::Ok([0u8; 32])),
        ready: ReadyState::Open,
        local_addr: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
        peer_addr: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
        name: None,
        connection_id: 0,
        remote_actors: HashMap::new(),
        advertised_local_pids: HashSet::new(),
        tcp: None,
        ping_task: None,
        epoch: Instant::now(),
        pong_warnings: PongWarnings::default(),
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
            connection_id: 0,
        },
        node_server: server_ref.clone(),
        connection_mode: NodeConnectionMode::Isolated,
        max_inbound_frame_size: super::super::DEFAULT_MAX_INBOUND_FRAME_SIZE,
        connection_id: 0,
    };

    let mut state = NodeSessionState {
        auth: AuthenticationState::AsClient(auth::ClientAuthenticationProcess::Ok),
        ready: ReadyState::Open,
        local_addr: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
        peer_addr: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
        name: None,
        connection_id: 0,
        remote_actors: HashMap::new(),
        advertised_local_pids: HashSet::new(),
        tcp: None,
        ping_task: None,
        epoch: Instant::now(),
        pong_warnings: PongWarnings::default(),
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
