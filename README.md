# ractor

A pure-Rust actor framework. Inspired from [Erlang's `gen_server`](https://www.erlang.org/doc/man/gen_server.html), with the speed + performance of Rust!

## About

`ractor` tries to solve the problem of building and maintaing an Erlang-like actor framework in Rust. It gives
a set of generic primitives and helps automate the supervision tree and management of our actors along with the traditional actor message processing logic. It's built *heavily* on `tokio` which is a
hard requirement for `ractor`. 

`ractor` is a modern actor framework written in 100% rust with NO `unsafe` code. 

## Installation

Install `ractor` by adding the following to your Cargo.toml dependencies

```toml
[dependencies]
ractor = "0.1"
```

## Working with Actors

Actors in `ractor` are very lightweight and can be treated as thread-safe. Each actor will only call one of it's handler functions at a time, and they will
never be executed in parallel. Following the actor model leads to microservices with well-defined state and processing logic.

An example `ping-pong` actor might be the following

```rust
use ractor::{Actor, ActorCell, ActorHandler};

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
impl ActorHandler for PingPong {
    // An actor has a message type
    type Msg = Message;
    // and (optionally) internal state
    type State = u8;

    // Initially we need to create our state, and potentially
    // start some internal processing (by posting a message for
    // example)
    async fn pre_start(&self, myself: ActorCell) -> Self::State {
        // startup the event processing
        self.send_message(myself, Message::Ping).unwrap();
        0u8
    }

    // This is our main message handler
    async fn handle(
        &self,
        myself: ActorCell,
        message: Self::Msg,
        state: &Self::State,
    ) -> Option<Self::State> {
        if *state < 10u8 {
            message.print();
            self.send_message(myself, message.next()).unwrap();
            Some(*state + 1)
        } else {
            myself.stop();
            // don't send another message, rather stop the agent after 10 iterations
            None
        }
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

## Contributors

The original authors of `ractor` are Sean Lawlor (@slawlor), Dillon George (@dillonrg), and Evan Au (@afterdusk). To learn more about contributing to `ractor` please see [CONTRIBUTING.md](https://github.com/slawlor/ractor/blob/main/CONTRIBUTING.md)

## License

This project is licensed under [MIT](https://github.com/slawlor/ractor/blob/main/LICENSE).