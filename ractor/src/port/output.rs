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

use std::fmt::Debug;

use crate::ActorId;
use crate::ActorRef;
use crate::Message;

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
    inner: inner::OutputPort<ActorId, TMsg>,
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
        Self {
            inner: inner::OutputPort::default(),
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
        self.inner.subscribe(receiver, converter)
    }

    /// Send a message on the output port
    ///
    /// * `msg`: The message to send
    pub fn send(&self, msg: TMsg) {
        self.inner.send(msg)
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

mod inner {

    use super::OutputMessage;
    use crate::concurrency::{mpsc_unbounded, MpscUnboundedSender};
    //use crate::concurrency::{mpsc_unbounded, oneshot, MpscUnboundedSender, OneshotSender};
    use crate::{ActorId, ActorRef, DerivedActorRef, Message};

    #[cfg(feature = "tokio_runtime")]
    const CONSUME_BUDGET_FACTOR: usize = 32;

    enum OutportMessage<Id, TMsg> {
        Data(TMsg),
        SetSubscriber(Box<dyn Subscriber<Id, TMsg>>),
        //RemoveSubscriber(Id),
        //Subscribers(OneshotSender<Vec<Id>>),
    }

    pub(super) trait Subscriber<Id, TMsg: OutputMessage>: Send + 'static {
        // return false if the subscriber should be
        // removed
        fn send(&self, value: &TMsg) -> bool;
        fn id(&self) -> Id;
    }

    #[derive(Debug, Clone)]
    pub(super) struct OutputPort<Id, TMsg>(MpscUnboundedSender<OutportMessage<Id, TMsg>>);

    impl<Id: Send + 'static + PartialEq + Clone + Sync, TMsg: OutputMessage> Default
        for OutputPort<Id, TMsg>
    {
        fn default() -> Self {
            Self::new(true)
        }
    }

    impl<Id: Send + 'static + PartialEq + Clone + Sync, TMsg: OutputMessage> OutputPort<Id, TMsg> {
        pub(super) fn new(allow_duplicate_subscription: bool) -> Self {
            let (tx, mut rx) = mpsc_unbounded::<OutportMessage<Id, TMsg>>();

            crate::concurrency::spawn(async move {
                let mut subscribers = Vec::<(Id, Box<dyn Subscriber<Id, TMsg>>)>::new();

                while let Some(msg) = rx.recv().await {
                    match msg {
                        OutportMessage::Data(v) => {
                            // We do not want to hold a reference to dyn Subscriber
                            // to cross an await, otherwise, Subscriber would need to be Sync.
                            // So we iterate by index. This also simplify extraction
                            // of subscribers.
                            let mut i = 0;
                            while i < subscribers.len() {
                                if !subscribers[i].1.send(&v) {
                                    subscribers.remove(i);
                                } else {
                                    i += 1;
                                }
                                // In case there is a very large number of subscribers and subscribers[i].1.send(&v) is heavy
                                // the execution of this loop iteration could be unfair.
                                //
                                // So every CONSUME_BUDGET_FACTOR send we consume budget for the task
                                // so that the tokio runtime can take when necessary.
                                //
                                // NB: every task get a budget of 128, this budget is decreased by one
                                // at each rx.recv() call and at each CONSUME_BUDGET_FACTOR call to subscriber.send
                                #[cfg(feature = "tokio_runtime")]
                                if i % CONSUME_BUDGET_FACTOR == 0 {
                                    tokio::task::consume_budget().await;
                                }
                            }
                        }
                        OutportMessage::SetSubscriber(subscriber) => {
                            let sid = subscriber.id();

                            // We ensure there is no duplicate subscription
                            if !allow_duplicate_subscription {
                                if let Some((_, prev_subscriber)) =
                                    subscribers.iter_mut().find(|(id, _)| id == &sid)
                                {
                                    // In case of duplication, previous subscription is overrided
                                    *prev_subscriber = subscriber;
                                } else {
                                    subscribers.push((subscriber.id(), subscriber));
                                }
                            } else {
                                subscribers.push((subscriber.id(), subscriber));
                            }
                        } //OutportMessage::RemoveSubscriber(id) => {
                          //    if allow_duplicate_subscription {
                          //        subscribers.retain(|(sid, _)| sid != &id);
                          //    } else {
                          //        // As we ensure there is only 1 id in the vector, we can only
                          //        // remove the first find subscriber which has the right id.
                          //        if let Some(i) = subscribers.iter().position(|(sid, _)| sid == &id)
                          //        {
                          //            subscribers.remove(i);
                          //        }
                          //    }
                          //}
                          //OutportMessage::Subscribers(sender) => {
                          //    _ = sender.send(subscribers.iter().map(|(id, _)| id.clone()).collect());
                          //}
                    }
                }
            });

            Self(tx)
        }

        pub(super) fn send(&self, value: TMsg) {
            _ = self.0.send(OutportMessage::Data(value));
        }

        //pub(super) async fn subscribers(&self) -> Vec<Id> {
        //    let (s, r) = oneshot();
        //    _ = self.0.send(OutportMessage::Subscribers(s));
        //    r.await.unwrap_or_default()
        //}

        //pub(super) fn set_subscriber(&self, subscriber: impl Subscriber<Id, TMsg>) {
        //    _ = self
        //        .0
        //        .send(OutportMessage::SetSubscriber(Box::new(subscriber)));
        //}

        //pub(super) fn remove_subscriber(&self, subscriber: &impl Subscriber<Id, TMsg>) {
        //    self.remove_subscriber_by_id(subscriber.id())
        //}

        //pub(super) fn remove_subscriber_by_id(&self, id: Id) {
        //    _ = self.0.send(OutportMessage::RemoveSubscriber(id));
        //}
    }

    impl<TMsg: OutputMessage> OutputPort<ActorId, TMsg> {
        pub(super) fn subscribe<TReceiverMsg, F>(
            &self,
            receiver: ActorRef<TReceiverMsg>,
            converter: F,
        ) where
            F: Fn(TMsg) -> Option<TReceiverMsg> + Send + 'static,
            TReceiverMsg: Message,
        {
            self.set_subscriber_with_filter(receiver, move |msg| converter(msg.clone()))
        }

        pub(super) fn set_subscriber_with_filter<R: ActorReference>(
            &self,
            actor_ref: R,
            filter: impl Fn(&TMsg) -> Option<R::Msg> + Send + 'static,
        ) {
            _ = self
                .0
                .send(OutportMessage::SetSubscriber(Box::new(Filtering {
                    actor_ref,
                    filter,
                })));
        }
    }

    impl<T: OutputMessage, U: Message> Subscriber<ActorId, T> for ActorRef<U>
    where
        U: TryFrom<T>,
    {
        fn send(&self, value: &T) -> bool {
            if let Ok(value) = value.clone().try_into() {
                self.send_message(value).is_ok()
            } else {
                true
            }
        }

        fn id(&self) -> ActorId {
            self.get_id()
        }
    }
    impl<T: OutputMessage> Subscriber<ActorId, T> for DerivedActorRef<T> {
        fn send(&self, value: &T) -> bool {
            self.send_message(value.clone()).is_ok()
        }

        fn id(&self) -> ActorId {
            self.get_id()
        }
    }
    struct Filtering<T, F> {
        pub actor_ref: T,
        pub filter: F,
    }
    impl<T: ActorReference, U: OutputMessage, F: Fn(&U) -> Option<T::Msg> + Send + 'static>
        Subscriber<ActorId, U> for Filtering<T, F>
    {
        fn send(&self, value: &U) -> bool {
            if let Some(v) = (self.filter)(value) {
                self.actor_ref.send_message(v)
            } else {
                true
            }
        }

        fn id(&self) -> ActorId {
            self.actor_ref.id()
        }
    }
    pub(super) trait ActorReference: Send + Sync + 'static {
        type Msg: Message;
        fn send_message(&self, value: Self::Msg) -> bool;
        fn id(&self) -> ActorId;
    }
    impl<T: Message> ActorReference for ActorRef<T> {
        type Msg = T;

        fn send_message(&self, value: T) -> bool {
            self.send_message(value).is_ok()
        }

        fn id(&self) -> ActorId {
            self.get_id()
        }
    }
    impl<T: Message> ActorReference for DerivedActorRef<T> {
        type Msg = T;

        fn send_message(&self, value: T) -> bool {
            self.send_message(value).is_ok()
        }

        fn id(&self) -> ActorId {
            self.get_id()
        }
    }
}
