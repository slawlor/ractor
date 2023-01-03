// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Actor error types

use std::fmt::Display;

/// Spawn errors starting an actor
#[derive(Debug)]
pub enum SpawnErr {
    /// Actor panic'd during startup
    StartupPanic(String),
}

impl Display for SpawnErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StartupPanic(panic_msg) => {
                write!(f, "Actor panicked during startup '{}'", panic_msg)
            }
        }
    }
}

/// Actor processing loop errors
#[derive(Debug)]
pub enum ActorProcessingErr {
    /// Actor had an internal panic
    Panic(String),
}

impl Display for ActorProcessingErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Panic(panic_msg) => {
                write!(f, "Actor panicked '{}'", panic_msg)
            }
        }
    }
}
