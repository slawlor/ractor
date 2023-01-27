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
//! after intially connecting a [TcpStream] to the desired endpoint and then attaching the [NodeSession]
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
use tokio::net::TcpStream;

use std::collections::HashMap;
use std::{cmp::Ordering, collections::hash_map::Entry};

use ractor::{cast, Actor, ActorId, ActorProcessingErr, ActorRef, RpcReplyPort, SupervisionEvent};

use crate::protocol::auth as auth_protocol;

const PROTOCOL_VERSION: u32 = 1;

/// Reply to a [SessionManagerMessage::CheckSession] message
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
pub enum SessionManagerMessage {
    /// Notifies the session manager that a new incoming (`is_server = true`) or outgoing (`is_server = false`)
    /// [TcpStream] was accepted
    ConnectionOpened {
        /// The [TcpStream] for this network connection
        stream: TcpStream,
        /// Flag denoting if it's a server (incoming) connection when [true], [false] for outgoing
        is_server: bool,
    },

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
}
impl ractor::Message for SessionManagerMessage {}

/// Message from the TCP `ractor_cluster::net::session::Session` actor and the
/// monitoring Sesson actor
pub enum SessionMessage {
    /// The Session actor is setting it's handle
    SetTcpStream(TcpStream),

    /// A network message was received from the network
    MessageReceived(crate::protocol::NetworkMessage),

    /// Send a message over the node channel to the remote `node()`
    SendMessage(crate::protocol::node::NodeMessage),
}
impl ractor::Message for SessionMessage {}

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
}

impl NodeServer {
    /// Create a new node server instance
    pub fn new(
        port: crate::net::NetworkPort,
        cookie: String,
        node_name: String,
        hostname: String,
    ) -> Self {
        Self {
            port,
            cookie,
            node_name,
            hostname,
        }
    }
}

struct NodeServerSessionInformation {
    actor: ActorRef<NodeSession>,
    peer_name: Option<auth_protocol::NameMessage>,
    is_server: bool,
}

impl NodeServerSessionInformation {
    fn new(actor: ActorRef<NodeSession>, is_server: bool) -> Self {
        Self {
            actor,
            peer_name: None,
            is_server,
        }
    }

    fn update(&mut self, peer_name: auth_protocol::NameMessage) {
        self.peer_name = Some(peer_name);
    }
}

/// The state of the node server
pub struct NodeServerState {
    listener: ActorRef<crate::net::listener::Listener>,
    node_sessions: HashMap<ActorId, NodeServerSessionInformation>,
    node_id_counter: u64,
    this_node_name: auth_protocol::NameMessage,
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

#[async_trait::async_trait]
impl Actor for NodeServer {
    type Msg = SessionManagerMessage;
    type State = NodeServerState;
    async fn pre_start(&self, myself: ActorRef<Self>) -> Result<Self::State, ActorProcessingErr> {
        let listener = crate::net::listener::Listener::new(self.port, myself.clone());

        let (actor_ref, _) = Actor::spawn_linked(None, listener, myself.get_cell()).await?;

        Ok(Self::State {
            node_sessions: HashMap::new(),
            listener: actor_ref,
            node_id_counter: 0,
            this_node_name: auth_protocol::NameMessage {
                flags: Some(auth_protocol::NodeFlags {
                    version: PROTOCOL_VERSION,
                }),
                name: format!("{}@{}", self.node_name, self.hostname),
            },
        })
    }

    async fn handle(
        &self,
        myself: ActorRef<Self>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            Self::Msg::ConnectionOpened { stream, is_server } => {
                let node_id = state.node_id_counter;
                if let Ok((actor, _)) = Actor::spawn_linked(
                    None,
                    NodeSession::new(
                        node_id,
                        is_server,
                        self.cookie.clone(),
                        myself.clone(),
                        state.this_node_name.clone(),
                    ),
                    myself.get_cell(),
                )
                .await
                {
                    let _ = cast!(actor, SessionMessage::SetTcpStream(stream));
                    state.node_sessions.insert(
                        actor.get_id(),
                        NodeServerSessionInformation::new(actor.clone(), is_server),
                    );
                    state.node_id_counter += 1;
                } else {
                    // failed to startup actor, drop the socket
                    log::warn!("Failed to startup `NodeSession`, dropping connection");
                    drop(stream);
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
            SupervisionEvent::ActorPanicked(actor, msg) => {
                if state.listener.get_id() == actor.get_id() {
                    log::error!(
                        "The Node server's TCP listener failed with '{}'. Respawning!",
                        msg
                    );

                    // try to re-create the listener. If it's a port-bind issue, we will have already panicked on
                    // trying to start the NodeServer
                    let listener = crate::net::listener::Listener::new(self.port, myself.clone());

                    let (actor_ref, _) =
                        Actor::spawn_linked(None, listener, myself.get_cell()).await?;
                    state.listener = actor_ref;
                } else {
                    match state.node_sessions.entry(actor.get_id()) {
                        Entry::Occupied(o) => {
                            log::warn!(
                                "Node session {:?} panicked with '{}'",
                                o.get().peer_name,
                                msg
                            );
                            o.remove();
                        }
                        Entry::Vacant(_) => {
                            log::warn!(
                                "An unknown actor ({:?}) panicked with '{}'",
                                actor.get_id(),
                                msg
                            );
                        }
                    }
                }
            }
            SupervisionEvent::ActorTerminated(actor, _, maybe_reason) => {
                if state.listener.get_id() == actor.get_id() {
                    log::error!(
                        "The Node server's TCP listener exited with '{:?}'. Respawning!",
                        maybe_reason
                    );

                    // try to re-create the listener. If it's a port-bind issue, we will have already panicked on
                    // trying to start the NodeServer
                    let listener = crate::net::listener::Listener::new(self.port, myself.clone());

                    let (actor_ref, _) =
                        Actor::spawn_linked(None, listener, myself.get_cell()).await?;
                    state.listener = actor_ref;
                } else {
                    match state.node_sessions.entry(actor.get_id()) {
                        Entry::Occupied(o) => {
                            log::warn!(
                                "Node session {:?} exited with '{:?}'",
                                o.get().peer_name,
                                maybe_reason
                            );
                            o.remove();
                        }
                        Entry::Vacant(_) => {
                            log::warn!(
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
