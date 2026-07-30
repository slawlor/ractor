struct ImplTraitReplyActor;

#[ractor::actor(message = enum Message)]
impl ImplTraitReplyActor {
    #[ractor::rpc(Read(reply))]
    fn read(&self) -> Option<impl std::fmt::Display> {
        Some(42_u64)
    }
}

fn main() {}
