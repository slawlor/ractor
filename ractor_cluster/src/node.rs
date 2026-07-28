// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Erlang `node()` host communication for managing remote actor communication in
//! a cluster
//!
//! ## Overview
//!
//! A [NodeServer] handles opening the TCP listener and managing incoming and outgoing
//! [NodeSession] requests. [NodeSession]s represent a remote server, locally.
//!
//! Additionally, you can open a session as a "client" by requesting a new session from the [NodeServer]
//! after initially connecting a TcpStream to the desired endpoint and then attaching the [NodeSession]
//! to the TcpStream (and linking the actors). See [client::connect] for client-based connections
//!
//! ## Supervision
//!
//! The supervision tree is the following
//!
//! [NodeServer] supervises
//!     1. The server-socket TCP `ractor_cluster::net::listener::Listener`
//!     2. All of the individual [NodeSession]s
//!
//! Each [NodeSession] supervises
//!     1. The TCP `ractor_cluster::net::session::Session` connection
//!     2. All of the remote referenced actors `ractor_cluster::remote_actor::RemoteActor`.
//!        That way if the overall node session closes (due to tcp err for example) will
//!        lose connectivity to all of the remote actors
//!
//! Each `actor_cluster::net::session::Session` supervises
//!     1. A TCP writer actor (`ractor_cluster::net::session::SessionWriter`)
//!     2. A TCP reader actor (`ractor_cluster::net::session::SessionReader`)
//! -> If either child actor closes, then it will terminate the overall `ractor_cluster::net::session::Session` which in
//!    turn will terminate the [NodeSession] and the [NodeServer] will de-register the [NodeSession] from its
//!    internal state

/*
What's there to do? See tracking issue <https://github.com/slawlor/ractor/issues/16> for the most
up-to-date information on the status of remoting and actors

4. Populating the global named registered actors (do we want this?)
*/

pub mod auth;
pub mod client;
pub mod node_session;
use std::cmp::Ordering;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::num::NonZeroU64;

pub use node_session::NodeSession;
use ractor::Actor;
use ractor::ActorId;
use ractor::ActorProcessingErr;
use ractor::ActorRef;
use ractor::RpcReplyPort;
use ractor::SupervisionEvent;

use crate::net::IncomingEncryptionMode;
use crate::protocol::auth as auth_protocol;
use crate::NodeId;
use crate::RactorMessage;

const PROTOCOL_VERSION: u32 = 1;

/// Default maximum size, in bytes, of a single inbound cluster frame.
///
/// Frames larger than this limit are rejected before allocating a payload
/// buffer. Use [`NodeServer::with_max_inbound_frame_size`] to override the
/// limit when cluster messages legitimately exceed 16 MiB.
pub const DEFAULT_MAX_INBOUND_FRAME_SIZE: u64 = 16 * 1024 * 1024;

/// Reply to a [NodeServerMessage::CheckSession] message
#[derive(Debug)]
pub enum SessionCheckReply {
    /// There is no other connection with this peer
    NoOtherConnection,
    /// There is another connection with this peer, and it
    /// should continue. Shutdown this connection.
    OtherConnectionContinues,
    /// There is another connection with this peer, but
    /// this connection should take over once it authenticates.
    ThisConnectionContinues,
    /// There is another connection with the peer,
    /// in the same format as this attempted connection.
    /// Perhaps the other connection is dying or the peer is
    /// confused
    DuplicateConnection,
}

impl From<SessionCheckReply> for auth_protocol::server_status::Status {
    fn from(value: SessionCheckReply) -> Self {
        match value {
            SessionCheckReply::NoOtherConnection => Self::Ok,
            SessionCheckReply::ThisConnectionContinues => Self::OkSimultaneous,
            SessionCheckReply::OtherConnectionContinues => Self::NotOk,
            SessionCheckReply::DuplicateConnection => Self::Alive,
        }
    }
}

/// Messages to/from the session manager
#[allow(missing_debug_implementations)]
#[derive(RactorMessage)]
pub enum NodeServerMessage {
    /// Notifies the session manager that a new incoming (`is_server = true`) or outgoing (`is_server = false`)
    /// [crate::NetworkStream] was accepted
    ConnectionOpened {
        /// The [crate::NetworkStream] for this network connection
        stream: Box<crate::net::NetworkStream>,
        /// Flag denoting if it's a server (incoming) connection when [true], [false] for outgoing
        is_server: bool,
    },

    /// Notifies the session manager that a new external incoming (`is_server = true`) or outgoing (`is_server = false`)
    /// connection was opened using a custom transport implementing [crate::ClusterBidiStream].
    ConnectionOpenedExternal {
        /// The external stream implementing the bidi transport
        stream: Box<dyn crate::net::ClusterBidiStream>,
        /// Flag denoting if it's a server (incoming) connection when [true], [false] for outgoing
        is_server: bool,
    },

    /// This specific node session has authenticated
    ConnectionAuthenticated(ActorId),

    /// This specific node session has finished all state exchange after authentication
    ConnectionReady(ActorId),

    /// A request to check if a session is currently open, and if it is is the ordering such that we should
    /// reject the incoming request
    ///
    /// i.e. if A is connected to B and A.name > B.name, but then B connects to A, B's request to connect
    /// to A should be rejected
    CheckSession {
        /// The peer's name to investigate
        peer_name: auth_protocol::NameMessage,
        /// Reply channel for RPC
        reply: RpcReplyPort<SessionCheckReply>,
    },

    /// A request to update the session mapping with this now known node's name
    UpdateSession {
        /// The ID of the [NodeSession] actor
        actor_id: ActorId,
        /// The node's name (now that we've received it)
        name: auth_protocol::NameMessage,
    },

    /// Retrieve the current status of the node server, listing the node sessions
    GetSessions(RpcReplyPort<HashMap<NodeId, NodeServerSessionInformation>>),

    /// Subscribe to node events from the node server
    SubscribeToEvents {
        /// The id of this subscription
        id: String,
        /// The subscription handler
        subscription: Box<dyn NodeEventSubscription>,
    },

    /// Unsubscribe to node events for the given subscription id
    UnsubscribeToEvents(String),

