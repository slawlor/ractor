struct GeneratedActor;

#[ractor::actor(messages = GeneratedMessage)]
impl GeneratedActor {
    #[ractor::message(Discard(_))]
    fn discard(&self) {}
}

fn main() {}
