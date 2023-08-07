// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Test the synchronisation of the name registry.
//!
//! This test creates a "Dummy" actor that does nothing, but exists as an actor
//! that we can freely spawn with a specific name. Any nodes created without
//! connecting to another node will spawn an actor with the name "dummy_actor",
//! and then wait for it to be stopped. Another node, which connects to this node,
//! will attempt to find this node by name, and will request for it to stop.
//!
//! The test completes, when the actor is stopped.

use clap::Args;
use ractor::concurrency::{sleep, Duration};
use ractor::{Actor, ActorProcessingErr, ActorRef};
struct DummyActor;

#[async_trait::async_trait]
impl Actor for DummyActor {
    type Msg = ();
    type State = ();
    type Arguments = ();

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        _: (),
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(())
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        _message: Self::Msg,
        _state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        myself.stop(None);
        Ok(())
    }
}

/// Configuration
#[derive(Args, Debug, Clone)]
pub struct NameRegistryConfig {
    /// Server port
    server_port: u16,
    /// If specified, represents the client to connect to
    client_port: Option<u16>,
    /// If specified, represents the client to connect to
    client_host: Option<String>,
}

pub(crate) async fn test(config: NameRegistryConfig) -> i32 {
    let cookie = "cookie".to_string();
    let hostname = "localhost".to_string();
    let dummy_actor_name = "dummy_actor";
    let should_spawn_actor = config.client_host.is_none() && config.client_port.is_none();

    let server = ractor_cluster::NodeServer::new(
        config.server_port,
        cookie,
        super::random_name(),
        hostname,
        None,
        None,
    );

    let (actor, handle) = Actor::spawn(None, server, ())
        .await
        .expect("Failed to start NodeServer A");

    let actor_resources = if should_spawn_actor {
        let (test_actor, test_handle) =
            Actor::spawn(Some(dummy_actor_name.to_owned()), DummyActor, ())
                .await
                .expect("Dummy actor failed to start up!");
        Some((test_actor, test_handle))
    } else {
        None
    };

    if let (Some(client_host), Some(client_port)) = (config.client_host, config.client_port) {
        log::info!(
            "Connecting to remote NodeServer at {}:{}",
            client_host,
            client_port
        );
        if let Err(error) =
            ractor_cluster::client_connect(&actor, format!("{client_host}:{client_port}")).await
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
    while rpc_reply.is_ok() {
        if let Some(item) = rpc_reply
            .unwrap()
            .into_values()
            .collect::<Vec<_>>()
            .first()
            .cloned()
        {
            let is_authenticated = ractor::call_t!(
                item.actor,
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

    if err_code == 0 {
        // we're authenticated
        if !should_spawn_actor {
            // If we didn't spawn the actor, check if it exists in the registry
            match ractor::registry::where_is(dummy_actor_name.to_owned()) {
                Some(remote_actor) => {
                    if remote_actor.get_id().is_local() {
                        log::error!("Expected dummy actor to be remote, not local");
                        return -2;
                    }
                    log::info!("Found the remote dummy actor");
                    // Shut down the remote actor
                    //remote_actor.stop(None);
                    remote_actor
                        .send_message(())
                        .expect("can send message to remote actor");
                }
                None => {
                    log::error!("Could not find the dummy actor by name");
                    return -1;
                }
            }
        }
    } else {
        log::warn!("Failed to authenticate, failing test");
    }

    log::info!("Terminating test - code {}", err_code);

    sleep(Duration::from_millis(250)).await;

    // cleanup
    if let Some((_, test_handle)) = actor_resources {
        // The actor should have been stopped by the client, so just wait for it
        log::info!("Waiting for the client to stop the actor");
        test_handle.await.unwrap();
    }

    log::warn!("stopping node");
    actor.stop(None);
    handle.await.unwrap();

    err_code
}
