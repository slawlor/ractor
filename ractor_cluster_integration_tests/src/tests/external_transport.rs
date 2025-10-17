// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Docker-backed end-to-end validation of the external transport API.
//!
//! Each node spins up a [`NodeServer`](ractor_cluster::NodeServer) and an
//! [`ExternalProbeActor`] that joins a process group shared between the
//! containers. Instead of relying on the built-in TCP listener, the nodes bind
//! their own TCP socket and inject the accepted stream through
//! [`NodeServerMessage::ConnectionOpenedExternal`]. The peer dials that socket
//! and registers the client half via [`client_connect_external`]. Once the
//! cluster session reports `Ready`, the actors exchange RPCs to prove remote
//! messaging works over the custom transport wiring.

use clap::Args;
use ractor::concurrency::sleep;
use ractor::concurrency::Duration;
use ractor::concurrency::Instant;
use ractor::concurrency::JoinHandle as RactorJoinHandle;
use ractor::Actor;
use ractor::ActorProcessingErr;
use ractor::ActorRef;
use ractor::RpcReplyPort;
use ractor_cluster::ClusterBidiStream;
use ractor_cluster::NodeServerMessage;
use ractor_cluster::NodeSessionMessage;
use ractor_cluster::RactorClusterMessage;
use ractor_cluster::{BoxRead, BoxWrite};
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio::task::JoinHandle as TokioJoinHandle;

const EXTERNAL_GROUP: &str = "external-transport";
const SESSION_TIMEOUT_MS: u128 = 5_000;
const PROBE_TIMEOUT_MS: u128 = 5_000;
const PROBE_RPC_TIMEOUT_MS: u64 = 500;
const PROBE_RETRY_DELAY_MS: u64 = 200;
const CONNECT_RETRY_DELAY_MS: u64 = 200;
const CONNECT_RETRY_ATTEMPTS: usize = 20;
const SESSION_AUTH_GRACE_MS: u128 = 1_000;
const SESSION_TEARDOWN_TIMEOUT_MS: u128 = 5_000;

struct ExternalProbeActor;

#[derive(RactorClusterMessage)]
enum ExternalProbeMessage {
    /// Begin searching for remote group members and ping them.
    Kickoff,
    /// RPC invoked by the remote probe to confirm transport reachability.
    #[rpc]
    Ping(String, RpcReplyPort<String>),
    /// Local RPC queried by the harness to see if the probe finished.
    #[rpc]
    IsComplete(RpcReplyPort<bool>),
}

struct ExternalProbeState {
    label: String,
    complete: bool,
}

struct ExternalProbeArgs {
    label: String,
}

#[cfg_attr(feature = "async-trait", ractor::async_trait)]
impl Actor for ExternalProbeActor {
    type Msg = ExternalProbeMessage;
    type State = ExternalProbeState;
    type Arguments = ExternalProbeArgs;

    async fn pre_start(
        &self,
        myself: ActorRef<Self::Msg>,
        args: ExternalProbeArgs,
    ) -> Result<Self::State, ActorProcessingErr> {
        ractor::pg::join(EXTERNAL_GROUP.to_string(), vec![myself.get_cell()]);
        Ok(ExternalProbeState {
            label: args.label,
            complete: false,
        })
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            ExternalProbeMessage::Kickoff => {
                if state.complete {
                    return Ok(());
                }

                let remote_members = ractor::pg::get_members(&EXTERNAL_GROUP.to_string())
                    .into_iter()
                    .filter(|actor| !actor.get_id().is_local())
                    .map(ActorRef::<Self::Msg>::from)
                    .collect::<Vec<_>>();

                if remote_members.is_empty() {
                    let handle = myself.clone();
                    ractor::concurrency::spawn(async move {
                        sleep(Duration::from_millis(PROBE_RETRY_DELAY_MS)).await;
                        let _ = handle.cast(ExternalProbeMessage::Kickoff);
                    });
                    return Ok(());
                }

                for remote in remote_members {
                    match ractor::call_t!(
                        remote,
                        ExternalProbeMessage::Ping,
                        PROBE_RPC_TIMEOUT_MS,
                        state.label.clone()
                    ) {
                        Ok(reply) => {
                            tracing::info!(
                                "{} received external transport reply '{reply}'",
                                state.label
                            );
                            if reply.contains(&state.label) {
                                state.complete = true;
                                break;
                            }
                        }
                        Err(err) => {
                            tracing::warn!(
                                "{} failed to ping remote external probe: {err}",
                                state.label
                            );
                        }
                    }
                }

                if !state.complete {
                    let handle = myself.clone();
                    ractor::concurrency::spawn(async move {
                        sleep(Duration::from_millis(PROBE_RETRY_DELAY_MS)).await;
                        let _ = handle.cast(ExternalProbeMessage::Kickoff);
                    });
                }
            }
            ExternalProbeMessage::Ping(requestor, reply) => {
                let response = format!("Hello {requestor} from {}", state.label);
                let _ = reply.send(response.clone());
                state.complete = true;
                tracing::info!(
                    "{} responded to external transport probe with '{response}'",
                    state.label
                );
            }
            ExternalProbeMessage::IsComplete(reply) => {
                let _ = reply.send(state.complete);
            }
        }

