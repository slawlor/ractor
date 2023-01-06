// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Helpers for remote-procedure calls

use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::{self, Duration};

use crate::{ActorCell, ActorHandler, Message, MessagingErr, RpcReplyPort};

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
pub fn cast<TActor, TMsg>(actor: &ActorCell, msg: TMsg) -> Result<(), MessagingErr>
where
    TActor: ActorHandler<Msg = TMsg>,
    TMsg: Message,
{
    actor.send_message::<TActor, _>(msg)
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
pub async fn call<TActor, TMsg, TReply, TMsgBuilder>(
    actor: &ActorCell,
    msg_builder: TMsgBuilder,
    timeout_option: Option<Duration>,
) -> Result<CallResult<TReply>, MessagingErr>
where
    TActor: ActorHandler<Msg = TMsg>,
    TMsg: Message,
    TMsgBuilder: FnOnce(RpcReplyPort<TReply>) -> TMsg,
{
    let (tx, rx) = oneshot::channel();
    actor.send_message::<TActor, _>(msg_builder(tx.into()))?;

    // wait for the reply
    Ok(if let Some(duration) = timeout_option {
        match time::timeout(duration, rx).await {
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
pub fn call_and_forward<
    TActor,
    TForwardActor,
    TMsg,
    TReply,
    TMsgBuilder,
    FwdMapFn,
    TForwardMessage,
>(
    actor: &ActorCell,
    msg_builder: TMsgBuilder,
    response_forward: ActorCell,
    forward_mapping: FwdMapFn,
    timeout_option: Option<Duration>,
) -> Result<JoinHandle<CallResult<Result<(), MessagingErr>>>, MessagingErr>
where
    TActor: ActorHandler<Msg = TMsg>,
    TMsg: Message,
    TReply: Message,
    TMsgBuilder: FnOnce(RpcReplyPort<TReply>) -> TMsg,
    TForwardActor: ActorHandler<Msg = TForwardMessage>,
    FwdMapFn: FnOnce(TReply) -> TForwardMessage + Send + 'static,
    TForwardMessage: Message,
{
    let (tx, rx) = oneshot::channel();
    actor.send_message::<TActor, _>(msg_builder(tx.into()))?;

    // wait for the reply
    Ok(tokio::spawn(async move {
        if let Some(duration) = timeout_option {
            match time::timeout(duration, rx).await {
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
        .map(|msg| response_forward.send_message::<TForwardActor, _>(forward_mapping(msg)))
    }))
}
