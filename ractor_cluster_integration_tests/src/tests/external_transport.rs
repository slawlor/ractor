// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use ractor::Actor;
use ractor::ActorProcessingErr;
use ractor::ActorRef;
use ractor_cluster::ClusterBidiStream;
use ractor_cluster::{BoxRead, BoxWrite};

/// A simple `ClusterBidiStream` wrapper around `tokio::io::DuplexStream` for tests
struct TestDuplex {
    stream: tokio::io::DuplexStream,
    peer: Option<String>,
    local: Option<String>,
}

impl TestDuplex {
    fn new(stream: tokio::io::DuplexStream, peer: Option<String>, local: Option<String>) -> Self {
        Self {
            stream,
            peer,
            local,
        }
    }
}

impl ClusterBidiStream for TestDuplex {
    fn split(self: Box<Self>) -> (BoxRead, BoxWrite) {
        let (r, w) = tokio::io::split(self.stream);
        (Box::new(r), Box::new(w))
    }

    fn peer_label(&self) -> Option<String> {
        self.peer.clone()
    }

    fn local_label(&self) -> Option<String> {
        self.local.clone()
    }
}

#[ractor::concurrency::test]
async fn external_transport_auth_and_ready() -> Result<(), ActorProcessingErr> {
    // Spin up two NodeServers
    let cookie = "cookie".to_string();
    let hostname = "localhost".to_string();

    let server_a = ractor_cluster::node::NodeServer::new(
        0,
        cookie.clone(),
        "node_a".to_string(),
        hostname.clone(),
        None,
        None,
    );
    let server_b = ractor_cluster::node::NodeServer::new(
        0,
        cookie,
        "node_b".to_string(),
        hostname,
        None,
        None,
    );

    let (actor_a, handle_a) = Actor::spawn(None, server_a, ()).await?;
    let (actor_b, handle_b) = Actor::spawn(None, server_b, ()).await?;

    // Create a paired in-memory stream and inject as external connections
    let (a_end, b_end) = tokio::io::duplex(64 * 1024);
    let a_stream = TestDuplex::new(a_end, Some("node_b".into()), Some("node_a".into()));
    let b_stream = TestDuplex::new(b_end, Some("node_a".into()), Some("node_b".into()));

    // Inject both sides. One acts as server, the other as client
    actor_a
        .cast(
            ractor_cluster::node::NodeServerMessage::ConnectionOpenedExternal {
                stream: Box::new(a_stream),
                is_server: true,
            },
        )
        .unwrap();
    actor_b
        .cast(
            ractor_cluster::node::NodeServerMessage::ConnectionOpenedExternal {
                stream: Box::new(b_stream),
                is_server: false,
            },
        )
        .unwrap();

    // Wait for both node servers to report one session and that session to be ready
    async fn wait_for_ready(
        node: &ActorRef<ractor_cluster::node::NodeServerMessage>,
    ) -> anyhow::Result<()> {
        use std::time::Duration as StdDuration;
        let deadline = std::time::Instant::now() + StdDuration::from_secs(5);
        loop {
            if std::time::Instant::now() > deadline {
                anyhow::bail!("timeout waiting for session ready");
            }
            if let Ok(sessions) = ractor::call_t!(
                *node,
                ractor_cluster::node::NodeServerMessage::GetSessions,
                500
            ) {
                if sessions.len() == 1 {
                    let info = sessions.values().next().unwrap().clone();
                    // Authenticated?
                    if let Ok(true) = ractor::call_t!(
                        info.actor,
                        ractor_cluster::node::NodeSessionMessage::GetAuthenticationState,
                        500
                    ) {
                        // Ready?
                        if let Ok(true) = ractor::call_t!(
                            info.actor,
                            ractor_cluster::node::NodeSessionMessage::GetReadyState,
                            1000
                        ) {
                            return Ok(());
                        }
                    }
                }
            }
            ractor::concurrency::sleep(ractor::concurrency::Duration::from_millis(50)).await;
        }
    }

    wait_for_ready(&actor_a).await.expect("A not ready");
    wait_for_ready(&actor_b).await.expect("B not ready");

    // Cleanup
    actor_a.stop(None);
    actor_b.stop(None);
    handle_a.await.unwrap();
    handle_b.await.unwrap();
    Ok(())
}
