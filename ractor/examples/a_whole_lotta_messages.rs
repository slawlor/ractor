// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Just creates a LOT of actors. Useful for measuring max memory util
//!
//! Execute with
//!
//! ```text
//! cargo run --example a_whole_lotta_messages
//! ```

#![allow(clippy::incompatible_msrv)]

extern crate ractor;

use ractor::{concurrency::Duration, Actor, ActorProcessingErr, ActorRef};

struct Counter;

#[cfg_attr(feature = "async-trait", ractor::async_trait)]
impl Actor for Counter {
    type Msg = u32;
    type State = ();
    type Arguments = ();

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        _: (),
    ) -> Result<Self::State, ActorProcessingErr> {
        tracing::info!("Starting the actor");
        // create the initial state
        Ok(())
    }

    async fn handle(
        &self,
        _: ActorRef<Self::Msg>,
        _: Self::Msg,
        _: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        // block the loop so we won't process messages
        ractor::concurrency::sleep(Duration::from_secs(10000)).await;
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

#[ractor_example_entry_proc::ractor_example_entry]
async fn main() {
    init_logging();

    tracing::info!("test");

    let (actor, handle) = Actor::spawn(None, Counter, ())
        .await
        .expect("Failed to start actor!");

    for i in 1..1_000_000 {
        actor.cast(i).expect("Failed to enqueue message to actor");
    }

    actor.kill();
    handle.await.expect("Failed to join handle");
}
