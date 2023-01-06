// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! `ractor`: A pure-Rust actor framework. Inspired from [Erlang's `gen_server`](https://www.erlang.org/doc/man/gen_server.html),
//! with the speed + performance of Rust!
//!
//! ## Installation
//!
//! Install `ractor` by adding the following to your Cargo.toml dependencies
//!
//! ```toml
//! [dependencies]
//! ractor = "0.1"
//! ```
//!
//! ## Getting started
//!
//! An example "ping-pong" actor might be the following
//!
//! ```rust
//! use ractor::{Actor, ActorCell, ActorHandler};
//!
//! /// [PingPong] is a basic actor that will print
//! /// ping..pong.. repeatedly until some exit
//! /// condition is met (a counter hits 10). Then
//! /// it will exit
//! pub struct PingPong;
//!
//! /// This is the types of message [PingPong] supports
//! #[derive(Debug, Clone)]
//! pub enum Message {
//!     Ping,
//!     Pong,
//! }
//!
//! impl Message {
//!     // retrieve the next message in the sequence
//!     fn next(&self) -> Self {
//!         match self {
//!             Self::Ping => Self::Pong,
//!             Self::Pong => Self::Ping,
//!         }
//!     }
//!     // print out this message
//!     fn print(&self) {
//!         match self {
//!             Self::Ping => print!("ping.."),
//!             Self::Pong => print!("pong.."),
//!         }
//!     }
//! }
//!
//! // the implementation of our actor's "logic"
//! #[async_trait::async_trait]
//! impl ActorHandler for PingPong {
//!     // An actor has a message type
//!     type Msg = Message;
//!     // and (optionally) internal state
//!     type State = u8;
//!
//!     // Initially we need to create our state, and potentially
//!     // start some internal processing (by posting a message for
//!     // example)
//!     async fn pre_start(&self, myself: ActorCell) -> Self::State {
//!         // startup the event processing
//!         self.send_message(myself, Message::Ping).unwrap();
//!         0u8
//!     }
//!
//!     // This is our main message handler
//!     async fn handle(
//!         &self,
//!         myself: ActorCell,
//!         message: Self::Msg,
//!         state: &Self::State,
//!     ) -> Option<Self::State> {
//!         if *state < 10u8 {
//!             message.print();
//!             self.send_message(myself, message.next()).unwrap();
//!             Some(*state + 1)
//!         } else {
//!             myself.stop();
//!             // don't send another message, rather stop the agent after 10 iterations
//!             None
//!         }
//!     }
//! }
//!
//! async fn run() {
//!     let (_, actor_handle) = Actor::spawn(None, PingPong).await.expect("Failed to start actor");
//!     actor_handle.await.expect("Actor failed to exit cleanly");
//! }
//! ```
//!
//! which will output
//!
//! ```bash
//! $ cargo run
//! ping..pong..ping..pong..ping..pong..ping..pong..ping..pong..
//! $
//! ```
//!
//! ## Supervision
//!
//! Actors in `ractor` also support supervision. This is done by "linking" actors together in a supervisor-child relationship.
//! A supervisor is "responsible" for the child actor, and as such is notified when the actor starts, stops, and fails (panics).
//!
//! Supervision is presently left to the implementor, but you can see a suite of supervision tests in `crate::actor::tests::supervisor`
//! for examples on the supported functionality.
//!
//! NOTE: panic's in `pre_start` of an actor will cause failures to spawn, rather than supervision notified failurs as the actor hasn't "linked"
//! to its supervisor yet. However failures in `post_start`, `handle`, `handle_supervisor_evt`, `post_stop` will notify the supervisor should a failure
//! occur. See [crate::ActorHandler] documentation for more information

#![deny(warnings)]
#![warn(unused_imports)]
#![warn(unsafe_code)]
#![warn(missing_docs)]
#![warn(unused_crate_dependencies)]
#![cfg_attr(docsrs, feature(doc_cfg))]

use std::sync::atomic::AtomicU64;

/// An actor's globally unique identifier
pub type ActorId = u64;
/// The global id allocator for actors
pub static ACTOR_ID_ALLOCATOR: AtomicU64 = AtomicU64::new(0u64);

pub mod actor;
pub mod port;
pub mod rpc;
pub mod time;

// re-exports
pub use actor::actor_cell::{ActorCell, ActorStatus, ACTIVE_STATES};
pub use actor::errors::{ActorErr, MessagingErr, SpawnErr};
pub use actor::messages::{Signal, SupervisionEvent};
pub use actor::{Actor, ActorHandler};
pub use port::{InputPort, RpcReplyPort};

/// Message type for an actor
pub trait Message: Send + 'static {}
impl<T: Send + 'static> Message for T {}

/// Represents the state of an actor
pub trait State: Clone + Sync + Send + 'static {}
impl<T: Clone + Sync + Send + 'static> State for T {}

/// Error types which can result from Ractor processes
#[derive(Debug)]
pub enum RactorErr {
    /// An error occurred spawning
    Spawn(SpawnErr),
    /// An error occurred in messaging (sending/receiving)
    Messaging(MessagingErr),
    /// An actor encountered an error while processing (canceled or panicked)
    Actor(ActorErr),
}

impl From<SpawnErr> for RactorErr {
    fn from(value: SpawnErr) -> Self {
        RactorErr::Spawn(value)
    }
}

impl From<MessagingErr> for RactorErr {
    fn from(value: MessagingErr) -> Self {
        RactorErr::Messaging(value)
    }
}

impl From<ActorErr> for RactorErr {
    fn from(value: ActorErr) -> Self {
        RactorErr::Actor(value)
    }
}

impl std::fmt::Display for RactorErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Actor(actor_err) => {
                write!(f, "{}", actor_err)
            }
            Self::Messaging(messaging_err) => {
                write!(f, "{}", messaging_err)
            }
            Self::Spawn(spawn_err) => {
                write!(f, "{}", spawn_err)
            }
        }
    }
}
