// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use std::cell::Cell;
use std::marker::PhantomData;
use std::rc::Rc;

use ractor::actor::messages::BoxedState;
use ractor::{
    Actor, ActorCell, ActorId, ActorProcessingErr, ActorRef, ActorStatus, RpcReplyPort,
    SupervisionEvent,
};

struct Counter;

#[ractor::actor(
    message = enum CounterMessage,
    state = std::primitive::i64,
    arguments = i64,
    crate_path = ::ractor,
)]
impl Counter {
    async fn pre_start(
        &self,
        _myself: ActorRef<CounterMessage>,
        initial: i64,
    ) -> Result<i64, ActorProcessingErr> {
        Ok(initial)
    }

    #[ractor::message(Add(amount))]
    #[cfg_attr(all(), tracing::instrument(skip(self, state)))]
    fn add(&self, amount: i64, state: &mut i64) {
        *state += amount;
    }

    #[cfg(any())]
    #[ractor::message(Disabled)]
    fn disabled(&self) {}

    #[cfg_attr(all(), cfg(any()))]
    #[ractor::message(DisabledByCfgAttr)]
    fn disabled_by_cfg_attr(&self) {}

    #[ractor::message(Replace { value })]
    async fn replace(&self, value: i64, state: &mut i64) -> Result<(), ActorProcessingErr> {
        *state = value;
        Ok(())
    }

    #[ractor::message(InternalNames {
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

    #[ractor::rpc(Read)]
    fn read(&self, state: &i64) -> i64 {
        *state
    }

    #[ractor::rpc(Sum {
        left,
        reply,
        right,
    })]
    async fn sum(
        &self,
        myself: ActorRef<CounterMessage>,
        left: i64,
        right: i64,
        state: &i64,
    ) -> i64 {
        assert_eq!(myself.get_status(), ActorStatus::Running);
        left + right + *state
    }

    #[ractor::rpc(ResultPayload(reply))]
    fn result_payload(&self, state: &i64) -> Result<i64, &'static str> {
        Ok(*state)
    }

    #[ractor::rpc(Fallible(reply), reply = i64)]
    fn fallible(&self, state: &i64) -> Result<i64, ActorProcessingErr> {
        Ok(*state)
    }

    #[ractor::rpc(DroppedReply(reply))]
    fn dropped_reply(&self, state: &i64) -> i64 {
        *state
    }

    #[cfg(any())]
    #[ractor::rpc(DisabledRpc(reply))]
    fn disabled_rpc(&self) -> i64 {
        0
    }

    #[ractor::message(Stop)]
    fn stop(&self, myself: ActorRef<CounterMessage>) {
        myself.stop(None);
    }
}

#[cfg(feature = "cluster")]
impl ractor::Message for CounterMessage {}

struct RawActor;

enum RawMessage {
    Ping(RpcReplyPort<&'static str>),
}

#[cfg(feature = "cluster")]
impl ractor::Message for RawMessage {}

#[ractor::actor(message = RawMessage, crate_path = ::ractor)]
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

struct ExistingRpcActor;

enum ExistingRpcMessage {
    NoPayload(RpcReplyPort<&'static str>),
    Tuple(i64, RpcReplyPort<i64>, i64),
    Struct {
        reply: RpcReplyPort<String>,
        prefix: String,
    },
    Fail(RpcReplyPort<u64>),
    Stop,
    #[cfg(any())]
    Disabled(RpcReplyPort<()>),
}

#[cfg(feature = "cluster")]
impl ractor::Message for ExistingRpcMessage {}

#[ractor::actor(message = ExistingRpcMessage, crate_path = ::ractor)]
impl ExistingRpcActor {
    #[ractor::rpc(ExistingRpcMessage::NoPayload)]
    fn no_payload(&self) -> &'static str {
        "pong"
    }

    #[ractor::rpc(ExistingRpcMessage::Tuple(left, reply, right))]
    async fn tuple(&self, left: i64, right: i64) -> i64 {
        left + right
    }

    #[ractor::rpc(ExistingRpcMessage::Struct { reply, prefix })]
    fn structure(&self, prefix: String) -> String {
        format!("{prefix}-reply")
    }

    #[ractor::rpc(ExistingRpcMessage::Fail(reply), reply = u64)]
    fn fail(&self) -> Result<u64, ActorProcessingErr> {
        Err("intentional RPC processing failure".into())
    }

    #[ractor::message(ExistingRpcMessage::Stop)]
    fn stop(&self, myself: ActorRef<ExistingRpcMessage>) {
        myself.stop(None);
    }