    /// Change the port used in the connection String for the `ractor_cluster::net::listener`.
    /// This is used if the port specified in [ NodeServer ] is 0 and the OS chooses an arbitrary
    /// free port.
    PortChanged {
        /// The new port number
        port: u16,
    },
}

/// Message from the TCP `ractor_cluster::net::session::Session` actor and the
/// monitoring Sesson actor
#[derive(RactorMessage, Debug)]
pub enum NodeSessionMessage {
    /// A network message was received from the network
    MessageReceived(crate::protocol::NetworkMessage),

    /// Send a message over the node channel to the remote `node()`
    SendMessage(crate::protocol::node::NodeMessage),

    /// Retrieve whether the session is authenticated or not
    GetAuthenticationState(RpcReplyPort<bool>),

    /// Retrieve whether the session has finished initial state exchange after authentication
    GetReadyState(RpcReplyPort<bool>),
}

/// Node connection mode from the [Erlang](https://www.erlang.org/doc/reference_manual/distributed.html#node-connections)
/// specification. f a node A connects to node B, and node B has a connection to node C,
/// then node A also tries to connect to node C
#[derive(Copy, Clone, Debug, Default)]
pub enum NodeConnectionMode {
    /// Transitive connection mode. Node A connecting to Node B will list Node B's peers and try and connect to those as well
    #[default]
    Transitive,
    /// Nodes only connect to peers which are manually specified
    Isolated,
}

/// Represents the server which is managing all node session instances
///
/// The [NodeServer] supervises a single `ractor_cluster::net::listener::Listener` actor which is
/// responsible for hosting a server port for incoming `node()` connections. It also supervises
/// all of the [NodeSession] actors which are tied to tcp sessions and manage the FSM around `node()`s
/// establishing inter connections.
///
/// Inbound frames are limited to [`DEFAULT_MAX_INBOUND_FRAME_SIZE`] bytes by default.
/// The limit can be changed with [`NodeServer::with_max_inbound_frame_size`].
#[derive(Debug)]
pub struct NodeServer {
    port: crate::net::NetworkPort,
    cookie: String,
    node_name: String,
    hostname: String,
    encryption_mode: IncomingEncryptionMode,
    connection_mode: NodeConnectionMode,
    listen_addr: Option<IpAddr>,
    max_inbound_frame_size: u64,
}

impl NodeServer {
    /// Create a new node server instance
    ///
    /// * `port` - The port to run the [NodeServer] on for incoming requests. 0 to auto-select a free port.
    /// * `cookie` - The magic cookie for authentication between [NodeServer]s
    /// * `node_name` - The name of this node
    /// * `hostname` - The hostname of the machine
    /// * `encryption_mode`- (optional) Node socket encryption functionality (Default = [IncomingEncryptionMode::Raw])
    /// * `connection_mode` - (optional) Connection mode for peer nodes (Default = [NodeConnectionMode::Isolated])
    pub fn new(
        port: crate::net::NetworkPort,
        cookie: String,
        node_name: String,
        hostname: String,
        encryption_mode: Option<IncomingEncryptionMode>,
        connection_mode: Option<NodeConnectionMode>,
    ) -> Self {
        Self {
            port,
            cookie,
            node_name,
            hostname,
            encryption_mode: encryption_mode.unwrap_or(IncomingEncryptionMode::Raw),
            connection_mode: connection_mode.unwrap_or(NodeConnectionMode::Isolated),
            listen_addr: None,
            max_inbound_frame_size: DEFAULT_MAX_INBOUND_FRAME_SIZE,
        }
    }

    /// Set a custom listen address for the TCP listener.
    ///
    /// By default, the listener binds to `[::]` with dual-stack enabled
    /// (accepting both IPv4 and IPv6 connections). Use this method to
    /// override the bind address, e.g. to listen only on a specific
    /// interface or only on IPv4.
    pub fn with_listen_addr(mut self, addr: IpAddr) -> Self {
        self.listen_addr = Some(addr);
        self
    }

    /// Set the maximum accepted size, in bytes, of one inbound cluster frame.
    ///
    /// The limit applies before payload allocation and protobuf decoding. A
    /// peer that sends a larger frame is disconnected. This setting does not
    /// constrain outbound frames, so every peer must be configured to accept
    /// the largest message used by the cluster.
    pub fn with_max_inbound_frame_size(mut self, max_frame_size: u64) -> Self {
        self.max_inbound_frame_size = max_frame_size;
        self
    }
}

/// Node session information
#[derive(Debug, Clone)]
pub struct NodeServerSessionInformation {
    /// The NodeSession actor
    pub actor: ActorRef<NodeSessionMessage>,
    /// This peer's name (if set)
    pub peer_name: Option<auth_protocol::NameMessage>,
    /// Is server-incoming connection
    pub is_server: bool,
    /// The node's id
    pub node_id: NodeId,
    /// The peer's network address
    pub peer_addr: String,
}

impl NodeServerSessionInformation {
    fn new(
        actor: ActorRef<NodeSessionMessage>,
        is_server: bool,
        node_id: NodeId,
        peer_addr: String,
    ) -> Self {
        Self {
            actor,
            peer_name: None,
            is_server,
            node_id,
            peer_addr,
        }
    }

    fn update(&mut self, peer_name: auth_protocol::NameMessage) {
        self.peer_name = Some(peer_name);
    }
}

/// Trait which is utilized to receive Node events (node session
/// startup, shutdown, etc).
///
/// Node events can be used to try and reconnect node sessions
/// or handle custom shutdown logic as needed. They methods are
/// synchronous because ideally they'd be message sends and we
/// don't want to risk blocking the NodeServer's logic
pub trait NodeEventSubscription: Send + 'static {
    /// A node session has started up
    ///
    /// * `ses`: The [NodeServerSessionInformation] representing the current state
    ///   of the node session
    fn node_session_opened(&self, ses: NodeServerSessionInformation);

    /// A node session has shutdown
    ///
    /// * `ses`: The [NodeServerSessionInformation] representing the current state
    ///   of the node session
    fn node_session_disconnected(&self, ses: NodeServerSessionInformation);

    /// A node session authenticated
    ///
    /// * `ses`: The [NodeServerSessionInformation] representing the current state
    ///   of the node session
    fn node_session_authenicated(&self, ses: NodeServerSessionInformation);

    /// A node session is ready
    ///
    /// * `ses`: The [NodeServerSessionInformation] representing the current state
    ///   of the node session
    #[allow(unused_variables)]
    fn node_session_ready(&self, ses: NodeServerSessionInformation) {}
}

