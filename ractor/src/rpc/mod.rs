// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Remote procedure calls (RPC) are helpful communication primitives to communicate with actors
//!
//! There are generally 2 kinds of RPCs, cast and call, and their definition comes from the
//! standard [Erlang `gen_server`](https://www.erlang.org/doc/man/gen_server.html#cast-2).
//! The tl;dr is that `cast` is an send without waiting on a reply while `call` is expecting
//! a reply from the actor being communicated with.

use crate::concurrency::{self, Duration, JoinHandle};

use crate::{Actor, ActorCell, ActorRef, Message, MessagingErr, RpcReplyPort};

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
pub fn cast<TActor>(actor: &ActorCell, msg: TActor::Msg) -> Result<(), MessagingErr>
where
    TActor: Actor,
{
    actor.send_message::<TActor>(msg)
}

/// Sends an asynchronous request to the specified actor, building a one-time
/// use reply channel and awaiting the result with the specified timeout
///
/// * `actor` - A reference to the [ActorCell] to communicate with
/// * `msg_builder` - The [FnOnce] to construct the message
/// * `timeout_option` - An optional [Duration] which represents the amount of
/// time until the operation times out
///
/// Returns [Ok(CallResult)] upon successful initial sending with the reply from
/// the [crate::Actor], [Err(MessagingErr)] if the initial send operation failed
pub async fn call<TActor, TReply, TMsgBuilder>(
    actor: &ActorCell,
    msg_builder: TMsgBuilder,
    timeout_option: Option<Duration>,
) -> Result<CallResult<TReply>, MessagingErr>
where
    TActor: Actor,
    TMsgBuilder: FnOnce(RpcReplyPort<TReply>) -> TActor::Msg,
{
    let (tx, rx) = concurrency::oneshot();
    actor.send_message::<TActor>(msg_builder(tx.into()))?;

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
/// time until the operation times out
///
/// Returns [Ok(`Vec<CallResult<TReply>>>`)] upon successful initial sending with the reply from
/// the [crate::Actor]s, [Err(MessagingErr)] if the initial send operation failed
pub async fn multi_call<TActor, TReply, TMsgBuilder>(
    actors: &[ActorCell],
    msg_builder: TMsgBuilder,
    timeout_option: Option<Duration>,
) -> Result<Vec<CallResult<TReply>>, MessagingErr>
where
    TActor: Actor,
    TReply: Send + 'static,
    TMsgBuilder: Fn(RpcReplyPort<TReply>) -> TActor::Msg,
{
    let mut rx_ports = Vec::with_capacity(actors.len());
    // send to all actors
    for actor in actors {
        let (tx, rx) = concurrency::oneshot();
        actor.send_message::<TActor>(msg_builder(tx.into()))?;
        rx_ports.push(rx);
    }

    let mut join_set = tokio::task::JoinSet::new();
    for (i, rx) in rx_ports.into_iter().enumerate() {
        if let Some(duration) = timeout_option {
            join_set.spawn(async move {
                (
                    i,
                    match tokio::time::timeout(duration, rx).await {
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
    let mut results = Vec::with_capacity(join_set.len());
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
/// and then forwaring the reply to a followup-actor. If this [CallResult] from the first
/// actor is not success, the forward is not sent.
///
/// * `actor` - A reference to the [ActorCell] to communicate with
/// * `msg_builder` - The [FnOnce] to construct the message
/// * `response_forward` - The [ActorCell] to forward the message to
/// * `forward_mapping` - The [FnOnce] which maps the response from the `actor` [ActorCell]'s reply message
/// type to the `response_forward` [ActorCell]'s message type
/// * `timeout_option` - An optional [Duration] which represents the amount of
/// time until the operation times out
///
/// Returns: A [JoinHandle<CallResult<()>>] which can be awaited to see if the
/// forward was successful or ignored
pub fn call_and_forward<TActor, TForwardActor, TReply, TMsgBuilder, FwdMapFn>(
    actor: &ActorCell,
    msg_builder: TMsgBuilder,
    response_forward: ActorCell,
    forward_mapping: FwdMapFn,
    timeout_option: Option<Duration>,
) -> Result<JoinHandle<CallResult<Result<(), MessagingErr>>>, MessagingErr>
where
    TActor: Actor,
    TReply: Message,
    TMsgBuilder: FnOnce(RpcReplyPort<TReply>) -> TActor::Msg,
    TForwardActor: Actor,
    FwdMapFn: FnOnce(TReply) -> TForwardActor::Msg + Send + 'static,
{
    let (tx, rx) = concurrency::oneshot();
    actor.send_message::<TActor>(msg_builder(tx.into()))?;

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
        .map(|msg| response_forward.send_message::<TForwardActor>(forward_mapping(msg)))
    }))
}

impl<TActor> ActorRef<TActor>
where
    TActor: Actor,
{
    /// Alias of [cast]
    pub fn cast(&self, msg: TActor::Msg) -> Result<(), MessagingErr>
    where
        TActor: Actor,
    {
        cast::<TActor>(&self.inner, msg)
    }

    /// Alias of [call]
    pub async fn call<TReply, TMsgBuilder>(
        &self,
        msg_builder: TMsgBuilder,
        timeout_option: Option<Duration>,
    ) -> Result<CallResult<TReply>, MessagingErr>
    where
        TMsgBuilder: FnOnce(RpcReplyPort<TReply>) -> TActor::Msg,
    {
        call::<TActor, TReply, TMsgBuilder>(&self.inner, msg_builder, timeout_option).await
    }

    /// Alias of [call_and_forward]
    pub fn call_and_forward<TReply, TForwardActor, TMsgBuilder, TFwdMessageBuilder>(
        &self,
        msg_builder: TMsgBuilder,
        response_forward: &ActorRef<TForwardActor>,
        forward_mapping: TFwdMessageBuilder,
        timeout_option: Option<Duration>,
    ) -> Result<crate::concurrency::JoinHandle<CallResult<Result<(), MessagingErr>>>, MessagingErr>
    where
        TReply: Message,
        TMsgBuilder: FnOnce(RpcReplyPort<TReply>) -> TActor::Msg,
        TForwardActor: Actor,
        TFwdMessageBuilder: FnOnce(TReply) -> TForwardActor::Msg + Send + 'static,
    {
        call_and_forward::<TActor, TForwardActor, TReply, TMsgBuilder, TFwdMessageBuilder>(
            &self.inner,
            msg_builder,
            response_forward.inner.clone(),
            forward_mapping,
            timeout_option,
        )
    }
}
