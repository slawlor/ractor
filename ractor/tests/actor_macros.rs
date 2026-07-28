// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use std::cell::Cell;
use std::marker::PhantomData;
use std::rc::Rc;

use ractor::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort};

struct Counter;

enum CounterMessage {
    Add(i64),
    #[cfg(any())]
    Disabled,
    #[cfg_attr(all(), cfg(any()))]
    DisabledByCfgAttr,
    Replace {
        value: i64,
    },
    InternalNames {
        __ractor_myself: i64,
        __ractor_message: i64,
        __ractor_state: i64,
    },
    Read(RpcReplyPort<i64>),
    Stop,
}

#[cfg(feature = "cluster")]
impl ractor::Message for CounterMessage {}

#[ractor::actor(
    message = CounterMessage,
    state = std::primitive::i64,
    arguments = i64,
)]
impl Counter {
    async fn pre_start(
        &self,
        _myself: ActorRef<CounterMessage>,
        initial: i64,
    ) -> Result<i64, ActorProcessingErr> {
        Ok(initial)
    }

    #[ractor::message(CounterMessage::Add(amount))]
    #[cfg_attr(all(), tracing::instrument(skip(self, state)))]
    fn add(&self, amount: i64, state: &mut i64) {
        *state += amount;
    }

    #[cfg(any())]
    #[ractor::message(CounterMessage::Disabled)]
    fn disabled(&self) {}

    #[cfg_attr(all(), cfg(any()))]
    #[ractor::message(CounterMessage::DisabledByCfgAttr)]
    fn disabled_by_cfg_attr(&self) {}

    #[ractor::message(CounterMessage::Replace { value })]
    async fn replace(&self, value: i64, state: &mut i64) -> Result<(), ActorProcessingErr> {
        *state = value;
        Ok(())
    }

    #[ractor::message(CounterMessage::InternalNames {
        __ractor_myself,
        __ractor_message,
        __ractor_state,
    })]
    fn internal_names(
        &self,
        myself: ActorRef<CounterMessage>,
        __ractor_myself: i64,
        __ractor_message: i64,
        __ractor_state: i64,
        state: &mut i64,
    ) {
        assert_eq!(myself.get_status(), ractor::ActorStatus::Running);
        *state = __ractor_myself + __ractor_message + __ractor_state;
    }

    #[ractor::message(CounterMessage::Read(reply))]
    fn read(&self, reply: RpcReplyPort<i64>, state: &i64) {
        let _ = reply.send(*state);
    }

    #[ractor::message(CounterMessage::Stop)]
    fn stop(&self, myself: ActorRef<CounterMessage>) {
        myself.stop(None);
    }
}

struct RawActor;

enum RawMessage {
    Ping(RpcReplyPort<&'static str>),
}

#[cfg(feature = "cluster")]
impl ractor::Message for RawMessage {}

#[ractor::actor(message = RawMessage)]
impl RawActor {
    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        _state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        let RawMessage::Ping(reply) = message;
        let _ = reply.send("pong");
        Ok(())
    }
}

#[cfg_attr(feature = "async-std", allow(dead_code))]
#[derive(Default)]
struct LocalCounter;

#[cfg_attr(feature = "async-std", allow(dead_code))]
enum LocalMessage {
    Add(i64),
    Read(RpcReplyPort<i64>),
    Stop,
}

#[cfg(feature = "cluster")]
impl ractor::Message for LocalMessage {}

#[ractor::actor(
    thread_local,
    message = LocalMessage,
    state = Rc<Cell<i64>>,
    arguments = i64,
)]
#[cfg_attr(feature = "async-std", allow(dead_code))]
impl LocalCounter {
    async fn pre_start(
        &self,
        _myself: ActorRef<LocalMessage>,
        initial: i64,
    ) -> Result<Rc<Cell<i64>>, ActorProcessingErr> {
        Ok(Rc::new(Cell::new(initial)))
    }

    #[ractor::message(LocalMessage::Add(amount))]
    fn add(&self, amount: i64, state: &Rc<Cell<i64>>) {
        state.set(state.get() + amount);
    }

    #[ractor::message(LocalMessage::Read(reply))]
    fn read(&self, reply: RpcReplyPort<i64>, state: &Rc<Cell<i64>>) {
        let _ = reply.send(state.get());
    }

