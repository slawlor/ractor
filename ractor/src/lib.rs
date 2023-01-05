// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! An actor framework for Rust

#![warn(missing_docs)]
#![warn(unused_crate_dependencies)]
#![cfg_attr(docsrs, feature(doc_cfg))]

use core::fmt::Debug;
use std::sync::atomic::AtomicU64;

/// An agent's identifier
pub type ActorId = u64;
/// The global id allocator for agents
pub static AGENT_ID_ALLOCATOR: AtomicU64 = AtomicU64::new(0u64);

pub mod actor;
pub mod port;

// re-exports
pub use actor::actor_cell::{ActorCell, ActorStatus};
pub use actor::errors::{ActorProcessingErr, SpawnErr};
pub use actor::messages::{Signal, SupervisionEvent};
pub use actor::{Actor, ActorHandler};

/// Message type for an agent
pub trait Message: Debug + Clone + Send + 'static {}
impl<T: Debug + Clone + Send + 'static> Message for T {}

/// An agent's state
pub trait State: Clone + Sync + Send + std::any::Any {}
impl<T: Clone + Sync + Send + std::any::Any> State for T {}
