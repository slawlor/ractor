# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

Ractor is a pure-Rust actor framework inspired by Erlang's `gen_server`, providing lightweight actors with single-threaded message processing, supervision trees, and optional distributed clustering capabilities.

## Workspace Structure

This is a Cargo workspace with multiple crates:
- **ractor**: Core actor framework library
- **ractor_cluster**: Distributed cluster support (experimental)
- **ractor_cluster_derive**: Procedural macros for cluster message serialization
- **ractor_cluster_integration_tests**: Integration tests for clustering
- **ractor_example_entry_proc**: Example entry point procedures
- **ractor_playground**: Playground/experimentation crate
- **xtask**: Build/development automation tasks

## Build and Test Commands

### Running Tests

```bash
# Run all tests
cargo test

# Run tests for a specific package
cargo test --package ractor
cargo test --package ractor_cluster

# Run tests with specific feature flags
cargo test --package ractor --features cluster
cargo test --package ractor --features async-std,message_span_propogation --no-default-features
cargo test --package ractor --features monitors
cargo test --package ractor --features blanket_serde

# Run a specific test
cargo test --package ractor --test <test_name>
cargo test --package ractor <test_name>
```

### Running Benchmarks

```bash
# Run benchmarks
cargo bench -p ractor

# Compile benchmarks without running
cargo bench --no-run
```

### Linting and Formatting

```bash
# Run clippy
cargo clippy --all -- -D clippy::all -D warnings

# Run rustfmt
cargo fmt --all

# Check formatting without modifying
cargo fmt --all -- --check
```

### Documentation

```bash
# Build documentation
cargo doc --lib

# Build documentation in release mode
cargo doc --lib -r
```

## Architecture

### Core Actor Model

Ractor implements the actor model with these key components:

1. **Actor**: The trait defining actor behavior (`Actor` trait for `Send` actors, `ThreadLocalActor` for `!Send` actors)
   - `pre_start`: Initialize actor state (fallible, errors returned to spawner)
   - `post_start`: Post-initialization hook
   - `handle`: Process incoming messages
   - `handle_supervisor_evt`: Handle supervision events from children
   - `post_stop`: Cleanup on shutdown

2. **ActorCell**: Reference-counted primitive representing an actor's communication channels
   - Contains message ports (signal, stop, supervision, message)
   - Manages actor lifecycle status (Unstarted, Starting, Running, Upgrading, Draining, Stopping, Stopped)

3. **ActorRef<TMessage>**: Strongly-typed wrapper over ActorCell for type-safe communication
   - Primary interface for sending messages to actors
   - Derefs to ActorCell for access to lower-level operations

### Message Priority System

Messages are processed in strict priority order (see docs/runtime-semantics.md:12-19):

1. **Signals** (highest): Immediate termination (e.g., `Signal::Kill`)
2. **Stop**: Graceful shutdown request
3. **SupervisionEvent**: Child actor lifecycle notifications
4. **Messages** (lowest): User-defined work messages

### Actor State vs Self

- `self`: Read-only reference containing configuration/constants (ideally empty)
- `State`: Mutable state managed by the actor, initialized in `pre_start`

### Supervision

- Actors form supervision trees through linking
- Parent actors receive `SupervisionEvent` messages on child lifecycle changes
- Panics in message handlers are caught and reported to supervisors (unless `panic = "abort"`)
- Panics in `pre_start` return errors to the spawner without starting the actor

### Factory Pattern

The `factory` module provides worker pools for parallel job processing:

- **Worker**: Trait for defining job processing logic
- **Factory**: Manages a pool of workers with configurable routing
- **Routing modes**: KeyPersistent, StickyQueuer, Queuer, RoundRobin, Custom
- **Queueing**: DefaultQueue, PriorityQueue
- Factory handles worker lifecycle, automatically restarting failed workers

### Thread-Local Actors

The `thread_local` module supports `!Send` types:

- Must use Tokio runtime with `rt` feature (relies on `tokio::task::LocalSet`)
- Use `ThreadLocalActor` trait instead of `Actor`
- Spawn with `ractor::spawn_local()` and `ThreadLocalActorSpawner`
- State does not need to be `Send` or `Sync`

## Feature Flags

### ractor crate features:
- `cluster`: Exposes functionality for `ractor_cluster` (network-distributed actors)
- `async-std`: Use async-std runtime instead of tokio (tokio sync primitives still used)
- `monitors`: Erlang-style monitoring API (alternative to direct linking)
- `message_span_propogation`: Propagate tracing spans through actor messages
- `tokio_runtime`: Tokio runtime support (default)
- `blanket_serde`: Serde serialization support
- `output-port-v2`: Experimental output port v2
- `async-trait`: Use async-trait crate instead of native async traits