        Ok(())
    }
}

struct TcpExternalStream {
    stream: TcpStream,
    peer_label: Option<String>,
    local_label: Option<String>,
}

impl TcpExternalStream {
    fn new(stream: TcpStream) -> Self {
        let peer_label = stream.peer_addr().ok().map(|addr| format!("tcp://{addr}"));
        let local_label = stream.local_addr().ok().map(|addr| format!("tcp://{addr}"));
        Self {
            stream,
            peer_label,
            local_label,
        }
    }
}

impl ClusterBidiStream for TcpExternalStream {
    fn split(self: Box<Self>) -> (BoxRead, BoxWrite) {
        let (reader, writer) = tokio::io::split(self.stream);
        (Box::new(reader), Box::new(writer))
    }

    fn peer_label(&self) -> Option<String> {
        self.peer_label.clone()
    }

    fn local_label(&self) -> Option<String> {
        self.local_label.clone()
    }
}

/// Configuration for the docker external transport scenario.
#[derive(Args, Debug, Clone)]
pub struct ExternalTransportConfig {
    /// Node's DNS name inside the docker network.
    node_name: String,
    /// Custom transport listener port. Use 0 to disable listening.
    listen_port: u16,
    /// Optional remote host to dial using the external transport.
    peer_host: Option<String>,
    /// Optional remote port to dial using the external transport.
    peer_port: Option<u16>,
}

pub async fn test(config: ExternalTransportConfig) -> i32 {
    let cookie = "cookie".to_string();
    let hostname = "localhost".to_string();

    let server =
        ractor_cluster::NodeServer::new(0, cookie, config.node_name.clone(), hostname, None, None);

    let (node_actor, node_handle) = Actor::spawn(None, server, ())
        .await
        .expect("Failed to start NodeServer");

    let (probe_actor, probe_handle) = Actor::spawn(
        None,
        ExternalProbeActor,
        ExternalProbeArgs {
            label: config.node_name.clone(),
        },
    )
    .await
    .expect("Failed to start external probe actor");

    let (listener_ready_rx, listener_handle) = if config.listen_port != 0 {
        let (tx, rx) = oneshot::channel();
        let handle = spawn_external_listener(
            node_actor.clone(),
            config.listen_port,
            config.node_name.clone(),
            tx,
        );
        (Some(rx), Some(handle))
    } else {
        (None, None)
    };

    let result: Result<(), i32> = async {
        if let Some(rx) = listener_ready_rx {
            match rx.await {
                Ok(Ok(())) => {
                    tracing::info!(
                        "{} external listener bound on port {}",
                        config.node_name,
                        config.listen_port
                    );
                }
                Ok(Err(err)) => {
                    tracing::error!(
                        "{} failed to bind external listener: {err}",
                        config.node_name
                    );
                    return Err(-6);
                }
                Err(_) => {
                    tracing::error!(
                        "{} external listener readiness channel dropped",
                        config.node_name
                    );
                    return Err(-6);
                }
            }
        }

        if let (Some(peer_host), Some(peer_port)) = (config.peer_host.clone(), config.peer_port) {
            let target = format!("{peer_host}:{peer_port}");
            if let Err(err) =
                connect_external_with_retry(&node_actor, &config.node_name, &target).await
            {
                tracing::error!(
                    "{} failed to connect external transport to {target}: {err}",
                    config.node_name
                );
                return Err(-3);
            }
        }

        if config.listen_port != 0 {
            wait_for_session_ready(&node_actor, &probe_actor, &config.node_name).await?;
        } else {
            wait_for_remote_probe_presence(&config.node_name).await?;
        }

        tracing::info!(
            "{} authenticated over external transport, starting probe",
            config.node_name
        );
        let _ = ractor::cast!(probe_actor, ExternalProbeMessage::Kickoff);

        wait_for_probe_completion(&probe_actor, &config.node_name).await?;

        if config.listen_port != 0 {
            wait_for_session_teardown(&node_actor, &config.node_name).await?;
        }

        Ok(())
    }
    .await;

    let err_code = match result {
        Ok(()) => 0,
        Err(code) => code,
    };

    tracing::info!(
        "Terminating external transport docker test for {} with code {err_code}",
        config.node_name
    );

    cleanup(
        node_actor,
        node_handle,
        probe_actor,
        probe_handle,
        listener_handle,
    )
    .await;

    err_code
}

