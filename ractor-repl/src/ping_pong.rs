// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! A ping-pong actor implementation

use ractor::{ActorCell, ActorHandler};

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

    async fn pre_start(&self, myself: ActorCell) -> Self::State {
        println!("pre_start called");
        // startup the event processing
        self.send_message(myself, Message::Ping).unwrap();
        // create the initial state
        0u8
    }

    async fn post_start(
        &self,
        _this_actor: ActorCell,
        _state: &Self::State,
    ) -> Option<Self::State> {
        println!("post_start called");
        None
    }

    /// Invoked after an actor has been stopped.
    async fn post_stop(&self, _this_actor: ActorCell, _state: Self::State) -> Self::State {
        println!("post_stop called");
        _state
    }

    async fn handle(
        &self,
        myself: ActorCell,
        message: Self::Msg,
        state: &Self::State,
    ) -> Option<Self::State> {
        if *state < 10u8 {
            message.print();
            self.send_message(myself, message.next()).unwrap();
            Some(*state + 1)
        } else {
            println!();
            myself.stop();
            // don't send another message, rather stop the agent after 10 iterations
            None
        }
    }
}
