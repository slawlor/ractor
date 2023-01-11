// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! A [RemoteActor] is an actor which handles serialized messages, and represents an actor
//! running on a remote `node()` server. See [crate::node::NodeServer] for more on inter-node
//! protocols

use std::collections::HashMap;

use ractor::cast;
use ractor::concurrency::JoinHandle;
use ractor::message::SerializedMessage;
use ractor::{Actor, ActorCell, ActorId, ActorName, ActorRef, RpcReplyPort, SpawnErr};

use crate::node::SessionMessage;
use crate::NodeId;

/// A Remote actor is an actor which represents an actor on another node
pub(crate) struct RemoteActor {
    /// The owning node session
    pub(crate) session: ActorRef<crate::node::NodeSession>,
}

impl RemoteActor {
    /// Spawn an actor of this type with a supervisor, automatically starting the actor
    ///
    /// * `name`: A name to give the actor. Useful for global referencing or debug printing
    /// * `handler`: The implementation of Self
    /// * `pid`: The actor's local id on the remote system
    /// * `node_id` The actor's node-identifier. Alongside `pid` this makes for a unique actor identifier
    /// * `supervisor`: The [ActorCell] which is to become the supervisor (parent) of this actor
    ///
    /// Returns a [Ok((ActorRef, JoinHandle<()>))] upon successful start, denoting the actor reference
    /// along with the join handle which will complete when the actor terminates. Returns [Err(SpawnErr)] if
    /// the actor failed to start
    pub(crate) async fn spawn_linked(
        self,
        name: Option<ActorName>,
        pid: u64,
        node_id: NodeId,
        supervisor: ActorCell,
    ) -> Result<(ActorRef<Self>, JoinHandle<()>), SpawnErr> {
        let actor_id = ActorId::Remote { node_id, pid };
        ractor::ActorRuntime::<_, _, Self>::spawn_linked_remote(name, self, actor_id, supervisor)
            .await
    }
}

#[derive(Default)]
pub(crate) struct RemoteActorState {
    tag: u64,
    pending_requests: HashMap<u64, RpcReplyPort<Vec<u8>>>,
}

impl RemoteActorState {
    fn next_tag(&mut self) -> u64 {
        self.tag += 1;
        self.tag
    }
}

// Placeholder message for a remote actor, as we won't actually
// handle anything but serialized messages on this channel
pub(crate) struct RemoteActorMessage;
impl ractor::Message for RemoteActorMessage {}

#[async_trait::async_trait]
impl Actor for RemoteActor {
    type Msg = RemoteActorMessage;
    type State = RemoteActorState;

    async fn pre_start(&self, _myself: ActorRef<Self>) -> Self::State {
        Self::State::default()
    }

    async fn handle(&self, _myself: ActorRef<Self>, _message: Self::Msg, _state: &mut Self::State) {
        panic!("Remote actors cannot handle local messages!");
    }

    async fn handle_serialized(
        &self,
        myself: ActorRef<Self>,
        message: SerializedMessage,
        state: &mut Self::State,
    ) {
        // get the local pid on the remote system
        let to = myself.get_id().pid();
        // messages should be forwarded over the network link (i.e. sent through the node session) to the intended
        // target node's relevant actor. The receiving runtime NodeSession will decode the message and pass it up
        // to the parent
        match message {
            SerializedMessage::Call(args, reply) => {
                let tag = state.next_tag();
                let node_msg = crate::protocol::node::NodeMessage {
                    msg: Some(crate::protocol::node::node_message::Msg::Call(
                        crate::protocol::node::Call {
                            to,
                            tag,
                            what: args,
                            timeout_ms: reply.get_timeout().map(|t| t.as_millis() as u64),
                        },
                    )),
                };
                state.pending_requests.insert(tag, reply);
                let _ = cast!(self.session, SessionMessage::SendMessage(node_msg));
            }
            SerializedMessage::Cast(args) => {
                let node_msg = crate::protocol::node::NodeMessage {
                    msg: Some(crate::protocol::node::node_message::Msg::Cast(
                        crate::protocol::node::Cast { to, what: args },
                    )),
                };
                let _ = cast!(self.session, SessionMessage::SendMessage(node_msg));
            }
            SerializedMessage::CallReply(message_tag, reply_data) => {
                if let Some(port) = state.pending_requests.remove(&message_tag) {
                    let _ = port.send(reply_data);
                }
            }
        }
    }
}
