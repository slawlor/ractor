---
permalink: /faq/
title: "Frequently asked questions"
toc: true
layout: single
author_profile: false
---

## Should actors be long lived or short lived?

General guidance is that actors should be long lived. There's a historical reason here (Erlang patterns are typically long-lived `gen_server`s).
The other reason, internally to ractor, is there's non-trivial overhead spawning and creating the actor. Not only from the real actor spawn, creating
necessary dependent resources and actor ref's, but also from any overhead incurred in the actor's `pre_start` routine.

The tl;dr; here is spawning off an actor to do a single task is generally discouraged.

## Why is the actor's state separate from the actor's `self`?

This was an initial design choice when building `ractor`. The reason is somewhat complex, but it relates to creation of the state. For one thing, if the actor's state was just the actor's `self`, that would mean that the creation of that actor instance occurred somewhere else. This means that if during the initialization, it were to have an unhandled error or `panic!` it wouldn't be caught in that actor's startup routine.

Either that or there would need to be a lot of boiler-plate logic so that `self` is half-initialized elsewhere and then finished initialization in `pre_start` of the fallible bits. Therefore it was decided to utilize a separate state struct which is fully-owned by the actor runtime, and is only instantiated within the wrapped `pre_start` routine. This way unhandled `panic!`s and errors are handled and can be cleanly managed.

## Timers and periodic operations

In actor models, you generally don't want to block the actor waiting for a period to elapse. A simple periodic operation example you might do in normal async processing might be

```rust
async fn periodic() {
    loop {
        do_something().await;

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
```

If you did this in an actor's `handle()` function it would prevent the actor's message pump to process any other messages. In order to avoid this, we provide the `ractor::time` module which exposes actor-friendly timed operations.

You generally have 2 choice for timed operations with actors, a "delay" (`send_after`) or a periodic operation which will send a message at a set frequency (`send_interval`). To rebuild our example above in an actor, you might have

```rust

async fn handle(&self, myself: ActorRef<Self::Msg>, message: Self::Msg, state: &mut Self::State) -> Result<(), ActorProcessingErr> {
    match message {
        Self::Msg::DoSomething => {
            do_something().await;
        }
    }
    myself.send_after(Duration::from_millis(100), || Self::Message::DoSomething);
    Ok(())
}
```

In this model, the underlying framework will send a message to the actor after the period elapses so the actor's message handler does the necessary processing on a period.

## Factories: Managing the queue for an actor and parallel processing

What happens when you want the following behavior in an actor context?

1. A bounded queue, with backpressure support, for an actor
2. Parallel processing of messages (beyond a single sequential processor)

This is where you want to start looking at a [Factory](https://docs.rs/ractor/latest/ractor/factory/struct.Factory.html). The [documentation on the introduction to a factory](https://docs.rs/ractor/latest/ractor/factory/index.html) has a good overview for the motivation and use-cases for a factory, however the tl;dr is that a factory manages a pool of `worker`s which are discrete processing units. They are all identical for a given factory, and the factory is responsible for dispatching work and managing queueing.

The factory can be used with a single worker as well, which essentially gives you automated maintenance of the worker lifecycle along with queueing management.

Some related discussions/issues which might be relevant if you have a question!

1. Concurrent long-running async tasks: [Issue 133](https://github.com/slawlor/ractor/issues/133)
2. Why jobs aren't cloneable [Issue 91](https://github.com/slawlor/ractor/issues/91)
3. Performance discussions [Discussion 92](https://github.com/slawlor/ractor/discussions/92)

## More to come