    #[cfg(any())]
    #[ractor::rpc(ExistingRpcMessage::Disabled(reply))]
    fn disabled(&self) {}
}

struct SupervisionOnlyActor {
    events: ractor::concurrency::MpscUnboundedSender<&'static str>,
}

#[ractor::actor(message = (), crate_path = ::ractor)]
impl SupervisionOnlyActor {
    #[ractor::supervision(SupervisionEvent::ActorStarted(_child))]
    fn child_started(&self, _child: ActorCell) {
        let _ = self.events.send("started");
    }

    #[ractor::supervision(SupervisionEvent::ActorTerminated(_child, _, _))]
    fn child_terminated(&self, _child: ActorCell) {
        let _ = self.events.send("terminated");
    }
}

struct GeneratedEmptyActor;

#[ractor::actor(
    message = enum GeneratedEmptyMessage,
    crate_path = ::ractor,
)]
impl GeneratedEmptyActor {}

#[cfg(feature = "cluster")]
impl ractor::Message for GeneratedEmptyMessage {}

#[derive(Default)]
struct LocalDefaultHandleActor;

#[ractor::actor(thread_local, message = (), crate_path = ::ractor)]
impl LocalDefaultHandleActor {}

struct FailingChild;

enum FailingChildMessage {
    Fail,
}

#[cfg(feature = "cluster")]
impl ractor::Message for FailingChildMessage {}

#[ractor::actor(message = FailingChildMessage, crate_path = ::ractor)]
impl FailingChild {
    #[ractor::message(FailingChildMessage::Fail)]
    fn fail(&self) {
        panic!("intentional child failure");
    }
}

struct GeneratedSupervisor;

#[derive(Clone, Debug, Eq, PartialEq)]
struct TerminationRecord {
    actor_id: ActorId,
    state_was_unit: bool,
    reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FailureRecord {
    actor_id: ActorId,
    error: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SupervisorState {
    started: Vec<ActorId>,
    terminated: Option<TerminationRecord>,
    failed: Option<FailureRecord>,
    value: i64,
}

#[ractor::actor(
    message = pub enum MacroSupervisorMessage,
    state = SupervisorState,
    crate_path = ::ractor,
)]
impl GeneratedSupervisor {
    async fn pre_start(
        &self,
        _myself: ActorRef<MacroSupervisorMessage>,
        _arguments: (),
    ) -> Result<SupervisorState, ActorProcessingErr> {
        Ok(SupervisorState::default())
    }

    #[ractor::message(Counts(reply))]
    fn counts(&self, reply: RpcReplyPort<SupervisorState>, state: &SupervisorState) {
        let _ = reply.send(state.clone());
    }

    #[ractor::message(Record { value })]
    fn record(&self, value: i64, state: &mut SupervisorState) {
        state.value = value;
    }

    #[ractor::message(Stop)]
    fn stop_generated_supervisor(&self, myself: ActorRef<MacroSupervisorMessage>) {
        myself.stop(None);
    }

    #[ractor::supervision(SupervisionEvent::ActorStarted(child))]
    fn child_started(&self, child: ActorCell, state: &mut SupervisorState) {
        state.started.push(child.get_id());
    }

    #[ractor::supervision(SupervisionEvent::ActorTerminated(child, final_state, reason,))]
    fn child_terminated(
        &self,
        child: ActorCell,
        final_state: Option<BoxedState>,
        reason: Option<String>,
        state: &mut SupervisorState,
    ) {
        let state_was_unit = final_state.is_some_and(|mut value| value.take::<()>().is_ok());
        state.terminated = Some(TerminationRecord {
            actor_id: child.get_id(),
            state_was_unit,
            reason,
        });
    }

    #[ractor::supervision(SupervisionEvent::ActorFailed(child, error))]
    fn child_failed(
        &self,
        child: ActorCell,
        error: ActorProcessingErr,
        state: &mut SupervisorState,
    ) {
        state.failed = Some(FailureRecord {
            actor_id: child.get_id(),
            error: error.to_string(),
        });
    }
}

#[cfg(feature = "cluster")]
impl ractor::Message for MacroSupervisorMessage {}

struct DefaultingSupervisor;

enum DefaultingSupervisorMessage {
    Ping,
}

#[cfg(feature = "cluster")]
impl ractor::Message for DefaultingSupervisorMessage {}

#[ractor::actor(
    message = DefaultingSupervisorMessage,
    state = usize,
    crate_path = ::ractor,
)]
impl DefaultingSupervisor {
    async fn pre_start(
        &self,
        _myself: ActorRef<DefaultingSupervisorMessage>,
        _arguments: (),
    ) -> Result<usize, ActorProcessingErr> {
        Ok(0)
    }

    #[ractor::message(DefaultingSupervisorMessage::Ping)]
    fn ping(&self) {}

    #[ractor::supervision(SupervisionEvent::ActorStarted(_child))]
    fn child_started(&self, _child: ActorCell, state: &mut usize) {
        *state += 1;
    }
}

#[cfg_attr(feature = "async-std", allow(dead_code))]
#[derive(Default)]
struct LocalCounter;

