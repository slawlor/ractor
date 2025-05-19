// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! An example ping-pong actor which posts back to itself to pass the
//! ping or pong back + forth
//!
//! Execute with
//!
//! ```text
//! cargo run --example ping_pong
//! ```

#![allow(clippy::incompatible_msrv)]

extern crate ractor;

use ractor::cast;
use ractor::Actor;
use ractor::ActorProcessingErr;
use ractor::ActorRef;

pub struct PingPong;

#[derive(Debug, Clone)]
pub enum Message {
    Ping,
    Pong,
}
#[cfg(feature = "cluster")]
impl ractor::Message for Message {}

impl Message {
    fn next(&self) -> Self {
        match self {
            Self::Ping => Self::Pong,
            Self::Pong => Self::Ping,
        }
    }

    fn print(&self) {
        match self {
            Self::Ping => print!("ping.."),
            Self::Pong => print!("pong.."),
        }
    }
}

#[cfg_attr(
    all(
        feature = "async-trait",
        not(all(target_arch = "wasm32", target_os = "unknown"))
    ),
    ractor::async_trait
)]
#[cfg_attr(all(feature = "async-trait", all(target_arch = "wasm32", target_os = "unknown")), ractor::async_trait(?Send))]
impl Actor for PingPong {
    type Msg = Message;

    type State = u8;
    type Arguments = ();

    async fn pre_start(
        &self,
        myself: ActorRef<Self::Msg>,
        _: (),
    ) -> Result<Self::State, ActorProcessingErr> {
        // startup the event processing
        cast!(myself, Message::Ping).unwrap();
        // create the initial state
        Ok(0u8)
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        if *state < 10u8 {
            message.print();
            cast!(myself, message.next()).unwrap();
            *state += 1;
        } else {
            tracing::info!("");
            myself.stop(None);
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

#[ractor_example_entry_proc::ractor_example_entry]
async fn main() {
    init_logging();

    let (_actor, handle) = Actor::spawn(None, PingPong, ())
        .await
        .expect("Failed to start ping-pong actor");
    handle
        .await
        .expect("Ping-pong actor failed to exit properly");
}
