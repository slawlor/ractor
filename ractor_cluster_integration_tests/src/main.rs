// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use std::env;
use std::io::stderr;
use std::io::IsTerminal;

use clap::Parser;
use rustyrepl::{Repl, ReplCommandProcessor};
use tracing_glog::Glog;
use tracing_glog::GlogFields;
use tracing_subscriber::filter::Directive;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::Registry;

mod repl;
mod tests;

// other tests mainly related to the procedural macros on `Ractor*Message`
// derivation, which can't be inside the `ractor_cluster` crate due to imported
// namespaces
#[cfg(test)]
mod derive_macro_tests;
#[cfg(test)]
mod ractor_forward_port_tests;

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
                    tracing::info!("CTRL-C pressed, exiting test");
                }
            }
        }
    }
    tracing::info!("Test exiting");
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
