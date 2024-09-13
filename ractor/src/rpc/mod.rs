// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Remote procedure calls (RPC) are helpful communication primitives to communicate with actors
//!
//! There are generally 2 kinds of RPCs, `cast` and `call`, and their definition comes from the
//! standard [Erlang `gen_server`](https://www.erlang.org/doc/man/gen_server.html#cast-2).
//! The tl;dr is that `cast` is an send without waiting on a reply while `call` is expecting
//! a reply from the actor being communicated with.
//!
//! ## Examples
//!
//! ```rust
//! use ractor::concurrency::Duration;
//! use ractor::{call, call_t, cast};
//! use ractor::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort};
//!
//! struct ExampleActor;
//!
//! enum ExampleMessage {
//!     Cast,
//!     Call(RpcReplyPort<String>),
//! }
//!
//! #[cfg(feature = "cluster")]
//! impl ractor::Message for ExampleMessage {}
//!
//! #[cfg_attr(feature = "async-trait", ractor::async_trait)]
//! impl Actor for ExampleActor {
//!     type Msg = ExampleMessage;
//!     type State = ();
//!     type Arguments = ();
//!
//!     async fn pre_start(
//!         &self,
//!         _myself: ActorRef<Self::Msg>,
//!         _args: Self::Arguments,
//!     ) -> Result<Self::State, ActorProcessingErr> {
//!         println!("Starting");
//!         Ok(())
//!     }
//!
//!     async fn handle(
//!         &self,
//!         _myself: ActorRef<Self::Msg>,
//!         message: Self::Msg,
//!         _state: &mut Self::State,
//!     ) -> Result<(), ActorProcessingErr> {
//!         match message {
//!             ExampleMessage::Cast => println!("Cast message"),
//!             ExampleMessage::Call(reply) => {
//!                 println!("Call message");
//!                 let _ = reply.send("a reply".to_string());
//!             }
//!         }
//!         Ok(())
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() {
//!     let (actor, handle) = Actor::spawn(None, ExampleActor, ())
//!         .await
//!         .expect("Failed to startup dummy actor");
//!
//!     // send a 1-way message (equivalent patterns)
//!     actor
//!         .cast(ExampleMessage::Cast)
//!         .expect("Failed to send message");
//!     cast!(actor, ExampleMessage::Cast).expect("Failed to send message");
//!
//!     // Send a message to the actor, with an associated reply channel,
//!     // and wait for the reply from the actor (optionally up to a timeout)
//!     let _result = actor
//!         .call(ExampleMessage::Call, Some(Duration::from_millis(100)))
//!         .await
//!         .expect("Failed to call actor");
//!     let _result = call!(actor, ExampleMessage::Call).expect("Failed to call actor");
//!     let _result =
//!         call_t!(actor, ExampleMessage::Call, 100).expect("Failed to call actor with timeout");
//!
//!     // wait for actor exit
//!     actor.stop(None);
//!     handle.await.unwrap();
//! }
//! ```

use crate::concurrency::{self, Duration, JoinHandle};

use crate::{ActorCell, ActorRef, Message, MessagingErr, RpcReplyPort};

pub mod call_result;
pub use call_result::CallResult;
#[cfg(test)]
mod tests;

/// Sends an asynchronous request to the specified actor, ignoring if the
/// actor is alive or healthy and simply returns immediately
///
/// * `actor` - A reference to the [ActorCell] to communicate with
/// * `msg` - The message to send to the actor
///
/// Returns [Ok(())] upon successful send, [Err(MessagingErr)] otherwise
pub fn cast<TMessage>(actor: &ActorCell, msg: TMessage) -> Result<(), MessagingErr<TMessage>>
where
    TMessage: Message,
{
    actor.send_message::<TMessage>(msg)
}

/// Sends an asynchronous request to the specified actor, building a one-time
/// use reply channel and awaiting the result with the specified timeout
///
/// * `actor` - A reference to the [ActorCell] to communicate with
/// * `msg_builder` - The [FnOnce] to construct the message
/// * `timeout_option` - An optional [Duration] which represents the amount of
///   time until the operation times out
///
/// Returns [Ok(CallResult)] upon successful initial sending with the reply from
/// the [crate::Actor], [Err(MessagingErr)] if the initial send operation failed
pub async fn call<TMessage, TReply, TMsgBuilder>(
    actor: &ActorCell,
    msg_builder: TMsgBuilder,
    timeout_option: Option<Duration>,
) -> Result<CallResult<TReply>, MessagingErr<TMessage>>
where
    TMessage: Message,
    TMsgBuilder: FnOnce(RpcReplyPort<TReply>) -> TMessage,
{
    let (tx, rx) = concurrency::oneshot();
    let port: RpcReplyPort<TReply> = match timeout_option {
        Some(duration) => (tx, duration).into(),
        None => tx.into(),
    };
    actor.send_message::<TMessage>(msg_builder(port))?;

    // wait for the reply
    Ok(if let Some(duration) = timeout_option {
        match crate::concurrency::timeout(duration, rx).await {
            Ok(Ok(result)) => CallResult::Success(result),
            Ok(Err(_send_err)) => CallResult::SenderError,
            Err(_timeout_err) => CallResult::Timeout,
        }
    } else {
        match rx.await {
            Ok(result) => CallResult::Success(result),
            Err(_send_err) => CallResult::SenderError,
        }
    })
}

/// Sends an asynchronous request to the specified actors, building a one-time
/// use reply channel for each actor and awaiting the results with the
/// specified timeout
///
/// * `actors` - A reference to the group of [ActorCell]s to communicate with
/// * `msg_builder` - The [FnOnce] to construct the message
/// * `timeout_option` - An optional [Duration] which represents the amount of
///   time until the operation times out
///
/// Returns [Ok(`Vec<CallResult<TReply>>>`)] upon successful initial sending with the reply from
/// the [crate::Actor]s, [Err(MessagingErr)] if the initial send operation failed
pub async fn multi_call<TMessage, TReply, TMsgBuilder>(
    actors: &[ActorRef<TMessage>],
    msg_builder: TMsgBuilder,
    timeout_option: Option<Duration>,
) -> Result<Vec<CallResult<TReply>>, MessagingErr<TMessage>>
where
    TMessage: Message,
    TReply: Send + 'static,
    TMsgBuilder: Fn(RpcReplyPort<TReply>) -> TMessage,
{
    let mut rx_ports = Vec::with_capacity(actors.len());
    // send to all actors
    for actor in actors {
        let (tx, rx) = concurrency::oneshot();
        let port: RpcReplyPort<TReply> = match timeout_option {
            Some(duration) => (tx, duration).into(),
            None => tx.into(),
        };
        actor.cast(msg_builder(port))?;
        rx_ports.push(rx);
    }

    let mut results = Vec::new();
    let mut join_set = crate::concurrency::JoinSet::new();
    for (i, rx) in rx_ports.into_iter().enumerate() {
        if let Some(duration) = timeout_option {
            join_set.spawn(async move {
                (
                    i,
                    match crate::concurrency::timeout(duration, rx).await {
                        Ok(Ok(result)) => CallResult::Success(result),
                        Ok(Err(_send_err)) => CallResult::SenderError,
                        Err(_) => CallResult::Timeout,
                    },
                )
            });
        } else {
            join_set.spawn(async move {
                (
                    i,
                    match rx.await {
                        Ok(result) => CallResult::Success(result),
                        Err(_send_err) => CallResult::SenderError,
                    },
                )
            });
        }
    }

    // we threaded the index in order to maintain ordering from the originally called
    // actors.
    results.resize_with(join_set.len(), || CallResult::Timeout);
    while let Some(result) = join_set.join_next().await {
        match result {
            Ok((i, r)) => results[i] = r,
            _ => return Err(MessagingErr::ChannelClosed),
        }
    }

    // wait for the replies
    Ok(results)
}

/// Send a message asynchronously to another actor, waiting in a new task for the reply
/// and then forwarding the reply to a followup-actor. If this [CallResult] from the first
/// actor is not success, the forward is not sent.
///
/// * `actor` - A reference to the [ActorCell] to communicate with
/// * `msg_builder` - The [FnOnce] to construct the message
/// * `response_forward` - The [ActorCell] to forward the message to
/// * `forward_mapping` - The [FnOnce] which maps the response from the `actor` [ActorCell]'s reply message
///   type to the `response_forward` [ActorCell]'s message type
/// * `timeout_option` - An optional [Duration] which represents the amount of
///   time until the operation times out
///
/// Returns: A [JoinHandle<CallResult<()>>] which can be awaited to see if the
/// forward was successful or ignored
#[allow(clippy::type_complexity)]
pub fn call_and_forward<TMessage, TForwardMessage, TReply, TMsgBuilder, FwdMapFn>(
    actor: &ActorCell,
    msg_builder: TMsgBuilder,
    response_forward: ActorCell,
    forward_mapping: FwdMapFn,
    timeout_option: Option<Duration>,
) -> Result<JoinHandle<CallResult<Result<(), MessagingErr<TForwardMessage>>>>, MessagingErr<TMessage>>
where
    TMessage: Message,
    TReply: Send + 'static,
    TMsgBuilder: FnOnce(RpcReplyPort<TReply>) -> TMessage,
    TForwardMessage: Message,
    FwdMapFn: FnOnce(TReply) -> TForwardMessage + Send + 'static,
{
    let (tx, rx) = concurrency::oneshot();
    let port: RpcReplyPort<TReply> = match timeout_option {
        Some(duration) => (tx, duration).into(),
        None => tx.into(),
    };
    actor.send_message::<TMessage>(msg_builder(port))?;

    // wait for the reply
    Ok(crate::concurrency::spawn(async move {
        if let Some(duration) = timeout_option {
            match crate::concurrency::timeout(duration, rx).await {
                Ok(Ok(result)) => CallResult::Success(result),
                Ok(Err(_send_err)) => CallResult::SenderError,
                Err(_timeout_err) => CallResult::Timeout,
            }
        } else {
            match rx.await {
                Ok(result) => CallResult::Success(result),
                Err(_send_err) => CallResult::SenderError,
            }
        }
        .map(|msg| response_forward.send_message::<TForwardMessage>(forward_mapping(msg)))
    }))
}

impl<TMessage> ActorRef<TMessage>
where
    TMessage: Message,
{
    /// Alias of [cast]
    pub fn cast(&self, msg: TMessage) -> Result<(), MessagingErr<TMessage>> {
        cast::<TMessage>(&self.inner, msg)
    }

    /// Alias of [call]
    pub async fn call<TReply, TMsgBuilder>(
        &self,
        msg_builder: TMsgBuilder,
        timeout_option: Option<Duration>,
    ) -> Result<CallResult<TReply>, MessagingErr<TMessage>>
    where
        TMsgBuilder: FnOnce(RpcReplyPort<TReply>) -> TMessage,
    {
        call::<TMessage, TReply, TMsgBuilder>(&self.inner, msg_builder, timeout_option).await
    }

    /// Alias of [call_and_forward]
    #[allow(clippy::type_complexity)]
    pub fn call_and_forward<TReply, TForwardMessage, TMsgBuilder, TFwdMessageBuilder>(
        &self,
        msg_builder: TMsgBuilder,
        response_forward: &ActorRef<TForwardMessage>,
        forward_mapping: TFwdMessageBuilder,
        timeout_option: Option<Duration>,
    ) -> Result<
        crate::concurrency::JoinHandle<CallResult<Result<(), MessagingErr<TForwardMessage>>>>,
        MessagingErr<TMessage>,
    >
    where
        TReply: Send + 'static,
        TMsgBuilder: FnOnce(RpcReplyPort<TReply>) -> TMessage,
        TForwardMessage: Message,
        TFwdMessageBuilder: FnOnce(TReply) -> TForwardMessage + Send + 'static,
    {
        call_and_forward::<TMessage, TForwardMessage, TReply, TMsgBuilder, TFwdMessageBuilder>(
            &self.inner,
            msg_builder,
            response_forward.inner.clone(),
            forward_mapping,
            timeout_option,
        )
    }
}
