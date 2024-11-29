---
permalink: /factories/
title: "Factories"
toc: true
layout: single
author_profile: false
---

## What are factories and why do they exist?

A single actor is effectively a single processing loop which can handle 1 message at a time. If it's internal logic takes some time (blocking or not), other messages are not processed and may queue up.

A factory is a collection of actors ("workers"), with a common supervisor and request routing actor (the "factory"). The notion of factories (and their higher-order organization, called "industries") were introduced by WhatsApp for Erlang OTP [in this talk](https://www.youtube.com/watch?v=c12cYAUTXXs).

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

### Factory requests

[docs](https://docs.rs/ractor/latest/ractor/factory/enum.FactoryMessage.html)

Requests to a factory have a specific structured, named `FactoryMessage`. It is the API with which to interface with the factory in all respects. Some are internal functions mainly intended for workers to communicate back to the factory, but most of the API is available externally.

```rust
pub enum FactoryMessage<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    Dispatch(Job<TKey, TMsg>),
    Finished(WorkerId, TKey),
    DoPings(Instant),
    Calculate,
    WorkerPong(WorkerId, Duration),
    IdentifyStuckWorkers,
    GetQueueDepth(RpcReplyPort<usize>),
    AdjustWorkerPool(usize),
    GetAvailableCapacity(RpcReplyPort<usize>),
    GetNumActiveWorkers(RpcReplyPort<usize>),
    DrainRequests,
}
```


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

## Worker Setup

In order to have a factory, you need to define a worker. A worker is wholely owned by the factory, and isn't exposed externally via the traditional API. This is because their lifecycle is completely managed by the factory, and they can be killed, terminated, restarted at any point for any reason.

If a worker fails internally (panic or unhandled error), it will exit, notify its supervisor (the factory) who is responsible for recreating a fresh worker at the same worker id and routing future messages to it.

The factory can only do a single operation at a time, which can either be routing a user message (`FactoryMessage`) or handlinging a `SupervisionEvent`, or logging metrics, etc. This means that it's imposible to have race conditions between message handling and worker management. This is an added safety benefit that even in high-throughput use-cases scales well.

### Worker Definition

A worker is just an actor, with predefined message and startup argument types. Any actor which follows this template can be made into a factory's worker. Each request to a factory is in the form of a `Job` which has a key and a message payload (along with some other fields). The key influences message routing, and the message is never cloned, just moved, so can be of a large payload size.

Each worker must have a message type of

```rust
pub enum WorkerMessage<TKey, TMsg>
where
    TKey: JobKey,
    TMsg: Message,
{
    FactoryPing(Instant),
    Dispatch(Job<TKey, TMsg>),
}
```

where the worker should ASAP reply to the factory's ping with a `FactoryMessage::WorkerPong` and this is used in timing and measurements.

Additionally after processing the job, the worker should reply to the factory with a `FactoryMessage::Finished` along with the job key, which helps the factory manage the processing state of the workers.

[docs](https://docs.rs/ractor/latest/ractor/factory/worker/enum.WorkerMessage.html)

### Worker startup

[docs](https://docs.rs/ractor/latest/ractor/factory/worker/struct.WorkerStartContext.html)

When a worker starts, it will receive a `WorkerStartContext` which contains a

1. Handle to the owning factory (`ActorRef<FactoryMessage<TKey, TJob>>`)
2. Id of the worker
3. A custom struct, which can be provided by the user, containing whatever data is necessary to construct the worker.

Workers instances and startup contexts are instantiated by a `WorkerBuidler` [docs](https://docs.rs/ractor/latest/ractor/factory/worker/trait.WorkerBuilder.html) which the factory utilizes to create workers when needed. The factory will assign the worker id and it should emit a worker instance along with the startup arguments needed. The factory will handle spawning the worker actor instance.

## Advanced Topics (WIP)

The following are more advanced topics related to factories. They aren't generally your common use-cases and outline extension points in the factory framework for more flexible usages.

1. Writing your own router & queue
2. Dynamic factories (dynamic worker count, dynamic queue depth, etc). This is so you can scale the factory safely based on system load if you want.