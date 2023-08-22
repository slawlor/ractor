---
permalink: /quickstart/
title: "Quick start"
toc: true
layout: single
author_profile: false
---

## Some notations to keep in mind

While working through this quickstart, a few notations we want to clarify for readers.

### Messaging actors

Since we're trying to model as best we can around [Erlang's practices](https://www.erlang.org/doc/man/gen_server.html#call-2), message sends in
Ractor can occur in 2 ways, first-and-forget and waiting on a reply. Their notations however follow the Erlang naming schemes of "cast" and "call"
respectively.

## Installation

Install `ractor` by adding the following to your Cargo.toml dependencies

```toml
[dependencies]
ractor = "0.8"
```

## Your first actor

We have to, of course, start with the iconic "Hello world" sample. We want to build an actor
that's going to print "Hello world" for every message sent to it. Let's begin by defining our
actor and filling in the necessary bits. We'll start with out message definition

```rust
pub enum MyFirstActorMessage {
    /// Print's hello world
    PrintHelloWorld,
}
```

Then we follow up with the most basic required actor definition

```rust
use ractor::{Actor, ActorRef, ActorProcessingErr};

pub struct MyFirstActor;

#[async_trait::async_trait]
impl Actor for MyFirstActor {
    type State = ();
    type Msg = MyFirstActorMessage;
    type Arguments = ();

    async fn pre_start(&self, _myself: ActorRef<Self::Msg>, _arguments: Self::Arguments)
        -> Result<Self::State, ActorProcessingErr> 
    {
        Ok(())
    }
}
```

Let's break down what we're doing here, firstly we need our actor's struct-type which we're calling `MyFirstActor`.
We are then defining our `Actor` behavior, which minimally needs to define three types

1. `State` - The "state" of the actor, for stateless actors this can be simply `()` denoting that the actor has no mutable state
2. `Msg` - The actor's message type.
3. `Arguments` - Startup arguments which are consumed by `pre_start` in order to construct initial state. This is helpful for say a
TCP actor which is spawned from a TCP listener actor. The listener needs to pass the owned stream to the new actor, and `Arguments` is
there to facilitate that so the other actor can properly build it's state without `clone()`ing structs with potential side effects.

Lastly we are defining the actor's startup routine in `pre_start` which emits the initial state of the actor upon success. Once this
is run, your actor is alive and healthy just waiting for messages to be received!

**Well that's all fine and dandy, but how is this going to print hello world?!** Well we haven't defined that bit yet, we need to
wire up a message handler. Let's do that!

```rust
#[async_trait::async_trait]
impl Actor for MyFirstActor {
    type State = ();
    type Msg = MyFirstActorMessage;
    type Arguments = ();

    async fn pre_start(&self, _myself: ActorRef<Self::Msg>, _arguments: Self::Arguments)
        -> Result<Self::State, ActorProcessingErr>
    {
        Ok(())
    }

    async fn handle(&self, _myself: ActorRef<Self::Msg>, message: Self::Msg, _state: &mut Self::State) 
        -> Result<(), ActorProcessingErr>
    {
        match message {
            MyFirstActorMessage::PrintHelloWorld => {
                println!("Hello world!");
            }
        }
        Ok(())
    }
}
```

Ok now that looks better! Here we've added the message handler `handle()` method which will be executed for every message received in
the queue.

## All together now

Let's wire it all up into a proper program now.

```rust
#[tokio::main]
async fn main() {
    // Build an ActorRef along with a JoinHandle which lives for the life of the 
    // actor. Most of the time we drop this handle, but it's handy in the 
    // main function to wait for clean actor shut-downs (all stop handlers will
    // have completed)
    let (actor, actor_handle) = Actor::spawn(None, MyFirstActor, ()).await.expect("Actor failed to start");
    
    for i in 1..10 {
        // Sends a message, with no reply
        actor.cast(MyFirstActorMessage::PrintHelloWorld).expect("Failed to send message to actor");
    }

    // give a little time to print out all the messages
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Cleanup
    actor.stop(None);
    actor_handle.await.unwrap();
}
```

## Adding State

Now what if we wanted to ask the actor for some information? Like the number of hello-worlds that it has printed thus far
in its lifecycle, let's see what that might look like.

```rust
use ractor::{Actor, ActorRef, ActorProcessingErr, RpcReplyPort};

pub enum MyFirstActorMessage {
    /// Print's hello world
    PrintHelloWorld,
    /// Replies with how many hello worlds have occurred
    HowManyHelloWorlds(RpcReplyPort<u16>),
}

pub struct MyFirstActor;

#[async_trait::async_trait]
impl Actor for MyFirstActor {
    type State = u16;
    type Msg = MyFirstActorMessage;
    type Arguments = ();

    async fn pre_start(&self, _myself: ActorRef<Self::Msg>, _arguments: Self::Arguments)
        -> Result<Self::State, ActorProcessingErr>
    {
        Ok(0)
    }

    async fn handle(&self, _myself: ActorRef<Self::Msg>, message: Self::Msg, state: &mut Self::State) 
        -> Result<(), ActorProcessingErr>
    {
        match message {
            MyFirstActorMessage::PrintHelloWorld => {
                println!("Hello world!");
                *state += 1;
            }
            MyFirstActorMessage::HowManyHelloWorlds(reply) => {
                if reply.send(*state).is_err() {
                    println!("Listener dropped their port before we could reply");
                }
            }
        }
        Ok(())
    }
}
```

There's a bit to unpack here, so let's start with the basics.

1. We changed the type of the `Actor::State` to be a `u16` so that the actor could maintain some internal state which is the count of the number of times it's printed "Hello world"
2. We changed the hello-world message handling to increment the state every time it prints
3. We added a new message type `MyFirstActorMessage::HowManyHelloWorlds` which has an argument of type `RpcReplyPort`. This is one of the primary ways actors can inter-communicate, via remote procedure calls. This call is a message which provides the response channel (the "port") as an argument, so the receiver doesn't need to know who asked. We'll look at how we construct this in a bit
4. We added a hander match arm for this message type, which sends the reply back when requested.

### Running a stateful sample

Very similar to the non-stateful example, we'll wire it up as such!

```rust
#[tokio::main]
async fn main() {
    // Build an ActorRef along with a JoinHandle which lives for the life of the 
    // actor. Most of the time we drop this handle, but it's handy in the 
    // main function to wait for clean actor shut-downs (all stop handlers will
    // have completed)
    let (actor, actor_handle) = 
        Actor::spawn(None, MyFirstActor, ())
            .await
            .expect("Actor failed to start");
    
    for i in 1..10 {
        // Sends a message, with no reply
        actor.cast(MyFirstActorMessage::PrintHelloWorld)
            .expect("Failed to send message to actor");
    }

    let hello_world_count = 
        ractor::call_t!(actor, MyFirstActorMessage::HowManyHelloWorlds, 100)
        .expect("RPC failed");
    
    println!("Actor replied with {} hello worlds!", hello_world_count);

    // Cleanup
    actor.stop(None);
    actor_handle.await.unwrap();
}
```

**WHOA** what is `call_t!`?! That's a handy macro which constructs our RPC call for us! There's are three macro variants to ease development use for actor messaging

1. `cast!` - alias of `actor.cast(MESG)`, simply send a message to the actor non-blocking
2. `call!` - alias of `actor.call(|reply| MESG(reply))` which builds our message for us without having to provide a lambda function to take the reply port as an argument to construct the message type. We don't need to actually build & wait on the port, the RPC functionality will do that for us.
3. `call_t!` - Same as `call!` but with a timeout argument

Checkout [docs.rs on RPCs](https://docs.rs/ractor/latest/ractor/macro.call.html) for more detailed information on these macros.

In this brief example, we're having our actor send our 10 messages, and then sending a final query message to read
the current count and print it. We're additionally giving it 100ms to execute (hence the use of `call_t!`) or return
a timeout result.
