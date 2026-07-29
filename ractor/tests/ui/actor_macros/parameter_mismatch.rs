struct MismatchedActor;

enum Message {
    Add(u64),
}

#[ractor::actor(message = Message)]
impl MismatchedActor {
    #[ractor::message(Message::Add(amount))]
    fn add(&self, value: u64) {}
}

fn main() {}
