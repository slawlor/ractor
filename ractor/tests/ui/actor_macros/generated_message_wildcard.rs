struct GeneratedActor;

#[ractor::actor(message = enum GeneratedMessage)]
impl GeneratedActor {
    #[ractor::message(Discard(_))]
    fn discard(&self) {}
}

fn main() {}
