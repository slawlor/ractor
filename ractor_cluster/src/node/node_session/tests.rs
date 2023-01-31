// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

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
