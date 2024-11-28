---
permalink: /factories/
title: "Factories"
toc: true
layout: single
author_profile: false
---

## What are factories and why do they exist?

A single actor is effectively a single processing loop which can handle 1 message at a time. If it's internal logic takes some time (blocking or not), other messages are not processed and may queue up.

A factory is a collection of actions, with a common supervisor and request router. The actor children are called "workers". The notion of factories (and their higher-order organization, called "industries") were introduced by WhatsApp for Erlang [in this talk](https://www.youtube.com/watch?v=c12cYAUTXXs).

You can think of a factory as having the following traits

1. A common processing logic template (worker model). A factory can only have a single worker template.
2. Routing logic. How messages are dispatched to which actors
3. Queueing logic. When there are no more free workers, should messages queue, and if yes where in the pipeline (at the factory or worker level).
4. Supervision logic. Specifically functionality like "dead mans switch" where the factory can identify and kill stuck workers to free capacity back up.
5. Hooks for "lifecycle" events in the factory. This means you can insert logic for startup, shutdown, and draining requests in order to isolate your lifecycle to a safe FSM model. In other words, some dependency needs to be up before workers can serve traffic would be a use-case here.

A great overview, and an important use-case for Meta, was presented at the 2024 RustConf. This can be viewed [here](https://youtu.be/zQ6EyQJRxIs).

## How do factories work in `ractor`?

[docs](https://docs.rs/ractor/latest/ractor/factory/index.html)

Each factory uses generic arguments to template compile-time safe logic specific to your use-case. At a bare minimum you need to provide the worker template (`TWorker`), the queueing template (`TQueue`), and the routing (`TRouter`) template.

You then will have an arguments builder to pass all the instances necessary to start and setup your factory.

```rust
use ractor::factory::*;

let factory_def = Factory::<
    (),
    ExampleMessage,
    (),
    ExampleWorker,
    routing::QueuerRouting<(), ExampleMessage>,
    queues::DefaultQueue<(), ExampleMessage>
>::default();
let factory_args = FactoryArguments::builder()
    .worker_builder(Box::new(ExampleWorkerBuilder))
    .queue(Default::default())
    .router(Default::default())
    .num_initial_workers(5)
    .build();
```

The factory itself is an actor, so you just spawn it to start it! The factory will guarantee that all workers are started, and the lifecycle hooks have run any startup requirements prior to dispatching traffic to workers.

A full example is in the docs for ractor available [here](https://docs.rs/ractor/latest/ractor/factory/index.html#example-factory).

## What's provided out of the box?

There are base traits you are welcome to implement any routing and queueing logic yourself, however some pre-defined versions are available.

### Queueing protocols

[docs](https://docs.rs/ractor/latest/ractor/factory/queue/index.html)

```rust
mod ractor::factory::queue
```

1. Standard FIFO queue - `DefaultQueue`
2. Priority queue, with a compile-time defined number of priorities. - `PriorityQueue`

### Routing protocols

[docs](https://docs.rs/ractor/latest/ractor/factory/routing/index.html)

```rust
mod ractor::factory::routing
```

1. The hash of the job's "key" will select the destination worker. `KeyPersistentRouting`
2. First available worker dispatch - `QueuerRouting`
3. Next available worker, in order - `RoundRobinRouting`
4. Custom hash of the job key - `CustomRouting`
5. Sticky key routing, where if a worker is already processing a key, it'll receive subsequent keys of the same value. - `StickyQueuerRouting`

### Job discarding

[docs](https://docs.rs/ractor/latest/ractor/factory/discard/trait.DiscardHandler.html)

If the factory's defined workers and queue "overflow", or jobs time-out after sitting in queue too long (both configurable), the job may be "discarded". Discarding means that the job was NOT routed to a worker nor started processing. In this case you can provide a discard handler to define what should happen with such jobs (should the be ignored, logged, retried, etc).

```rust
ractor::factory::discard::DiscardHandler
```

Your callback will be provided with a "reason" and a mutable handle to the job. The reason can be either the job TTLd (timed out in queue), Loadshed (actively overflowed the factory's queue), or Shutdown (the factory is draining and new requests are rejected during shutdown flow).

### Statistics

[docs](https://docs.rs/ractor/latest/ractor/factory/stats/trait.FactoryStatsLayer.html)

You can provide a user-defined trait implementation in `ractor::factory::stats::FactoryStatsLayer` which can capture various statistics about the factory's processing. This is helpful for inspecting what your factory is doing. Callbacks will be triggered for you by the factory, at set intervals, or upon necessary events (i.e. job discarded).

### Lifecycle management

[docs](https://docs.rs/ractor/latest/ractor/factory/lifecycle/trait.FactoryLifecycleHooks.html)

As previously mentioned, there is support for connecting into the factory's lifecycle, and blocking the factory from processing other work, until a lifecycle stage is completed. This is controlled by

```rust
ractor::factory::lifecycle::FactoryLifecycleHooks
```

### Dead mans switch

[docs](https://docs.rs/ractor/latest/ractor/factory/worker/struct.DeadMansSwitchConfiguration.html)

If a worker gets "stuck" you might want to have the factory kill the worker (and the in-flight work), restart it (giving you your capacity back), and handle other jobs. This is possible with a `DeadMansSwitchConfiguration` on the factory. Basically you give an amount of time the worker needs to be "stuck" for and tell the factory if it should kill identified workers, if not kill, then a message is logged.

## Topics to come

The following are possible, but not yet documented. We plan to add more as time allows.

1. Worker setup
2. Writing your own router & queue
3. Dynamic factories (dynamic worker count, dynamic queue depth, etc). This is so you can scale the factory safely based on system load if you want.