// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! A [NodeSession] is an individual connection between a specific pair of
//! `node()`s and all of its authentication and communication for that
//! pairing

use std::collections::HashMap;
use std::collections::HashSet;
use std::convert::TryInto;
use std::net::SocketAddr;
use std::time::Instant;
use std::time::SystemTime;

use ractor::message::SerializedMessage;
use ractor::pg::get_scoped_local_members;
use ractor::pg::which_scopes_and_groups;
use ractor::pg::GroupChangeMessage;
use ractor::registry::PidLifecycleEvent;
use ractor::rpc::CallResult;
use ractor::Actor;
use ractor::ActorId;
use ractor::ActorProcessingErr;
use ractor::ActorRef;
use ractor::SpawnErr;
use ractor::SupervisionEvent;
use rand::Rng;
use tokio::time::Duration;

use super::auth;
use crate::net::SessionMessage;
use crate::node::NodeConnectionMode;
use crate::protocol::auth as auth_protocol;
use crate::protocol::control as control_protocol;
use crate::protocol::node as node_protocol;
use crate::remote_actor::RemoteActor;
use crate::remote_actor::RemoteActorMessage;
use crate::NodeServerMessage;

const MIN_PING_LATENCY_MS: u64 = 1000;
const MAX_PING_LATENCY_MS: u64 = 5000;

#[cfg(test)]
mod tests;

#[derive(Debug)]
enum AuthenticationState {
    AsClient(auth::ClientAuthenticationProcess),
    AsServer(auth::ServerAuthenticationProcess),
}

impl AuthenticationState {
    fn is_ok(&self) -> bool {
        match self {
            Self::AsClient(c) => matches!(c, auth::ClientAuthenticationProcess::Ok),
            Self::AsServer(s) => matches!(s, auth::ServerAuthenticationProcess::Ok(_)),
        }
    }

    fn is_close(&self) -> bool {
        match self {
            Self::AsClient(c) => matches!(c, auth::ClientAuthenticationProcess::Close),
            Self::AsServer(s) => matches!(s, auth::ServerAuthenticationProcess::Close),
        }
    }
}

#[derive(Debug)]
enum ReadyState {
    Open,
    SyncSent,
    SyncReceived,
    Ready,
}

impl ReadyState {
    fn is_ok(&self) -> bool {
        matches!(self, ReadyState::Ready)
    }
}

/// Represents a remote connection to a `node()`. The [NodeSession] is the main
/// handler for all inter-node communication and handles
///
/// 1. The state of the authentication handshake
/// 2. Control messages for actor synchronization + group membership changes
/// 3. `RemoteActor`s wishing to send messages to their remote counterparts on the
///    remote system (and receive replies)
///
/// A [NodeSession] can either be a client or server session, depending on the connection sequence.
/// If it was an incoming request to the [crate::NodeServer] then it's a "server" session, as
/// the server spawned this actor. Otherwise it's an outgoing "client" request.
///
/// If the [NodeSession] is a client session, it will start the authentication handshake with
/// a `auth_protocol::NameMessage` announcing this node's name to the remote system for deduplication
/// and starting the rest of the handshake. For full authentication pattern details, see
///
/// 1. `src/protocol/auth.proto`
/// 2. `src/node/auth.rs`
///
/// Lastly the node's have an intern-node "ping" operation which occurs to keep the TCP session alive
/// and additionally measure peer latency.
#[derive(Debug)]
pub struct NodeSession {
    node_id: crate::NodeId,
    is_server: bool,
    cookie: String,
    node_server: ActorRef<NodeServerMessage>,
    this_node_name: auth_protocol::NameMessage,
    connection_mode: super::NodeConnectionMode,
}

impl NodeSession {
    /// Construct a new [NodeSession] with the supplied
    /// arguments
    ///
    /// * `node_id`: This peer's node id
    /// * `is_server`: Is a server-received session (if false, this is the client)
    /// * `cookie`: The authorization cookie
    /// * `node_server`: The parent node server
    /// * `node_name`: This node's name and connection details
    /// * `connection_mode`: The connection mode for peer connections
    pub fn new(
        node_id: crate::NodeId,
        is_server: bool,
        cookie: String,
        node_server: ActorRef<NodeServerMessage>,
        node_name: auth_protocol::NameMessage,
        connection_mode: super::NodeConnectionMode,
    ) -> Self {
        Self {
            node_id,
            is_server,
            cookie,
            node_server,
            this_node_name: node_name,
            connection_mode,
        }
    }
}

