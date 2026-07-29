#![allow(unused_imports)]

use ractor::{ActorProcessingErr, ActorRef};

struct MixedActor;

enum Message {
    Go,
}

#[ractor::actor(message = Message)]
impl MixedActor {
    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        _message: Self::Msg,
        _state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        Ok(())
    }

    #[ractor::message(Message::Go)]
    fn go(&self) {}
}

fn main() {}
