# Actor definition macros

The optional `actor-macros` feature removes the repetitive associated types and message dispatch
from an actor definition while leaving its behavior visible as ordinary Rust methods.

```toml
[dependencies]
ractor = { version = "0.17", features = ["actor-macros"] }
```

## Defining an actor

Apply `#[ractor::actor]` to an inherent implementation and mark each message handler with
`#[ractor::message]`. The usual form uses an explicitly declared message enum:

```rust
use ractor::{ActorProcessingErr, ActorRef, RpcReplyPort};

struct Counter;

enum CounterMessage {
    Add(i64),
    Read(RpcReplyPort<i64>),
}

#[ractor::actor(
    message = CounterMessage,
    state = i64,
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
    fn add(&self, amount: i64, state: &mut i64) {
        *state += amount;
    }

    #[ractor::rpc(CounterMessage::Read(reply))]
    fn read(&self, state: &i64) -> i64 {
        *state
    }
}
```

Either `message` or the generated-message form described below is required. `state` and
`arguments` default to `()`. The macro generates the
`Actor` implementation and, when focused message or RPC handlers are present, an exhaustive
`match` over the message enum. Rust therefore reports a new or forgotten message variant at
compile time in focused-dispatch mode. Cargo dependency renames are detected automatically, and
non-Cargo builds default to `::ractor`. The `crate_path = some::path` option is only necessary when
a non-Cargo build renames the dependency.

Tuple, struct, and unit variants are supported. Bindings in the attribute pattern become handler
parameters with matching names in the same order; `_` can discard an unused field. Keep patterns
flat so that the method signature remains an immediate description of the message payload.

## Generating local message enums

For local actors, `message = [visibility] enum EnumName` generates the message enum from the
handler patterns and parameter types:

```rust
struct Counter;

#[ractor::actor(message = pub enum CounterMessage, state = i64, arguments = i64)]
impl Counter {
    async fn pre_start(
        &self,
        _myself: ActorRef<CounterMessage>,
        initial: i64,
    ) -> Result<i64, ActorProcessingErr> {
        Ok(initial)
    }

    #[ractor::message(Add { amount })]
    fn add(&self, amount: i64, state: &mut i64) {
        *state += amount;
    }

    #[ractor::rpc(Read)]
    fn read(&self, state: &i64) -> i64 {
        *state
    }
}
```

This generates the equivalent of:

```rust
pub enum CounterMessage {
    Add { amount: i64 },
    Read(RpcReplyPort<i64>),
}
```

Unit, tuple, and struct variants are supported. Every generated field must have a named binding so
the macro can find its type; `_` fields are therefore only supported when using an explicit enum.
The optional visibility defaults to private and can be any normal Rust visibility, such as
`pub(crate)`. Generated enums currently do not support generic actor implementations.

The unit-form RPC attribute above describes a request with no payload. The generated enum still
contains the reply port, so `Read` becomes `Read(RpcReplyPort<i64>)`. A tuple or struct RPC pattern
can instead name the reply binding explicitly, as described below.

Generated-message mode deliberately does not add cluster serialization derives or define a wire
contract. Continue using an explicit message enum for remotely serializable actors. In a build with
the `cluster` feature, a generated enum used only for local messaging can opt into `Message` with a
manual implementation:

```rust
#[cfg(feature = "cluster")]
impl ractor::Message for CounterMessage {}
```

## Handler methods

A generated handler can be synchronous or asynchronous and has this shape:

```text
&self, [ActorRef<Message>], [bound message fields...], [&State | &mut State]
```

The actor reference and state parameters are optional. Use `&State` for read-only access or
`&mut State` to update state. A handler with no explicit return type (or `-> ()`) is infallible. A
handler with an explicit non-unit return type is propagated through `Actor::handle` with `?`, so it
must support that operator; `Result` is the usual choice.

Lifecycle hooks such as `pre_start`, `post_start`, `handle_supervisor_evt`, and `post_stop` can be
written with their normal trait signatures inside the same implementation. If both state and
arguments are `()`, an omitted `pre_start` defaults to `Ok(())`; otherwise it is required.

For actors that need custom dispatch, define the normal `handle` method instead of any
`#[ractor::message]` or `#[ractor::rpc]` methods. Raw `handle` and focused handlers are
intentionally mutually exclusive, keeping the dispatch path unambiguous.

An actor may also omit both forms. In that case the macro emits no `handle` method and inherits
the actor trait's no-op default. This is useful for actors that only run lifecycle hooks or process
focused `#[ractor::supervision(...)]` events. The `message` option is still required to select the
actor's message type; `message = enum EmptyMessage` produces an empty local enum when there are no
focused message or RPC handlers.

