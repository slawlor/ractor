# ractor

<p align="center">
    <img src="https://raw.githubusercontent.com/slawlor/ractor/main/docs/ractor_logo.svg" width="50%" /> 
</p>

*Pronounced ract-er*

A pure-Rust actor framework. Inspired from [Erlang's `gen_server`](https://www.erlang.org/doc/man/gen_server.html), with the speed + performance of Rust!

* [<img alt="github" src="https://img.shields.io/badge/github-slawlor/ractor-8da0cb?style=for-the-badge&labelColor=555555&logo=github" height="20">](https://github.com/slawlor/ractor)
* [<img alt="crates.io" src="https://img.shields.io/crates/v/ractor.svg?style=for-the-badge&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/ractor)
* [<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-ractor-66c2a5?style=for-the-badge&labelColor=555555&logo=docs.rs" height="20">](https://docs.rs/ractor)
* [<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-ractor_cluster-66c2a5?style=for-the-badge&labelColor=555555&logo=docs.rs" height="20">](https://docs.rs/ractor_cluster)
* [![CI/main](https://github.com/slawlor/ractor/actions/workflows/ci.yaml/badge.svg?branch=main)](https://github.com/slawlor/ractor/actions/workflows/ci.yaml)
* [![codecov](https://codecov.io/gh/slawlor/ractor/branch/main/graph/badge.svg?token=61AGYYPWBA)](https://codecov.io/gh/slawlor/ractor)
* `ractor`: ![ractor Downloads](https://img.shields.io/crates/d/ractor.svg)
* `ractor_cluster`: ![ractor_cluster Downloads](https://img.shields.io/crates/d/ractor_cluster.svg)

**Website** Ractor has a companion website for more detailed getting-started guides along with some best practices and is updated regularly. Api docs will still be available at [docs.rs](https://docs.rs/ractor) however this will be a supplimentary site for `ractor`. Try it out! <https://slawlor.github.io/ractor/>

## About

`ractor` tries to solve the problem of building and maintaining an Erlang-like actor framework in Rust. It gives
a set of generic primitives and helps automate the supervision tree and management of our actors along with the traditional actor message processing logic. It's built *heavily* on `tokio` which is a
hard requirement for `ractor` (today).

`ractor` is a modern actor framework written in 100% Rust.

Additionally `ractor` has a companion library, `ractor_cluster` which is needed for `ractor` to be deployed in a distributed (cluster-like) scenario. `ractor_cluster` shouldn't be considered production ready, but it is relatively stable and we'd love your feedback!

### Why ractor?

There are other actor frameworks written in Rust ([Actix](https://github.com/actix/actix), [riker](https://github.com/riker-rs/riker), or [just actors in Tokio](https://ryhl.io/blog/actors-with-tokio/)) plus a bunch of others like this list compiled on [this Reddit post](https://www.reddit.com/r/rust/comments/n2cmvd/there_are_a_lot_of_actor_framework_projects_on/).

Ractor tries to be different by modelling more on a pure Erlang `gen_server`. This means that each actor can also simply be a supervisor to other actors with no additional cost (simply link them together!). Additionally we're aiming to maintain close logic with Erlang's patterns, as they work quite well and are well utilized in the industry.

Additionally we wrote `ractor` without building on some kind of "Runtime" or "System" which needs to be spawned. Actors can be run independently, in conjunction with other basic `tokio` runtimes with little additional overhead.

We currently have full support for:

1. Single-threaded message processing
2. Actor supervision tree
3. Remote procedure calls to actors in the `rpc` module
4. Timers in the `time` module
5. Named actor registry (`registry` module) from [Erlang's `Registered processes`](https://www.erlang.org/doc/reference_manual/processes.html)
6. Process groups (`ractor::pg` module) from [Erlang's `pg` module](https://www.erlang.org/doc/man/pg.html)

On our roadmap is to add more of the Erlang functionality including potentially a distributed actor cluster.

### Performance

Actors in `ractor` are generally quite lightweight and there are benchmarks which you are welcome to run on your own host system with:

```bash
cargo bench -p ractor
```

## Installation

Install `ractor` by adding the following to your Cargo.toml dependencies.

```toml
[dependencies]
ractor = "0.8"
```

The minimum supported Rust version (MSRV) of `ractor` is `1.64`

## Features

`ractor` exposes a single feature currently, namely:

1. `cluster`, which exposes various functionality required for `ractor_cluster` to set up and manage a cluster of actors over a network link. This is work-in-progress and is being tracked in [#16](https://github.com/slawlor/ractor/issues/16).

## Working with Actors

Actors in `ractor` are very lightweight and can be treated as thread-safe. Each actor will only call one of its handler functions at a time, and they will
never be executed in parallel. Following the actor model leads to microservices with well-defined state and processing logic.

An example `ping-pong` actor might be the following

```rust
use ractor::{async_trait, cast, Actor, ActorProcessingErr, ActorRef};

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
#[async_trait]
impl Actor for PingPong {
    // An actor has a message type
    type Msg = Message;
    // and (optionally) internal state
    type State = u8;
    // Startup initialization args
    type Arguments = ();

    // Initially we need to create our state, and potentially
    // start some internal processing (by posting a message for
    // example)
    async fn pre_start(
        &self,
        myself: ActorRef<Self::Msg>,
        _: (),
    ) -> Result<Self::State, ActorProcessingErr> {
        // startup the event processing
        cast!(myself, Message::Ping)?;
        // create the initial state
        Ok(0u8)
    }

    // This is our main message handler
    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        if *state < 10u8 {
            message.print();
            cast!(myself, message.next())?;
            *state += 1;
        } else {
            println!();
            myself.stop(None);
            // don't send another message, rather stop the agent after 10 iterations
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let (_actor, handle) = Actor::spawn(None, PingPong, ())
        .await
        .expect("Failed to start ping-pong actor");
    handle
        .await
        .expect("Ping-pong actor failed to exit properly");
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
work will complete, and on the next message processing iteration Stop will take priority over future supervision events or regular messages. It will **not** terminate
currently executing work, regardless of the provided reason.
3. SupervisionEvent: Supervision events are messages from child actors to their supervisors in the event of their startup, death, and/or unhandled panic. Supervision events
are how an actor's supervisor(s) are notified of events of their children and can handle lifetime events for them.
4. Messages: Regular, user-defined, messages are the last channel of communication to actors. They are the lowest priority of the 4 message types and denote general actor work. The first
3 messages types (signals, stop, supervision) are generally quiet unless it's a lifecycle event for the actor, but this channel is the "work" channel doing what your actor wants to do!

## Ractor in distributed clusters

Ractor actors can also be used to build a distributed pool of actors, similar to [Erlang's EPMD](https://www.erlang.org/doc/man/epmd.html) which manages inter-node connections + node naming. In our implementation, we have [`ractor_cluster`](https://crates.io/crates/ractor_cluster) in order to facilitate distributed `ractor` actors.

`ractor_cluster` has a single main type in it, namely the `NodeServer` which represents a host of a `node()` process. It additionally has some macros and a procedural macros to facilitate developer efficiency when building distributed actors. The `NodeServer` is responsible for

1. Managing all incoming and outgoing `NodeSession` actors which represent a remote node connected to this host.
2. Managing the `TcpListener` which hosts the server socket to accept incoming session requests.

The bulk of the logic for node interconnections however is held in the `NodeSession` which manages

1. The underlying TCP connection managing reading and writing to the stream.
2. The authentication between this node and the connection to the peer
3. Managing actor lifecycle for actors spawned on the remote system.
4. Transmitting all inter-actor messages between nodes.
5. Managing PG group synchronization

etc..

The `NodeSession` makes local actors available on a remote system by spawning `RemoteActor`s which are essentially untyped actors that only handle serialized messages, leaving message deserialization up to the originating system. It also keeps track of pending RPC requests, to match request to response upon reply. There are special extension points in `ractor` which are added to specifically support `RemoteActor`s that aren't generally meant to be used outside of the standard

```rust
Actor::spawn(Some("name".to_string()), MyActor).await
```

pattern.

### Designing remote-supported actors

**Note** not all actors are created equal. Actors need to support having their message types sent over the network link. This is done by overriding specific methods of the `ractor::Message` trait all messages need to support. Due to the lack of specialization support in Rust, if you choose to use `ractor_cluster` you'll need to derive the `ractor::Message` trait for **all** message types in your crate. However to support this, we have a few procedural macros to make this a more painless process

#### Deriving the basic Message trait for in-process only actors

Many actors are going to be local-only and have no need sending messages over the network link. This is the most basic scenario and in this case the default `ractor::Message` trait implementation is fine. You can derive it quickly with:

```rust
use ractor_cluster::RactorMessage;
use ractor::RpcReplyPort;

#[derive(RactorMessage)]
enum MyBasicMessageType {
    Cast1(String, u64),
    Call1(u8, i64, RpcReplyPort<Vec<String>>),
}
```

This will implement the default ```ractor::Message``` trait for you without you having to write it out by hand.

#### Deriving the network serializable message trait for remote actors

If you want your actor to *support* remoting, then you should use a different derive statement, namely:

```rust
use ractor_cluster::RactorClusterMessage;
use ractor::RpcReplyPort;

#[derive(RactorClusterMessage)]
enum MyBasicMessageType {
    Cast1(String, u64),
    #[rpc]
    Call1(u8, i64, RpcReplyPort<Vec<String>>),
}
```

which adds a significant amount of underlying boilerplate (take a look yourself with `cargo expand`!) for the implementation. But the short answer is, each enum variant needs to serialize to a byte array of arguments, a variant name, and if it's an RPC give a port that receives a byte array and de-serialize the reply back. Each of the types inside of either the arguments or reply type need to implement the ```ractor_cluster::BytesConvertable``` trait which just says this value can be written to a byte array and decoded from a byte array. If you're using `prost` for your message type definitions (protobuf), we have a macro to auto-implement this for your types.

```rust
ractor_cluster::derive_serialization_for_prost_type! {MyProtobufType}
```

Besides that, just write your actor as you would. The actor itself will live where you define it and will be capable of receiving messages sent over the network link from other clusters!

## Differences between an actor's "state" and `self`

Actors can (but don't need to!) have internal state. In order to facilitate this `ractor` gives implementors of the `Actor` trait the ability to define the state type for an actor. The actor's `pre_start` routine is what initializes and sets up this state. You can imagine doing things like

1. Opening a network socket + storing the `TcpListener` in the state
2. Setting up a database connection + authenticating to the DB
3. Initializing basic state variables (counters, stats, whatever)

Because of this and the possibility that some of these operations are fallible, `pre_start` captures panic's in the method during the initialization and returns them to the caller of `Actor::spawn`.

When designing `ractor`, we made the explicit decision to make a separate state type for an actor, rather than passing around a mutable `self` reference. The reason for this is that if we were to use a `&mut self` reference, creation + instantiation of the `Self` struct would be outside of the actor's specification (i.e. not in `pre_start`) and the safety it gives would be potentially lost, causing potential crashes in the caller when it maybe shouldn't.

Lastly is that we would need to change some of the ownership properties that `ractor` is currently based on to pass an owned `self` in each call, returning a `Self` reference which seems clunky in this context.

**In the current realization** an actor's `self` is passed as a read-only reference which shouldn't ideally contain state information, but could contain configuration / startup information if you want. However there is also `Arguments` to each `Actor` which allows passing owned values to the state of an actor. In an ideal world, all actor structs would be empty with no stored values.

## Contributors

The original authors of `ractor` are Sean Lawlor (@slawlor), Dillon George (@dillonrg), and Evan Au (@afterdusk). To learn more about contributing to `ractor` please see [CONTRIBUTING.md](https://github.com/slawlor/ractor/blob/main/CONTRIBUTING.md).

## License

This project is licensed under [MIT](https://github.com/slawlor/ractor/blob/main/LICENSE).