impl NodeSession {
    async fn handle_auth(
        &self,
        state: &mut NodeSessionState,
        message: auth_protocol::AuthenticationMessage,
        myself: ActorRef<super::NodeSessionMessage>,
    ) {
        if state.auth.is_ok() {
            // nothing to do, we're already authenticated
            return;
        }
        if state.auth.is_close() {
            tracing::info!(
                "Node Session {} is shutting down due to authentication failure",
                self.node_id
            );
            // we need to shutdown, the session needs to be terminated
            myself.stop(Some("auth_fail".to_string()));
            if let Some(tcp) = &state.tcp {
                tcp.stop(Some("auth_fail".to_string()));
            }
        }

        match &state.auth {
            AuthenticationState::AsClient(client_auth) => {
                let mut next = client_auth.next(message, &self.cookie);
                match &next {
                    auth::ClientAuthenticationProcess::WaitingForServerChallenge(server_status) => {
                        match server_status.status() {
                            auth_protocol::server_status::Status::Ok => {
                                // this handshake will continue
                            }
                            auth_protocol::server_status::Status::OkSimultaneous => {
                                // this handshake will continue, but there is another handshake underway
                                // that will be shut down (i.e. this was a server connection and we're currently trying
                                // a client connection)
                            }
                            auth_protocol::server_status::Status::NotOk => {
                                // The handshake will not continue, as there's already another client handshake underway
                                // which itself initiated (Simultaneous connect where the other connection's name is > this node
                                // name)
                                next = auth::ClientAuthenticationProcess::Close;
                            }
                            auth_protocol::server_status::Status::NotAllowed => {
                                // unspecified auth reason
                                next = auth::ClientAuthenticationProcess::Close;
                            }
                            auth_protocol::server_status::Status::Alive => {
                                // A connection to the node is already alive, which means either the
                                // node is confused in its connection state or the previous TCP connection is
                                // breaking down. Send ClientStatus
                                // TODO: check the status properly
                                state.tcp_send_auth(auth_protocol::AuthenticationMessage {
                                    msg: Some(
                                        auth_protocol::authentication_message::Msg::ClientStatus(
                                            auth_protocol::ClientStatus { status: true },
                                        ),
                                    ),
                                });
                            }
                        }
                    }
                    auth::ClientAuthenticationProcess::WaitingForServerChallengeAck(
                        server_challenge_value,
                        reply_to_server,
                        our_challenge,
                        _expected_digest,
                    ) => {
                        // record the name
                        let name_message = auth_protocol::NameMessage {
                            name: server_challenge_value.name.clone(),
                            flags: server_challenge_value.flags,
                            connection_string: server_challenge_value.connection_string.clone(),
                        };
                        state.name = Some(name_message.clone());
                        // tell the node server that we now know this peer's name information
                        let _ = self
                            .node_server
                            .cast(super::NodeServerMessage::UpdateSession {
                                actor_id: myself.get_id(),
                                name: name_message,
                            });
                        // send the client challenge to the server
                        let reply = auth_protocol::AuthenticationMessage {
                            msg: Some(auth_protocol::authentication_message::Msg::ClientChallenge(
                                auth_protocol::ChallengeReply {
                                    digest: reply_to_server.to_vec(),
                                    challenge: *our_challenge,
                                },
                            )),
                        };
                        state.tcp_send_auth(reply);
                    }
                    _ => {
                        // no message to send
                    }
                }

                if let auth::ClientAuthenticationProcess::Close = &next {
                    tracing::info!(
                        "Node Session {} is shutting down due to authentication failure",
                        self.node_id
                    );
                    myself.stop(Some("auth_fail".to_string()));
                }
                if let auth::ClientAuthenticationProcess::Ok = &next {
                    tracing::info!("Node Session {} is authenticated", self.node_id);
                }
                tracing::debug!("Next client auth state: {:?}", next);
                state.auth = AuthenticationState::AsClient(next);
            }
            AuthenticationState::AsServer(server_auth) => {
                let mut next = server_auth.next(message, &self.cookie);

                match &next {
                    auth::ServerAuthenticationProcess::HavePeerName(peer_name) => {
                        // store the peer node's name in the session state
                        state.name = Some(peer_name.clone());
                        // send the status message, followed by the server's challenge
                        let server_status_result = self
                            .node_server
                            .call(
                                |tx| super::NodeServerMessage::CheckSession {
                                    peer_name: peer_name.clone(),
                                    reply: tx,
                                },
                                Some(Duration::from_millis(500)),
                            )
                            .await;
                        // tell the node server that we now know this peer's name information
                        let _ = self
                            .node_server
                            .cast(super::NodeServerMessage::UpdateSession {
                                actor_id: myself.get_id(),
                                name: peer_name.clone(),
                            });
                        match server_status_result {
                            Err(_) | Ok(CallResult::Timeout) | Ok(CallResult::SenderError) => {
                                next = auth::ServerAuthenticationProcess::Close;
                            }
                            Ok(CallResult::Success(reply)) => {
                                let server_status: auth_protocol::server_status::Status =
                                    reply.into();
                                // Send the server's status message
                                let status_msg = auth_protocol::AuthenticationMessage {
                                    msg: Some(
                                        auth_protocol::authentication_message::Msg::ServerStatus(
                                            auth_protocol::ServerStatus {
                                                status: server_status.into(),
                                            },
                                        ),
                                    ),
                                };
                                state.tcp_send_auth(status_msg);

                                match server_status {
                                    auth_protocol::server_status::Status::Ok
                                    | auth_protocol::server_status::Status::OkSimultaneous => {
                                        // Good to proceed, start a challenge
                                        next = next.start_challenge(&self.cookie);
                                        if let auth::ServerAuthenticationProcess::WaitingOnClientChallengeReply(
                                            challenge,
                                            _digest,
                                        ) = &next
                                        {
                                            let challenge_msg = auth_protocol::AuthenticationMessage {
                                                msg: Some(
                                                    auth_protocol::authentication_message::Msg::ServerChallenge(
                                                        auth_protocol::Challenge {
                                                            name: self.this_node_name.name.clone(),
                                                            flags: self.this_node_name.flags,
                                                            challenge: *challenge,
                                                            connection_string: self.this_node_name.connection_string.clone(),
                                                        },
                                                    ),
                                                ),
                                            };
                                            state.tcp_send_auth(challenge_msg);
                                        }
                                    }
                                    auth_protocol::server_status::Status::NotOk
                                    | auth_protocol::server_status::Status::NotAllowed => {
                                        next = auth::ServerAuthenticationProcess::Close;
                                    }
                                    auth_protocol::server_status::Status::Alive => {
                                        // we sent the `Alive` status, so we're waiting on the client to confirm their status
                                        // before continuing
                                        next = auth::ServerAuthenticationProcess::WaitingOnClientStatus;
                                    }
                                }
                            }
                        }
                    }
                    auth::ServerAuthenticationProcess::Ok(digest) => {
                        let client_challenge_reply = auth_protocol::AuthenticationMessage {
                            msg: Some(auth_protocol::authentication_message::Msg::ServerAck(
                                auth_protocol::ChallengeAck {
                                    digest: digest.to_vec(),
                                },
                            )),
                        };
                        state.tcp_send_auth(client_challenge_reply);
                    }
                    _ => {
                        // no message to send
                    }
                }

                if let auth::ServerAuthenticationProcess::Close = &next {
                    tracing::info!(
                        "Node Session {} is shutting down due to authentication failure",
                        self.node_id
                    );
                    myself.stop(Some("auth_fail".to_string()));
                }
                if let auth::ServerAuthenticationProcess::Ok(_) = &next {
                    tracing::info!("Node Session {} is authenticated", self.node_id);
                }
                tracing::debug!("Next server auth state: {:?}", next);
                state.auth = AuthenticationState::AsServer(next);
            }
        }
    }