## RPC handlers

Use `#[ractor::rpc]` when a message contains an `RpcReplyPort` and the handler should return its
reply directly. Exactly one named binding from the pattern must be absent from the method
parameters. That binding is the reply port; all other bindings follow the same ordering and
optional actor-reference/state rules as one-way message handlers:

```rust
enum StoreMessage {
    Get {
        key: String,
        reply: RpcReplyPort<Option<String>>,
    },
    AddAndRead(i64, RpcReplyPort<i64>, i64),
}

#[ractor::rpc(StoreMessage::Get { key, reply })]
async fn get(&self, key: String, state: &StoreState) -> Option<String> {
    state.get(&key).cloned()
}

#[ractor::rpc(StoreMessage::AddAndRead(left, reply, right))]
fn add_and_read(&self, left: i64, right: i64, state: &StoreState) -> i64 {
    left + right + state.offset
}
```

The reply port may occur at any tuple or struct field position. A unit pattern such as
`#[ractor::rpc(StoreMessage::Status)]` is shorthand for an RPC with no request fields; the
explicit enum must still define `Status(RpcReplyPort<Reply>)`. For generated message enums, the
macro adds that field automatically.

The `call!` and `call_t!` convenience forms place the reply port after their supplied tuple
arguments. For a middle-position port or a struct variant, use `ActorRef::call` with a closure so
the message construction remains explicit:

```rust
let result = store
    .call(
        |reply| StoreMessage::AddAndRead(20, reply, 22),
        Some(ractor::concurrency::Duration::from_millis(100)),
    )
    .await?;

let value = store
    .call(
        |reply| StoreMessage::Get {
            key: "answer".to_owned(),
            reply,
        },
        Some(ractor::concurrency::Duration::from_millis(100)),
    )
    .await?;
```

By default, the handler's complete return type is the reply type. This is intentional: a domain
result such as `Result<Value, DomainError>` is sent to the caller unchanged. Reply types must be
concrete; `impl Trait` cannot appear within them because generated message fields and reply ports
must be able to name the type.

```rust
#[ractor::rpc(StoreMessage::Validate(reply))]
fn validate(&self, state: &StoreState) -> Result<Value, DomainError> {
    state.validate()
}
```

To propagate an actor-processing error from the generated `handle` method instead, specify the
successful reply type explicitly. The generated dispatch applies `?` to the handler call and only
sends the successful value:

```rust
#[ractor::rpc(StoreMessage::Read(reply), reply = Value)]
async fn read(&self, state: &StoreState) -> Result<Value, ActorProcessingErr> {
    state.read().await
}
```

Failure to send because the caller dropped its receiver is ignored and does not fail the actor.
Use an ordinary `#[ractor::message]` handler when code needs to inspect, retain, forward, or send
through the reply port manually.

## Supervision handlers

Supervision events can be split into focused methods in the same way as actor messages:

```rust
#[ractor::supervision(SupervisionEvent::ActorStarted(child))]
fn child_started(&self, child: ActorCell, state: &mut SupervisorState) {
    state.children.insert(child.get_id());
}

#[ractor::supervision(SupervisionEvent::ActorFailed(child, error))]
async fn child_failed(
    &self,
    myself: ActorRef<SupervisorMessage>,
    child: ActorCell,
    error: ActorProcessingErr,
    state: &mut SupervisorState,
) -> Result<(), ActorProcessingErr> {
    // Apply the supervisor's restart policy.
    Ok(())
}
```

Supervision handlers may be synchronous or asynchronous, may return a fallible result, and may
take an optional leading actor reference and trailing state reference. Events matched by a handler
are fully handled by that method. Unmatched events retain Ractor's default behavior, including
stopping the supervisor after an unhandled child termination or failure.

A raw `handle_supervisor_evt` method remains available for custom dispatch, but cannot be combined
with `#[ractor::supervision(...)]` methods in the same actor implementation.

## Thread-local and cluster actors

Add `thread_local` to generate a `ThreadLocalActor` implementation:

```rust
#[derive(Default)]
struct UiActor;

#[ractor::actor(thread_local, message = UiMessage, state = UiState)]
impl UiActor {
    // lifecycle hooks and #[ractor::message(...)] handlers
}
```

The macro only defines actor behavior. Cluster wire compatibility remains an explicit property of
the message type: continue deriving `RactorMessage` for local-only cluster builds or
`RactorClusterMessage` for remotely serializable messages. A focused actor
`#[ractor::rpc(...)]` handler does not generate or alter the enum's cluster `#[rpc]` metadata,
serialization, or wire contract; those remain on the explicit message enum.

See the complete [`actor_macro` example](../ractor/examples/actor_macro.rs).
