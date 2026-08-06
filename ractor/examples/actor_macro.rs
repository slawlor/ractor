// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! A concise counter actor with a macro-generated, documented message enum.
//!
//! Execute with
//!
//! ```text
//! cargo run --example actor_macro --features actor-macros
//! ```

#![allow(clippy::incompatible_msrv)]

extern crate ractor;

use ractor::call_t;
use ractor::Actor;
use ractor::ActorProcessingErr;
use ractor::ActorRef;

struct Counter;

#[ractor::actor(
    message = enum CounterMessage,
    state = i64,
    arguments = i64,
    crate_path = ::ractor,
)]
impl Counter {
    async fn pre_start(
        &self,
        _myself: ActorRef<CounterMessage>,
        initial: i64,
    ) -> Result<i64, ActorProcessingErr> {
        Ok(initial)
    }

    /// Add an amount to the counter.
    #[ractor::message(Add(amount))]
    fn add(&self, amount: i64, state: &mut i64) {
        *state += amount;
    }

    /// Read the current counter value.
    #[ractor::rpc(Read)]
    fn read(&self, state: &i64) -> i64 {
        *state
    }

    /// Stop the counter actor.
    #[ractor::message(Stop)]
    fn stop(&self, myself: ActorRef<CounterMessage>) {
        myself.stop(None);
    }
}

#[cfg(feature = "cluster")]
impl ractor::Message for CounterMessage {}

#[ractor_example_entry_proc::ractor_example_entry]
async fn main() {
    let (actor, handle) = Actor::spawn(None, Counter, 10)
        .await
        .expect("counter failed to start");

    actor
        .send_message(CounterMessage::Add(5))
        .expect("increment message failed");
    let count =
        call_t!(actor, CounterMessage::Read, 100).expect("counter failed to answer the RPC");
    assert_eq!(count, 15);

    actor
        .send_message(CounterMessage::Stop)
        .expect("stop message failed");
    handle.await.expect("counter failed to stop cleanly");
}
