// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

mod distributed;
mod ping_pong;
pub mod proc_macros;

use std::env;

use clap::Parser;

#[derive(Parser, Debug)]
enum Cli {
    /// Run the ping-pong actor test
    PingPong,
    /// Run two dist nodes and try and have them connect together
    ClusterHandshake {
        /// The server port for the first node
        port_a: u16,
        /// The server port for the second node
        port_b: u16,
        /// Flag indicating if the cookies between the hosts match
        valid_cookies: Option<bool>,
    },

    /// Represents a "remote" cluster based ping-pong using paging groups
    RemotePingPong {
        /// The host port for this NodeServer
        port: u16,
        /// The remote server port to connect to (if Some)
        remote_port: Option<u16>,
    },
}

// MAIN //
#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let cli = Cli::parse();

    // if it's not set, set the log level to debug
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "debug");
    }
    env_logger::builder().format_timestamp_millis().init();

    // parse the CLI and run the correct playground scenario
    match cli {
        Cli::PingPong => {
            ping_pong::run_ping_pong().await;
        }
        Cli::ClusterHandshake {
            port_a,
            port_b,
            valid_cookies,
        } => {
            distributed::test_auth_handshake(port_a, port_b, valid_cookies.unwrap_or(true)).await;
        }
        Cli::RemotePingPong { port, remote_port } => {
            distributed::startup_ping_pong_test_node(port, remote_port).await;
        }
    }
}
