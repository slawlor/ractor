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

use ractor::{Actor, ActorHandler, ActorRef};

pub struct PingPong;

#[derive(Debug, Clone)]
pub enum Message {
    Ping,
    Pong,
}

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
impl ActorHandler for PingPong {
    type Msg = Message;

    type State = u8;

    async fn pre_start(&self, myself: ActorRef<Self>) -> Self::State {
        // startup the event processing
        myself.send_message(Message::Ping).unwrap();
        // create the initial state
        0u8
    }

    async fn handle(
        &self,
        myself: ActorRef<Self>,
        message: Self::Msg,
        state: &Self::State,
    ) -> Option<Self::State> {
        if *state < 10u8 {
            message.print();
            myself.send_message(message.next()).unwrap();
            Some(*state + 1)
        } else {
            println!();
            myself.stop(None);
            None
        }
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