/// The state of the node server
#[allow(missing_debug_implementations)]
pub struct NodeServerState {
    listener: ActorRef<crate::net::ListenerMessage>,
    node_sessions: HashMap<ActorId, NodeServerSessionInformation>,
    node_id_counter: NodeId,
    this_node_name: auth_protocol::NameMessage,
    subscriptions: HashMap<String, Box<dyn NodeEventSubscription>>,
    connection_ids: HashMap<ActorId, Option<NonZeroU64>>,
    authenticated_sessions: HashSet<ActorId>,
}

#[derive(Clone, Copy, Debug)]
struct SessionElectionCandidate {
    actor_id: ActorId,
    is_server: bool,
    connection_id: Option<NonZeroU64>,
}

#[derive(Debug)]
struct SessionElection {
    candidate_survives: bool,
    losers: Vec<ActorRef<NodeSessionMessage>>,
}

fn elect_sessions(
    this_node_name: &str,
    peer_name: &str,
    mut candidates: Vec<SessionElectionCandidate>,
) -> Vec<ActorId> {
    if candidates.len() <= 1 {
        return candidates
            .into_iter()
            .map(|candidate| candidate.actor_id)
            .collect();
    }

    let has_server = candidates.iter().any(|candidate| candidate.is_server);
    let has_client = candidates.iter().any(|candidate| !candidate.is_server);

    if has_server && has_client {
        // Match the Erlang simultaneous-connect rule: retain the connection
        // initiated by the node whose full name sorts last.
        let preferred_is_server = match peer_name.cmp(this_node_name) {
            Ordering::Less => Some(false),
            Ordering::Greater => Some(true),
            Ordering::Equal => None,
        };
        if let Some(preferred_is_server) = preferred_is_server {
            candidates.retain(|candidate| candidate.is_server == preferred_is_server);
        }
    }

    if let Some(connection_id) = candidates
        .iter()
        .filter_map(|candidate| candidate.connection_id)
        .min()
    {
        candidates.retain(|candidate| candidate.connection_id == Some(connection_id));
    }

    // Only the accepting endpoint can break a legacy/repeated-nonce tie safely:
    // its status reply tells the initiator which physical connection survived.
    // Outgoing ties remain alive until that reply arrives from the peer.
    if candidates.len() > 1 && candidates.iter().all(|candidate| candidate.is_server) {
        let winner = candidates
            .iter()
            .map(|candidate| candidate.actor_id)
            .min()
            .expect("multiple election candidates cannot be empty");
        candidates.retain(|candidate| candidate.actor_id == winner);
    }

    candidates
        .into_iter()
        .map(|candidate| candidate.actor_id)
        .collect()
}

impl NodeServerState {
    fn register_session(
        &mut self,
        actor_id: ActorId,
        mut peer_name: auth_protocol::NameMessage,
    ) -> bool {
        let connection_id = peer_name.connection_id;
        peer_name.connection_id = 0;
        let session = match self.node_sessions.get_mut(&actor_id) {
            Some(session) => session,
            None => return false,
        };
        session.update(peer_name);
        self.connection_ids
            .insert(actor_id, NonZeroU64::new(connection_id));
        true
    }

    fn candidates_for_peer(
        &self,
        peer_name: &str,
        authenticated_only: bool,
    ) -> Vec<SessionElectionCandidate> {
        self.node_sessions
            .iter()
            .filter_map(|(id, session)| {
                (session.peer_name.as_ref().map(|name| name.name.as_str()) == Some(peer_name)
                    && (!authenticated_only || self.authenticated_sessions.contains(id)))
                .then_some(SessionElectionCandidate {
                    actor_id: *id,
                    is_server: session.is_server,
                    connection_id: self.connection_ids.get(id).copied().flatten(),
                })
            })
            .collect()
    }

    fn check_candidate(&self, actor_id: ActorId) -> SessionCheckReply {
        let session = match self.node_sessions.get(&actor_id) {
            Some(session) => session,
            None => return SessionCheckReply::OtherConnectionContinues,
        };
        let peer_name = match session.peer_name.as_ref() {
            Some(name) => name.name.as_str(),
            None => return SessionCheckReply::OtherConnectionContinues,
        };

        // Unauthenticated candidates cannot displace or reject one another. A
        // claimed node name is not trusted until the cookie challenge succeeds.
        let mut candidates = self.candidates_for_peer(peer_name, true);
        if !self.authenticated_sessions.contains(&actor_id) {
            candidates.push(SessionElectionCandidate {
                actor_id,
                is_server: session.is_server,
                connection_id: self.connection_ids.get(&actor_id).copied().flatten(),
            });
        }
        let had_competition = candidates.len() > 1;
        let elected = elect_sessions(&self.this_node_name.name, peer_name, candidates);
        let candidate_survives = elected.contains(&actor_id);

        match (candidate_survives, had_competition) {
            (true, false) => SessionCheckReply::NoOtherConnection,
            (true, true) => SessionCheckReply::ThisConnectionContinues,
            (false, _) => SessionCheckReply::OtherConnectionContinues,
        }
    }

