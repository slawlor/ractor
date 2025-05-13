// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Test the transitive dist-connect functionality of the cluster. If B -> A and C -> A
//! then C should auto-connect to B

use clap::Args;
use ractor::concurrency::sleep;
use ractor::concurrency::Duration;
use ractor::concurrency::Instant;
use ractor::Actor;

const DIST_CONNECT_TIME_ALLOWANCE_MS: u128 = 2000;

/// Configuration
#[derive(Args, Debug, Clone)]
pub struct DistConnectConfig {
    /// Node's name (also DNS name)
    node_name: String,
    /// Server port
    server_port: u16,
    /// If specified, represents the client to connect to
    client_port: Option<u16>,
    /// If specified, represents the client to connect to
    client_host: Option<String>,
}

pub async fn test(config: DistConnectConfig) -> i32 {
    let cookie = "cookie".to_string();

    let server = ractor_cluster::NodeServer::new(
        config.server_port,
        cookie,
        super::random_name(),
        config.node_name.clone(),
        None,
        if config.node_name.as_str() == "node-c" {
            Some(ractor_cluster::node::NodeConnectionMode::Transitive)
        } else {
            Some(ractor_cluster::node::NodeConnectionMode::Isolated)
        },
    );

    tracing::info!("Starting NodeServer on port {}", config.server_port);
    // startup the node server
    let (actor, handle) = Actor::spawn(None, server, ())
        .await
        .expect("Failed to start NodeServer");

    // let the nodeserver startup and start listening on the port
    sleep(Duration::from_millis(100)).await;

    // if you're Node-c, wait for Node-b to be fully authenticated first
    if config.node_name.as_str() == "node-c" {
        // Let B fully authenticate to A before starting C so it'll be available in the listing.
        sleep(Duration::from_millis(100)).await;
    }

    // If this server should connect to a client server, initiate that connection
    if let (Some(client_host), Some(client_port)) = (config.client_host, config.client_port) {
        tracing::info!("Connecting to remote NodeServer at {client_host}:{client_port}");
        if let Err(error) =
            ractor_cluster::node::client::connect(&actor, format!("{client_host}:{client_port}"))
                .await
        {
            tracing::error!("Failed to connect with error {error}");
            return -3;
        } else {
            tracing::info!(
                "Client connected {} to {client_host}:{client_port}",
                config.node_name
            );
        }
    }

    let mut err_code = -1;
    tracing::info!("Waiting for NodeSession status updates");

    let mut rpc_reply = ractor::call_t!(actor, ractor_cluster::NodeServerMessage::GetSessions, 200);
    let tic = Instant::now();

    while rpc_reply.is_ok() {
        let time: Duration = Instant::now() - tic;
        if time.as_millis() > DIST_CONNECT_TIME_ALLOWANCE_MS {
            err_code = -2;
            tracing::error!(
                "The dist-connect test time has been going on for over > {}ms. Failing the test",
                time.as_millis()
            );
            break;
        }

        let values = rpc_reply
            .unwrap()
            .into_values()
            .filter_map(|v| v.peer_name)
            .collect::<Vec<_>>();
        if values.len() >= 2 {
            // Our node as at least 2 connections
            tracing::debug!("Connected session information: {:?}", values);
            tracing::info!("Transitive connections succeeded. Exiting");
            err_code = 0;
            break;
        }

        // try again
        rpc_reply = ractor::call_t!(actor, ractor_cluster::NodeServerMessage::GetSessions, 200);
    }

    tracing::info!("Terminating test - code {err_code}");

    // Let the other nodes exist for some time to make sure we get to a stable network state before actually terminating nodes
    sleep(Duration::from_millis(500)).await;

    // cleanup
    actor.stop(None);
    handle.await.unwrap();

    err_code
}
