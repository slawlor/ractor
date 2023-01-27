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

extern crate ractor;

use ractor::{cast, Actor, ActorProcessingErr, ActorRef};

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

#[async_trait::async_trait]
impl Actor for PingPong {
    type Msg = Message;

    type State = u8;

    async fn pre_start(&self, myself: ActorRef<Self>) -> Result<Self::State, ActorProcessingErr> {
        // startup the event processing
        cast!(myself, Message::Ping).unwrap();
        // create the initial state
        Ok(0u8)
    }

    async fn handle(
        &self,
        myself: ActorRef<Self>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        if *state < 10u8 {
            message.print();
            cast!(myself, message.next()).unwrap();
            *state += 1;
        } else {
            println!();
            myself.stop(None);
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let (_actor, handle) = Actor::spawn(None, PingPong)
        .await
        .expect("Failed to start ping-pong actor");
    handle
        .await
        .expect("Ping-pong actor failed to exit properly");
}
