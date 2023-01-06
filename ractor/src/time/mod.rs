// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Timers for posting to actors periodically

use tokio::{task::JoinHandle, time::Duration};

use crate::{ActorCell, ActorHandler, Message, MessagingErr, ACTIVE_STATES};

#[cfg(test)]
mod tests;

/// Sends a message to a given actor repeatedly after a specifid time
/// using the provided message generation function
///
/// Returns: The [JoinHandle] which represents the backgrounded work (can be ignored to
/// "fire and forget")
pub fn send_interval<TActor, TMsg, F>(period: Duration, actor: ActorCell, msg: F) -> JoinHandle<()>
where
    TActor: ActorHandler<Msg = TMsg>,
    TMsg: Message,
    F: Fn() -> TMsg + Send + 'static,
{
    tokio::spawn(async move {
        while ACTIVE_STATES.contains(&actor.get_status()) {
            tokio::time::sleep(period).await;
            // if we receive an error trying to send, the channel is closed and we should stop trying
            // actor died
            if actor.send_message::<TActor, TMsg>(msg()).is_err() {
                break;
            }
        }
    })
}

/// Sends a message after a given period to the specified actor.
///
/// Returns: The [JoinHandle] which represents the backgrounded work. Awaiting the handle
/// will yield the result of the send operation. Can be safely ignored to "fire and forget"
pub fn send_after<TActor, TMsg, F>(
    period: Duration,
    actor: ActorCell,
    msg: F,
) -> JoinHandle<Result<(), MessagingErr>>
where
    TActor: ActorHandler<Msg = TMsg>,
    TMsg: Message,
    F: Fn() -> TMsg + Send + 'static,
{
    tokio::spawn(async move {
        tokio::time::sleep(period).await;
        actor.send_message::<TActor, TMsg>(msg())
    })
}

/// Sends the [crate::Signal::Exit] signal to the actor after a specified duration
///
/// Returns: The [JoinHandle] which denotes the backgrounded operation. To cancel the
/// exit operation, you can abort the handle to cancel the work.
pub fn exit_after(period: Duration, actor: ActorCell) -> JoinHandle<()> {
    tokio::spawn(async move {
        tokio::time::sleep(period).await;
        actor.stop()
    })
}
