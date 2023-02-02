// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Test the authentication handshake, making sure 2 nodes can interconnect together and authenticate
//! with the secret cookie challenge scheme

use clap::Args;

use ractor::concurrency::{sleep, Duration, Instant};
use ractor::Actor;

const AUTH_TIME_ALLOWANCE_MS: u128 = 1500;

/// Configuration
#[derive(Args, Debug, Clone)]
pub struct AuthHandshakeConfig {
    /// Server port
    server_port: u16,
    /// If specified, represents the client to connect to
    client_port: Option<u16>,
    /// If specified, represents the client to connect to
    client_host: Option<String>,
}

pub async fn test(config: AuthHandshakeConfig) -> i32 {
    let cookie = "cookie".to_string();
    let hostname = "localhost".to_string();

    let server = ractor_cluster::NodeServer::new(
        config.server_port,
        cookie,
        super::random_name(),
        hostname.clone(),
    );

    log::info!("Starting NodeServer on port {}", config.server_port);

    let (actor, handle) = Actor::spawn(None, server, ())
        .await
        .expect("Failed to start NodeServer");

    if let (Some(client_host), Some(client_port)) = (config.client_host, config.client_port) {
        log::info!(
            "Connecting to remote NodeServer at {}:{}",
            client_host,
            client_port
        );
        if let Err(error) =
            ractor_cluster::node::client::connect(&actor, format!("{client_host}:{client_port}"))
                .await
        {
            log::error!("Failed to connect with error {error}");
            return -3;
        } else {
            log::info!("Client connected NodeServer b to NodeServer a");
        }
    }

    let mut err_code = -1;
    log::info!("Waiting for NodeSession status updates");

    let mut rpc_reply = ractor::call_t!(actor, ractor_cluster::NodeServerMessage::GetSessions, 200);
    let mut tic = None;

    while rpc_reply.is_ok() {
        if let Some(timestamp) = tic {
            let time: Duration = Instant::now() - timestamp;
            if time.as_millis() > AUTH_TIME_ALLOWANCE_MS {
                err_code = -2;
                log::error!(
                    "The authentcation time has been going on for over > {}ms. Failing the test",
                    time.as_millis()
                );
                break;
            }
        }

        if let Some(item) = rpc_reply
            .unwrap()
            .into_values()
            .collect::<Vec<_>>()
            .first()
            .cloned()
        {
            // we got an actor, track how long it took to auth, maxing out at 500ms
            if tic.is_none() {
                tic = Some(Instant::now());
            }

            let is_authenticated = ractor::call_t!(
                item,
                ractor_cluster::NodeSessionMessage::GetAuthenticationState,
                200
            );
            match is_authenticated {
                Err(err) => {
                    log::warn!("NodeSession returned error on rpc query {}", err);
                    break;
                }
                Ok(false) => {
                    // Still waiting
                }
                Ok(true) => {
                    err_code = 0;
                    log::info!("Authentication succeeded. Exiting test");
                    break;
                }
            }
        }
        // try again
        rpc_reply = ractor::call_t!(actor, ractor_cluster::NodeServerMessage::GetSessions, 200);
    }

    log::info!("Terminating test - code {}", err_code);

    sleep(Duration::from_millis(250)).await;

    // cleanup
    actor.stop(None);
    handle.await.unwrap();

    err_code
}
