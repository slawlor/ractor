enum Message {
    Go,
}

type MessageAlias = Message;

struct DuplicateActor;

#[ractor::actor(message = Message)]
impl DuplicateActor {
    #[ractor::message(Message::Go)]
    fn direct(&self) {}

    #[ractor::message(MessageAlias::Go)]
    fn through_alias(&self) {}
}

fn main() {}
