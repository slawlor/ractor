// Starting from here the code is copy pasted for the rust standard library

use std::{any::Any, marker::PhantomData};

#[cfg(all(
    feature = "tokio_runtime",
    not(all(target_arch = "wasm32", target_os = "unknown"))
))]
use crate::thread_local::ThreadLocalActor;
use crate::{Actor, ActorCell, ActorRef, DerivedActorRef};

use super::Message;

pub(crate) struct DerivedProviderType<TActor>(PhantomData<fn() -> TActor>);
impl<T> DerivedProviderType<T> {
    pub(crate) fn new() -> Self {
        DerivedProviderType(PhantomData)
    }
}
#[cfg(all(
    feature = "tokio_runtime",
    not(all(target_arch = "wasm32", target_os = "unknown"))
))]
pub(crate) struct DerivedProviderTypeLocal<TActor>(PhantomData<fn() -> TActor>);
#[cfg(all(
    feature = "tokio_runtime",
    not(all(target_arch = "wasm32", target_os = "unknown"))
))]
impl<T> DerivedProviderTypeLocal<T> {
    pub(crate) fn new() -> Self {
        DerivedProviderTypeLocal(PhantomData)
    }
}

pub(crate) trait DerivedProvider: Send + Sync + 'static {
    fn provide_derived_actor_ref<'a>(
        &self,
        actor_cell: ActorCell,
        request: RequestDerived<'a>,
    ) -> RequestDerived<'a>;
}
impl<TActor: Actor> DerivedProvider for DerivedProviderType<TActor> {
    fn provide_derived_actor_ref<'a>(
        &self,
        actor_cell: ActorCell,
        request: RequestDerived<'a>,
    ) -> RequestDerived<'a> {
        TActor::provide_derived_actor_ref(ActorRef::<TActor::Msg>::from(actor_cell), request)
    }
}
#[cfg(all(
    feature = "tokio_runtime",
    not(all(target_arch = "wasm32", target_os = "unknown"))
))]
impl<TActor: ThreadLocalActor> DerivedProvider for DerivedProviderTypeLocal<TActor> {
    fn provide_derived_actor_ref<'a>(
        &self,
        actor_cell: ActorCell,
        request: RequestDerived<'a>,
    ) -> RequestDerived<'a> {
        TActor::provide_derived_actor_ref(ActorRef::<TActor::Msg>::from(actor_cell), request)
    }
}
pub(crate) struct OwnedRequest<TMessage: 'static>(Option<DerivedActorRef<TMessage>>);
impl<TMessage: 'static> OwnedRequest<TMessage> {
    pub(crate) fn new() -> Self {
        Self(None)
    }
    pub(crate) fn as_request(&mut self) -> RequestDerived<'_> {
        RequestDerived(&mut self.0)
    }
    pub(crate) fn extract(self) -> Option<DerivedActorRef<TMessage>> {
        self.0
    }
}

/// Type used as argument to [Message::proved_derived_actor_ref]
#[repr(transparent)]
pub struct RequestDerived<'a>(&'a mut dyn Any);

impl RequestDerived<'_> {
    /// Provides a derived actor
    pub fn provide_derived_actor<T: Message>(&mut self, derived: DerivedActorRef<T>) -> &mut Self {
        if let Some(r) = self.0.downcast_mut::<Option<DerivedActorRef<T>>>() {
            *r = Some(derived)
        }
        self
    }
}

impl std::fmt::Debug for RequestDerived<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request").finish_non_exhaustive()
    }
}
#[cfg(test)]
mod tests {
    use crate::{concurrency::MpscSender, Actor};

    #[cfg(feature = "cluster")]
    use super::Message;
    use super::RequestDerived;

    struct TestActor;

    #[derive(Debug, Eq, Clone, PartialEq)]
    enum Msg {
        I32(i32),
        I64(i64),
    }
    impl From<i32> for Msg {
        fn from(value: i32) -> Self {
            Msg::I32(value)
        }
    }
    impl From<i64> for Msg {
        fn from(value: i64) -> Self {
            Msg::I64(value)
        }
    }
    impl TryFrom<Msg> for i32 {
        type Error = ();
        fn try_from(value: Msg) -> Result<Self, Self::Error> {
            match value {
                Msg::I32(v) => Ok(v),
                _ => Err(()),
            }
        }
    }
    impl TryFrom<Msg> for i64 {
        type Error = ();
        fn try_from(value: Msg) -> Result<Self, Self::Error> {
            match value {
                Msg::I64(v) => Ok(v),
                _ => Err(()),
            }
        }
    }
    #[cfg(feature = "cluster")]
    impl Message for Msg {}
    impl Actor for TestActor {
        type Msg = Msg;

        type State = MpscSender<Msg>;

        type Arguments = MpscSender<Msg>;

        async fn pre_start(
            &self,
            _myself: crate::ActorRef<Self::Msg>,
            args: Self::Arguments,
        ) -> Result<Self::State, crate::ActorProcessingErr> {
            Ok(args)
        }
        async fn handle(
            &self,
            _myself: crate::ActorRef<Self::Msg>,
            message: Self::Msg,
            state: &mut Self::State,
        ) -> Result<(), crate::ActorProcessingErr> {
            state.send(message).await?;
            Ok(())
        }
        fn provide_derived_actor_ref<'a>(
            myself: crate::ActorRef<Msg>,
            mut request: RequestDerived<'a>,
        ) -> RequestDerived<'a> {
            request.provide_derived_actor(myself.get_derived::<i32>());
            request.provide_derived_actor(myself.get_derived::<i64>());
            request
        }
    }
    #[tokio::test]
    async fn derived_actor_from_cell() {
        let (sx, mut rx) = crate::concurrency::mpsc_bounded(10);
        let (ar, _) = Actor::spawn(None, TestActor, sx).await.unwrap();
        let cell = ar.get_cell();
        let dar = cell.provide_derived::<i32>().unwrap();
        dar.send_message(42).unwrap();
        assert_eq!(rx.recv().await.unwrap(), Msg::I32(42));
        let dar = cell.provide_derived::<i64>().unwrap();
        dar.send_message(33).unwrap();
        assert_eq!(rx.recv().await.unwrap(), Msg::I64(33));
    }
}