fn spawn_external_listener(
    node_actor: ActorRef<NodeServerMessage>,
    port: u16,
    label: String,
    ready: oneshot::Sender<Result<(), String>>,
) -> TokioJoinHandle<Result<(), String>> {
    tokio::spawn(async move {
        tracing::info!("{} awaiting external transport on 0.0.0.0:{port}", label);
        let listener = match TcpListener::bind(("0.0.0.0", port)).await {
            Ok(listener) => {
                let _ = ready.send(Ok(()));
                listener
            }
            Err(err) => {
                let msg = format!("failed to bind external transport listener: {err}");
                let _ = ready.send(Err(msg.clone()));
                return Err(msg);
            }
        };

        match listener.accept().await {
            Ok((stream, peer_addr)) => {
                let _ = stream.set_nodelay(true);
                let transport = TcpExternalStream::new(stream);
                tracing::info!("{} accepted external transport from {peer_addr}", label);
                if let Err(err) = node_actor.cast(NodeServerMessage::ConnectionOpenedExternal {
                    stream: Box::new(transport),
                    is_server: true,
                }) {
                    let msg = format!("{label} failed to register external session: {err}");
                    tracing::error!("{msg}");
                    Err(msg)
                } else {
                    Ok(())
                }
            }
            Err(err) => {
                let msg = format!("{label} failed to accept external connection: {err}");
                tracing::error!("{msg}");
                Err(msg)
            }
        }
    })
}

async fn connect_external_with_retry(
    node_actor: &ActorRef<NodeServerMessage>,
    label: &str,
    target: &str,
) -> Result<(), String> {
    for attempt in 1..=CONNECT_RETRY_ATTEMPTS {
        match TcpStream::connect(target).await {
            Ok(stream) => {
                let _ = stream.set_nodelay(true);
                let transport = TcpExternalStream::new(stream);
                if let Err(err) =
                    ractor_cluster::client_connect_external(node_actor, Box::new(transport)).await
                {
                    return Err(format!("failed to register external session: {err}"));
                }
                tracing::info!(
                    "{} opened external transport to {target} on attempt {attempt}",
                    label
                );
                return Ok(());
            }
            Err(err) => {
                tracing::warn!(
                    "{} external transport attempt {attempt}/{CONNECT_RETRY_ATTEMPTS} to {target} failed: {err}",
                    label
                );
                if attempt == CONNECT_RETRY_ATTEMPTS {
                    return Err(err.to_string());
                }
                sleep(Duration::from_millis(CONNECT_RETRY_DELAY_MS)).await;
            }
        }
    }

    Err("exhausted external transport retries".to_string())
}

