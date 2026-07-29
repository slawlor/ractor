#![allow(unused_imports)]

use ractor::{ActorCell, ActorProcessingErr, ActorRef, SupervisionEvent};

struct MixedSupervisor;

enum Message {
    Go,
}

#[ractor::actor(message = Message)]
impl MixedSupervisor {
    #[ractor::message(Message::Go)]
    fn go(&self) {}

    async fn handle_supervisor_evt(
        &self,
        _myself: ActorRef<Self::Msg>,
        _event: SupervisionEvent,
        _state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        Ok(())
    }

    #[ractor::supervision(SupervisionEvent::ActorStarted(child))]
    fn child_started(&self, child: ActorCell) {}
}

fn main() {}
