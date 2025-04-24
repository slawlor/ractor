// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Define's a node's authentication process between peers. Definition
//! can be found in [Erlang's handshake](https://www.erlang.org/doc/apps/erts/erl_dist_protocol.html)

use rand::RngCore;

use crate::hash::Digest;
use crate::protocol::auth as proto;

/// Server authentication FSM
#[derive(Debug)]
pub(crate) enum ServerAuthenticationProcess {
    /// (1) Client initiates handshake by sending their peer name
    WaitingOnPeerName,

    /// (2) We have the peer name, and have replied with our own [proto::ServerStatus]
    /// reply
    HavePeerName(proto::NameMessage),

    /// (2B) Waiting on the client's status (true/false), if [proto::ClientStatus] was `alive`
    WaitingOnClientStatus,

    /// (3) Waiting on the client's reply to the [proto::Challenge] from the server.
    /// State is the name message from the client, the challenge, and the expected digest reply
    /// from the client
    ///
    /// Arguments are the challenge to send to the client and the expected digest we should get back
    WaitingOnClientChallengeReply(u32, Digest),

    /// (4) We processed the client challenge value, and replied and we're ok with the channel.
    /// The client has the final decision after they check our challenge computation which we send
    /// with [proto::ChallengeAck]
    ///
    /// Argument is the digest to send to the client
    Ok(Digest),

    /// Close
    Close,
}

impl ServerAuthenticationProcess {
    /// Initialize the FSM state
    pub(crate) fn init() -> Self {
        Self::WaitingOnPeerName
    }

    pub(crate) fn start_challenge(&self, cookie: &'_ str) -> Self {
        if matches!(self, Self::WaitingOnClientStatus | Self::HavePeerName(_)) {
            let challenge = rand::thread_rng().next_u32();
            let digest = crate::hash::challenge_digest(cookie, challenge);
            Self::WaitingOnClientChallengeReply(challenge, digest)
        } else {
            Self::Close
        }
    }

    /// Implement the FSM state transitions
    pub(crate) fn next(&self, auth_message: proto::AuthenticationMessage, cookie: &'_ str) -> Self {
        if let Some(msg) = auth_message.msg {
            match msg {
                proto::authentication_message::Msg::Name(name) => {
                    if let Self::WaitingOnPeerName = &self {
                        return Self::HavePeerName(name);
                    }
                }
                proto::authentication_message::Msg::ClientStatus(status) => {
                    if let Self::WaitingOnClientStatus = &self {
                        // client says to not continue the session
                        if !status.status {
                            return Self::Close;
                        } else {
                            return self.start_challenge(cookie);
                        }
                    }
                }
                proto::authentication_message::Msg::ClientChallenge(challenge_reply) => {
                    if let Self::WaitingOnClientChallengeReply(_, digest) = &self {
                        if digest.to_vec() == challenge_reply.digest {
                            let reply_digest =
                                crate::hash::challenge_digest(cookie, challenge_reply.challenge);
                            return Self::Ok(reply_digest);
                        } else {
                            // digest's don't match!
                            return Self::Close;
                        }
                    }
                }
                _ => {}
            }
        }
        // received either an empty message or an out-of-order message. The node can't be trusted
        Self::Close
    }
}

/// Client authentication FSM
#[derive(Debug)]
pub(crate) enum ClientAuthenticationProcess {
    /// (1) After the client has sent their peer name
    /// they wait for the [proto::ServerStatus] from the server
    WaitingForServerStatus,

    /// (2) We've potentially sent our client status. Either way
    /// we're waiting for the [proto::Challenge] from the server
    WaitingForServerChallenge(proto::ServerStatus),

    /// (3) We've sent our challenge to the server, and we're waiting
    /// on the server's calculation to determine if we should open the
    /// channel. State is our challenge value and the expected digest
    ///
    /// Arguments are servers_challenge, server_digest_reply, client_challenge_value, expected_digest
    WaitingForServerChallengeAck(proto::Challenge, Digest, u32, Digest),

    /// (4) We've validated the server's challenge digest and agree
    /// that the channel is now open for node inter-communication
    Ok,

    /// Close
    Close,
}

impl ClientAuthenticationProcess {
    /// Initialize the FSM state
    pub(crate) fn init() -> Self {
        Self::WaitingForServerStatus
    }

    /// Implement the client FSM transitions
    pub(crate) fn next(&self, auth_message: proto::AuthenticationMessage, cookie: &'_ str) -> Self {
        if let Some(msg) = auth_message.msg {
            match msg {
                proto::authentication_message::Msg::ServerStatus(status) => {
                    if let Self::WaitingForServerStatus = &self {
                        return Self::WaitingForServerChallenge(status);
                    }
                }
                proto::authentication_message::Msg::ServerChallenge(challenge_msg) => {
                    if let Self::WaitingForServerChallenge(_) = &self {
                        let server_digest =
                            crate::hash::challenge_digest(cookie, challenge_msg.challenge);
                        let challenge = rand::thread_rng().next_u32();
                        let expected_digest = crate::hash::challenge_digest(cookie, challenge);
                        return Self::WaitingForServerChallengeAck(
                            challenge_msg,
                            server_digest,
                            challenge,
                            expected_digest,
                        );
                    }
                }
                proto::authentication_message::Msg::ServerAck(challenge_ack) => {
                    if let Self::WaitingForServerChallengeAck(_, _, _, expected_digest) = &self {
                        if expected_digest.to_vec() == challenge_ack.digest {
                            return Self::Ok;
                        } else {
                            return Self::Close;
                        }
                    }
                }
                _ => {}
            }
        }
        // received either an empty message or an out-of-order message. The node can't be trusted
        Self::Close
    }
}

