// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! A ping-pong actor implementation

use ractor::actor::*;
use ractor::ActorCell;

pub struct PingPong;

#[derive(Debug, Clone)]
pub enum Message {
    Ping,
    Pong,
    Stop,
}

impl Message {
    fn next(&self) -> Self {
        match self {
            Self::Ping => Self::Pong,
            Self::Pong => Self::Ping,
            Self::Stop => Self::Stop,
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
        myself: ractor::ActorCell,
        message: Self::Msg,
        state: &Self::State,
    ) -> Option<Self::State> {
        let result = match message {
            Message::Ping => {
                print!("ping..");
                Some(*state + 1u8)
            }
            Message::Pong => {
                print!("pong..");
                Some(*state + 1u8)
            }
            Message::Stop => {
                println!("Stopping");
                myself.stop().await;
                return None;
            }
        };
        if *state > 10u8 {
            println!("\nSending stop...");
            self.send_message(myself, Message::Stop).unwrap();
        } else {
            let next = message.next();
            self.send_message(myself, next).unwrap();
        }
        result
    }
}
