// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! A ping-pong actor implementation

use ractor::{ActorHandler, ActorRef};

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
        println!("pre_start called");
        // startup the event processing
        myself.send_message(Message::Ping).unwrap();
        // create the initial state
        0u8
    }

    async fn post_start(&self, _this_actor: ActorRef<Self>, _state: &mut Self::State) {
        println!("post_start called");
    }

    /// Invoked after an actor has been stopped.
    async fn post_stop(&self, _this_actor: ActorRef<Self>, _state: &mut Self::State) {
        println!("post_stop called");
    }

    async fn handle(&self, myself: ActorRef<Self>, message: Self::Msg, state: &mut Self::State) {
        if *state < 10u8 {
            message.print();
            myself.send_message(message.next()).unwrap();
            *state += 1;
        } else {
            println!();
            myself.stop(None);
            // don't send another message, rather stop the agent after 10 iterations
        }
    }
}
