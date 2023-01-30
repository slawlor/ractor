// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use std::env;

mod repl;
mod tests;

// other tests mainly related to the procedural macros on `Ractor*Message`
// derivation, which can't be inside the `ractor_cluster` crate due to imported
// namespaces
#[cfg(test)]
mod derive_macro_tests;
#[cfg(test)]
mod ractor_forward_port_tests;

use clap::Parser;
use rustyrepl::{Repl, ReplCommandProcessor};

#[derive(Debug, clap::Subcommand)]
enum Command {
    /// Run a test-case directly
    #[command(subcommand)]
    Test(tests::TestCase),
    /// Start the Read-evaluate-print-loop interface
    Repl,
}

/// Test-suite runner
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

// MAIN //
#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let args = Args::parse();

    // if it's not set, set the log level to debug
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "debug");
    }
    env_logger::builder().format_timestamp_millis().init();

    let repl_processor = repl::TestRepl;
    match args.command {
        Command::Repl => {
            let processor: Box<dyn ReplCommandProcessor<tests::TestCase>> =
                Box::new(repl_processor);
            let mut repl = Repl::<tests::TestCase>::new(processor, None, Some(">>".to_string()))
                .expect("Failed to create REPL");
            repl.process().await.expect("REPL exited with error");
        }

        Command::Test(case) => {
            tokio::select! {
                out = repl_processor.process_command(case) => {
                    out.expect("Failed to process command");
                }
                _ = tokio::signal::ctrl_c() => {
                    log::info!("CTRL-C pressed, exiting test");
                }
            }
        }
    }
    log::info!("Test exiting");
}
