// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! [DerivedActorRef] wraps an [ActorCell] to send messages that can be converted
//! to its accepted type using [From]. It represents a subset of the messages supported
//! by the original actor.

use crate::{ActorCell, ActorRef, Message, MessagingErr};
use std::sync::Arc;

/// [DerivedActorRef] wraps an [ActorCell] to send messages that can be converted
/// into its accepted type using [From]. [DerivedActorRef] allows to create isolation
/// between actors by hiding the actual message type.
#[derive(Clone)]
pub struct DerivedActorRef<TFrom> {
    converter: Arc<dyn Fn(TFrom) -> Result<(), MessagingErr<TFrom>>>,
    inner: ActorCell,
}

impl<TFrom> std::fmt::Debug for DerivedActorRef<TFrom> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DerivedActorRef")
            .field("cell", &self.inner)
            .finish()
    }
}

// Allows all the functionality of ActorCell on DerivedActorRef
impl<TMessage> std::ops::Deref for DerivedActorRef<TMessage> {
    type Target = ActorCell;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<TFrom> DerivedActorRef<TFrom> {
    /// Casts the message to the target message type of [ActorCell] and sends it
    ///
    /// * `message` - The message to send
    ///
    /// Returns [Ok(())] on successful message send, [Err(MessagingErr)] otherwise
    pub fn send_message(&self, message: TFrom) -> Result<(), MessagingErr<TFrom>> {
        (self.converter)(message)
    }

    /// Retrieve a cloned [ActorCell] representing this [DerivedActorRef]
    pub fn get_cell(&self) -> ActorCell {
        self.inner.clone()
    }
}

impl<TMessage: Message> ActorRef<TMessage> {
    /// Constructs the [DerivedActorRef] for a specific type from [ActorRef]. This allows an
    /// actor which handles either a subset of the full actor's messages or a convertable type,
    /// allowing hiding of the original message type through implementation of [From] conversion.
    ///
    /// Returns a [DerivedActorRef] where the message type is convertable to the [ActorRef]'s
    /// original message type via [From]. In order to facilitate [MessagingErr::SendErr] the origina
    /// message type also needs to be reverse convertable via [TryFrom] to the `TFrom` type.
    ///
    /// This method will panic if a send error occurs, and the returned message cannot be converted back
    /// to the `TFrom` type. This should never happen, unless conversions are created incorrectly.
    pub fn get_derived<TFrom>(&self) -> DerivedActorRef<TFrom>
    where
        TMessage: From<TFrom>,
        TFrom: TryFrom<TMessage>,
    {
        let actor_ref = self.clone();
        let cast_and_send = move |msg: TFrom| {
            actor_ref.send_message(msg.into()).map_err(|err| match err {
                MessagingErr::SendErr(returned) => {
                    let Ok(err) = TFrom::try_from(returned) else {
                        panic!("Failed to deconvert message to from type");
                    };
                    MessagingErr::SendErr(err)
                }
                MessagingErr::ChannelClosed => MessagingErr::ChannelClosed,
                MessagingErr::InvalidActorType => MessagingErr::InvalidActorType,
            })
        };
        DerivedActorRef::<TFrom> {
            converter: Arc::new(cast_and_send),
            inner: self.get_cell(),
        }
    }
}