    fn check_session(&self, peer_name: &auth_protocol::NameMessage) -> SessionCheckReply {
        let connection_id = NonZeroU64::new(peer_name.connection_id);
        let matching_sessions = self
            .node_sessions
            .iter()
            .filter_map(|(actor_id, session)| {
                (session.peer_name.as_ref().map(|name| name.name.as_str())
                    == Some(peer_name.name.as_str())
                    && self.connection_ids.get(actor_id).copied().flatten() == connection_id)
                    .then_some(*actor_id)
            })
            .collect::<Vec<_>>();

        if let [actor_id] = matching_sessions.as_slice() {
            return self.check_candidate(*actor_id);
        }

        // A missing registration is the compatibility path for direct callers
        // of CheckSession. A repeated or legacy nonce cannot identify one
        // candidate, so all such candidates may authenticate and the accepting
        // endpoint resolves the tie once their actor IDs are trustworthy.
        if !matching_sessions.is_empty() {
            return SessionCheckReply::NoOtherConnection;
        }

        let existing = self.candidates_for_peer(&peer_name.name, true);
        if existing.is_empty() {
            return SessionCheckReply::NoOtherConnection;
        }

        // This compatibility path cannot identify the calling actor and is
        // intentionally read-only.
        if existing.iter().any(|candidate| candidate.is_server) {
            SessionCheckReply::DuplicateConnection
        } else if peer_name.name < self.this_node_name.name {
            SessionCheckReply::ThisConnectionContinues
        } else {
            SessionCheckReply::OtherConnectionContinues
        }
    }

    fn commit_authenticated(&mut self, actor_id: ActorId) -> Option<SessionElection> {
        let peer_name = self
            .node_sessions
            .get(&actor_id)?
            .peer_name
            .as_ref()?
            .name
            .clone();
        self.authenticated_sessions.insert(actor_id);
        let candidates = self.candidates_for_peer(&peer_name, true);
        let elected = elect_sessions(&self.this_node_name.name, &peer_name, candidates);
        let candidate_survives = elected.contains(&actor_id);
        let losers = self
            .node_sessions
            .iter()
            .filter_map(|(id, session)| {
                (self.authenticated_sessions.contains(id)
                    && session.peer_name.as_ref().map(|name| name.name.as_str())
                        == Some(peer_name.as_str())
                    && !elected.contains(id))
                .then_some((*id, session.actor.clone()))
            })
            .collect::<Vec<_>>();
        for (loser_id, _) in &losers {
            self.authenticated_sessions.remove(loser_id);
        }

        Some(SessionElection {
            candidate_survives,
            losers: losers.into_iter().map(|(_, actor)| actor).collect(),
        })
    }

    fn is_elected(&self, actor_id: ActorId) -> bool {
        if !self.authenticated_sessions.contains(&actor_id) {
            return false;
        }
        let session = match self.node_sessions.get(&actor_id) {
            Some(session) => session,
            None => return false,
        };
        let peer_name = match session.peer_name.as_ref() {
            Some(name) => name.name.as_str(),
            None => return false,
        };
        let candidates = self.candidates_for_peer(peer_name, true);

        elect_sessions(&self.this_node_name.name, peer_name, candidates).contains(&actor_id)
    }
}

