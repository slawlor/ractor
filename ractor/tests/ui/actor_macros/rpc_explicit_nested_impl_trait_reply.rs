struct ExplicitImplTraitReplyActor;

#[ractor::actor(message = enum Message)]
impl ExplicitImplTraitReplyActor {
    #[ractor::rpc(Read(reply), reply = Option<impl std::fmt::Display>)]
    fn read(&self) -> Result<Option<u64>, ractor::ActorProcessingErr> {
        Ok(Some(42))
    }
}

fn main() {}
