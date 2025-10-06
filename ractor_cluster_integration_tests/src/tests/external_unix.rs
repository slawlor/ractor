// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use ractor::Actor;
use ractor::ActorProcessingErr;
use ractor::ActorRef;
use ractor_cluster::ClusterBidiStream;
use ractor_cluster::{BoxRead, BoxWrite};

/// A `ClusterBidiStream` adapter for `tokio::net::UnixStream` using owned halves
struct UnixBidi {
    stream: tokio::net::UnixStream,
    peer: Option<String>,
    local: Option<String>,
}

impl UnixBidi {
    fn new(stream: tokio::net::UnixStream, peer: Option<String>, local: Option<String>) -> Self {
        Self {
            stream,
            peer,
            local,
        }
    }
}

impl ClusterBidiStream for UnixBidi {
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
async fn external_unix_auth_and_ready() -> Result<(), ActorProcessingErr> {
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

    // Create a connected UnixStream pair
    let (a_end, b_end) = tokio::net::UnixStream::pair()?;
    let a_stream = UnixBidi::new(a_end, Some("uds:node_b".into()), Some("uds:node_a".into()));
    let b_stream = UnixBidi::new(b_end, Some("uds:node_a".into()), Some("uds:node_b".into()));

    // Inject both sides
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

    // Wait for both sessions to be authenticated and ready
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
                    if let Ok(true) = ractor::call_t!(
                        info.actor,
                        ractor_cluster::node::NodeSessionMessage::GetAuthenticationState,
                        500
                    ) {
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