#[ractor::actor(
    thread_local,
    message = enum LocalMessage,
    state = Rc<Cell<i64>>,
    arguments = i64,
    crate_path = ::ractor,
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

    #[ractor::message(Add(amount))]
    fn add(&self, amount: i64, state: &Rc<Cell<i64>>) {
        state.set(state.get() + amount);
    }

    #[ractor::rpc(Read(reply))]
    fn read(&self, state: &Rc<Cell<i64>>) -> i64 {
        state.get()
    }

    #[ractor::message(Stop)]
    fn stop(&self, myself: ActorRef<LocalMessage>) {
        myself.stop(None);
    }

    #[ractor::supervision(SupervisionEvent::ActorStarted(_child))]
    fn child_started(&self, _child: ActorCell, _state: &Rc<Cell<i64>>) {}
}

#[cfg(feature = "cluster")]
impl ractor::Message for LocalMessage {}

struct GenericActor<T>(PhantomData<T>);

enum GenericMessage<T> {
    Echo { value: T, reply: RpcReplyPort<T> },
}

#[cfg(feature = "cluster")]
impl<T: Send + 'static> ractor::Message for GenericMessage<T> {}

#[ractor::actor(message = GenericMessage<T>, crate_path = ::ractor)]
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

    #[ractor::actor(message = Message, crate_path = ::ractor)]
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

    let sum = actor
        .call(
            |reply| CounterMessage::Sum {
                left: 8,
                reply,
                right: 9,
            },
            Some(ractor::concurrency::Duration::from_millis(100)),
        )
        .await
        .expect("sum RPC message failed")
        .expect("sum RPC failed to answer");
    assert_eq!(sum, 29);

    let result_payload = ractor::call_t!(actor, CounterMessage::ResultPayload, 100)
        .expect("result-payload RPC failed to answer");
    assert_eq!(result_payload, Ok(12));
    let fallible = ractor::call_t!(actor, CounterMessage::Fallible, 100)
        .expect("fallible RPC failed to answer");
    assert_eq!(fallible, 12);

    let (reply, receiver) = ractor::concurrency::oneshot();
    drop(receiver);
    actor
        .send_message(CounterMessage::DroppedReply(reply.into()))
        .expect("dropped-receiver RPC message failed");
    let value_after_dropped_receiver = ractor::call_t!(actor, CounterMessage::Read, 100)
        .expect("counter stopped after an RPC receiver was dropped");
    assert_eq!(value_after_dropped_receiver, 12);

    actor
        .send_message(CounterMessage::Stop)
        .expect("stop message failed");
    handle.await.expect("counter failed to stop cleanly");
}

