# Actor definition macros

The optional `actor-macros` feature removes the repetitive associated types and message dispatch
from an actor definition while leaving its behavior visible as ordinary Rust methods.

```toml
[dependencies]
ractor = { version = "0.16", features = ["actor-macros"] }
```

## Defining an actor

Apply `#[ractor::actor]` to an inherent implementation and mark each message handler with
`#[ractor::message]`:

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

    #[ractor::message(CounterMessage::Read(reply))]
    fn read(&self, reply: RpcReplyPort<i64>, state: &i64) {
        let _ = reply.send(*state);
    }
}
```

`message` is required. `state` and `arguments` default to `()`. The macro generates the
`Actor` implementation and an exhaustive `match` over the message enum. Rust therefore reports a
new or forgotten message variant at compile time. Renamed `ractor` dependencies are detected
automatically; `crate_path = some::path` is available when a re-export requires an explicit path.

Tuple, struct, and unit variants are supported. Bindings in the attribute pattern become handler
parameters with matching names in the same order; `_` can discard an unused field. Keep patterns
flat so that the method signature remains an immediate description of the message payload.

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
`#[ractor::message]` methods. Raw `handle` and generated handlers are intentionally mutually
exclusive, keeping the dispatch path unambiguous.

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
`RactorClusterMessage` for remotely serializable messages.

See the complete [`actor_macro` example](../ractor/examples/actor_macro.rs).