    fn handle_node(
        &self,
        state: &mut NodeSessionState,
        message: node_protocol::NodeMessage,
        myself: ActorRef<super::NodeSessionMessage>,
    ) {
        if !state.auth.is_ok() {
            tracing::warn!("Inter-node message received on unauthenticated NodeSession");
            return;
        }

        if let Some(msg) = message.msg {
            match msg {
                node_protocol::node_message::Msg::Cast(cast_args) => {
                    if let Some(actor) =
                        ractor::registry::where_is_pid(ActorId::Local(cast_args.to))
                    {
                        let _ = actor.send_serialized(SerializedMessage::Cast {
                            variant: cast_args.variant,
                            args: cast_args.what,
                            metadata: cast_args.metadata,
                        });
                    }
                }
                node_protocol::node_message::Msg::Call(call_args) => {
                    let to = call_args.to;
                    let tag = call_args.tag;
                    if let Some(actor) =
                        ractor::registry::where_is_pid(ActorId::Local(call_args.to))
                    {
                        let (tx, rx) = ractor::concurrency::oneshot();

                        // send off the transmission in the serialized format, letting the message's own deserialization handle
                        // the conversion
                        let maybe_timeout = call_args.timeout_ms.map(Duration::from_millis);
                        if let Some(timeout) = maybe_timeout {
                            let _ = actor.send_serialized(SerializedMessage::Call {
                                args: call_args.what,
                                reply: (tx, timeout).into(),
                                variant: call_args.variant,
                                metadata: call_args.metadata,
                            });
                        } else {
                            let _ = actor.send_serialized(SerializedMessage::Call {
                                args: call_args.what,
                                reply: tx.into(),
                                variant: call_args.variant,
                                metadata: call_args.metadata,
                            });
                        }

                        // kick off a background task to reply to the channel request, threading the tag and who to reply to
                        #[allow(clippy::let_underscore_future)]
                        let _ = ractor::concurrency::spawn(async move {
                            if let Some(timeout) = maybe_timeout {
                                if let Ok(Ok(result)) =
                                    ractor::concurrency::timeout(timeout, rx).await
                                {
                                    let reply = node_protocol::node_message::Msg::Reply(
                                        node_protocol::CallReply {
                                            tag,
                                            to,
                                            what: result,
                                        },
                                    );
                                    let _ = ractor::cast!(
                                        myself,
                                        super::NodeSessionMessage::SendMessage(
                                            node_protocol::NodeMessage { msg: Some(reply) }
                                        )
                                    );
                                }
                            } else if let Ok(result) = rx.await {
                                let reply = node_protocol::node_message::Msg::Reply(
                                    node_protocol::CallReply {
                                        tag,
                                        to,
                                        what: result,
                                    },
                                );
                                let _ = ractor::cast!(
                                    myself,
                                    super::NodeSessionMessage::SendMessage(
                                        node_protocol::NodeMessage { msg: Some(reply) }
                                    )
                                );
                            }
                        });
                    }
                }
                node_protocol::node_message::Msg::Reply(call_reply_args) => {
                    if let Some(actor) = state.remote_actors.get(&call_reply_args.to) {
                        let _ = actor.send_serialized(SerializedMessage::CallReply(
                            call_reply_args.tag,
                            call_reply_args.what,
                        ));
                    }
                }
            }
        }
    }

