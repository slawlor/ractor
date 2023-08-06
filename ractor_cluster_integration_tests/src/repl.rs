// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use anyhow::Result;
use rustyrepl::*;

use crate::tests::TestCase;

#[derive(Debug)]
pub struct TestRepl;

#[async_trait::async_trait]
impl ReplCommandProcessor<TestCase> for TestRepl {
    fn is_quit(&self, command: &str) -> bool {
        matches!(command, "quit" | "exit")
    }

    async fn process_command(&self, command: TestCase) -> Result<()> {
        let code = match command {
            TestCase::AuthHandshake(config) => crate::tests::auth_handshake::test(config).await,
            TestCase::PgGroups(config) => crate::tests::pg_groups::test(config).await,
            TestCase::Encryption(config) => crate::tests::encryption::test(config).await,
            TestCase::DistConnect(config) => crate::tests::dist_connect::test(config).await,

            TestCase::Nan => {
                ractor::concurrency::sleep(ractor::concurrency::Duration::from_secs(2)).await;
                0
            }
        };

        if code < 0 {
            tracing::error!("Test failed with code {}", code);
            // BLOW UP THE WORLD!
            std::process::exit(code);
        }

        Ok(())
    }
}
