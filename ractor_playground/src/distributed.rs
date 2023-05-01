// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Distributed cluster playground

use ractor::RpcReplyPort;
use ractor::{concurrency::Duration, Actor, ActorProcessingErr, ActorRef};
use ractor_cluster::RactorClusterMessage;

/// Run test with
///
/// ```bash
/// RUST_LOG=debug cargo run -p ractor-playground -- cluster-handshake 8198 8199 true
/// ```
pub(crate) async fn test_auth_handshake(port_a: u16, port_b: u16, valid_cookies: bool) {
    let cookie_a = "cookie".to_string();
    let cookie_b = if valid_cookies {
        cookie_a.clone()
    } else {
        "bad cookie".to_string()
    };
    let hostname = "localhost".to_string();

    let server_a = ractor_cluster::NodeServer::new(
        port_a,
        cookie_a,
        "node_a".to_string(),
        hostname.clone(),
        None,
        None,
    );
    let server_b = ractor_cluster::NodeServer::new(
        port_b,
        cookie_b,
        "node_b".to_string(),
        hostname,
        None,
        None,
    );

    let (actor_a, handle_a) = Actor::spawn(None, server_a, ())
        .await
        .expect("Failed to start NodeServer A");
    let (actor_b, handle_b) = Actor::spawn(None, server_b, ())
        .await
        .expect("Failed to start NodeServer B");

    if let Err(error) =
        ractor_cluster::node::client::connect(&actor_b, format!("127.0.0.1:{port_a}")).await
    {
        log::error!("Failed to connect with error {error}")
    } else {
        log::info!("Client connected NodeServer b to NodeServer a");
    }

    ractor::concurrency::sleep(Duration::from_millis(10000)).await;
    log::warn!("Terminating test");

    // cleanup
    actor_a.stop(None);
    actor_b.stop(None);
    handle_a.await.unwrap();
    handle_b.await.unwrap();
}

// TODO: protocol integration tests

struct PingPongActor;

#[derive(RactorClusterMessage)]
enum PingPongActorMessage {
    Ping,
    #[rpc]
    Rpc(String, RpcReplyPort<String>),
}

#[async_trait::async_trait]
impl Actor for PingPongActor {
    type Msg = PingPongActorMessage;
    type State = ();
    type Arguments = ();

    async fn pre_start(
        &self,
        myself: ActorRef<Self::Msg>,
        _: (),
    ) -> Result<Self::State, ActorProcessingErr> {
        ractor::pg::join("test".to_string(), vec![myself.get_cell()]);
        Ok(())
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        _state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        let group = "test".to_string();
        let remote_actors = ractor::pg::get_members(&group)
            .into_iter()
            .filter(|actor| !actor.get_id().is_local())
            .map(ActorRef::<Self::Msg>::from)
            .collect::<Vec<_>>();
        match message {
            Self::Msg::Ping => {
                log::info!(
                    "Received a ping, replying in kind to {} remote actors",
                    remote_actors.len()
                );
                for act in remote_actors {
                    act.cast(PingPongActorMessage::Ping)?;
                }
            }
            Self::Msg::Rpc(request, reply) => {
                log::info!(
                    "Received an RPC of '{}' replying in kind to {} remote actors",
                    request,
                    remote_actors.len()
                );
                let reply_msg = format!("{request}.");
                reply.send(reply_msg.clone())?;
                for act in remote_actors {
                    let _reply =
                        ractor::call_t!(act, PingPongActorMessage::Rpc, 100, reply_msg.clone())?;
                }
            }
        }

        Ok(())
    }
}

pub(crate) async fn startup_ping_pong_test_node(port: u16, connect_client: Option<u16>) {
    let cookie = "cookie".to_string();
    let hostname = "localhost".to_string();

    let server =
        ractor_cluster::NodeServer::new(port, cookie, "node_a".to_string(), hostname, None, None);

    let (actor, handle) = Actor::spawn(None, server, ())
        .await
        .expect("Failed to start NodeServer A");

    let (test_actor, test_handle) = Actor::spawn(None, PingPongActor, ())
        .await
        .expect("Ping pong actor failed to start up!");

    if let Some(cport) = connect_client {
        if let Err(error) =
            ractor_cluster::node::client::connect(&actor, format!("127.0.0.1:{cport}")).await
        {
            log::error!("Failed to connect with error {error}")
        } else {
            log::info!("Client connected to NdoeServer");
        }
    }

    // wait for server startup to complete (and in the event of a client, wait for auth to complete), then startup the test actor
    ractor::concurrency::sleep(Duration::from_millis(1000)).await;
    // test_actor.cast(PingPongActorMessage::Ping).unwrap();
    let _ = test_actor
        .call(
            |tx| PingPongActorMessage::Rpc("".to_string(), tx),
            Some(Duration::from_millis(50)),
        )
        .await
        .unwrap();

    // wait for exit
    tokio::signal::ctrl_c()
        .await
        .expect("failed to listen for event");

    // cleanup
    test_actor.stop(None);
    test_handle.await.unwrap();
    actor.stop(None);
    handle.await.unwrap();
}
