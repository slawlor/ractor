use ractor::{ActorCell, SupervisionEvent};

type EventAlias = SupervisionEvent;

struct DuplicateSupervisor;

#[ractor::actor(message = ())]
impl DuplicateSupervisor {
    #[ractor::supervision(SupervisionEvent::ActorStarted(child))]
    fn direct(&self, child: ActorCell) {
        let _ = child;
    }

    #[ractor::supervision(EventAlias::ActorStarted(child))]
    fn through_alias(&self, child: ActorCell) {
        let _ = child;
    }
}

fn main() {}
