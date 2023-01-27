# ractor

*Pronounced **R**-aktor*

A pure-Rust actor framework. Inspired from [Erlang's `gen_server`](https://www.erlang.org/doc/man/gen_server.html), with the speed + performance of Rust!

[<img alt="github" src="https://img.shields.io/badge/github-slawlor/ractor-8da0cb?style=for-the-badge&labelColor=555555&logo=github" height="20">](https://github.com/slawlor/ractor)
[<img alt="crates.io" src="https://img.shields.io/crates/v/ractor.svg?style=for-the-badge&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/ractor)
[<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-ractor-66c2a5?style=for-the-badge&labelColor=555555&logo=docs.rs" height="20">](https://docs.rs/ractor)
[![CI/main](https://github.com/slawlor/repl/actions/workflows/ci.yaml/badge.svg?branch=main)](https://github.com/slawlor/repl/actions/workflows/ci.yaml)
[![codecov](https://codecov.io/gh/slawlor/ractor/branch/main/graph/badge.svg?token=61AGYYPWBA)](https://codecov.io/gh/slawlor/ractor)
![Downloads](https://img.shields.io/crates/d/ractor.svg)

## About

`ractor` tries to solve the problem of building and maintaing an Erlang-like actor framework in Rust. It gives
a set of generic primitives and helps automate the supervision tree and management of our actors along with the traditional actor message processing logic. It's built *heavily* on `tokio` which is a
hard requirement for `ractor`.

`ractor` is a modern actor framework written in 100% rust with NO `unsafe` code.

Additionally `ractor` has a companion library, `ractor_cluster` which is needed for `ractor` to be deployed in a distributed (cluster-like) scenario. `ractor_cluster` is not yet ready for public release, but is work-in-progress and coming shortly!

### Why ractor?

There are other actor frameworks written in Rust ([Actix](https://github.com/actix/actix), [riker](https://github.com/riker-rs/riker), or [just actors in Tokio](https://ryhl.io/blog/actors-with-tokio/)) plus a whole list compiled on [this Reddit post](https://www.reddit.com/r/rust/comments/n2cmvd/there_are_a_lot_of_actor_framework_projects_on/)

Ractor tries to be different my modelling more on a pure Erlang `gen_server`. This means that each actor can also simply be a supervisor to other actors with no additional cost (simply link them together!). Additionally we're aiming to maintain close logic with Erlang's patterns, as they work quite well and are well utilized in the industry.

Additionally we wrote `ractor` without building on some kind of "Runtime" or "System" which needs to be spawned. Actors can be run independently, in conjunction with other basic `tokio` runtimes with little additional overhead.

We currently have full support for:

1. Single-threaded message processing
2. Actor supervision tree
3. Remote procedure calls to actors
4. Timers
5. Named actor registry (`ractor::registry`) from [Erlang's `Registered processes`](https://www.erlang.org/doc/reference_manual/processes.html)
6. Process groups (`ractor::pg`) from [Erlang's `pg` module](https://www.erlang.org/doc/man/pg.html)

On our roadmap is to add more of the Erlang functionality including potentially a distributed actor cluster.

## Installation

Install `ractor` by adding the following to your Cargo.toml dependencies

```toml
[dependencies]
ractor = "0.4"
```

## Features

`ractor` exposes a single feature currently, namely

1. `cluster` which exposes various functionality required for `ractor_cluster` to setup and manage a cluster of actors over a network link. This is work-in-progress and is being tracked in [#16](https://github.com/slawlor/ractor/issues/16).

## Working with Actors

Actors in `ractor` are very lightweight and can be treated as thread-safe. Each actor will only call one of it's handler functions at a time, and they will
never be executed in parallel. Following the actor model leads to microservices with well-defined state and processing logic.

An example `ping-pong` actor might be the following

```rust
use ractor::{Actor, ActorCell, ActorProcessingErr};

/// [PingPong] is a basic actor that will print
/// ping..pong.. repeatedly until some exit
/// condition is met (a counter hits 10). Then
/// it will exit
pub struct PingPong;

/// This is the types of message [PingPong] supports
#[derive(Debug, Clone)]
pub enum Message {
    Ping,
    Pong,
}

impl Message {
    // retrieve the next message in the sequence
    fn next(&self) -> Self {
        match self {
            Self::Ping => Self::Pong,
            Self::Pong => Self::Ping,
        }
    }
    // print out this message
    fn print(&self) {
        match self {
            Self::Ping => print!("ping.."),
            Self::Pong => print!("pong.."),
        }
    }
}

// the implementation of our actor's "logic"
#[async_trait::async_trait]
impl Actor for PingPong {
    // An actor has a message type
    type Msg = Message;
    // and (optionally) internal state
    type State = u8;

    // Initially we need to create our state, and potentially
    // start some internal processing (by posting a message for
    // example)
    async fn pre_start(&self, myself: ActorCell) -> Result<Self::State, ActorProcessingErr> {
        // startup the event processing
        myself.cast(Message::Ping)?;
        Ok(0u8)
    }

    // This is our main message handler
    async fn handle(
        &self,
        myself: ActorCell,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        if *state < 10u8 {
            message.print();
            myself.cast(message.next())?;
            *state += 1;
        } else {
            myself.stop(None);
            // don't send another message, rather stop the agent after 10 iterations
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let (_, actor_handle) = Actor::spawn(None, PingPong).await.expect("Failed to start actor");
    actor_handle.await.expect("Actor failed to exit cleanly");
}
```

which will output

```bash
$ cargo run
ping..pong..ping..pong..ping..pong..ping..pong..ping..pong..
$ 
```

## Messaging actors

The means of communication between actors is that they pass messages to each other. A developer can define any message type which is `Send + 'static` and it
will be supported by `ractor`. There are 4 concurrent message types, which are listened to in priority. They are

1. Signals: Signals are the highest-priority of all and will interrupt the actor wherever processing currently is (this includes terminating async work). There
is only 1 signal today, which is `Signal::Kill`, and it immediately terminates all work. This includes message processing or supervision event processing.
2. Stop: There is also a pre-defined stop signal. You can give a "stop reason" if you want, but it's optional. Stop is a graceful exit, meaning currently executing async
work will complete, and on the next message processing iteration Stop will take prioritity over future supervision events or regular messages. It will **not** terminate
currently executing work, regardless of the provided reason.
3. SupervisionEvent: Supervision events are messages from child actors to their supervisors in the event of their startup, death, and/or unhandled panic. Supervision events
are how an actor's supervisor(s) are notified of events of their children and can handle lifetime events for them.
4. Messages: Regular, user-defined, messages are the last channel of communication to actors. They are the lowest priority of the 4 message types and denote general actor work. The first
3 messages types (signals, stop, supervision) are generally quiet unless it's a lifecycle event for the actor, but this channel is the "work" channel doing what your actor wants to do!

## Contributors

The original authors of `ractor` are Sean Lawlor (@slawlor), Dillon George (@dillonrg), and Evan Au (@afterdusk). To learn more about contributing to `ractor` please see [CONTRIBUTING.md](https://github.com/slawlor/ractor/blob/main/CONTRIBUTING.md)

## License

This project is licensed under [MIT](https://github.com/slawlor/ractor/blob/main/LICENSE).