#[cfg_attr(feature = "async-trait", ractor::async_trait)]
impl Actor for NodeServer {
    type Msg = NodeServerMessage;
    type State = NodeServerState;
    type Arguments = ();
    async fn pre_start(
        &self,
        myself: ActorRef<Self::Msg>,
        _: (),
    ) -> Result<Self::State, ActorProcessingErr> {
        let listener = crate::net::Listener::new(
            self.port,
            myself.clone(),
            self.encryption_mode.clone(),
            self.listen_addr,
        );

        let (actor_ref, _) =
            Actor::spawn_linked(None, listener, myself.clone(), myself.get_cell()).await?;

        Ok(Self::State {
            node_sessions: HashMap::new(),
            listener: actor_ref,
            node_id_counter: 0,
            this_node_name: auth_protocol::NameMessage {
                flags: Some(auth_protocol::NodeFlags {
                    version: PROTOCOL_VERSION,
                }),
                name: format!("{}@{}", self.node_name, self.hostname),
                connection_string: format!("{}:{}", self.hostname, self.port),
                connection_id: 0,
            },
            subscriptions: HashMap::new(),
            connection_ids: HashMap::new(),
            authenticated_sessions: HashSet::new(),
        })
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            Self::Msg::ConnectionOpened { stream, is_server } => {
                let node_id = state.node_id_counter;
                let peer_addr = stream.peer_addr().to_string();
                if let Ok((actor, _)) = Actor::spawn_linked(
                    None,
                    NodeSession::new(
                        node_id,
                        is_server,
                        self.cookie.clone(),
                        myself.clone(),
                        state.this_node_name.clone(),
                        self.connection_mode,
                    )
                    .with_max_inbound_frame_size(self.max_inbound_frame_size),
                    *stream,
                    myself.get_cell(),
                )
                .await
                {
                    let ses = NodeServerSessionInformation::new(
                        actor.clone(),
                        is_server,
                        node_id,
                        peer_addr,
                    );
                    for sub in state.subscriptions.values() {
                        sub.node_session_opened(ses.clone());
                    }
                    state.node_sessions.insert(actor.get_id(), ses);
                    state.node_id_counter += 1;
                } else {
                    // failed to startup actor, drop the socket
                    tracing::warn!("Failed to startup `NodeSession`, dropping connection");
                }
            }
            Self::Msg::ConnectionOpenedExternal { stream, is_server } => {
                // Capture labels before consuming the stream
                let peer_label = stream.peer_label();
                let local_label = stream.local_label();
                let (reader, writer) = stream.split();

                // Wrap into a NetworkStream::External and proceed as usual
                let external_stream = Box::new(crate::net::NetworkStream::External {
                    peer_label: peer_label.clone(),
                    local_label,
                    reader,
                    writer,
                });

                let node_id = state.node_id_counter;
                // Prefer label if present for diagnostics, else fall back to placeholder address
                let peer_addr = peer_label.unwrap_or_else(|| "external".to_string());

                if let Ok((actor, _)) = Actor::spawn_linked(
                    None,
                    NodeSession::new(
                        node_id,
                        is_server,
                        self.cookie.clone(),
                        myself.clone(),
                        state.this_node_name.clone(),
                        self.connection_mode,
                    )
                    .with_max_inbound_frame_size(self.max_inbound_frame_size),
                    *external_stream,
                    myself.get_cell(),
                )
                .await
                {
                    let ses = NodeServerSessionInformation::new(
                        actor.clone(),
                        is_server,
                        node_id,
                        peer_addr,
                    );
                    for sub in state.subscriptions.values() {
                        sub.node_session_opened(ses.clone());
                    }
                    state.node_sessions.insert(actor.get_id(), ses);
                    state.node_id_counter += 1;
                } else {
                    tracing::warn!(
                        "Failed to startup `NodeSession` for external transport, dropping connection"
                    );
                }
            }
            Self::Msg::ConnectionAuthenticated(actor_id) => {
                if let Some(election) = state.commit_authenticated(actor_id) {
                    for loser in election.losers {
                        loser.stop(Some("duplicate_connection".to_string()));
                    }
                    if !election.candidate_survives {
                        return Ok(());
                    }
                    let entry = &state.node_sessions[&actor_id];
                    for sub in state.subscriptions.values() {
                        sub.node_session_authenicated(entry.clone());
                    }
                }
            }
            Self::Msg::ConnectionReady(actor_id) => {
                if state.is_elected(actor_id) {
                    let entry = &state.node_sessions[&actor_id];
                    for sub in state.subscriptions.values() {
                        sub.node_session_ready(entry.clone());
                    }
                }
            }
            Self::Msg::UpdateSession { actor_id, name } => {
                state.register_session(actor_id, name);
            }
            Self::Msg::CheckSession { peer_name, reply } => {
                let _ = reply.send(state.check_session(&peer_name));
            }
            Self::Msg::GetSessions(reply) => {
                let mut map = HashMap::new();
                for value in state.node_sessions.values() {
                    map.insert(value.node_id, value.clone());
                }
                let _ = reply.send(map);
            }
            Self::Msg::SubscribeToEvents { id, subscription } => {
                state.subscriptions.insert(id, subscription);
            }
            Self::Msg::UnsubscribeToEvents(id) => {
                let _ = state.subscriptions.remove(&id);
            }
            Self::Msg::PortChanged { port } => {
                state.this_node_name.connection_string = format!("{}:{}", self.hostname, port);
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
            SupervisionEvent::ActorFailed(actor, msg) => {
                if state.listener.get_id() == actor.get_id() {
                    tracing::error!(
                        "The Node server's TCP listener failed with '{msg}'. Respawning!"
                    );

                    // try to re-create the listener. If it's a port-bind issue, we will have already panicked on
                    // trying to start the NodeServer
                    let listener = crate::net::Listener::new(
                        self.port,
                        myself.clone(),
                        self.encryption_mode.clone(),
                        self.listen_addr,
                    );

                    let (actor_ref, _) =
                        Actor::spawn_linked(None, listener, myself.clone(), myself.get_cell())
                            .await?;
                    state.listener = actor_ref;
                } else {
                    match state.node_sessions.entry(actor.get_id()) {
                        Entry::Occupied(o) => {
                            tracing::warn!(
                                "Node session {:?} panicked with '{msg}'",
                                o.get().peer_name
                            );
                            let ses = o.remove();
                            state.connection_ids.remove(&actor.get_id());
                            state.authenticated_sessions.remove(&actor.get_id());
                            for sub in state.subscriptions.values() {
                                sub.node_session_disconnected(ses.clone());
                            }
                        }
                        Entry::Vacant(_) => {
                            tracing::warn!(
                                "An unknown actor ({:?}) panicked with '{msg}'",
                                actor.get_id()
                            );
                        }
                    }
                }
            }
            SupervisionEvent::ActorTerminated(actor, _, maybe_reason) => {
                if state.listener.get_id() == actor.get_id() {
                    tracing::error!(
                        "The Node server's TCP listener exited with '{maybe_reason:?}'. Respawning!"
                    );

                    // try to re-create the listener. If it's a port-bind issue, we will have already panicked on
                    // trying to start the NodeServer
                    let listener = crate::net::Listener::new(
                        self.port,
                        myself.clone(),
                        self.encryption_mode.clone(),
                        self.listen_addr,
                    );

                    let (actor_ref, _) =
                        Actor::spawn_linked(None, listener, myself.clone(), myself.get_cell())
                            .await?;
                    state.listener = actor_ref;
                } else {
                    match state.node_sessions.entry(actor.get_id()) {
                        Entry::Occupied(o) => {
                            tracing::warn!(
                                "Node session {:?} exited with '{:?}'",
                                o.get().peer_name,
                                maybe_reason
                            );
                            let ses = o.remove();
                            state.connection_ids.remove(&actor.get_id());
                            state.authenticated_sessions.remove(&actor.get_id());
                            for sub in state.subscriptions.values() {
                                sub.node_session_disconnected(ses.clone());
                            }
                        }
                        Entry::Vacant(_) => {
                            tracing::warn!(
                                "An unknown actor ({:?}) exited with '{:?}'",
                                actor.get_id(),
                                maybe_reason
                            );
                        }
                    }
                }
            }
            _ => {
                //no-op
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BoxRead, BoxWrite, ClusterBidiStream};
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use std::sync::Arc;
    use tokio::sync::{mpsc, Barrier};
    use tokio::time::{timeout, Duration};

    fn candidate(actor_id: u64, is_server: bool, connection_id: u64) -> SessionElectionCandidate {
        SessionElectionCandidate {
            actor_id: ActorId::Local(actor_id),
            is_server,
            connection_id: NonZeroU64::new(connection_id),
        }
    }

    #[test]
    fn missing_wire_nonce_decodes_as_legacy() {
        use prost::Message;

        // A legacy NameMessage containing only field 1 (`name`).
        let decoded = auth_protocol::NameMessage::decode(b"\x0a\x06a@host".as_slice())
            .expect("legacy name should decode");
        assert_eq!(decoded.connection_id, 0);
    }

    #[test]
    fn simultaneous_election_keeps_the_same_physical_connection() {
        // Connection 1 was initiated by node A; connection 2 by node B. Node B
        // sorts last, so both endpoints must retain connection 2 regardless of
        // its nonce or the order in which the candidates are observed.
        let at_a = elect_sessions(
            "a@host",
            "b@host",
            vec![candidate(2, true, 7), candidate(1, false, 19)],
        );
        let at_b = elect_sessions(
            "b@host",
            "a@host",
            vec![candidate(3, false, 7), candidate(4, true, 19)],
        );

        assert_eq!(at_a, vec![ActorId::Local(2)]);
        assert_eq!(at_b, vec![ActorId::Local(3)]);
    }

    #[test]
    fn connection_nonce_selects_the_same_same_direction_duplicate() {
        // Both connections were initiated by node A. Their actor IDs are
        // intentionally ordered differently at each endpoint; the shared nonce
        // must still select the same physical connection.
        let at_a = elect_sessions(
            "a@host",
            "b@host",
            vec![candidate(8, false, 31), candidate(9, false, 11)],
        );
        let at_b = elect_sessions(
            "b@host",
            "a@host",
            vec![candidate(15, true, 11), candidate(14, true, 31)],
        );

        assert_eq!(at_a, vec![ActorId::Local(9)]);
        assert_eq!(at_b, vec![ActorId::Local(15)]);
    }

    #[test]
    fn accepting_endpoint_resolves_legacy_and_repeated_nonce_ties() {
        let with_legacy = elect_sessions(
            "b@host",
            "a@host",
            vec![candidate(1, true, 0), candidate(2, true, 23)],
        );
        assert_eq!(with_legacy, vec![ActorId::Local(2)]);

        let all_legacy = elect_sessions(
            "b@host",
            "a@host",
            vec![candidate(6, true, 0), candidate(5, true, 0)],
        );
        assert_eq!(all_legacy, vec![ActorId::Local(5)]);

        let repeated = elect_sessions(
            "b@host",
            "a@host",
            vec![candidate(12, true, 41), candidate(11, true, 41)],
        );
        assert_eq!(repeated, vec![ActorId::Local(11)]);

        let outgoing_repeated = elect_sessions(
            "a@host",
            "b@host",
            vec![candidate(21, false, 41), candidate(22, false, 41)],
        );
        assert_eq!(
            outgoing_repeated,
            vec![ActorId::Local(21), ActorId::Local(22)]
        );
    }

    struct TestDuplex {
        stream: tokio::io::DuplexStream,
        peer_label: String,
        local_label: String,
    }

    impl TestDuplex {
        fn new(stream: tokio::io::DuplexStream, peer_label: &str, local_label: &str) -> Self {
            Self {
                stream,
                peer_label: peer_label.to_string(),
                local_label: local_label.to_string(),
            }
        }
    }

    impl ClusterBidiStream for TestDuplex {
        fn split(self: Box<Self>) -> (BoxRead, BoxWrite) {
            let (reader, writer) = tokio::io::split(self.stream);
            (Box::new(reader), Box::new(writer))
        }

        fn peer_label(&self) -> Option<String> {
            Some(self.peer_label.clone())
        }

        fn local_label(&self) -> Option<String> {
            Some(self.local_label.clone())
        }
    }

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum TestNode {
        A,
        B,
    }

    #[derive(Debug)]
    enum TestEvent {
        Disconnected(TestNode, ActorId, String),
        Ready(TestNode, ActorId),
    }

    struct TestSubscription {
        node: TestNode,
        events: mpsc::UnboundedSender<TestEvent>,
    }

    impl NodeEventSubscription for TestSubscription {
        fn node_session_opened(&self, _session: NodeServerSessionInformation) {}

        fn node_session_disconnected(&self, session: NodeServerSessionInformation) {
            let _ = self.events.send(TestEvent::Disconnected(
                self.node,
                session.actor.get_id(),
                session.peer_addr,
            ));
        }

        fn node_session_authenicated(&self, _session: NodeServerSessionInformation) {}

        fn node_session_ready(&self, session: NodeServerSessionInformation) {
            let _ = self
                .events
                .send(TestEvent::Ready(self.node, session.actor.get_id()));
        }
    }

    struct TestNodes {
        a: ActorRef<NodeServerMessage>,
        a_handle: ractor::concurrency::JoinHandle<()>,
        b: ActorRef<NodeServerMessage>,
        b_handle: ractor::concurrency::JoinHandle<()>,
        events: mpsc::UnboundedReceiver<TestEvent>,
    }

    impl TestNodes {
        async fn spawn() -> Self {
            let (events, event_receiver) = mpsc::unbounded_channel();
            let (a, a_handle) = Actor::spawn(
                None,
                NodeServer::new(
                    0,
                    "cookie".to_string(),
                    "a".to_string(),
                    "host".to_string(),
                    None,
                    None,
                ),
                (),
            )
            .await
            .expect("node A should start");
            let (b, b_handle) = Actor::spawn(
                None,
                NodeServer::new(
                    0,
                    "cookie".to_string(),
                    "b".to_string(),
                    "host".to_string(),
                    None,
                    None,
                ),
                (),
            )
            .await
            .expect("node B should start");

            a.cast(NodeServerMessage::SubscribeToEvents {
                id: "test".to_string(),
                subscription: Box::new(TestSubscription {
                    node: TestNode::A,
                    events: events.clone(),
                }),
            })
            .expect("node A subscription should enqueue");
            b.cast(NodeServerMessage::SubscribeToEvents {
                id: "test".to_string(),
                subscription: Box::new(TestSubscription {
                    node: TestNode::B,
                    events,
                }),
            })
            .expect("node B subscription should enqueue");

            // RPCs are mailbox barriers ensuring subscriptions and dynamic
            // listener ports are installed before connections are injected.
            ractor::call_t!(a, NodeServerMessage::GetSessions, 1_000)
                .expect("node A mailbox barrier should succeed");
            ractor::call_t!(b, NodeServerMessage::GetSessions, 1_000)
                .expect("node B mailbox barrier should succeed");

            Self {
                a,
                a_handle,
                b,
                b_handle,
                events: event_receiver,
            }
        }

        async fn wait_for_single_ready_pair(
            &mut self,
        ) -> (NodeServerSessionInformation, NodeServerSessionInformation) {
            timeout(Duration::from_secs(5), async {
                let mut ready = HashSet::new();
                let mut disconnected = HashSet::new();
                loop {
                    match self
                        .events
                        .recv()
                        .await
                        .expect("event channel should stay open")
                    {
                        TestEvent::Disconnected(node, _, _) => {
                            disconnected.insert(node);
                        }
                        TestEvent::Ready(node, actor_id) => {
                            ready.insert((node, actor_id));
                        }
                    }

                    if disconnected.len() != 2 {
                        continue;
                    }

                    let a_sessions = ractor::call_t!(self.a, NodeServerMessage::GetSessions, 1_000)
                        .expect("node A sessions should be available");
                    let b_sessions = ractor::call_t!(self.b, NodeServerMessage::GetSessions, 1_000)
                        .expect("node B sessions should be available");
                    if a_sessions.len() != 1 || b_sessions.len() != 1 {
                        continue;
                    }
                    let a_session = a_sessions
                        .into_values()
                        .next()
                        .expect("node A should have one session");
                    let b_session = b_sessions
                        .into_values()
                        .next()
                        .expect("node B should have one session");
                    if ready.contains(&(TestNode::A, a_session.actor.get_id()))
                        && ready.contains(&(TestNode::B, b_session.actor.get_id()))
                    {
                        return (a_session, b_session);
                    }
                }
            })
            .await
            .expect("duplicate session election should complete")
        }

        async fn wait_for_ready_pair(
            &mut self,
        ) -> (NodeServerSessionInformation, NodeServerSessionInformation) {
            timeout(Duration::from_secs(5), async {
                let mut ready = HashSet::new();
                loop {
                    match self.events.recv().await.expect("event channel should stay open") {
                        TestEvent::Disconnected(node, actor_id, peer_addr) => {
                            panic!(
                                "node {node:?} session {actor_id} ({peer_addr}) disconnected before ready"
                            );
                        }
                        TestEvent::Ready(node, actor_id) => {
                            ready.insert((node, actor_id));
                        }
                    }
                    if !ready.iter().any(|(node, _)| *node == TestNode::A)
                        || !ready.iter().any(|(node, _)| *node == TestNode::B)
                    {
                        continue;
                    }

                    let a_sessions = ractor::call_t!(self.a, NodeServerMessage::GetSessions, 1_000)
                        .expect("node A sessions should be available");
                    let b_sessions = ractor::call_t!(self.b, NodeServerMessage::GetSessions, 1_000)
                        .expect("node B sessions should be available");
                    if a_sessions.len() != 1 || b_sessions.len() != 1 {
                        continue;
                    }
                    let a_session = a_sessions
                        .into_values()
                        .next()
                        .expect("node A should have one session");
                    let b_session = b_sessions
                        .into_values()
                        .next()
                        .expect("node B should have one session");
                    if ready.contains(&(TestNode::A, a_session.actor.get_id()))
                        && ready.contains(&(TestNode::B, b_session.actor.get_id()))
                    {
                        return (a_session, b_session);
                    }
                }
            })
            .await
            .expect("session pair should become ready")
        }

        async fn stop(self) {
            self.a.stop(None);
            self.b.stop(None);
            self.a_handle.await.expect("node A should stop cleanly");
            self.b_handle.await.expect("node B should stop cleanly");
        }
    }

    struct TestConnection {
        a: TestDuplex,
        a_is_server: bool,
        b: TestDuplex,
        b_is_server: bool,
    }

    fn connection(label: &str, a_is_server: bool) -> TestConnection {
        let (a_stream, b_stream) = tokio::io::duplex(64 * 1024);
        TestConnection {
            a: TestDuplex::new(a_stream, &format!("{label}:b"), &format!("{label}:a")),
            a_is_server,
            b: TestDuplex::new(b_stream, &format!("{label}:a"), &format!("{label}:b")),
            b_is_server: !a_is_server,
        }
    }

    async fn open_simultaneously(
        a: ActorRef<NodeServerMessage>,
        b: ActorRef<NodeServerMessage>,
        first: TestConnection,
        second: TestConnection,
    ) {
        let barrier = Arc::new(Barrier::new(3));
        let first_barrier = barrier.clone();
        let first_a = a.clone();
        let first_b = b.clone();
        let first_task = tokio::spawn(async move {
            first_barrier.wait().await;
            first_a
                .cast(NodeServerMessage::ConnectionOpenedExternal {
                    stream: Box::new(first.a),
                    is_server: first.a_is_server,
                })
                .expect("first node A connection should enqueue");
            first_b
                .cast(NodeServerMessage::ConnectionOpenedExternal {
                    stream: Box::new(first.b),
                    is_server: first.b_is_server,
                })
                .expect("first node B connection should enqueue");
        });
        let second_barrier = barrier.clone();
        let second_task = tokio::spawn(async move {
            second_barrier.wait().await;
            a.cast(NodeServerMessage::ConnectionOpenedExternal {
                stream: Box::new(second.a),
                is_server: second.a_is_server,
            })
            .expect("second node A connection should enqueue");
            b.cast(NodeServerMessage::ConnectionOpenedExternal {
                stream: Box::new(second.b),
                is_server: second.b_is_server,
            })
            .expect("second node B connection should enqueue");
        });

        barrier.wait().await;
        first_task
            .await
            .expect("first connection task should finish");
        second_task
            .await
            .expect("second connection task should finish");
    }

    fn open_connection(
        a: &ActorRef<NodeServerMessage>,
        b: &ActorRef<NodeServerMessage>,
        connection: TestConnection,
    ) {
        a.cast(NodeServerMessage::ConnectionOpenedExternal {
            stream: Box::new(connection.a),
            is_server: connection.a_is_server,
        })
        .expect("node A connection should enqueue");
        b.cast(NodeServerMessage::ConnectionOpenedExternal {
            stream: Box::new(connection.b),
            is_server: connection.b_is_server,
        })
        .expect("node B connection should enqueue");
    }

    #[ractor::concurrency::test]
    async fn simultaneous_connections_converge_on_name_ordered_direction() {
        let mut nodes = TestNodes::spawn().await;
        open_simultaneously(
            nodes.a.clone(),
            nodes.b.clone(),
            connection("from-a", false),
            connection("from-b", true),
        )
        .await;

        let (at_a, at_b) = nodes.wait_for_single_ready_pair().await;
        assert!(at_a.is_server);
        assert!(!at_b.is_server);
        assert!(at_a.peer_addr.starts_with("from-b:"));
        assert!(at_b.peer_addr.starts_with("from-b:"));
        nodes.stop().await;
    }

    #[ractor::concurrency::test]
    async fn same_direction_duplicates_converge_on_one_physical_connection() {
        let mut nodes = TestNodes::spawn().await;
        open_simultaneously(
            nodes.a.clone(),
            nodes.b.clone(),
            connection("first", false),
            connection("second", false),
        )
        .await;

        let (at_a, at_b) = nodes.wait_for_single_ready_pair().await;
        assert!(!at_a.is_server);
        assert!(at_b.is_server);
        assert_eq!(
            at_a.peer_addr.split(':').next(),
            at_b.peer_addr.split(':').next()
        );
        nodes.stop().await;
    }

    #[ractor::concurrency::test]
    async fn unauthenticated_name_spoof_cannot_evict_a_ready_session() {
        let mut nodes = TestNodes::spawn().await;

        // Establish the non-preferred direction first. A later B -> A attempt
        // would win deterministic election, but only after it proves the cookie.
        open_connection(&nodes.a, &nodes.b, connection("trusted", false));
        let (trusted_at_a, _) = nodes.wait_for_ready_pair().await;
        assert!(!trusted_at_a.is_server);

        let (impostor, impostor_handle) = Actor::spawn(
            None,
            NodeServer::new(
                0,
                "wrong-cookie".to_string(),
                "b".to_string(),
                "host".to_string(),
                None,
                None,
            ),
            (),
        )
        .await
        .expect("impostor node should start");
        // Ensure the impostor's dynamic connection string is initialized.
        ractor::call_t!(impostor, NodeServerMessage::GetSessions, 1_000)
            .expect("impostor mailbox barrier should succeed");
        open_connection(&nodes.a, &impostor, connection("spoof", true));

        timeout(Duration::from_secs(5), async {
            loop {
                if let TestEvent::Disconnected(TestNode::A, actor_id, peer_addr) = nodes
                    .events
                    .recv()
                    .await
                    .expect("event channel should stay open")
                {
                    assert_ne!(actor_id, trusted_at_a.actor.get_id());
                    if peer_addr.starts_with("spoof:") {
                        break;
                    }
                }
            }
        })
        .await
        .expect("spoofed session should fail authentication");

        let sessions = ractor::call_t!(nodes.a, NodeServerMessage::GetSessions, 1_000)
            .expect("node A sessions should be available");
        assert_eq!(sessions.len(), 1);
        let surviving = sessions
            .into_values()
            .next()
            .expect("trusted session should survive");
        assert_eq!(surviving.actor.get_id(), trusted_at_a.actor.get_id());
        assert!(
            ractor::call_t!(surviving.actor, NodeSessionMessage::GetReadyState, 1_000)
                .expect("trusted session readiness should be available")
        );

        impostor.stop(None);
        impostor_handle
            .await
            .expect("impostor node should stop cleanly");
        nodes.stop().await;
    }

    #[test]
    fn test_node_server_creation() {
        let node = NodeServer::new(
            9090,
            "test_cookie".to_string(),
            "test_node".to_string(),
            "localhost".to_string(),
            None,
            None,
        );

        assert_eq!(node.port, 9090);
        assert_eq!(node.cookie, "test_cookie");
        assert_eq!(node.node_name, "test_node");
        assert_eq!(node.hostname, "localhost");
        // listen_addr should default to None
        assert!(node.listen_addr.is_none());
        assert_eq!(node.max_inbound_frame_size, DEFAULT_MAX_INBOUND_FRAME_SIZE);
    }

    #[test]
    fn test_node_server_with_listen_addr_ipv4() {
        let ipv4_addr = IpAddr::V4(Ipv4Addr::LOCALHOST);

        let node = NodeServer::new(
            9090,
            "test_cookie".to_string(),
            "test_node".to_string(),
            "localhost".to_string(),
            None,
            None,
        )
        .with_listen_addr(ipv4_addr);

        assert_eq!(node.listen_addr, Some(ipv4_addr));
        assert_eq!(node.port, 9090); // port should remain unchanged
    }

    #[test]
    fn test_node_server_with_listen_addr_ipv6() {
        let ipv6_addr = IpAddr::V6(Ipv6Addr::LOCALHOST);

        let node = NodeServer::new(
            9090,
            "test_cookie".to_string(),
            "test_node".to_string(),
            "localhost".to_string(),
            None,
            None,
        )
        .with_listen_addr(ipv6_addr);

        assert_eq!(node.listen_addr, Some(ipv6_addr));
    }

    #[test]
    fn test_node_server_default_encryption_raw() {
        let node = NodeServer::new(
            9090,
            "test_cookie".to_string(),
            "test_node".to_string(),
            "localhost".to_string(),
            None,
            None,
        );

        // Should default to Raw encryption mode
        match node.encryption_mode {
            IncomingEncryptionMode::Raw => {
                // Expected
            }
            _ => {
                panic!("Expected IncomingEncryptionMode::Raw");
            }
        }
    }

    #[test]
    fn test_node_server_default_connection_mode() {
        let node = NodeServer::new(
            9090,
            "test_cookie".to_string(),
            "test_node".to_string(),
            "localhost".to_string(),
            None,
            None,
        );

        // Should default to Isolated connection mode
        match node.connection_mode {
            NodeConnectionMode::Isolated => {
                // Expected
            }
            _ => {
                panic!("Expected NodeConnectionMode::Isolated");
            }
        }
    }

    #[test]
    fn test_node_server_builder_chaining() {
        let ipv4_addr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));

        let node = NodeServer::new(
            9090,
            "test_cookie".to_string(),
            "test_node".to_string(),
            "localhost".to_string(),
            None,
            None,
        )
        .with_listen_addr(ipv4_addr);

        assert_eq!(node.listen_addr, Some(ipv4_addr));
        assert_eq!(node.port, 9090);
        assert_eq!(node.node_name, "test_node");
    }

    #[test]
    fn test_node_server_with_max_inbound_frame_size() {
        let node = NodeServer::new(
            9090,
            "test_cookie".to_string(),
            "test_node".to_string(),
            "localhost".to_string(),
            None,
            None,
        )
        .with_max_inbound_frame_size(1024);

        assert_eq!(node.max_inbound_frame_size, 1024);
    }
}
