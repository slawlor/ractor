struct StatefulActor;

enum Message {
    Read,
}

#[ractor::actor(message = Message, state = u64)]
impl StatefulActor {
    #[ractor::message(Message::Read)]
    fn read(&self, _state: &u64) {}
}

fn main() {}
