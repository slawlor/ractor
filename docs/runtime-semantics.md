# Runtime semantics

This document describes the runtime semantics, guarantees, and important edge-cases for ractor. It is intended to be a concise, actionable reference for developers building on ractor or ractor_cluster.

## Core guarantees

- Single-message processing: An actor processes at most one message handler at a time. Handlers for a single actor are never executed in parallel.
- Message types: User message types must be `Send + 'static`. For clustered usage, messages must additionally implement the network serialization traits (`ractor::Message` via derive macros, and `ractor_cluster::BytesConvertable` for payloads).
- Pre-start initialization: Actor state is created in `pre_start`. `pre_start` is run at spawn time and may fail; failures are returned to the caller of `Actor::spawn`.
- Actor `self` vs state: `self` is read-only and intended for configuration. Mutable actor state belongs to the `State` type returned by `pre_start`.

## Message priority & ordering

Message delivery is multiplexed into four prioritized channels. Higher priorities are processed before lower ones:

1. Signals (highest) — e.g., `Signal::Kill`. Signals immediately interrupt processing and will terminate execution (see Signals section).
2. Stop — a graceful stop request. Ongoing asynchronous work is allowed to complete; on the actor's next processing iteration Stop takes priority over supervision events and user messages.
3. SupervisionEvent — lifecycle notifications from child actors (startup, exit, panic).
4. Messages (lowest) — user-defined messages (cast/call).

Ordering guarantees:

- Per-sender FIFO: Messages sent from a single sender to a single actor are delivered in the order they were sent (FIFO) for the user message channel when using the provided `ActorRef` APIs.
- Cross-sender ordering: No ordering guarantee between messages from different senders.
- Priority preemption: Higher-priority channel messages can preempt lower-priority messages in the queue. A Stop message will be processed before later-arriving user messages.
- Local vs remote: Local ordering guarantees apply for in-process messaging. On the network (ractor_cluster), ordering depends on the transport and node session behavior — ordering can be preserved per-connection but is not guaranteed across reconnections or multi-path routing.

## Signals vs Stop (behavioral differences)

- Signal (Kill)
  - Highest priority and immediate.
  - Terminates all work including currently executing async handlers; cancellation semantics are immediate and not graceful.
  - Used for forced termination or emergency shutdowns.
- Stop
  - Graceful termination: currently executing async work is allowed to finish.
  - On the next message processing iteration the Stop is processed and actor will transition to exiting.
  - Useful for normal shutdowns where cleanup should run.

Important: Stop does not interrupt running handlers. If you require immediate stop, send Kill.

## Supervision & failure handling

- Supervision events notify supervising actors of child lifecycle events (started, exited, spawned with error, panicked).
- Panic handling:
  - Panics inside `pre_start` are captured and returned to the spawner.
  - Panics during message processing are caught and surfaced in supervision events (unless the binary is built with `panic = "abort"`, which will terminate the process).
- Supervisors decide restart/stop policies by handling supervision events and using provided restart utilities.

## RPC semantics

- RPC calls use `RpcReplyPort` to correlate requests and responses.
- RPCs are matched by a reply port id; the origin node holds the awaiting port and the remote node serializes the reply payload.
- On network partitions or disconnected nodes, pending RPCs may time out or fail — the user should add timeouts and retry logic for robust behavior.

## Timers & scheduling

- Timers are implemented as scheduled events that enqueue messages to the actor mailbox.
- Timer delivery follows the same priority rules (timers are user messages unless otherwise specified).
- Timers are best-effort accurate — network latencies, scheduling, and runtime load affect delivery time.

## Blocking and long-running work

- Actor handlers are asynchronous; long synchronous blocking operations will block the actor and delay its mailbox processing.
- Best practice: perform blocking operations in offloaded tasks (spawn on runtime thread pool) and send results back to the actor as messages.

## Concurrency model & runtime integration

- Actors are runtime-agnostic (works with tokio and async-std). Internals use async primitives and some tokio sync types even when using async-std (see feature flags).
- Actors are cooperative: asynchronous handlers must yield to allow other tasks and timers to run.
- Actors may be scheduled across threads by the async runtime, but handler execution remains single-threaded per actor.

## Mailbox semantics

- Mailboxes are bounded/unbounded depending on factory/configuration. Check the factory settings used to spawn workers/actors.
- Backpressure: when using bounded queues, attempts to send when full may fail or be delayed depending on the chosen factory/queue policy (discard, backpressure, etc.).

## ractor_cluster (remote) specifics

- Remote actors are represented by `RemoteActor` wrappers that contain only serialized payloads. Deserialization occurs on the owning node.
- `NodeSession` manages transport, authentication, and pending RPC mapping.
- Network behavior:
  - Ordering: per-connection ordering is generally preserved, but there is no distributed global ordering guarantee.
  - Reconnects: reconnection semantics are best-effort and may result in message loss unless application-level retries/acknowledgement are implemented.
  - Authentication & encryption: the cluster supports auth and TLS; these features are considered experimental and require production hardening.
- Serialization: cluster messages must implement `BytesConvertable` (or be produced via procedural macros for prost types). Versioning of serialized formats should be considered for rolling upgrades.

## Observability & diagnostics

- Insert tracing and metrics at:
  - Mailbox enqueue/dequeue
  - NodeSession read/write loops
  - Supervision events and restarts
- For debugging actor lifecycles, enable debug logs and use supervision events to trace restarts and failures.

## Safety gotchas & recommendations

- Do not rely on synchronous blocking inside handlers — offload blocking work.
- Do not assume RPC or remote message delivery is reliable; always add timeouts and retries.
- Test supervision logic under stress (panics, rapid spawn/kill cycles).
- When using cluster features, add tests for auth, reconnection, and partition behavior.

## Summary of best practices

- Keep actors focused and small; offload heavy work.
- Use Stop for graceful shutdowns and Kill for forced termination.
- Make RPCs resilient: set timeouts and retries.
- For production clusters: implement message-level acknowledgements or idempotency when required, and add monitoring/metrics on NodeSession and pending RPC queues.
