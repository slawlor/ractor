// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! A [NodeSession] is an individual connection between a specific pair of
//! `node()`s and all of its authentication and communication for that
//! pairing

use std::collections::HashMap;
use std::convert::TryInto;
use std::net::SocketAddr;

use ractor::message::SerializedMessage;
use ractor::pg::GroupChangeMessage;
use ractor::registry::PidLifecycleEvent;
use ractor::rpc::CallResult;
use ractor::{Actor, ActorId, ActorProcessingErr, ActorRef, SpawnErr, SupervisionEvent};
use rand::Rng;
use tokio::time::Duration;

use super::{auth, NodeServer};
use crate::net::session::SessionMessage;
use crate::protocol::auth as auth_protocol;
use crate::protocol::control as control_protocol;
use crate::protocol::node as node_protocol;
use crate::remote_actor::RemoteActor;

const MIN_PING_LATENCY_MS: u64 = 1000;
const MAX_PING_LATENCY_MS: u64 = 5000;

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

/// Represents a remote connection to a `node()`. The [NodeSession] is the main
/// handler for all inter-node communication and handles
///
/// 1. The state of the authentication handshake
/// 2. Control messages for actor synchronization + group membership changes
/// 3. `RemoteActor`s wishing to send messages to their remote counterparts on the
/// remote system (and receive replies)
///
/// A [NodeSession] can either be a client or server session, depending on the connection sequence.
/// If it was an incoming request to the [NodeServer] then it's a "server" session, as
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
pub struct NodeSession {
    node_id: crate::NodeId,
    is_server: bool,
    cookie: String,
    node_server: ActorRef<NodeServer>,
    node_name: auth_protocol::NameMessage,
}

impl NodeSession {
    /// Construct a new [NodeSession] with the supplied
    /// arguments
    pub fn new(
        node_id: crate::NodeId,
        is_server: bool,
        cookie: String,
        node_server: ActorRef<NodeServer>,
        node_name: auth_protocol::NameMessage,
    ) -> Self {
        Self {
            node_id,
            is_server,
            cookie,
            node_server,
            node_name,
        }
    }
}

