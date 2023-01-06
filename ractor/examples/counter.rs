// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! A basic counting agent. Demonstrates remote procedure calls to interact
//! with the agent externally and safely acquire the "count"
//!
//! Execute with
//!
//! ```text
//! cargo run --example counter
//! ```

extern crate ractor;

use ractor::{rpc, Actor, ActorCell, ActorHandler, RpcReplyPort};
use tokio::time::Duration;

struct Counter;

#[derive(Clone)]
struct CounterState {
    count: i64,
}

enum CounterMessage {
    Increment(i64),
    Decrement(i64),
    Retrieve(RpcReplyPort<i64>),
}

#[async_trait::async_trait]
impl ActorHandler for Counter {
    type Msg = CounterMessage;

    type State = CounterState;

    async fn pre_start(&self, _myself: ActorCell) -> Self::State {
        // create the initial state
        CounterState { count: 0 }
    }

    async fn handle(
        &self,
        _myself: ActorCell,
        message: Self::Msg,
        state: &Self::State,
    ) -> Option<Self::State> {
        match message {
            CounterMessage::Increment(how_much) => Some(CounterState {
                count: state.count + how_much,
            }),
            CounterMessage::Decrement(how_much) => Some(CounterState {
                count: state.count - how_much,
            }),
            CounterMessage::Retrieve(reply_port) => {
                if !reply_port.is_closed() {
                    reply_port.send(state.count).unwrap();
                }
                None
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let (actor, handle) = Actor::spawn(None, Counter)
        .await
        .expect("Failed to start actor!");

    // +5 +10 -5 a few times, printing the value via RPC
    for _i in 0..4 {
        actor
            .send_message::<Counter>(CounterMessage::Increment(5))
            .expect("Failed to send message");
        actor
            .send_message::<Counter>(CounterMessage::Increment(10))
            .expect("Failed to send message");
        actor
            .send_message::<Counter>(CounterMessage::Decrement(5))
            .expect("Failed to send message");

        let rpc_result = rpc::call::<Counter, _, _>(
            &actor,
            CounterMessage::Retrieve,
            Some(Duration::from_millis(10)),
        )
        .await
        .expect("Failed to send RPC");

        println!(
            "Count is: {}",
            rpc_result.expect("RPC failed to reply successfully")
        );
    }

    actor.stop(None);
    handle.await.expect("Actor failed to exit cleanly");
}
