use ractor::RpcReplyPort;

struct MissingReplyActor;

enum Message {
    Read(u64, RpcReplyPort<u64>),
}

#[ractor::actor(message = Message)]
impl MissingReplyActor {
    #[ractor::rpc(Message::Read(value, reply))]
    fn read(&self, value: u64, reply: RpcReplyPort<u64>) -> u64 {
        value + usize::from(reply.is_closed()) as u64
    }
}

fn main() {}