#[cfg(test)]
mod tests {
    use crate::protocol::auth::NodeFlags;

    use super::*;

    #[test]
    fn server_auth_state() {
        let cookie = "cookie";

        let other_message = proto::AuthenticationMessage {
            msg: Some(proto::authentication_message::Msg::ServerStatus(
                proto::ServerStatus {
                    status: proto::server_status::Status::Ok as i32,
                },
            )),
        };

        let init = ServerAuthenticationProcess::init();
        assert!(matches!(
            init,
            ServerAuthenticationProcess::WaitingOnPeerName
        ));

        // out-of-order message should close at any point
        assert!(matches!(
            init.next(
                proto::AuthenticationMessage {
                    msg: Some(proto::authentication_message::Msg::ClientStatus(
                        proto::ClientStatus { status: false }
                    ))
                },
                cookie
            ),
            ServerAuthenticationProcess::Close
        ));
        let next = init.next(
            proto::AuthenticationMessage {
                msg: Some(proto::authentication_message::Msg::Name(
                    proto::NameMessage {
                        name: "howdy".to_string(),
                        flags: Some(NodeFlags { version: 1 }),
                        connection_string: "localhost:123".to_string(),
                    },
                )),
            },
            cookie,
        );
        assert!(matches!(next, ServerAuthenticationProcess::HavePeerName(_)));

        // out-of-order message should close at any point
        assert!(matches!(
            next.next(other_message.clone(), cookie),
            ServerAuthenticationProcess::Close
        ));

        // we retrieved the peer name, stored it, and now are waiting on the client status
        // bad message should close at any point
        let next = ServerAuthenticationProcess::WaitingOnClientStatus;

        // out-of-order message should close at any point
        assert!(matches!(
            next.next(other_message.clone(), cookie),
            ServerAuthenticationProcess::Close
        ));

        let next = next.next(
            proto::AuthenticationMessage {
                msg: Some(proto::authentication_message::Msg::ClientStatus(
                    proto::ClientStatus { status: true },
                )),
            },
            cookie,
        );
        assert!(matches!(
            next,
            ServerAuthenticationProcess::WaitingOnClientChallengeReply(_, _)
        ));
        let digest = if let ServerAuthenticationProcess::WaitingOnClientChallengeReply(_, d) = &next
        {
            *d
        } else {
            panic!("C'est impossible!");
        };

        // out-of-order message should close at any point
        assert!(matches!(
            next.next(other_message, cookie),
            ServerAuthenticationProcess::Close
        ));

        // client sends their challenge reply, we need to check the challenge against our provided cookie hash
        let next = next.next(
            proto::AuthenticationMessage {
                msg: Some(proto::authentication_message::Msg::ClientChallenge(
                    proto::ChallengeReply {
                        challenge: 1u32,
                        digest: digest.to_vec(),
                    },
                )),
            },
            cookie,
        );
        assert!(matches!(next, ServerAuthenticationProcess::Ok(_)));
    }

    #[test]
    fn client_auth_state() {
        let cookie = "cookie";

        let other_message = proto::AuthenticationMessage {
            msg: Some(proto::authentication_message::Msg::ClientStatus(
                proto::ClientStatus { status: false },
            )),
        };

        let init = ClientAuthenticationProcess::init();
        assert!(matches!(
            init,
            ClientAuthenticationProcess::WaitingForServerStatus
        ));

        // out-of-order message should close at any point
        assert!(matches!(
            init.next(other_message.clone(), cookie),
            ClientAuthenticationProcess::Close
        ));

        let next = init.next(
            proto::AuthenticationMessage {
                msg: Some(proto::authentication_message::Msg::ServerStatus(
                    proto::ServerStatus {
                        status: proto::server_status::Status::Ok as i32,
                    },
                )),
            },
            cookie,
        );
        assert!(matches!(
            next,
            ClientAuthenticationProcess::WaitingForServerChallenge(_)
        ));

        // out-of-order message should close at any point
        assert!(matches!(
            next.next(other_message.clone(), cookie),
            ClientAuthenticationProcess::Close
        ));

        let next = next.next(
            proto::AuthenticationMessage {
                msg: Some(proto::authentication_message::Msg::ServerChallenge(
                    proto::Challenge {
                        name: "challenger".to_string(),
                        flags: Some(NodeFlags { version: 1 }),
                        challenge: 123,
                        connection_string: "localhost:123".to_string(),
                    },
                )),
            },
            cookie,
        );
        assert!(matches!(
            next,
            ClientAuthenticationProcess::WaitingForServerChallengeAck(_, _, _, _)
        ));
        let (
            _server_challenge_msg,
            _server_challenge_digest,
            _client_challenge,
            client_challenge_digest,
        ) = if let ClientAuthenticationProcess::WaitingForServerChallengeAck(
            server_challenge_msg,
            server_challenge_digest,
            client_challenge,
            client_challenge_digest,
        ) = &next
        {
            (
                server_challenge_msg.clone(),
                *server_challenge_digest,
                *client_challenge,
                *client_challenge_digest,
            )
        } else {
            panic!("C'est impossible!");
        };

        assert!(matches!(
            next.next(other_message, cookie),
            ClientAuthenticationProcess::Close
        ));

        let next = next.next(
            proto::AuthenticationMessage {
                msg: Some(proto::authentication_message::Msg::ServerAck(
                    proto::ChallengeAck {
                        digest: client_challenge_digest.to_vec(),
                    },
                )),
            },
            cookie,
        );
        assert!(matches!(next, ClientAuthenticationProcess::Ok));
    }
}
