// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! [FromActorRef] wraps an [ActorCell] to send messages that can be converted
//! to its accepted type using [From]

use crate::{ActorCell, ActorRef, Message, MessagingErr};

/// [FromActorRef] wraps an [ActorCell] to send messages that can be converted
/// into its accepted type using [From]. [FromActorRef] allows to create isolation
/// between actors by hiding the actual message type.
pub struct FromActorRef<TFrom> {
    converter: Box<dyn Fn(TFrom) -> Result<(), MessagingErr<TFrom>>>,
    cell: ActorCell,
}

impl<TFrom> std::fmt::Debug for FromActorRef<TFrom> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FromActorRef")
            .field("cell", &self.cell)
            .finish()
    }
}

impl<TFrom> FromActorRef<TFrom> {
    /// Casts the message to the target message type of [ActorCell] and sends it 
    ///
    /// * `message` - The message to send
    ///
    /// Returns [Ok(())] on successful message send, [Err(MessagingErr)] otherwise
    pub fn send_message(&self, message: TFrom) -> Result<(), MessagingErr<TFrom>> {
        (self.converter)(message)
    }

    /// Retrieve a cloned [ActorCell] representing this [FromActorRef]
    pub fn get_cell(&self) -> ActorCell {
        self.cell.clone()
    }
}

impl<TMessage: Message> ActorRef<TMessage> {
    /// Constructs the [FromActorRef] for a specific type from [ActorRef]. Consumes
    /// the [ActorRef].
    pub fn from_ref<TFrom: Into<TMessage> + Clone>(self) -> FromActorRef<TFrom> {
        let actor_ref = self.clone();
        let cast_and_send = move |msg: TFrom| match actor_ref.send_message(msg.clone().into()) {
            Ok(_) => Ok(()),
            Err(MessagingErr::SendErr(_)) => Err(MessagingErr::SendErr(msg)),
            Err(MessagingErr::ChannelClosed) => Err(MessagingErr::ChannelClosed),
            Err(MessagingErr::InvalidActorType) => Err(MessagingErr::InvalidActorType),
        };
        FromActorRef::<TFrom> {
            converter: Box::new(cast_and_send),
            cell: self.get_cell(),
        }
    }
}
