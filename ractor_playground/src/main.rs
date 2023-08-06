// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use std::env;
use std::io::stderr;
use std::io::IsTerminal;

use clap::Parser;
use tracing_glog::Glog;
use tracing_glog::GlogFields;
use tracing_subscriber::filter::Directive;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::Registry;

mod distributed;
mod ping_pong;

#[derive(Debug, clap::Subcommand)]
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

#[derive(Parser, Debug)]
struct Args {
    #[command(subcommand)]
    command: Cli,

    /// Set the logging level based on the set of filter directives.
    ///
    /// Normal logging levels are supported (e.g. trace, debug, info, warn,
    /// error), but it's possible to set verbosity for specific spans and
    /// events.
    #[clap(short, long, default_value = "info", use_value_delimiter = true)]
    log: Vec<Directive>,
}

// MAIN //
#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let args = Args::parse();

    // if it's not set, set the log level to debug
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "debug");
    }
    init_logging(args.log);

    // parse the CLI and run the correct playground scenario
    match args.command {
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

fn init_logging(directives: Vec<Directive>) {
    let fmt = tracing_subscriber::fmt::Layer::default()
        .with_ansi(stderr().is_terminal())
        .with_writer(std::io::stderr)
        .event_format(Glog::default().with_timer(tracing_glog::LocalTime::default()))
        .fmt_fields(GlogFields);

    let filter = directives
        .into_iter()
        .fold(EnvFilter::from_default_env(), |filter, directive| {
            filter.add_directive(directive)
        });

    let subscriber = Registry::default().with(filter).with(fmt);
    tracing::subscriber::set_global_default(subscriber).expect("to set global subscriber");
}
