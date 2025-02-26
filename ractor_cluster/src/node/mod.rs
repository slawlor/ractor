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
//!

/*
What's there to do? See tracking issue <https://github.com/slawlor/ractor/issues/16> for the most
up-to-date information on the status of remoting and actors

4. Populating the global named registered actors (do we want this?)
*/

pub mod auth;
pub mod client;
pub mod node_session;
pub use node_session::NodeSession;

use std::collections::HashMap;
use std::{cmp::Ordering, collections::hash_map::Entry};

use ractor::{Actor, ActorId, ActorProcessingErr, ActorRef, RpcReplyPort, SupervisionEvent};

use crate::net::IncomingEncryptionMode;
use crate::protocol::auth as auth_protocol;
use crate::{NodeId, RactorMessage};

const PROTOCOL_VERSION: u32 = 1;

/// Reply to a [NodeServerMessage::CheckSession] message
pub enum SessionCheckReply {
    /// There is no other connection with this peer
    NoOtherConnection,
    /// There is another connection with this peer, and it
    /// should continue. Shutdown this connection.
    OtherConnectionContinues,
    /// There is another connection with this peer, but
    /// this connection should take over. Terminating the other
    /// connection
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
#[derive(RactorMessage)]
pub enum NodeServerMessage {
    /// Notifies the session manager that a new incoming (`is_server = true`) or outgoing (`is_server = false`)
    /// [crate::NetworkStream] was accepted
    ConnectionOpened {
        /// The [crate::NetworkStream] for this network connection
        stream: crate::net::NetworkStream,
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

    /// Change the port used in the connection String for the [ crate::net::listener ].
    /// This is used if the port specified in [ NodeServer ] is 0 and the OS chooses an arbitrary
    /// free port.
    PortChanged {
        /// The new port number
        port: u16,
    },
}

/// Message from the TCP `ractor_cluster::net::session::Session` actor and the
/// monitoring Sesson actor
#[derive(RactorMessage)]
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
#[derive(Copy, Clone)]
pub enum NodeConnectionMode {
    /// Transitive connection mode. Node A connecting to Node B will list Node B's peers and try and connect to those as well
    Transitive,
    /// Nodes only connect to peers which are manually specified
    Isolated,
}
impl Default for NodeConnectionMode {
    fn default() -> Self {
        Self::Transitive
    }
}

/// Represents the server which is managing all node session instances
///
/// The [NodeServer] supervises a single `ractor_cluster::net::listener::Listener` actor which is
/// responsible for hosting a server port for incoming `node()` connections. It also supervises
/// all of the [NodeSession] actors which are tied to tcp sessions and manage the FSM around `node()`s
/// establishing inter connections.
pub struct NodeServer {
    port: crate::net::NetworkPort,
    cookie: String,
    node_name: String,
    hostname: String,
    encryption_mode: IncomingEncryptionMode,
    connection_mode: NodeConnectionMode,
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
        }
    }
}

/// Node session information
#[derive(Clone)]
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
pub struct NodeServerState {
    listener: ActorRef<crate::net::listener::ListenerMessage>,
    node_sessions: HashMap<ActorId, NodeServerSessionInformation>,
    node_id_counter: NodeId,
    this_node_name: auth_protocol::NameMessage,
    subscriptions: HashMap<String, Box<dyn NodeEventSubscription>>,
}

impl NodeServerState {
    fn check_peers(&self, new_peer: auth_protocol::NameMessage) -> SessionCheckReply {
        for (_key, value) in self.node_sessions.iter() {
            if let Some(existing_peer) = &value.peer_name {
                if existing_peer.name == new_peer.name {
                    match (
                        existing_peer.name.cmp(&self.this_node_name.name),
                        value.is_server,
                    ) {
                        // the peer's name is > this node's name and they connected to us
                        // or
                        // the peer's name is < this node's name and we connected to them
                        (Ordering::Greater, true) | (Ordering::Less, false) => {
                            value.actor.stop(Some("duplicate_connection".to_string()));
                            return SessionCheckReply::OtherConnectionContinues;
                        }
                        (Ordering::Greater, false) | (Ordering::Less, true) => {
                            // the inverse of the first two conditions, terminate the other
                            // connection and let this one continue
                            return SessionCheckReply::ThisConnectionContinues;
                        }
                        _ => {
                            // something funky is going on...
                            return SessionCheckReply::DuplicateConnection;
                        }
                    }
                }
            }
        }
        SessionCheckReply::NoOtherConnection
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
        let listener = crate::net::listener::Listener::new(
            self.port,
            myself.clone(),
            self.encryption_mode.clone(),
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
            },
            subscriptions: HashMap::new(),
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
                    ),
                    stream,
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
                    for (_, sub) in state.subscriptions.iter() {
                        sub.node_session_opened(ses.clone());
                    }
                    state.node_sessions.insert(actor.get_id(), ses);
                    state.node_id_counter += 1;
                } else {
                    // failed to startup actor, drop the socket
                    tracing::warn!("Failed to startup `NodeSession`, dropping connection");
                }
            }
            Self::Msg::ConnectionAuthenticated(actor_id) => {
                if let Some(entry) = state.node_sessions.get(&actor_id) {
                    for (_, sub) in state.subscriptions.iter() {
                        sub.node_session_authenicated(entry.clone());
                    }
                }
            }
            Self::Msg::ConnectionReady(actor_id) => {
                if let Some(entry) = state.node_sessions.get(&actor_id) {
                    for (_, sub) in state.subscriptions.iter() {
                        sub.node_session_ready(entry.clone());
                    }
                }
            }
            Self::Msg::UpdateSession { actor_id, name } => {
                if let Some(entry) = state.node_sessions.get_mut(&actor_id) {
                    entry.update(name);
                }
            }
            Self::Msg::CheckSession { peer_name, reply } => {
                let _ = reply.send(state.check_peers(peer_name));
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
                    let listener = crate::net::listener::Listener::new(
                        self.port,
                        myself.clone(),
                        self.encryption_mode.clone(),
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
                            for (_, sub) in state.subscriptions.iter() {
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
                    let listener = crate::net::listener::Listener::new(
                        self.port,
                        myself.clone(),
                        self.encryption_mode.clone(),
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
                            for (_, sub) in state.subscriptions.iter() {
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
