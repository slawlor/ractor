// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! A [RemoteActor] is an actor which handles serialized messages, and represents an actor
//! running on a remote `node()` server. See [crate::node::NodeServer] for more on inter-node
//! protocols

use std::collections::HashMap;

use ractor::concurrency::JoinHandle;
use ractor::message::SerializedMessage;
use ractor::{cast, ActorProcessingErr};
use ractor::{Actor, ActorCell, ActorId, ActorName, ActorRef, RpcReplyPort, SpawnErr};
use ractor_cluster_derive::RactorMessage;

use crate::node::NodeSessionMessage;
use crate::NodeId;

#[cfg(test)]
mod tests;

/// A [RemoteActor] is an actor which represents an actor on another node
///
/// A [RemoteActor] handles serialized messages without decoding them, forwarding them
/// to the remote system for decoding + handling by the real implementation.
/// Therefore [RemoteActor]s can be thought of as a "shim" to a real actor on a remote system
pub(crate) struct RemoteActor;

impl RemoteActor {
    /// Spawn a [RemoteActor] with a supervisor (which should always be a [super::node::NodeSession])
    /// and automatically start the actor
    ///
    /// * `name`: A name to give the actor. Useful for global referencing or debug printing
    /// * `pid`: The actor's local id on the remote system
    /// * `node_id` The id of the [super::node::NodeSession]. Alongside `pid` this makes for a unique actor identifier
    /// * `supervisor`: The [super::node::NodeSession]'s [ActorCell] handle which will be linked in
    /// the supervision tree
    ///
    /// Returns a [Ok((ActorRef, JoinHandle<()>))] upon successful start, denoting the actor reference
    /// along with the join handle which will complete when the actor terminates. Returns [Err(SpawnErr)] if
    /// the actor failed to start
    pub(crate) async fn spawn_linked(
        self,
        session: ActorRef<super::node::NodeSession>,
        name: Option<ActorName>,
        pid: u64,
        node_id: NodeId,
        supervisor: ActorCell,
    ) -> Result<(ActorRef<Self>, JoinHandle<()>), SpawnErr> {
        let actor_id = ActorId::Remote { node_id, pid };
        ractor::ActorRuntime::<Self>::spawn_linked_remote(name, self, actor_id, session, supervisor)
            .await
    }
}

pub(crate) struct RemoteActorState {
    message_tag: u64,
    /// The map of <message_tag, serialized_rpc_reply> port for network
    /// handling of [SerializedMessage::CallReply]s
    pending_requests: HashMap<u64, RpcReplyPort<Vec<u8>>>,

    /// Owning session
    session: ActorRef<crate::node::NodeSession>,
}

impl RemoteActorState {
    /// Increment the message tag and return the "next" value
    fn get_and_increment_mtag(&mut self) -> u64 {
        self.message_tag += 1;
        self.message_tag
    }
}

// Placeholder message for a remote actor, as we won't actually
// handle anything but serialized messages on this channel
#[derive(RactorMessage)]
pub(crate) struct RemoteActorMessage;

#[async_trait::async_trait]
impl Actor for RemoteActor {
    type Msg = RemoteActorMessage;
    type State = RemoteActorState;
    type Arguments = ActorRef<crate::node::NodeSession>;
    async fn pre_start(
        &self,
        _myself: ActorRef<Self>,
        session: ActorRef<crate::node::NodeSession>,
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(Self::State {
            session,
            message_tag: 0,
            pending_requests: HashMap::new(),
        })
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self>,
        _message: Self::Msg,
        _state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        Err(From::from("`RemoteActor`s cannot handle local messages!"))
    }

    async fn handle_serialized(
        &self,
        myself: ActorRef<Self>,
        message: SerializedMessage,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        // get the local pid on the remote system
        let to = myself.get_id().pid();
        // messages should be forwarded over the network link (i.e. sent through the node session) to the intended
        // target node's relevant actor. The receiving runtime NodeSession will decode the message and pass it up
        // to the parent. However `SerializedMessage::CallReply` is a network reply to a send call request
        match message {
            SerializedMessage::Call {
                args,
                reply,
                variant,
                metadata,
            } => {
                // Handle Call
                let tag = state.get_and_increment_mtag();
                let node_msg = crate::protocol::node::NodeMessage {
                    msg: Some(crate::protocol::node::node_message::Msg::Call(
                        crate::protocol::node::Call {
                            to,
                            tag,
                            what: args,
                            timeout_ms: reply.get_timeout().map(|t| t.as_millis() as u64),
                            variant,
                            metadata,
                        },
                    )),
                };
                state.pending_requests.insert(tag, reply);
                let _ = cast!(state.session, NodeSessionMessage::SendMessage(node_msg));
            }
            SerializedMessage::Cast {
                args,
                variant,
                metadata,
            } => {
                // Handle Cast
                let node_msg = crate::protocol::node::NodeMessage {
                    msg: Some(crate::protocol::node::node_message::Msg::Cast(
                        crate::protocol::node::Cast {
                            to,
                            what: args,
                            variant,
                            metadata,
                        },
                    )),
                };
                let _ = cast!(state.session, NodeSessionMessage::SendMessage(node_msg));
            }
            SerializedMessage::CallReply(message_tag, reply_data) => {
                // Handle the reply to a "Call" message
                if let Some(port) = state.pending_requests.remove(&message_tag) {
                    let _ = port.send(reply_data);
                }
            }
        }
        Ok(())
    }
}
