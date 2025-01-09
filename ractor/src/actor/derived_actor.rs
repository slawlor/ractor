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
///
/// ## Example
/// ```
/// // In this example the actor is a ghost kitchen which can accept orders of different types,
/// // representing multiple virtual restaurants at once. More specifically, it can accept
/// // pizza or sushi orders.
/// //
/// // Derived actor allows to hide the order message accepted by the kitchen actor, therefore
/// // we can pass the ref to other components without creating a direct dependency on the
/// // kitchen actor. We can easily replace or split the kitchen actor without affecting
/// // other components communicating with it.
/// use ractor::{Actor, ActorProcessingErr, ActorRef, DerivedActorRef, Message};
///
/// // First we define order types
/// struct PizzaOrder {
///     topping: String,
/// }
///
/// struct SushiOrder {
///     r#type: String,
///     quantity: usize,
/// }
///
/// // The order message which is actually sent to the kitchen actor can be either pizza or sushi
/// enum Order {
///     Pizza(PizzaOrder),
///     Sushi(SushiOrder),
/// }
///
/// // Implementing conversion methods from different order types to the actual order
/// impl From<PizzaOrder> for Order {
///     fn from(value: PizzaOrder) -> Self {
///         Order::Pizza(value)
///     }
/// }
///
/// impl TryFrom<Order> for PizzaOrder {
///     type Error = String;
///
///     fn try_from(value: Order) -> Result<Self, Self::Error> {
///         match value {
///             Order::Pizza(order) => Ok(order),
///             _ => Err("Order has invalid type".to_string()),
///         }
///     }
/// }
///
/// impl From<SushiOrder> for Order {
///     fn from(value: SushiOrder) -> Self {
///         Order::Sushi(value)
///     }
/// }
///
/// impl TryFrom<Order> for SushiOrder {
///     type Error = String;
///
///     fn try_from(value: Order) -> Result<Self, Self::Error> {
///         match value {
///             Order::Sushi(order) => Ok(order),
///             _ => Err("Order has invalid type".to_string()),
///         }
///     }
/// }
///
/// #[cfg(feature = "cluster")]
/// impl Message for Order {
///     fn serializable() -> bool {
///         false
///     }
/// }
///
/// struct Kitchen;
///
/// #[cfg_attr(feature = "async-trait", ractor::async_trait)]
/// impl Actor for Kitchen {
///     type Msg = Order;
///     type State = ();
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
///         match message {
///             Order::Pizza(order) => {
///                 println!("Preparing pizza with topping {}", order.topping);
///             }
///             Order::Sushi(order) => {
///                 println!(
///                     "Preparing {} sushi of type {}",
///                     order.quantity, order.r#type
///                 );
///             }
///         }
///         Ok(())
///     }
/// }
///
/// async fn example() {
///     let (kitchen_actor_ref, kitchen_actor_handle) = Actor::spawn(None, Kitchen, ()).await.unwrap();
///     
///     // derived actor ref can be passed to the pizza restaurant actor which accepts pizza orders from delivery apps
///     let pizza_restaurant: DerivedActorRef<PizzaOrder> = kitchen_actor_ref.get_derived();
///     pizza_restaurant.send_message(PizzaOrder {
///         topping: String::from("pepperoni"),
///     }).expect("Failed to order pizza");
///
///     // same way, we can also get a derived actor ref which can only accept sushi orders
///     let sushi_restaurant: DerivedActorRef<SushiOrder> = kitchen_actor_ref.get_derived();
///     sushi_restaurant.send_message(SushiOrder {
///         r#type: String::from("sashimi"),
///         quantity: 3,
///     }).expect("Failed to order sushi");
///
///     kitchen_actor_handle.await.unwrap();
/// }
/// ```
pub struct DerivedActorRef<TFrom> {
    converter: Arc<dyn Fn(TFrom) -> Result<(), MessagingErr<TFrom>> + Send + Sync + 'static>,
    pub(crate) inner: ActorCell,
}

impl<TFrom> Clone for DerivedActorRef<TFrom> {
    fn clone(&self) -> Self {
        Self {
            converter: self.converter.clone(),
            inner: self.inner.clone(),
        }
    }
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
    /// original message type via [From]. In order to facilitate [MessagingErr::SendErr] the original
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
