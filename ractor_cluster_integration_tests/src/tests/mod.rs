// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Different test scenarios are defined here

use clap::Parser;
use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};

pub mod auth_handshake;
pub mod encryption;
pub mod pg_groups;

fn random_name() -> String {
    thread_rng()
        .sample_iter(&Alphanumeric)
        .take(30)
        .map(char::from)
        .collect()
}

#[derive(Parser, Debug, Clone)]
pub enum TestCase {
    /// Test auth handshake
    AuthHandshake(auth_handshake::AuthHandshakeConfig),
    /// Test pg groups through a ractor cluster
    PgGroups(pg_groups::PgGroupsConfig),
    /// Test encrypted socket communications (through the auth handshake)
    Encryption(encryption::EncryptionConfig),
}