impl NodeSession {
    async fn handle_auth(
        &self,
        state: &mut NodeSessionState,
        message: auth_protocol::AuthenticationMessage,
        myself: ActorRef<Self>,
    ) {
        if state.auth.is_ok() {
            // nothing to do, we're already authenticated
            return;
        }
        if state.auth.is_close() {
            log::info!(
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
                        state.name = Some(auth_protocol::NameMessage {
                            name: server_challenge_value.name.clone(),
                            flags: server_challenge_value.flags.clone(),
                        });
                        // tell the node server that we now know this peer's name information
                        let _ =
                            self.node_server
                                .cast(super::SessionManagerMessage::UpdateSession {
                                    actor_id: myself.get_id(),
                                    name: self.node_name.clone(),
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
                    log::info!(
                        "Node Session {} is shutting down due to authentication failure",
                        self.node_id
                    );
                    myself.stop(Some("auth_fail".to_string()));
                }
                if let auth::ClientAuthenticationProcess::Ok = &next {
                    log::info!("Node Session {} is authenticated", self.node_id);
                }
                log::debug!("Next client auth state: {:?}", next);
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
                                |tx| super::SessionManagerMessage::CheckSession {
                                    peer_name: peer_name.clone(),
                                    reply: tx,
                                },
                                Some(Duration::from_millis(500)),
                            )
                            .await;
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
                                                            name: self.node_name.name.clone(),
                                                            flags: self.node_name.flags.clone(),
                                                            challenge: *challenge,
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
                    log::info!(
                        "Node Session {} is shutting down due to authentication failure",
                        self.node_id
                    );
                    myself.stop(Some("auth_fail".to_string()));
                }
                if let auth::ServerAuthenticationProcess::Ok(_) = &next {
                    log::info!("Node Session {} is authenticated", self.node_id);
                }
                log::debug!("Next server auth state: {:?}", next);
                state.auth = AuthenticationState::AsServer(next);
            }
        }
    }

    fn handle_node(
        &self,
        state: &mut NodeSessionState,
        message: node_protocol::NodeMessage,
        myself: ActorRef<Self>,
    ) {
        if !state.auth.is_ok() {
            log::warn!("Inter-node message received on unauthenticated NodeSession");
            return;
        }

        if let Some(msg) = message.msg {
            match msg {
                node_protocol::node_message::Msg::Cast(cast_args) => {
                    if let Some(actor) =
                        ractor::registry::where_is_pid(ActorId::Local(cast_args.to))
                    {
                        let _ = actor.send_serialized(SerializedMessage::Cast(cast_args.what));
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
                            let _ = actor.send_serialized(SerializedMessage::Call(
                                call_args.what,
                                (tx, timeout).into(),
                            ));
                        } else {
                            let _ = actor.send_serialized(SerializedMessage::Call(
                                call_args.what,
                                tx.into(),
                            ));
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
                                        super::SessionMessage::SendMessage(
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
                                    super::SessionMessage::SendMessage(
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
        myself: ActorRef<Self>,
    ) {
        if !state.auth.is_ok() {
            log::warn!("Control message received on unauthenticated NodeSession");
            return;
        }

        if let Some(msg) = message.msg {
            match msg {
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
                            log::error!("Failed to spawn remote actor with {}", spawn_err);
                        }
                    }
                }
                control_protocol::control_message::Msg::Terminate(termination) => {
                    for pid in termination.ids {
                        if let Some(actor) = state.remote_actors.remove(&pid) {
                            actor.stop(Some("remote".to_string()));
                            log::debug!(
                                "Actor {} on node {} exited, terminating local `RemoteActor` {}",
                                pid,
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
                    let delta_ms = std::time::SystemTime::now()
                        .duration_since(ts)
                        .expect("Time went backwards")
                        .as_millis();
                    log::debug!("Ping -> Pong took {}ms", delta_ms);
                    if delta_ms > 50 {
                        let default = || {
                            SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0)
                        };
                        log::warn!(
                            "Super long ping detected {} - {} ({}ms)",
                            state.local_addr.unwrap_or_else(default),
                            state.peer_addr.unwrap_or_else(default),
                            delta_ms
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
                                log::error!("Failed to spawn remote actor with '{}'", spawn_err);
                            }
                        }
                    }
                    // join the remote actors to the local PG group
                    if !cells.is_empty() {
                        ractor::pg::join(join.group, cells);
                    }
                }
                control_protocol::control_message::Msg::PgLeave(leave) => {
                    let mut cells = vec![];
                    for control_protocol::Actor { name, pid } in leave.actors {
                        match self
                            .get_or_spawn_remote_actor(&myself, name, pid, state)
                            .await
                        {
                            Ok(actor) => {
                                cells.push(actor.get_cell());
                            }
                            Err(spawn_err) => {
                                log::error!("Failed to spawn remote actor with '{}'", spawn_err);
                            }
                        }
                    }
                    // join the remote actors to the local PG group
                    if !cells.is_empty() {
                        ractor::pg::leave(leave.group, cells);
                    }
                }
            }
        }
    }

    /// Called once the session is authenticated
    fn after_authenticated(&self, myself: ActorRef<Self>, state: &mut NodeSessionState) {
        log::info!(
            "Session authenticated on NodeSession {} - ({:?})",
            self.node_id,
            state.peer_addr
        );

        // startup the ping healthcheck activity
        state.schedule_tcp_ping();

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

        // setup PG monitoring
        ractor::pg::monitor(
            ractor::pg::ALL_GROUPS_NOTIFICATION.to_string(),
            myself.get_cell(),
        );

        // Scan all PG groups + synchronize them
        let groups = ractor::pg::which_groups();
        for group in groups {
            let local_members = ractor::pg::get_local_members(&group)
                .into_iter()
                .filter(|v| v.supports_remoting())
                .map(|act| control_protocol::Actor {
                    name: act.get_name(),
                    pid: act.get_id().get_pid(),
                })
                .collect::<Vec<_>>();
            if !local_members.is_empty() {
                let control_message = control_protocol::ControlMessage {
                    msg: Some(control_protocol::control_message::Msg::PgJoin(
                        control_protocol::PgJoin {
                            group,
                            actors: local_members,
                        },
                    )),
                };
                state.tcp_send_control(control_message);
            }
        }
        // TODO: subscribe to the named registry and synchronize it? What happes on a name clash? How would this be handled
        // if both sessions had a "node_a" for example? Which resolves, local only?
    }

    /// Get a given remote actor, or spawn it if it doesn't exist.
    async fn get_or_spawn_remote_actor(
        &self,
        myself: &ActorRef<Self>,
        actor_name: Option<String>,
        actor_pid: u64,
        state: &mut NodeSessionState,
    ) -> Result<ActorRef<RemoteActor>, SpawnErr> {
        match state.remote_actors.get(&actor_pid) {
            Some(actor) => Ok(actor.clone()),
            _ => {
                let (remote_actor, _) = crate::remote_actor::RemoteActor {
                    session: myself.clone(),
                }
                .spawn_linked(actor_name, actor_pid, self.node_id, myself.get_cell())
                .await?;
                state.remote_actors.insert(actor_pid, remote_actor.clone());
                Ok(remote_actor)
            }
        }
    }
}

/// The state of the node session
pub struct NodeSessionState {
    tcp: Option<ActorRef<crate::net::session::Session>>,
    peer_addr: Option<SocketAddr>,
    local_addr: Option<SocketAddr>,
    name: Option<auth_protocol::NameMessage>,
    auth: AuthenticationState,
    remote_actors: HashMap<u64, ActorRef<RemoteActor>>,
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
        if let Some(tcp) = &self.tcp {
            #[allow(clippy::let_underscore_future)]
            let _ = tcp.send_after(Self::get_send_delay(), || {
                let ping = control_protocol::ControlMessage {
                    msg: Some(control_protocol::control_message::Msg::Ping(
                        control_protocol::Ping {
                            timestamp: Some(prost_types::Timestamp::from(
                                std::time::SystemTime::now(),
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

#[async_trait::async_trait]
impl Actor for NodeSession {
    type Msg = super::SessionMessage;
    type State = NodeSessionState;

    async fn pre_start(&self, _myself: ActorRef<Self>) -> Result<Self::State, ActorProcessingErr> {
        Ok(Self::State {
            tcp: None,
            name: None,
            auth: if self.is_server {
                AuthenticationState::AsServer(auth::ServerAuthenticationProcess::init())
            } else {
                AuthenticationState::AsClient(auth::ClientAuthenticationProcess::init())
            },
            remote_actors: HashMap::new(),
            peer_addr: None,
            local_addr: None,
        })
    }

    async fn post_stop(
        &self,
        myself: ActorRef<Self>,
        _state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        // unhook monitoring sessions
        ractor::pg::demonitor(
            ractor::pg::ALL_GROUPS_NOTIFICATION.to_string(),
            myself.get_id(),
        );
        ractor::registry::pid_registry::demonitor(myself.get_id());

        Ok(())
    }

    async fn handle(
        &self,
        myself: ActorRef<Self>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            super::SessionMessage::SetTcpStream(stream) if state.tcp.is_none() => {
                let peer_addr = stream.peer_addr()?;
                let my_addr = stream.local_addr()?;
                // startup the TCP socket handler for message write + reading
                let actor = crate::net::session::Session::spawn_linked(
                    myself.clone(),
                    stream,
                    peer_addr,
                    my_addr,
                    myself.get_cell(),
                )
                .await?;

                state.tcp = Some(actor);
                state.peer_addr = Some(peer_addr);
                state.local_addr = Some(my_addr);

                // If a client-connection, startup the handshake
                if !self.is_server {
                    state.tcp_send_auth(auth_protocol::AuthenticationMessage {
                        msg: Some(auth_protocol::authentication_message::Msg::Name(
                            self.node_name.clone(),
                        )),
                    });
                }
            }
            Self::Msg::MessageReceived(maybe_network_message) if state.tcp.is_some() => {
                if let Some(network_message) = maybe_network_message.message {
                    match network_message {
                        crate::protocol::meta::network_message::Message::Auth(auth_message) => {
                            let p_state = state.auth.is_ok();
                            self.handle_auth(state, auth_message, myself.clone()).await;
                            // If we were not originally authenticated, but now we are, startup the node sync'ing logic
                            if !p_state && state.auth.is_ok() {
                                self.after_authenticated(myself, state);
                            }
                        }
                        crate::protocol::meta::network_message::Message::Node(node_message) => {
                            self.handle_node(state, node_message, myself);
                        }
                        crate::protocol::meta::network_message::Message::Control(
                            control_message,
                        ) => {
                            self.handle_control(state, control_message, myself).await;
                        }
                    }
                }
            }
            Self::Msg::SendMessage(node_message) if state.tcp.is_some() => {
                state.tcp_send_node(node_message);
            }
            _ => {
                // no-op, ignore
            }
        }
        Ok(())
    }

    async fn handle_supervisor_evt(
        &self,
        myself: ActorRef<Self>,
        message: SupervisionEvent,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            SupervisionEvent::ActorStarted(_) => {}
            SupervisionEvent::ActorPanicked(actor, msg) => {
                if state.is_tcp_actor(actor.get_id()) {
                    log::error!(
                        "Node session {:?}'s TCP session panicked with '{}'",
                        state.name,
                        msg
                    );
                    myself.stop(Some("tcp_session_err".to_string()));
                } else if let Some(actor) = state.remote_actors.remove(&actor.get_id().get_pid()) {
                    log::warn!(
                        "Node session {:?} had a remote actor ({}) panic with {}",
                        state.name,
                        actor.get_id(),
                        msg
                    );
                    actor.kill();

                    // NOTE: This is a legitimate panic of the `RemoteActor`, not the actor on the remote machine panicking (which
                    // is handled by the remote actor's supervisor). Therefore we should re-spawn the actor
                    let pid = actor.get_id().get_pid();
                    let name = actor.get_name();
                    let _ = self
                        .get_or_spawn_remote_actor(&myself, name, pid, state)
                        .await?;
                } else {
                    log::error!("NodeSesion {:?} received an unknown child panic superivision message from {} - '{}'",
                        state.name,
                        actor.get_id(),
                        msg
                    );
                }
            }
            SupervisionEvent::ActorTerminated(actor, _, maybe_reason) => {
                if state.is_tcp_actor(actor.get_id()) {
                    log::info!("NodeSession {:?} connection closed", state.name);
                    myself.stop(Some("tcp_session_closed".to_string()));
                } else if let Some(actor) = state.remote_actors.remove(&actor.get_id().get_pid()) {
                    log::debug!(
                        "NodeSession {:?} received a child exit with reason '{:?}'",
                        state.name,
                        maybe_reason
                    );
                    actor.stop(Some("remote_exit".to_string()));
                } else {
                    log::warn!("NodeSession {:?} received an unknown child actor exit event from {} - '{:?}'",
                        state.name,
                        actor.get_id(),
                        maybe_reason,
                    );
                }
            }
            // ======== Lifecycle event handlers (PG groups + PID registry) ======== //
            SupervisionEvent::ProcessGroupChanged(change) => match change {
                GroupChangeMessage::Join(group, actors) => {
                    let filtered = actors
                        .into_iter()
                        .filter(|act| act.supports_remoting())
                        .map(|act| control_protocol::Actor {
                            name: act.get_name(),
                            pid: act.get_id().get_pid(),
                        })
                        .collect::<Vec<_>>();
                    if !filtered.is_empty() {
                        let msg = control_protocol::ControlMessage {
                            msg: Some(control_protocol::control_message::Msg::PgJoin(
                                control_protocol::PgJoin {
                                    group,
                                    actors: filtered,
                                },
                            )),
                        };
                        state.tcp_send_control(msg);
                    }
                }
                GroupChangeMessage::Leave(group, actors) => {
                    let filtered = actors
                        .into_iter()
                        .filter(|act| act.supports_remoting())
                        .map(|act| control_protocol::Actor {
                            name: act.get_name(),
                            pid: act.get_id().get_pid(),
                        })
                        .collect::<Vec<_>>();
                    if !filtered.is_empty() {
                        let msg = control_protocol::ControlMessage {
                            msg: Some(control_protocol::control_message::Msg::PgLeave(
                                control_protocol::PgLeave {
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
                                        pid: who.get_id().get_pid(),
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
                                    ids: vec![who.get_id().get_pid()],
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
