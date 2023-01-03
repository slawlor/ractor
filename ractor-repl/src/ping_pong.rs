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

    fn pre_start(&self, myself: ActorCell) -> Self::State {
        println!("Pre_start called");
        // startup the event processing
        let _ = myself.send_message_t(Message::Ping);
        // create the initial state
        0u8
    }

    async fn handle(
        &self,
        myself: ractor::ActorCell,
        message: Self::Msg,
        state: &Self::State,
    ) -> Option<Self::State> {
        let result = match message {
            Message::Ping => {
                println!("ping");
                Some(*state + 1u8)
            }
            Message::Pong => {
                println!("pong");
                Some(*state + 1u8)
            }
            Message::Stop => {
                println!("Stopping");
                let _ = myself.stop().await;
                return None;
            }
        };
        if *state > 10u8 {
            println!("Sending stop...");
            let _ = myself.send_message_t(Message::Stop);
        } else {
            let next = message.next();
            let _ = myself.send_message_t(next);
        }
        result
    }
}