    async fn handle_control(
        &self,
        state: &mut NodeSessionState,
        message: control_protocol::ControlMessage,
        myself: ActorRef<super::NodeSessionMessage>,
    ) -> Result<(), ActorProcessingErr> {
        if !state.auth.is_ok() {
            tracing::warn!("Control message received on unauthenticated NodeSession");
            return Ok(());
        }

        if let Some(msg) = message.msg {
            match msg {
                control_protocol::control_message::Msg::Ready(_) => match state.ready {
                    ReadyState::Open => {
                        state.ready = ReadyState::SyncReceived;
                    }
                    ReadyState::SyncSent => {
                        state.ready = ReadyState::Ready;
                        ractor::cast!(
                            self.node_server,
                            NodeServerMessage::ConnectionReady(myself.get_id())
                        )?;
                    }
                    ReadyState::SyncReceived | ReadyState::Ready => {
                        tracing::warn!("Received duplicate Ready signal");
                    }
                },
                control_protocol::control_message::Msg::Spawn(spawned_actors) => {
                    for net_actor in spawned_actors.actors {
                        if let Err(spawn_err) = self
                            .get_or_spawn_remote_actor(
                                &myself,
                                net_actor.name,
                                net_actor.pid,
                                state,
                            )
                            .await
                        {
                            tracing::error!("Failed to spawn remote actor with {spawn_err}");
                        } else {
                            tracing::debug!("Spawned remote actor");
                        }
                    }
                }
                control_protocol::control_message::Msg::Terminate(termination) => {
                    for pid in termination.ids {
                        if let Some(actor) = state.remote_actors.remove(&pid) {
                            actor
                                .stop_and_wait(Some("remote".to_string()), None)
                                .await?;
                            tracing::debug!(
                                "Actor {pid} on node {} exited, terminating local `RemoteActor` {}",
                                self.node_id,
                                actor.get_id()
                            );
                        }
                    }
                }
                control_protocol::control_message::Msg::Ping(ping) => {
                    state.tcp_send_control(control_protocol::ControlMessage {
                        msg: Some(control_protocol::control_message::Msg::Pong(
                            control_protocol::Pong {
                                timestamp: ping.timestamp,
                            },
                        )),
                    });
                }
                control_protocol::control_message::Msg::Pong(pong) => {
                    let ts: std::time::SystemTime = pong
                        .timestamp
                        .expect("Timestamp missing in Pong")
                        .try_into()
                        .expect("Failed to convert Pong(Timestamp) to SystemTime");
                    let inst = ts
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .expect("Time went backwards");
                    let delta_ms = (state.epoch.elapsed() - inst).as_millis();
                    tracing::debug!("Ping -> Pong took {delta_ms}ms");
                    if delta_ms > 50 {
                        tracing::warn!(
                            "Super long ping detected {} - {} ({delta_ms}ms)",
                            state.local_addr,
                            state.peer_addr,
                        );
                    }
                    // schedule next ping
                    state.schedule_tcp_ping();
                }
                control_protocol::control_message::Msg::PgJoin(join) => {
                    let mut cells = vec![];
                    for control_protocol::Actor { name, pid } in join.actors {
                        match self
                            .get_or_spawn_remote_actor(&myself, name, pid, state)
                            .await
                        {
                            Ok(actor) => {
                                cells.push(actor.get_cell());
                            }
                            Err(spawn_err) => {
                                tracing::error!("Failed to spawn remote actor with '{spawn_err}'");
                            }
                        }
                    }
                    // join the remote actors to the local PG group
                    if !cells.is_empty() {
                        tracing::debug!(
                            "PG Join scope '{}' and group '{}' for {} remote actors",
                            join.scope,
                            join.group,
                            cells.len()
                        );
                        ractor::pg::join_scoped(join.scope, join.group, cells);
                    }
                }
                control_protocol::control_message::Msg::PgLeave(leave) => {
                    let mut cells = vec![];
                    for control_protocol::Actor { pid, .. } in leave.actors {
                        if let Some(actor) = state.remote_actors.get(&pid) {
                            cells.push(actor.get_cell());
                        }
                    }
                    // join the remote actors to the local PG group
                    if !cells.is_empty() {
                        tracing::debug!(
                            "PG Leave scope '{}' and group '{}' for {} remote actors",
                            leave.scope,
                            leave.group,
                            cells.len()
                        );
                        ractor::pg::leave_scoped(leave.scope, leave.group, cells);
                    }
                }
                control_protocol::control_message::Msg::EnumerateNodeSessions(whos_asking) => {
                    let existing_sessions = ractor::call_t!(
                        self.node_server,
                        crate::NodeServerMessage::GetSessions,
                        500
                    );
                    if let Ok(sessions) = existing_sessions {
                        tracing::info!(
                            "{:?}",
                            sessions
                                .values()
                                .map(|s| s.peer_name.clone())
                                .collect::<Vec<_>>()
                        );
                        let names = sessions
                            .into_values()
                            .filter_map(|local_session| local_session.peer_name)
                            .filter(|local_session| {
                                // don't send back the node who's asking so we don't trigger self-connections
                                whos_asking.name != local_session.name
                                    && whos_asking.connection_string
                                        != local_session.connection_string
                            })
                            .collect::<Vec<_>>();
                        let reply_message = control_protocol::ControlMessage {
                            msg: Some(control_protocol::control_message::Msg::NodeSessions(
                                control_protocol::NodeSessions { sessions: names },
                            )),
                        };
                        state.tcp_send_control(reply_message);
                    }
                }
                control_protocol::control_message::Msg::NodeSessions(sessions) => {
                    if let NodeConnectionMode::Transitive = self.connection_mode {
                        let existing_sessions = if let Ok(sessions) = ractor::call_t!(
                            self.node_server,
                            crate::NodeServerMessage::GetSessions,
                            500
                        ) {
                            sessions
                                .into_values()
                                .filter_map(|v| v.peer_name.map(|p| [p.name, p.connection_string]))
                                .flatten()
                                .collect()
                        } else {
                            HashSet::new()
                        };

                        for session_name in sessions.sessions.into_iter() {
                            // check if we're already connected to this host or if it's a request to self-connect,
                            // if so ignore
                            if !(existing_sessions.contains(&session_name.name)
                                || existing_sessions.contains(&session_name.connection_string)
                                || session_name.name == self.this_node_name.name
                                || session_name.connection_string
                                    == self.this_node_name.connection_string)
                            {
                                // we aren't connected to this peer, start connecting
                                let node_server = self.node_server.clone();
                                ractor::concurrency::spawn(async move {
                                    if let Err(connect_err) = super::client::connect(
                                        &node_server,
                                        session_name.connection_string.clone(),
                                    )
                                    .await
                                    {
                                        tracing::warn!("Node transitive connection, failed to connect to {} with '{connect_err}'", session_name.connection_string);
                                    }
                                });
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Called once the session is authenticated
    fn after_authenticated(
        &self,
        myself: ActorRef<super::NodeSessionMessage>,
        state: &mut NodeSessionState,
    ) {
        tracing::info!(
            "Session authenticated on NodeSession {} - ({:?})",
            self.node_id,
            state.peer_addr
        );

        // startup the ping healthcheck activity
        state.schedule_tcp_ping();

        // trigger enumeration of the remote peer's node sessions for transitive connections
        if let NodeConnectionMode::Transitive = self.connection_mode {
            state.tcp_send_control(control_protocol::ControlMessage {
                msg: Some(
                    control_protocol::control_message::Msg::EnumerateNodeSessions(
                        self.this_node_name.clone(),
                    ),
                ),
            });
        }

        // setup PID monitoring
        ractor::registry::pid_registry::monitor(myself.get_cell());

        // Scan all PIDs and spawn them on the remote host
        let pids = ractor::registry::pid_registry::get_all_pids()
            .into_iter()
            .filter(|act| act.supports_remoting())
            .map(|a| control_protocol::Actor {
                name: a.get_name(),
                pid: a.get_id().pid(),
            })
            .collect::<Vec<_>>();
        if !pids.is_empty() {
            let msg = control_protocol::ControlMessage {
                msg: Some(control_protocol::control_message::Msg::Spawn(
                    control_protocol::Spawn { actors: pids },
                )),
            };
            state.tcp_send_control(msg);
        }

        // setup scope monitoring
        ractor::pg::monitor_world(&myself.get_cell());

        // Scan all scopes with their PG groups + synchronize them
        let scopes_and_groups = which_scopes_and_groups();
        for key in scopes_and_groups {
            let local_members = get_scoped_local_members(key.get_scope(), &key.get_group())
                .into_iter()
                .filter(|v| v.supports_remoting())
                .map(|act| control_protocol::Actor {
                    name: act.get_name(),
                    pid: act.get_id().pid(),
                })
                .collect::<Vec<_>>();
            if !local_members.is_empty() {
                let control_message = control_protocol::ControlMessage {
                    msg: Some(control_protocol::control_message::Msg::PgJoin(
                        control_protocol::PgJoin {
                            scope: key.get_scope(),
                            group: key.get_group(),
                            actors: local_members,
                        },
                    )),
                };
                state.tcp_send_control(control_message);
            }
        }
        state.tcp_send_control(control_protocol::ControlMessage {
            msg: Some(control_protocol::control_message::Msg::Ready(
                control_protocol::Ready {},
            )),
        });
        match state.ready {
            ReadyState::Open => state.ready = ReadyState::SyncSent,
            ReadyState::SyncReceived => state.ready = ReadyState::Ready,
            ReadyState::SyncSent | ReadyState::Ready => {
                unreachable!("after_authenticated() executed twice")
            }
        }
        // TODO: subscribe to the named registry and synchronize it? What happes on a name clash? How would this be handled
        // if both sessions had a "node_a" for example? Which resolves, local only?
    }

    /// Get a given remote actor, or spawn it if it doesn't exist.
    async fn get_or_spawn_remote_actor(
        &self,
        myself: &ActorRef<super::NodeSessionMessage>,
        actor_name: Option<String>,
        actor_pid: u64,
        state: &mut NodeSessionState,
    ) -> Result<ActorRef<RemoteActorMessage>, SpawnErr> {
        match state.remote_actors.get(&actor_pid) {
            Some(actor) => Ok(actor.clone()),
            _ => {
                let (remote_actor, _) = RemoteActor
                    .spawn_linked(
                        myself.clone(),
                        actor_name,
                        actor_pid,
                        self.node_id,
                        myself.get_cell(),
                    )
                    .await?;
                state.remote_actors.insert(actor_pid, remote_actor.clone());
                Ok(remote_actor)
            }
        }
    }
}

/// The state of the node session
#[derive(Debug)]
pub struct NodeSessionState {
    tcp: Option<ActorRef<SessionMessage>>,
    peer_addr: SocketAddr,
    local_addr: SocketAddr,
    epoch: Instant,
    name: Option<auth_protocol::NameMessage>,
    auth: AuthenticationState,
    ready: ReadyState,
    remote_actors: HashMap<u64, ActorRef<RemoteActorMessage>>,
}

impl NodeSessionState {
    fn is_tcp_actor(&self, actor: ActorId) -> bool {
        self.tcp
            .as_ref()
            .map(|t| t.get_id() == actor)
            .unwrap_or(false)
    }

    fn tcp_send_auth(&self, msg: auth_protocol::AuthenticationMessage) {
        if let Some(tcp) = &self.tcp {
            let net_msg = crate::protocol::NetworkMessage {
                message: Some(crate::protocol::meta::network_message::Message::Auth(msg)),
            };
            let _ = ractor::cast!(tcp, SessionMessage::Send(net_msg));
        }
    }

    fn tcp_send_node(&self, msg: node_protocol::NodeMessage) {
        if let Some(tcp) = &self.tcp {
            let net_msg = crate::protocol::NetworkMessage {
                message: Some(crate::protocol::meta::network_message::Message::Node(msg)),
            };
            let _ = ractor::cast!(tcp, SessionMessage::Send(net_msg));
        }
    }

    fn tcp_send_control(&self, msg: control_protocol::ControlMessage) {
        if let Some(tcp) = &self.tcp {
            let net_msg = crate::protocol::NetworkMessage {
                message: Some(crate::protocol::meta::network_message::Message::Control(
                    msg,
                )),
            };
            let _ = ractor::cast!(tcp, SessionMessage::Send(net_msg));
        }
    }

    fn schedule_tcp_ping(&self) {
        let epoch = self.epoch;
        if let Some(tcp) = &self.tcp {
            #[allow(clippy::let_underscore_future)]
            let _ = tcp.send_after(Self::get_send_delay(), move || {
                let ping = control_protocol::ControlMessage {
                    msg: Some(control_protocol::control_message::Msg::Ping(
                        control_protocol::Ping {
                            timestamp: Some(prost_types::Timestamp::from(
                                SystemTime::UNIX_EPOCH + epoch.elapsed(),
                            )),
                        },
                    )),
                };
                let net_msg = crate::protocol::NetworkMessage {
                    message: Some(crate::protocol::meta::network_message::Message::Control(
                        ping,
                    )),
                };
                SessionMessage::Send(net_msg)
            });
        }
    }

    fn get_send_delay() -> Duration {
        Duration::from_millis(
            rand::thread_rng().gen_range(MIN_PING_LATENCY_MS..MAX_PING_LATENCY_MS),
        )
    }
}

#[cfg_attr(feature = "async-trait", ractor::async_trait)]
impl Actor for NodeSession {
    type Msg = super::NodeSessionMessage;
    type Arguments = crate::net::NetworkStream;
    type State = NodeSessionState;

    async fn pre_start(
        &self,
        myself: ActorRef<Self::Msg>,
        stream: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        let peer_addr = stream.peer_addr();
        let local_addr = stream.local_addr();
        // startup the TCP socket handler for message write + reading
        let actor = crate::net::Session::spawn_linked(
            myself.clone(),
            stream,
            peer_addr,
            local_addr,
            myself.get_cell(),
        )
        .await?;

        let state = Self::State {
            tcp: Some(actor),
            name: None,
            auth: if self.is_server {
                AuthenticationState::AsServer(auth::ServerAuthenticationProcess::init())
            } else {
                AuthenticationState::AsClient(auth::ClientAuthenticationProcess::init())
            },
            ready: ReadyState::Open,
            remote_actors: HashMap::new(),
            peer_addr,
            local_addr,
            epoch: Instant::now(),
        };

        // If a client-connection, startup the handshake
        if !self.is_server {
            state.tcp_send_auth(auth_protocol::AuthenticationMessage {
                msg: Some(auth_protocol::authentication_message::Msg::Name(
                    self.this_node_name.clone(),
                )),
            });
        }

        Ok(state)
    }

    async fn post_stop(
        &self,
        myself: ActorRef<Self::Msg>,
        _state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        // unhook monitoring sessions
        ractor::pg::demonitor_world(&myself);
        ractor::registry::pid_registry::demonitor(myself.get_id());

        Ok(())
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        if let Some(peer) = &state.name {
            if self.this_node_name.connection_string == peer.connection_string
                || self.this_node_name.name == peer.name
            {
                // self-connection detected, exit session
                tracing::warn!("Cannot establish a connection to self. Exiting");
                myself.stop(Some("self_connection".to_string()));
                return Ok(());
            }
        }

        match message {
            Self::Msg::MessageReceived(maybe_network_message) if state.tcp.is_some() => {
                if let Some(network_message) = maybe_network_message.message {
                    match network_message {
                        crate::protocol::meta::network_message::Message::Auth(auth_message) => {
                            let p_state = state.auth.is_ok();
                            self.handle_auth(state, auth_message, myself.clone()).await;
                            // If we were not originally authenticated, but now we are, startup the node sync'ing logic
                            if !p_state && state.auth.is_ok() {
                                self.node_server.cast(
                                    NodeServerMessage::ConnectionAuthenticated(myself.get_id()),
                                )?;
                                self.after_authenticated(myself.clone(), state);
                                if state.ready.is_ok() {
                                    self.node_server.cast(NodeServerMessage::ConnectionReady(
                                        myself.get_id(),
                                    ))?;
                                }
                            }
                        }
                        crate::protocol::meta::network_message::Message::Node(node_message) => {
                            self.handle_node(state, node_message, myself);
                        }
                        crate::protocol::meta::network_message::Message::Control(
                            control_message,
                        ) => {
                            self.handle_control(state, control_message, myself).await?;
                        }
                    }
                }
            }
            Self::Msg::SendMessage(node_message) if state.tcp.is_some() => {
                state.tcp_send_node(node_message);
            }
            Self::Msg::GetAuthenticationState(reply) => {
                let _ = reply.send(state.auth.is_ok());
            }
            Self::Msg::GetReadyState(reply) => {
                let _ = reply.send(state.ready.is_ok());
            }
            _ => {
                // no-op, ignore
            }
        }
        Ok(())
    }

    async fn handle_supervisor_evt(
        &self,
        myself: ActorRef<Self::Msg>,
        message: SupervisionEvent,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            SupervisionEvent::ActorStarted(_) => {}
            SupervisionEvent::ActorFailed(actor, msg) => {
                if state.is_tcp_actor(actor.get_id()) {
                    tracing::error!(
                        "Node session {:?}'s TCP session panicked with '{msg}'",
                        state.name
                    );
                    myself.stop(Some("tcp_session_err".to_string()));
                } else if let Some(actor) = state.remote_actors.remove(&actor.get_id().pid()) {
                    tracing::warn!(
                        "Node session {:?} had a remote actor ({}) panic with {msg}",
                        state.name,
                        actor.get_id(),
                    );
                    actor.kill();

                    // NOTE: This is a legitimate panic of the `RemoteActor`, not the actor on the remote machine panicking (which
                    // is handled by the remote actor's supervisor). Therefore we should re-spawn the actor, and if we can't we
                    // should ourself die. Something is seriously wrong...
                    let pid = actor.get_id().pid();
                    let name = actor.get_name();
                    let _ = self
                        .get_or_spawn_remote_actor(&myself, name, pid, state)
                        .await?;
                } else {
                    tracing::error!("NodeSesion {:?} received an unknown child panic superivision message from {} - '{msg}'",
                        state.name,
                        actor.get_id()
                    );
                }
            }
            SupervisionEvent::ActorTerminated(actor, _, maybe_reason) => {
                if state.is_tcp_actor(actor.get_id()) {
                    tracing::info!("NodeSession {:?} connection closed", state.name);
                    myself.stop(Some("tcp_session_closed".to_string()));
                    // TODO: resilient connection?
                } else if let Some(actor) = state.remote_actors.remove(&actor.get_id().pid()) {
                    tracing::debug!(
                        "NodeSession {:?} received a child exit with reason '{maybe_reason:?}'",
                        state.name
                    );
                    actor
                        .stop_and_wait(Some("remote_exit".to_string()), None)
                        .await?;
                } else {
                    tracing::warn!("NodeSession {:?} received an unknown child actor exit event from {} - '{maybe_reason:?}'",
                        state.name,
                        actor.get_id(),
                    );
                }
            }
            // ======== Lifecycle event handlers (PG groups + PID registry) ======== //
            SupervisionEvent::ProcessGroupChanged(change) => match change {
                GroupChangeMessage::Join(scope, group, actors) => {
                    let filtered = actors
                        .into_iter()
                        .filter(|act| act.supports_remoting())
                        .map(|act| control_protocol::Actor {
                            name: act.get_name(),
                            pid: act.get_id().pid(),
                        })
                        .collect::<Vec<_>>();
                    if !filtered.is_empty() {
                        let msg = control_protocol::ControlMessage {
                            msg: Some(control_protocol::control_message::Msg::PgJoin(
                                control_protocol::PgJoin {
                                    scope,
                                    group,
                                    actors: filtered,
                                },
                            )),
                        };
                        state.tcp_send_control(msg);
                    }
                }
                GroupChangeMessage::Leave(scope, group, actors) => {
                    let filtered = actors
                        .into_iter()
                        .filter(|act| act.supports_remoting())
                        .map(|act| control_protocol::Actor {
                            name: act.get_name(),
                            pid: act.get_id().pid(),
                        })
                        .collect::<Vec<_>>();
                    if !filtered.is_empty() {
                        let msg = control_protocol::ControlMessage {
                            msg: Some(control_protocol::control_message::Msg::PgLeave(
                                control_protocol::PgLeave {
                                    scope,
                                    group,
                                    actors: filtered,
                                },
                            )),
                        };
                        state.tcp_send_control(msg);
                    }
                }
            },
            SupervisionEvent::PidLifecycleEvent(pid) => match pid {
                PidLifecycleEvent::Spawn(who) => {
                    if who.supports_remoting() {
                        let msg = control_protocol::ControlMessage {
                            msg: Some(control_protocol::control_message::Msg::Spawn(
                                control_protocol::Spawn {
                                    actors: vec![control_protocol::Actor {
                                        pid: who.get_id().pid(),
                                        name: who.get_name(),
                                    }],
                                },
                            )),
                        };
                        state.tcp_send_control(msg);
                    }
                }
                PidLifecycleEvent::Terminate(who) => {
                    if who.supports_remoting() {
                        let msg = control_protocol::ControlMessage {
                            msg: Some(control_protocol::control_message::Msg::Terminate(
                                control_protocol::Terminate {
                                    ids: vec![who.get_id().pid()],
                                },
                            )),
                        };
                        state.tcp_send_control(msg);
                    }
                }
            },
        }
        Ok(())
    }
}