#[ractor::concurrency::test]
async fn existing_message_enums_support_focused_rpc_handlers() {
    let (actor, handle) = Actor::spawn(None, ExistingRpcActor, ())
        .await
        .expect("existing-message RPC actor failed to start");

    let no_payload =
        ractor::call_t!(actor, ExistingRpcMessage::NoPayload, 100).expect("unit-form RPC failed");
    assert_eq!(no_payload, "pong");

    let tuple = actor
        .call(
            |reply| ExistingRpcMessage::Tuple(20, reply, 22),
            Some(ractor::concurrency::Duration::from_millis(100)),
        )
        .await
        .expect("tuple RPC message failed")
        .expect("tuple RPC failed to answer");
    assert_eq!(tuple, 42);

    let structure = actor
        .call(
            |reply| ExistingRpcMessage::Struct {
                reply,
                prefix: "focused".to_owned(),
            },
            Some(ractor::concurrency::Duration::from_millis(100)),
        )
        .await
        .expect("struct RPC message failed")
        .expect("struct RPC failed to answer");
    assert_eq!(structure, "focused-reply");

    actor
        .send_message(ExistingRpcMessage::Stop)
        .expect("existing-message RPC actor stop failed");
    handle
        .await
        .expect("existing-message RPC actor failed to stop cleanly");

    let (failing_actor, failing_handle) = Actor::spawn(None, ExistingRpcActor, ())
        .await
        .expect("failing RPC actor failed to start");
    let failure = failing_actor
        .call(
            ExistingRpcMessage::Fail,
            Some(ractor::concurrency::Duration::from_millis(100)),
        )
        .await
        .expect("failing RPC message could not be sent");
    assert!(failure.is_send_error());
    failing_handle
        .await
        .expect("failing RPC actor runtime failed to stop");
    assert_eq!(failing_actor.get_status(), ActorStatus::Stopped);
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

#[ractor::concurrency::test]
async fn generated_messages_and_supervision_handlers_dispatch() {
    let (supervisor, supervisor_handle) = Actor::spawn(None, GeneratedSupervisor, ())
        .await
        .expect("generated supervisor failed to start");
    let (child, child_handle) = Actor::spawn_linked(None, RawActor, (), supervisor.get_cell())
        .await
        .expect("linked child failed to start");

    child.stop(None);
    child_handle.await.expect("linked child failed to stop");

    let (failing_child, failing_child_handle) =
        Actor::spawn_linked(None, FailingChild, (), supervisor.get_cell())
            .await
            .expect("failing child failed to start");
    failing_child
        .send_message(FailingChildMessage::Fail)
        .expect("failed to send failure message");
    failing_child_handle
        .await
        .expect("failing child runtime failed to stop");

    supervisor
        .send_message(MacroSupervisorMessage::Record { value: 42 })
        .expect("generated struct message failed");

    let snapshot = ractor::call_t!(supervisor, MacroSupervisorMessage::Counts, 100)
        .expect("generated supervisor failed to report its state");
    assert_eq!(
        snapshot.started,
        vec![child.get_id(), failing_child.get_id()]
    );
    assert_eq!(
        snapshot.terminated,
        Some(TerminationRecord {
            actor_id: child.get_id(),
            state_was_unit: true,
            reason: None,
        })
    );
    assert_eq!(
        snapshot.failed.as_ref().map(|failure| failure.actor_id),
        Some(failing_child.get_id())
    );
    assert!(snapshot
        .failed
        .as_ref()
        .is_some_and(|failure| failure.error.contains("intentional child failure")));
    assert_eq!(snapshot.value, 42);
    assert_eq!(supervisor.get_status(), ActorStatus::Running);

    supervisor
        .send_message(MacroSupervisorMessage::Stop)
        .expect("generated supervisor stop message failed");
    supervisor_handle
        .await
        .expect("generated supervisor failed to stop");
}

#[ractor::concurrency::test]
async fn unhandled_supervision_events_keep_the_default_shutdown_behavior() {
    let (supervisor, supervisor_handle) = Actor::spawn(None, DefaultingSupervisor, ())
        .await
        .expect("defaulting supervisor failed to start");
    let (child, child_handle) = Actor::spawn_linked(None, RawActor, (), supervisor.get_cell())
        .await
        .expect("linked child failed to start");

    supervisor
        .send_message(DefaultingSupervisorMessage::Ping)
        .expect("defaulting supervisor ping failed");
    child.stop(None);
    child_handle.await.expect("linked child failed to stop");
    supervisor_handle
        .await
        .expect("defaulting supervisor failed to stop");
    assert_eq!(supervisor.get_status(), ActorStatus::Stopped);
}

#[ractor::concurrency::test]
async fn supervision_only_actor_inherits_the_default_message_handler() {
    let (events, mut received_events) = ractor::concurrency::mpsc_unbounded();
    let (supervisor, supervisor_handle) = Actor::spawn(None, SupervisionOnlyActor { events }, ())
        .await
        .expect("supervision-only actor failed to start");

    supervisor
        .send_message(())
        .expect("unit message failed to send to supervision-only actor");

    let (child, child_handle) = Actor::spawn_linked(None, RawActor, (), supervisor.get_cell())
        .await
        .expect("linked child failed to start");
    let started = ractor::concurrency::timeout(
        ractor::concurrency::Duration::from_secs(1),
        received_events.recv(),
    )
    .await
    .expect("timed out waiting for child-started event")
    .expect("supervision event channel closed");
    assert_eq!(started, "started");

    child.stop(None);
    child_handle.await.expect("linked child failed to stop");
    let terminated = ractor::concurrency::timeout(
        ractor::concurrency::Duration::from_secs(1),
        received_events.recv(),
    )
    .await
    .expect("timed out waiting for child-terminated event")
    .expect("supervision event channel closed");
    assert_eq!(terminated, "terminated");

    supervisor
        .send_message(())
        .expect("second unit message failed to send");
    ractor::concurrency::sleep(ractor::concurrency::Duration::from_millis(10)).await;
    assert_eq!(supervisor.get_status(), ActorStatus::Running);

    supervisor.stop(None);
    supervisor_handle
        .await
        .expect("supervision-only actor failed to stop cleanly");
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

#[test]
fn actors_without_dispatch_methods_inherit_trait_defaults() {
    fn assert_actor<T: Actor>() {}
    fn assert_generated_actor<T: Actor<Msg = GeneratedEmptyMessage>>() {}
    fn assert_thread_local<T: ractor::thread_local::ThreadLocalActor<Msg = ()>>() {}

    assert_actor::<SupervisionOnlyActor>();
    assert_generated_actor::<GeneratedEmptyActor>();
    assert_thread_local::<LocalDefaultHandleActor>();
}

#[cfg(all(not(feature = "async-trait"), not(feature = "cluster")))]
#[test]
fn actor_macro_diagnostics_are_actionable() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/actor_macros/*.rs");
}
