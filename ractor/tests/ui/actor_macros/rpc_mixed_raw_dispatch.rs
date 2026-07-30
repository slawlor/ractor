#![allow(unused_imports)]

use ractor::{ActorProcessingErr, ActorRef, RpcReplyPort};

struct MixedRpcActor;

enum Message {
    Read(RpcReplyPort<u64>),
}

#[ractor::actor(message = Message)]
impl MixedRpcActor {
    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        _message: Self::Msg,
        _state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        Ok(())
    }

    #[ractor::rpc(Message::Read(reply))]
    fn read(&self) -> u64 {
        42
    }
}

fn main() {}
