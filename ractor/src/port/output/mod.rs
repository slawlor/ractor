// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Output ports for publish-subscribe notifications between actors
//!
//! This notion extends beyond traditional actors in this that is a publish-subscribe
//! mechanism we've added in `ractor`. Output ports are ports which can have messages published
//! to them which are automatically forwarded to downstream actors waiting for inputs. They optionally
//! have a message transformer attached to them to convert them to the appropriate message type

use std::sync::RwLock;

use crate::concurrency::JoinHandle;
use tokio::sync::broadcast as pubsub;

use crate::{Actor, ActorRef, Message};

#[cfg(test)]
mod tests;

/// Output messages, since they need to be replicated, require [Clone] in addition
/// to the base [Message] constraints
pub trait OutputMessage: Message + Clone {}
impl<T: Message + Clone> OutputMessage for T {}

/// An [OutputPort] is a publish-subscribe mechanism for connecting actors together.
/// It allows actors to emit messages without knowing which downstream actors are subscribed.
///
/// You can subscribe to the output port with an [ActorRef] and a message converter from the output
/// type to the actor's expected input type. If the actor is dropped or stops, the subscription will
/// be dropped and if the output port is dropped, then the subscription will also be dropped
/// automatically.
pub struct OutputPort<TMsg>
where
    TMsg: OutputMessage,
{
    tx: pubsub::Sender<Option<TMsg>>,
    subscriptions: RwLock<Vec<OutputPortSubscription>>,
}

impl<TMsg> Default for OutputPort<TMsg>
where
    TMsg: OutputMessage,
{
    fn default() -> Self {
        // We only need enough buffer for the subscription task to forward to the input port
        // of the receiving actor. Hence 10 should be plenty.
        let (tx, _rx) = pubsub::channel(10);
        Self {
            tx,
            subscriptions: RwLock::new(vec![]),
        }
    }
}

impl<TMsg> OutputPort<TMsg>
where
    TMsg: OutputMessage,
{
    /// Subscribe to the output port, passing in a converter to convert to the input message
    /// of another actor
    ///
    /// * `receiver` - The reference to the actor which will receive forwarded messages
    /// * `converter` - The converter which will convert the output message type to the
    /// receiver's input type and return [Some(_)] if the message should be forwarded, [None]
    /// if the message should be skipped.
    pub fn subscribe<TReceiver, F>(&self, receiver: ActorRef<TReceiver>, converter: F)
    where
        F: Fn(TMsg) -> Option<TReceiver::Msg> + Send + 'static,
        TReceiver: Actor,
    {
        let mut subs = self.subscriptions.write().unwrap();

        // filter out dead subscriptions, since they're no longer valid
        subs.retain(|sub| !sub.is_dead());

        let sub = OutputPortSubscription::new::<TMsg, F, TReceiver>(
            self.tx.subscribe(),
            converter,
            receiver,
        );
        subs.push(sub);
    }

    /// Send a message on the output port
    ///
    /// * `msg`: The message to send
    pub fn send(&self, msg: TMsg) {
        if self.tx.receiver_count() > 0 {
            let _ = self.tx.send(Some(msg));
        }
    }
}

impl<TMsg> Drop for OutputPort<TMsg>
where
    TMsg: OutputMessage,
{
    fn drop(&mut self) {
        let mut subs = self.subscriptions.write().unwrap();
        for sub in subs.iter() {
            sub.stop();
        }
        subs.clear();
    }
}

// ============== Subscription implementation ============== //

/// The output port's subscription handle. It holds a handle to a [JoinHandle]
/// which listens to the [pubsub::Receiver] to see if there's a new message, and if there is
/// forwards it to the [ActorRef] asynchronously using the specified converter.
struct OutputPortSubscription {
    handle: JoinHandle<()>,
}

impl OutputPortSubscription {
    /// Determine if the subscription is dead
    pub fn is_dead(&self) -> bool {
        self.handle.is_finished()
    }

    /// Stop the subscription, by aborting the underlying [JoinHandle]
    pub fn stop(&self) {
        self.handle.abort();
    }

    /// Create a new subscription
    pub fn new<TMsg, F, TReceiver>(
        mut port: pubsub::Receiver<Option<TMsg>>,
        converter: F,
        receiver: ActorRef<TReceiver>,
    ) -> Self
    where
        TMsg: OutputMessage,
        F: Fn(TMsg) -> Option<TReceiver::Msg> + Send + 'static,
        TReceiver: Actor,
    {
        let handle = crate::concurrency::spawn(async move {
            while let Ok(Some(msg)) = port.recv().await {
                if let Some(new_msg) = converter(msg) {
                    if receiver.cast(new_msg).is_err() {
                        // kill the subscription process, as the forwarding agent is stopped
                        return;
                    }
                }
            }
        });

        Self { handle }
    }
}