Default features: `tokio_runtime`, `message_span_propogation`

## Cluster Support (Experimental)

### Distributed Actors (ractor_cluster)

- **NodeServer**: Manages node connections and incoming sessions
- **NodeSession**: Handles TCP connections, authentication, message routing, and remote actor lifecycle
- **RemoteActor**: Untyped actor handling serialized messages from remote nodes

### Message Serialization

For cluster-enabled actors:

1. Use `#[derive(RactorMessage)]` for local-only actors
2. Use `#[derive(RactorClusterMessage)]` for remote-capable actors
3. Mark RPC messages with `#[rpc]` attribute
4. All message fields must implement `ractor_cluster::BytesConvertable`
5. For protobuf types: `ractor_cluster::derive_serialization_for_prost_type! {MyProtobufType}`

## Common Patterns

### Spawning Actors

```rust
// Standard Send actor
let (actor_ref, join_handle) = Actor::spawn(
    Some("actor_name".to_string()),
    MyActor,
    startup_args,
).await?;

// Thread-local (!Send) actor
let spawner = ThreadLocalActorSpawner::new();
let (actor_ref, join_handle) = ractor::spawn_local::<MyActor>(
    startup_args,
    spawner,
).await?;
```

### Sending Messages

```rust
use ractor::{cast, call};

// Fire-and-forget
cast!(actor_ref, MyMessage::DoSomething)?;

// RPC-style call
let result = call!(actor_ref, MyMessage::GetValue)?;
```

### Factory Pattern

```rust
use ractor::factory::*;

// Define worker
struct MyWorker;
impl Worker for MyWorker {
    type Key = String;
    type Message = MyMessage;
    type State = ();
    type Arguments = ();
    // ... implement pre_start, handle, etc.
}

// Build factory
let factory = Factory::new()
    .with_name("my_factory")
    .with_worker_count(10)
    .with_routing(routing::KeyPersistentRouting)
    .with_queue(queues::DefaultQueue)
    .spawn(MyWorker, ())
    .await?;
```

## Important Implementation Notes

### Runtime Requirements

- MSRV: Rust 1.64
- Native async fn in traits requires Rust >= 1.75 (otherwise use `async-trait` feature)
- Thread-local actors REQUIRE tokio runtime with `rt` feature
- WASM support available (target `wasm32-unknown-unknown`)

### Messaging Semantics

- Per-sender FIFO ordering guaranteed for user messages
- No cross-sender ordering guarantees
- Remote (cluster) messages may lose ordering on reconnection
- RPCs timeout/fail on network partitions—add application-level retries

### Best Practices

1. Keep actor handlers async and non-blocking—offload CPU-intensive work to thread pools
2. Use `Stop` for graceful shutdown, `Kill` for forced termination
3. Add timeouts and retries for all RPC calls
4. Actor `self` should ideally be empty—configuration belongs in `Arguments`, mutable state in `State`
5. Test supervision logic under failure scenarios (panics, rapid spawn/kill)
6. For distributed setups: implement message acknowledgements or idempotency where required

## Testing

### Test Organization

- Unit tests: `#[cfg(test)] mod tests` within each module
- Integration tests: `ractor_cluster_integration_tests/` crate
- Some tests require `serial_test` crate for sequential execution

### WASM Testing

See `unit-tests-for-wasm32-unknown-unknown.md` for WASM-specific testing procedures.

## Key Source Files

Critical files for understanding the architecture:

- `ractor/src/actor.rs`: Core Actor trait and spawn logic
- `ractor/src/actor/actor_cell.rs`: ActorCell and ActorPortSet (message multiplexing)
- `ractor/src/actor/actor_ref.rs`: Strongly-typed ActorRef wrapper
- `ractor/src/factory.rs`: Worker pool pattern
- `ractor/src/thread_local.rs`: Thread-local actor support
- `ractor/src/message.rs`: Message trait definition
- `ractor_cluster/src/node/`: NodeServer and NodeSession implementations
- `docs/runtime-semantics.md`: Detailed runtime guarantees and edge cases

## Additional Resources

- Website: https://slawlor.github.io/ractor/
- API docs: https://docs.rs/ractor
- Cluster docs: https://docs.rs/ractor_cluster
- Runtime semantics: See `docs/runtime-semantics.md` for message priority, ordering guarantees, RPC semantics, and cluster behavior
