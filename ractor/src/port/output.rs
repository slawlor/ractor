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

use std::{fmt::Debug, sync::RwLock};

use crate::concurrency::JoinHandle;
use tokio::sync::broadcast as pubsub;

use crate::{ActorRef, Message};

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

impl<TMsg: OutputMessage> Debug for OutputPort<TMsg> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "OutputPort({})", std::any::type_name::<TMsg>())
    }
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
    ///   receiver's input type and return [Some(_)] if the message should be forwarded, [None]
    ///   if the message should be skipped.
    pub fn subscribe<TReceiverMsg, F>(&self, receiver: ActorRef<TReceiverMsg>, converter: F)
    where
        F: Fn(TMsg) -> Option<TReceiverMsg> + Send + 'static,
        TReceiverMsg: Message,
    {
        let mut subs = self.subscriptions.write().unwrap();

        // filter out dead subscriptions, since they're no longer valid
        subs.retain(|sub| !sub.is_dead());

        let sub = OutputPortSubscription::new::<TMsg, F, TReceiverMsg>(
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

// ============== Subscription implementation ============== //

/// The output port's subscription handle. It holds a handle to a [JoinHandle]
/// which listens to the [pubsub::Receiver] to see if there's a new message, and if there is
/// forwards it to the [ActorRef] asynchronously using the specified converter.
struct OutputPortSubscription {
    handle: JoinHandle<()>,
}

impl OutputPortSubscription {
    /// Determine if the subscription is dead
    pub(crate) fn is_dead(&self) -> bool {
        self.handle.is_finished()
    }

    /// Create a new subscription
    pub(crate) fn new<TMsg, F, TReceiverMsg>(
        mut port: pubsub::Receiver<Option<TMsg>>,
        converter: F,
        receiver: ActorRef<TReceiverMsg>,
    ) -> Self
    where
        TMsg: OutputMessage,
        F: Fn(TMsg) -> Option<TReceiverMsg> + Send + 'static,
        TReceiverMsg: Message,
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

/// Represents a boxed `ActorRef` subscriber capable of handling messages from a
/// publisher via an `OutputPort`, employing a publish-subscribe pattern to
/// decouple message broadcasting from handling. For a subscriber `ActorRef` to
/// function as an `OutputPortSubscriber<T>`, its message type must implement
/// `From<T>` to convert the published message type to its own message format.
///
/// # Example
/// ```
/// // First, define the publisher's message types, including a variant for
/// // subscribing `OutputPortSubscriber`s and another for publishing messages:
/// use ractor::{
///     cast,
///     port::{OutputPort, OutputPortSubscriber},
///     Actor, ActorProcessingErr, ActorRef, Message,
/// };
///
/// enum PublisherMessage {
///     Publish(u8),                         // Message type for publishing
///     Subscribe(OutputPortSubscriber<u8>), // Message type for subscribing an actor to the output port
/// }
///
/// #[cfg(feature = "cluster")]
/// impl Message for PublisherMessage {
///     fn serializable() -> bool {
///         false
///     }
/// }
///
/// // In the publisher actor's `handle` function, handle subscription requests and
/// // publish messages accordingly:
///
/// struct Publisher;
/// struct State {
///     output_port: OutputPort<u8>,
/// }
///
/// #[cfg_attr(feature = "async-trait", ractor::async_trait)]
/// impl Actor for Publisher {
///     type State = State;
///     type Msg = PublisherMessage;
///     type Arguments = ();
///
///     async fn pre_start(
///         &self,
///         _myself: ActorRef<Self::Msg>,
///         _: (),
///     ) -> Result<Self::State, ActorProcessingErr> {
///         Ok(State {
///             output_port: OutputPort::default(),
///         })
///     }
///
///     async fn handle(
///         &self,
///         _myself: ActorRef<Self::Msg>,
///         message: Self::Msg,
///         state: &mut Self::State,
///     ) -> Result<(), ActorProcessingErr> {
///         match message {
///             PublisherMessage::Subscribe(subscriber) => {
///                 // Subscribes the `OutputPortSubscriber` wrapped actor to the `OutputPort`
///                 subscriber.subscribe_to_port(&state.output_port);
///             }
///             PublisherMessage::Publish(value) => {
///                 // Broadcasts the `u8` value to all subscribed actors, which will handle the type conversion
///                 state.output_port.send(value);
///             }
///         }
///         Ok(())
///     }
/// }
///
/// // The subscriber's message type demonstrates how to transform the publisher's
/// // message type by implementing `From<T>`:
///
/// #[derive(Debug)]
/// enum SubscriberMessage {
///     Handle(String), // Subscriber's intent for message handling
/// }
///
/// #[cfg(feature = "cluster")]
/// impl Message for SubscriberMessage {
///     fn serializable() -> bool {
///         false
///     }
/// }
///
/// impl From<u8> for SubscriberMessage {
///     fn from(value: u8) -> Self {
///         SubscriberMessage::Handle(value.to_string()) // Converts u8 to String
///     }
/// }
///
/// // To subscribe a subscriber actor to the publisher and broadcast a message:
/// struct Subscriber;
/// #[cfg_attr(feature = "async-trait", ractor::async_trait)]
/// impl Actor for Subscriber {
///     type State = ();
///     type Msg = SubscriberMessage;
///     type Arguments = ();
///
///     async fn pre_start(
///         &self,
///         _myself: ActorRef<Self::Msg>,
///         _: (),
///     ) -> Result<Self::State, ActorProcessingErr> {
///         Ok(())
///     }
///
///     async fn handle(
///         &self,
///         _myself: ActorRef<Self::Msg>,
///         message: Self::Msg,
///         _state: &mut Self::State,
///     ) -> Result<(), ActorProcessingErr> {
///         Ok(())
///     }
/// }
/// async fn example() {
///     let (publisher_actor_ref, publisher_actor_handle) =
///         Actor::spawn(None, Publisher, ()).await.unwrap();
///     let (subscriber_actor_ref, subscriber_actor_handle) =
///         Actor::spawn(None, Subscriber, ()).await.unwrap();
///
///     publisher_actor_ref
///         .send_message(PublisherMessage::Subscribe(Box::new(subscriber_actor_ref)))
///         .unwrap();
///
///     // Broadcasting a message to all subscribers
///     publisher_actor_ref
///         .send_message(PublisherMessage::Publish(123))
///         .unwrap();
///
///     publisher_actor_handle.await.unwrap();
///     subscriber_actor_handle.await.unwrap();
/// }
/// ```
pub type OutputPortSubscriber<InputMessage> = Box<dyn OutputPortSubscriberTrait<InputMessage>>;
/// A trait for subscribing to an [OutputPort]
pub trait OutputPortSubscriberTrait<I>: Send
where
    I: Message + Clone,
{
    /// Subscribe to the output port
    fn subscribe_to_port(&self, port: &OutputPort<I>);
}

impl<I, O> OutputPortSubscriberTrait<I> for ActorRef<O>
where
    I: Message + Clone,
    O: Message + From<I>,
{
    fn subscribe_to_port(&self, port: &OutputPort<I>) {
        port.subscribe(self.clone(), |msg| Some(O::from(msg)));
    }
}