async fn wait_for_session_ready(
    node_actor: &ActorRef<NodeServerMessage>,
    probe_actor: &ActorRef<ExternalProbeMessage>,
    label: &str,
) -> Result<(), i32> {
    let start = Instant::now();
    let mut auth_only_session: Option<(u64, String, Instant)> = None;
    loop {
        match ractor::call_t!(node_actor, NodeServerMessage::GetSessions, 500) {
            Ok(map) => {
                if map.is_empty() {
                    if let Some((node_id, peer_addr, _since)) = &auth_only_session {
                        match ractor::call_t!(probe_actor, ExternalProbeMessage::IsComplete, 200) {
                            Ok(true) => {
                                tracing::info!(
                                    "{} proceeding after authenticated external session {} ({peer_addr}) closed post-probe",
                                    label,
                                    node_id
                                );
                                return Ok(());
                            }
                            Ok(false) => {}
                            Err(err) => {
                                tracing::warn!(
                                    "{} failed to query probe completion after session close: {err}",
                                    label
                                );
                            }
                        }
                    }
                }
                for info in map.into_values() {
                    match ractor::call_t!(info.actor, NodeSessionMessage::GetReadyState, 500) {
                        Ok(true) => return Ok(()),
                        Ok(false) => {
                            match ractor::call_t!(
                                info.actor,
                                NodeSessionMessage::GetAuthenticationState,
                                500
                            ) {
                                Ok(true) => match auth_only_session {
                                    Some((node_id, _, since))
                                        if node_id == info.node_id
                                            && (Instant::now() - since).as_millis()
                                                >= SESSION_AUTH_GRACE_MS =>
                                    {
                                        tracing::info!(
                                                "{} proceeding with authenticated external session {} lacking ready signal",
                                                label,
                                                info.peer_addr.clone()
                                            );
                                        return Ok(());
                                    }
                                    Some((node_id, _, _)) if node_id != info.node_id => {
                                        auth_only_session = Some((
                                            info.node_id,
                                            info.peer_addr.clone(),
                                            Instant::now(),
                                        ));
                                    }
                                    None => {
                                        auth_only_session = Some((
                                            info.node_id,
                                            info.peer_addr.clone(),
                                            Instant::now(),
                                        ));
                                    }
                                    _ => {}
                                },
                                Ok(false) => {
                                    auth_only_session = None;
                                }
                                Err(err) => {
                                    tracing::error!(
                                        "{} failed to query external session auth state: {err}",
                                        label
                                    );
                                    return Err(-4);
                                }
                            }
                        }
                        Err(err) => {
                            tracing::error!(
                                "{} failed to query external session readiness: {err}",
                                label
                            );
                            return Err(-4);
                        }
                    }
                }
            }
            Err(err) => {
                tracing::error!("{} failed to fetch external session list: {err}", label);
                return Err(-4);
            }
        }

        if (Instant::now() - start).as_millis() > SESSION_TIMEOUT_MS {
            tracing::error!(
                "{} timed out waiting for external transport session readiness",
                label
            );
            return Err(-2);
        }

        sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_for_probe_completion(
    probe_actor: &ActorRef<ExternalProbeMessage>,
    label: &str,
) -> Result<(), i32> {
    let start = Instant::now();
    loop {
        match ractor::call_t!(probe_actor, ExternalProbeMessage::IsComplete, 500) {
            Ok(true) => {
                tracing::info!("{} external transport probe completed", label);
                return Ok(());
            }
            Ok(false) => {
                if (Instant::now() - start).as_millis() > PROBE_TIMEOUT_MS {
                    tracing::error!(
                        "{} timed out waiting for external transport acknowledgement",
                        label
                    );
                    return Err(-5);
                }
            }
            Err(err) => {
                tracing::error!("{} failed to query external probe state: {err}", label);
                return Err(-5);
            }
        }

        sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_for_session_teardown(
    node_actor: &ActorRef<NodeServerMessage>,
    label: &str,
) -> Result<(), i32> {
    let start = Instant::now();
    loop {
        match ractor::call_t!(node_actor, NodeServerMessage::GetSessions, 500) {
            Ok(map) => {
                if map.is_empty() {
                    tracing::info!(
                        "{} observed external transport session teardown, proceeding to cleanup",
                        label
                    );
                    return Ok(());
                }
            }
            Err(err) => {
                tracing::error!(
                    "{} failed to fetch external session list during teardown: {err}",
                    label
                );
                return Err(-4);
            }
        }

        if (Instant::now() - start).as_millis() > SESSION_TEARDOWN_TIMEOUT_MS {
            tracing::warn!(
                "{} timed out waiting for external transport session teardown; continuing anyway",
                label
            );
            return Ok(());
        }

        sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_for_remote_probe_presence(label: &str) -> Result<(), i32> {
    let start = Instant::now();
    loop {
        let remote_members = ractor::pg::get_members(&EXTERNAL_GROUP.to_string())
            .into_iter()
            .filter(|actor| !actor.get_id().is_local())
            .collect::<Vec<_>>();

        if !remote_members.is_empty() {
            tracing::info!(
                "{} discovered remote probe members ({:?}) in process group '{EXTERNAL_GROUP}'",
                label,
                remote_members
                    .iter()
                    .map(|actor| actor.get_id())
                    .collect::<Vec<_>>()
            );
            return Ok(());
        }

        if (Instant::now() - start).as_millis() > SESSION_TIMEOUT_MS {
            tracing::error!(
                "{} timed out waiting for remote probe members in process group '{EXTERNAL_GROUP}'",
                label
            );
            return Err(-2);
        }

        sleep(Duration::from_millis(100)).await;
    }
}

async fn cleanup(
    node_actor: ActorRef<NodeServerMessage>,
    node_handle: RactorJoinHandle<()>,
    probe_actor: ActorRef<ExternalProbeMessage>,
    probe_handle: RactorJoinHandle<()>,
    listener_handle: Option<TokioJoinHandle<Result<(), String>>>,
) {
    probe_actor.stop(None);
    node_actor.stop(None);

    if let Some(handle) = listener_handle {
        if !handle.is_finished() {
            handle.abort();
        }
        match handle.await {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                tracing::warn!("External listener terminated with error: {err}");
            }
            Err(err) => {
                tracing::warn!("Failed to join external listener task: {err}");
            }
        }
    }

    let _ = probe_handle.await;
    let _ = node_handle.await;
}

// -----------------------------------------------------------------------------
// Legacy in-memory regression test
// -----------------------------------------------------------------------------

#[cfg(test)]
mod regression {
    use super::*;

    /// A simple `ClusterBidiStream` wrapper around `tokio::io::DuplexStream` for tests
    struct TestDuplex {
        stream: tokio::io::DuplexStream,
        peer: Option<String>,
        local: Option<String>,
    }

    impl TestDuplex {
        fn new(
            stream: tokio::io::DuplexStream,
            peer: Option<String>,
            local: Option<String>,
        ) -> Self {
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

    /// In-memory verification of the external transport ready state handling.
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
}