    #[ractor::message(LocalMessage::Stop)]
    fn stop(&self, myself: ActorRef<LocalMessage>) {
        myself.stop(None);
    }
}

struct GenericActor<T>(PhantomData<T>);

enum GenericMessage<T> {
    Echo { value: T, reply: RpcReplyPort<T> },
}

#[cfg(feature = "cluster")]
impl<T: Send + 'static> ractor::Message for GenericMessage<T> {}

#[ractor::actor(message = GenericMessage<T>)]
impl<T> GenericActor<T>
where
    T: Send + Sync + 'static,
{
    #[ractor::message(GenericMessage::Echo { value, reply })]
    fn echo(&self, value: T, reply: RpcReplyPort<T>) {
        let _ = reply.send(value);
    }
}

mod shadowed_prelude_names {
    use super::Actor;

    type Result<T> = Option<T>;

    enum Shadow {
        Ok,
    }

    use Shadow::Ok;

    struct ShadowedActor;

    enum Message {
        Run,
    }

    #[cfg(feature = "cluster")]
    impl ractor::Message for Message {}

    #[ractor::actor(message = Message)]
    impl ShadowedActor {
        #[ractor::message(Message::Run)]
        fn run(&self) {
            let _: Result<()> = Some(());
            let _ = Ok;
        }
    }

    pub(super) fn assert_compiles() {
        fn assert_actor<T: Actor<Msg = Message>>() {}

        assert_actor::<ShadowedActor>();
        let _ = Message::Run;
    }
}

#[ractor::concurrency::test]
async fn generated_handlers_dispatch_messages_and_state() {
    let (actor, handle) = Actor::spawn(None, Counter, 5)
        .await
        .expect("counter failed to start");

    actor
        .send_message(CounterMessage::Add(7))
        .expect("add message failed");
    actor
        .send_message(CounterMessage::Replace { value: 20 })
        .expect("replace message failed");
    actor
        .send_message(CounterMessage::InternalNames {
            __ractor_myself: 3,
            __ractor_message: 4,
            __ractor_state: 5,
        })
        .expect("internal-name message failed");
    let value =
        ractor::call_t!(actor, CounterMessage::Read, 100).expect("counter failed to answer");
    assert_eq!(value, 12);

    actor
        .send_message(CounterMessage::Stop)
        .expect("stop message failed");
    handle.await.expect("counter failed to stop cleanly");
}

#[ractor::concurrency::test]
async fn raw_handle_mode_remains_available() {
    let (actor, handle) = Actor::spawn(None, RawActor, ())
        .await
        .expect("raw actor failed to start");

    let response = ractor::call_t!(actor, RawMessage::Ping, 100).expect("raw actor call failed");
    assert_eq!(response, "pong");

    actor.stop(None);
    handle.await.expect("raw actor failed to stop cleanly");
}

#[cfg(not(feature = "async-std"))]
#[ractor::concurrency::test]
async fn thread_local_actor_supports_non_send_state() {
    let spawner = ractor::thread_local::ThreadLocalActorSpawner::new();
    let (actor, handle) = ractor::spawn_local::<LocalCounter>(10, spawner)
        .await
        .expect("local counter failed to start");

    actor
        .send_message(LocalMessage::Add(4))
        .expect("local add message failed");
    let value =
        ractor::call_t!(actor, LocalMessage::Read, 100).expect("local counter failed to answer");
    assert_eq!(value, 14);

    actor
        .send_message(LocalMessage::Stop)
        .expect("local stop message failed");
    actor
        .wait(Some(ractor::concurrency::Duration::from_secs(1)))
        .await
        .expect("local counter failed to stop cleanly");
    drop(handle);
}

#[test]
fn generic_actor_impls_preserve_bounds() {
    fn assert_actor<T: Actor<Msg = GenericMessage<String>>>() {}

    assert_actor::<GenericActor<String>>();
    let (reply, _receiver) = ractor::concurrency::oneshot();
    let _message = GenericMessage::Echo {
        value: String::new(),
        reply: reply.into(),
    };
}

#[test]
fn generated_code_ignores_shadowed_prelude_names() {
    shadowed_prelude_names::assert_compiles();
}

#[cfg(all(not(feature = "async-trait"), not(feature = "cluster")))]
#[test]
fn actor_macro_diagnostics_are_actionable() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/actor_macros/*.rs");
}
