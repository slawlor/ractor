// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! A concise counter actor defined with the optional actor macros.
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
use ractor::RpcReplyPort;

struct Counter;

enum CounterMessage {
    Add(i64),
    Read(RpcReplyPort<i64>),
    Stop,
}

#[cfg(feature = "cluster")]
impl ractor::Message for CounterMessage {}

#[ractor::actor(
    message = CounterMessage,
    state = i64,
    arguments = i64,
)]
impl Counter {
    async fn pre_start(
        &self,
        _myself: ActorRef<CounterMessage>,
        initial: i64,
    ) -> Result<i64, ActorProcessingErr> {
        Ok(initial)
    }

    #[ractor::message(CounterMessage::Add(amount))]
    fn add(&self, amount: i64, state: &mut i64) {
        *state += amount;
    }

    #[ractor::message(CounterMessage::Read(reply))]
    fn read(&self, reply: RpcReplyPort<i64>, state: &i64) {
        let _ = reply.send(*state);
    }

    #[ractor::message(CounterMessage::Stop)]
    fn stop(&self, myself: ActorRef<CounterMessage>) {
        myself.stop(None);
    }
}

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
