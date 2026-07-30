struct AmbiguousReplyActor;

enum Message {
    Read(u64, u64),
}

#[ractor::actor(message = Message)]
impl AmbiguousReplyActor {
    #[ractor::rpc(Message::Read(reply, reply))]
    fn read(&self, reply: u64) -> u64 {
        reply
    }
}

fn main() {}
