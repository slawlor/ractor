// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Timers for sending messages to actors periodically
//!
//! The methodology of timers in `ractor` are based on [Erlang's `timer` module](https://www.erlang.org/doc/man/timer.html).
//! We aren't supporting all timer functions, as many of them don't make sense but we
//! support the relevant ones for `ractor`. In short
//!
//! 1. Send on a period
//! 2. Send after a delay
//! 3. Stop after a delay
//! 4. Kill after a delay
//!
//! ## Examples
//!
//! ```rust
//! use ractor::concurrency::Duration;
//! use ractor::{Actor, ActorRef, ActorProcessingErr};
//!
//! struct ExampleActor;
//!
//! enum ExampleMessage {
//!     AfterDelay,
//!     OnPeriod,
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
//!     async fn pre_start(&self, _myself: ActorRef<Self::Msg>, _args: Self::Arguments) -> Result<Self::State, ActorProcessingErr> {
//!         println!("Starting");
//!         Ok(())
//!     }
//!
//!     async fn handle(&self, _myself: ActorRef<Self::Msg>, message: Self::Msg, _state: &mut Self::State) -> Result<(), ActorProcessingErr> {
//!         match message {
//!             ExampleMessage::AfterDelay => println!("After delay"),
//!             ExampleMessage::OnPeriod => println!("On period"),
//!         }
//!         Ok(())
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() {
//!     let (actor, handle) = Actor::spawn(None, ExampleActor, ()).await.expect("Failed to startup dummy actor");
//!     
//!     // send the message after a 100ms delay
//!     actor.send_after(Duration::from_millis(100), || ExampleMessage::AfterDelay);
//!     
//!     // send this message every 10ms
//!     actor.send_interval(Duration::from_millis(10), || ExampleMessage::OnPeriod);
//!
//!     // Exit the actor after 200ms (equivalent of calling `stop(maybe_reason)`)
//!     actor.exit_after(Duration::from_millis(200));
//!
//!     // Kill the actor after 300ms (won't execute since we did stop before, but here
//!     // as an example)
//!     actor.kill_after(Duration::from_millis(300));
//!
//!     // wait for actor exit
//!     handle.await.unwrap();
//! }
//! ```

use crate::concurrency::{Duration, JoinHandle};

use crate::{ActorCell, Message, MessagingErr, ACTIVE_STATES};

#[cfg(test)]
mod tests;

/// Sends a message to a given actor repeatedly after a specified time
/// using the provided message generation function. The task will exit
/// once the channel is closed (meaning the underlying [crate::Actor]
/// has terminated)
///
/// * `period` - The [Duration] representing the period for the send interval
/// * `actor` - The [ActorCell] representing the [crate::Actor] to communicate with
/// * `msg` - The [Fn] message builder which is called to generate a message for each send
/// operation.
///
/// Returns: The [JoinHandle] which represents the backgrounded work (can be ignored to
/// "fire and forget")
pub fn send_interval<TMessage, F>(period: Duration, actor: ActorCell, msg: F) -> JoinHandle<()>
where
    TMessage: Message,
    F: Fn() -> TMessage + Send + 'static,
{
    // As per #57, the traditional sleep operation is subject to drift over long periods.
    // Tokio and our internal version for `async_std` provide an interval timer which
    // accounts for execution time to send a message and changes in polling to wake
    // the task to assure that the period doesn't drift over long runtimes.
    crate::concurrency::spawn(async move {
        let mut timer = crate::concurrency::interval(period);
        // timer tick's immediately the first time
        timer.tick().await;
        while ACTIVE_STATES.contains(&actor.get_status()) {
            timer.tick().await;
            // if we receive an error trying to send, the channel is closed and we should stop trying
            // actor died
            if actor.send_message::<TMessage>(msg()).is_err() {
                break;
            }
        }
    })
}

/// Sends a message after a given period to the specified actor. The task terminates
/// once the send has completed
///
/// * `period` - The [Duration] representing the time to delay before sending
/// * `actor` - The [ActorCell] representing the [crate::Actor] to communicate with
/// * `msg` - The [Fn] message builder which is called to generate a message for the send
/// operation
///
/// Returns: The [JoinHandle<Result<(), MessagingErr>>] which represents the backgrounded work.
/// Awaiting the handle will yield the result of the send operation. Can be safely ignored to
/// "fire and forget"
pub fn send_after<TMessage, F>(
    period: Duration,
    actor: ActorCell,
    msg: F,
) -> JoinHandle<Result<(), MessagingErr<TMessage>>>
where
    TMessage: Message,
    F: Fn() -> TMessage + Send + 'static,
{
    crate::concurrency::spawn(async move {
        crate::concurrency::sleep(period).await;
        actor.send_message::<TMessage>(msg())
    })
}

/// Sends the stop signal to the actor after a specified duration, attaching a reason
/// of "Exit after {}ms" by default
///
/// * `period` - The [Duration] representing the time to delay before sending
/// * `actor` - The [ActorCell] representing the [crate::Actor] to exit after the duration
///
/// Returns: The [JoinHandle] which denotes the backgrounded operation. To cancel the
/// exit operation, you can abort the handle
pub fn exit_after(period: Duration, actor: ActorCell) -> JoinHandle<()> {
    crate::concurrency::spawn(async move {
        crate::concurrency::sleep(period).await;
        actor.stop(Some(format!("Exit after {}ms", period.as_millis())))
    })
}

/// Sends the KILL signal to the actor after a specified duration
///
/// * `period` - The [Duration] representing the time to delay before sending
/// * `actor` - The [ActorCell] representing the [crate::Actor] to kill after the duration
///
/// Returns: The [JoinHandle] which denotes the backgrounded operation. To cancel the
/// kill operation, you can abort the handle
pub fn kill_after(period: Duration, actor: ActorCell) -> JoinHandle<()> {
    crate::concurrency::spawn(async move {
        crate::concurrency::sleep(period).await;
        actor.kill()
    })
}

/// Add the timing functionality on top of the [crate::ActorRef]
impl<TMessage> crate::ActorRef<TMessage>
where
    TMessage: crate::Message,
{
    /// Alias of [send_interval]
    pub fn send_interval<F>(&self, period: Duration, msg: F) -> JoinHandle<()>
    where
        F: Fn() -> TMessage + Send + 'static,
    {
        send_interval::<TMessage, F>(period, self.get_cell(), msg)
    }

    /// Alias of [send_after]
    pub fn send_after<F>(
        &self,
        period: Duration,
        msg: F,
    ) -> JoinHandle<Result<(), MessagingErr<TMessage>>>
    where
        F: Fn() -> TMessage + Send + 'static,
    {
        send_after::<TMessage, F>(period, self.get_cell(), msg)
    }

    /// Alias of [exit_after]
    pub fn exit_after(&self, period: Duration) -> JoinHandle<()> {
        exit_after(period, self.get_cell())
    }

    /// Alias of [kill_after]
    pub fn kill_after(&self, period: Duration) -> JoinHandle<()> {
        kill_after(period, self.get_cell())
    }
}
