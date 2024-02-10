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

use ractor::{call_t, Actor, ActorProcessingErr, ActorRef, RpcReplyPort};

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
#[cfg(feature = "cluster")]
impl ractor::Message for CounterMessage {}

#[cfg_attr(feature = "async-trait", ractor::async_trait)]
impl Actor for Counter {
    type Msg = CounterMessage;

    type State = CounterState;
    type Arguments = ();

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        _: (),
    ) -> Result<Self::State, ActorProcessingErr> {
        tracing::info!("Starting the counter actor");
        // create the initial state
        Ok(CounterState { count: 0 })
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        tracing::info!("Counting actor handle message...");
        match message {
            CounterMessage::Increment(how_much) => {
                state.count += how_much;
            }
            CounterMessage::Decrement(how_much) => {
                state.count -= how_much;
            }
            CounterMessage::Retrieve(reply_port) => {
                if !reply_port.is_closed() {
                    reply_port.send(state.count).unwrap();
                }
            }
        }
        Ok(())
    }
}

fn init_logging() {
    let dir = tracing_subscriber::filter::Directive::from(tracing::Level::DEBUG);

    use std::io::stderr;
    use std::io::IsTerminal;
    use tracing_glog::Glog;
    use tracing_glog::GlogFields;
    use tracing_subscriber::filter::EnvFilter;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::Registry;

    let fmt = tracing_subscriber::fmt::Layer::default()
        .with_ansi(stderr().is_terminal())
        .with_writer(std::io::stderr)
        .event_format(Glog::default().with_timer(tracing_glog::LocalTime::default()))
        .fmt_fields(GlogFields::default().compact());

    let filter = vec![dir]
        .into_iter()
        .fold(EnvFilter::from_default_env(), |filter, directive| {
            filter.add_directive(directive)
        });

    let subscriber = Registry::default().with(filter).with(fmt);
    tracing::subscriber::set_global_default(subscriber).expect("to set global subscriber");
}

#[tokio::main]
async fn main() {
    init_logging();

    let (actor, handle) = Actor::spawn(Some("test_name".to_string()), Counter, ())
        .await
        .expect("Failed to start actor!");

    // +5 +10 -5 a few times, printing the value via RPC
    for _i in 0..4 {
        actor
            .send_message(CounterMessage::Increment(5))
            .expect("Failed to send message");
        actor
            .send_message(CounterMessage::Increment(10))
            .expect("Failed to send message");
        actor
            .send_message(CounterMessage::Decrement(5))
            .expect("Failed to send message");

        let rpc_result = call_t!(actor, CounterMessage::Retrieve, 10);

        tracing::info!(
            "Count is: {}",
            rpc_result.expect("RPC failed to reply successfully")
        );
    }

    actor.stop(None);
    handle.await.expect("Actor failed to exit cleanly");
}
