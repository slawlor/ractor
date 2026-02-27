# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Mandatory Pre-Completion Checks

**Before declaring any code change complete, you MUST run the following and resolve ALL errors and warnings:**

```bash
# Format all code (fix in place)
cargo fmt --all

# Run clippy with strict settings — ALL warnings must be resolved
cargo clippy --all -- -D clippy::all -D warnings
```

Do NOT skip these steps. Do NOT declare a task done if either command produces errors or warnings. Fix any issues and re-run until clean.

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

### CI Feature Flag Matrix

To reproduce a CI failure locally, find the matching row and run `cargo test --package <package> <flags>`:

| CI Job Name | Package | Flags |
|---|---|---|
| Run the default tests | ractor | *(none)* |
| Test ractor with async-trait | ractor | `-F async-trait` |
| Test ractor without span propogation | ractor | `--no-default-features -F tokio_runtime` |
| Test ractor with the `cluster` feature | ractor | `-F cluster` |
| Test ractor with the `blanket_serde` feature | ractor | `-F blanket_serde` |
| Test ractor with async-std runtime | ractor | `--no-default-features -F async-std,message_span_propogation` |
| Test ractor with async-std runtime but no span propagation | ractor | `--no-default-features -F async-std` |
| Test ractor with async-std runtime and async-trait | ractor | `--no-default-features -F async-std,async-trait` |
| Test ractor_cluster with native async traits | ractor_cluster | *(none)* |
| Test ractor_cluster with async_trait | ractor_cluster | `-F async-trait` |
| Test ractor with the monitor API | ractor | `-F monitors` |
| Test ractor with output-port-v2 feature | ractor | `-F output-port-v2` |
| Test ractor without output-port-v2 feature (explicit) | ractor | `--no-default-features -F tokio_runtime,message_span_propogation` |

CI also runs: `clippy`, `rustfmt --check`, `cargo doc --lib -r`, and `cargo bench --no-run`.

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

### Message Types

Without `cluster` feature: blanket `impl<T: Any + Send + Sized + 'static> Message for T` — any `Send + 'static` type works as a message with zero boilerplate.

With `cluster` feature: the blanket impl is replaced by one requiring `BytesConvertable`. For enum message types, use derive macros:
- `#[derive(RactorMessage)]` — local-only actors (marks as non-serializable)
- `#[derive(RactorClusterMessage)]` — remote-capable actors (generates serialization). Mark RPC variants with `#[rpc]`.

`RactorClusterMessage` supports:
- Both **tuple-style** (`Variant(A, B)`) and **struct-style** (`Variant { a: A, b: B }`) fields
- `RpcReplyPort<T>` at **any position** in `#[rpc]` variants (detected by type scan, not position)
- Exactly one `RpcReplyPort<T>` per `#[rpc]` variant (compile error on 0 or 2+)
- Using `RpcReplyPort` without `#[rpc]` produces a helpful compile error

The wire format is purely positional — field names and port position don't affect serialization. Internal codegen variables are `__`-prefixed to avoid collisions with user field names.

See `ractor/src/message.rs` for trait definition and `ractor_cluster_derive/` for the proc macros.

### Message Priority System

Messages are processed in strict priority order (see `docs/runtime-semantics.md`):

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

### DerivedActorRef

`DerivedActorRef<TFrom>` (`ractor/src/actor/derived_actor.rs`) wraps an `ActorCell` to accept a *subset* of an actor's message type via `From` conversion. This provides interface isolation — callers only see the subset type, not the full message enum. Obtain one via `actor_ref.get_derived::<SubsetType>()`. Requires `impl From<SubsetType> for FullMsg` and `impl TryFrom<FullMsg> for SubsetType`.

### Process Groups (pg)

`ractor::pg` (`ractor/src/pg.rs`) provides Erlang-style named process groups — global, scope-aware collections of actors.

- `pg::join(group, actors)` / `pg::leave(group, actors)` — add/remove actors (actors auto-leave on shutdown)
- `pg::get_members(group)` / `pg::get_local_members(group)` — retrieve group members
- `pg::which_groups()` / `pg::which_scopes()` — enumerate registered names
- `pg::monitor(group, actor)` / `pg::demonitor(group, actor_id)` — subscribe to `SupervisionEvent::ProcessGroupChanged`
- Scoped variants: `join_scoped`, `get_scoped_members`, `monitor_scope`, `demonitor_scope`

### Named Registry

`ractor::registry` (`ractor/src/registry.rs`) provides actor lookup by name.

- Actors with a `Some(name)` passed to `Actor::spawn` are auto-registered and auto-unregistered on drop
- `registry::where_is(name) -> Option<ActorCell>` — look up by name
- `registry::registered() -> Vec<ActorName>` — list all registered names
- With `cluster` feature: `pid_registry` sub-module adds `where_is_pid` and `PidLifecycleEvent`

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

- **NodeServer**: Manages node connections and incoming sessions
- **NodeSession**: Handles TCP connections, authentication, message routing, and remote actor lifecycle
- **RemoteActor**: Untyped actor handling serialized messages from remote nodes
- See `ractor_cluster/src/node/` for implementation and `ractor_cluster_derive/` for message serialization macros

## Important Implementation Notes

### Runtime Requirements

- MSRV: Rust 1.64
- Native async fn in traits requires Rust >= 1.75 (otherwise use `async-trait` feature)
- Thread-local actors REQUIRE tokio runtime with `rt` feature
- WASM support available (target `wasm32-unknown-unknown`)

## Testing

### Test Organization

- Unit tests: `#[cfg(test)] mod tests` within each module
- Integration tests: `ractor_cluster_integration_tests/` crate
- Some tests require `serial_test` crate for sequential execution

### Test Utilities

`ractor/src/common_test.rs` provides helpers used throughout the test suite:

- `periodic_check(closure, timeout)` — polls a `Fn() -> bool` every 50ms until true or timeout (then asserts)
- `periodic_async_check(closure, timeout)` — same but for `async Fn() -> Future<Output = bool>`

## Key Source Files

Critical files for understanding the architecture:

- `ractor/src/actor.rs`: Core Actor trait and spawn logic
- `ractor/src/actor/actor_cell.rs`: ActorCell and ActorPortSet (message multiplexing)
- `ractor/src/actor/actor_ref.rs`: Strongly-typed ActorRef wrapper
- `ractor/src/actor/derived_actor.rs`: DerivedActorRef for interface isolation
- `ractor/src/message.rs`: Message trait definition and blanket impls
- `ractor/src/concurrency.rs`: Runtime-abstracted channel/timer primitives (tokio vs async-std)
- `ractor/src/pg.rs`: Process groups (named actor groups)
- `ractor/src/registry.rs`: Named actor registry
- `ractor/src/factory.rs`: Worker pool pattern
- `ractor/src/thread_local.rs`: Thread-local actor support
- `ractor/src/common_test.rs`: Test utilities (`periodic_check`, `periodic_async_check`)
- `ractor_cluster/src/node/`: NodeServer and NodeSession implementations
- `ractor_cluster_derive/src/ir.rs`: Intermediate representation (ParsedEnum, ParsedVariant, FieldStyle, VariantKind)
- `ractor_cluster_derive/src/parse.rs`: Parsing and validation of derive input
- `ractor_cluster_derive/src/codegen.rs`: Code generation for `impl Message`
- `docs/runtime-semantics.md`: Detailed runtime guarantees and edge cases
