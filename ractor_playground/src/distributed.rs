// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Distributed cluster playground

use ractor::{concurrency::Duration, Actor};

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

    let server_a =
        ractor_cluster::NodeServer::new(port_a, cookie_a, "node_a".to_string(), hostname.clone());
    let server_b =
        ractor_cluster::NodeServer::new(port_b, cookie_b, "node_b".to_string(), hostname);

    let (actor_a, handle_a) = Actor::spawn(None, server_a)
        .await
        .expect("Failed to start NodeServer A");
    let (actor_b, handle_b) = Actor::spawn(None, server_b)
        .await
        .expect("Failed to start NodeServer B");

    if let Err(error) =
        ractor_cluster::node::client::connect(actor_b.clone(), format!("127.0.0.1:{port_a}")).await
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
