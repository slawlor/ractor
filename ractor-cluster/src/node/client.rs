// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! This module contains the logic for initiating client requests to other [super::NodeServer]s

use std::fmt::Display;

use ractor::{cast, ActorRef, MessagingErr, SpawnErr};
use tokio::net::TcpStream;

/// Client connection error types
#[derive(Debug)]
pub enum ClientConnectError {
    /// Socket failed to bind, returning the underlying tokio error
    Socket(tokio::io::Error),
    /// Error communicating to the [super::NodeServer] actor. Actor receiving port is
    /// closed
    Messaging(MessagingErr),
    /// A timeout in trying to start a new [NodeSession]
    Timeout,
    /// Error spawning the tcp session actor supervision tree
    TcpSpawn(SpawnErr),
}

impl Display for ClientConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl From<tokio::io::Error> for ClientConnectError {
    fn from(value: tokio::io::Error) -> Self {
        Self::Socket(value)
    }
}

impl From<MessagingErr> for ClientConnectError {
    fn from(value: MessagingErr) -> Self {
        Self::Messaging(value)
    }
}

impl From<SpawnErr> for ClientConnectError {
    fn from(value: SpawnErr) -> Self {
        Self::TcpSpawn(value)
    }
}

/// Connect to another [super::NodeServer] instance
///
/// * `host` - The hostname to connect to
/// * `port` - The host's port to connect to
///
/// Returns: [Ok(())] if the connection was successful and the [NodeSession] was started. Handshake will continue
/// automatically. Results in a [Err(ClientConnectError)] if any part of the process failed to initiate
pub async fn connect(
    node_server: ActorRef<super::NodeServer>,
    host: &'static str,
    port: crate::net::NetworkPort,
) -> Result<(), ClientConnectError> {
    // connect to the socket
    let stream = TcpStream::connect(format!("{host}:{port}")).await?;

    // Startup the TCP handler, linked to the newly created `NodeSession`
    let addr = stream.peer_addr()?;

    let _ = cast!(
        node_server,
        super::SessionManagerMessage::ConnectionOpened {
            stream,
            is_server: false
        }
    );

    // // notify the `NodeSession` about it's tcp connection
    // let _ = session_handler.cast(super::SessionMessage::SetTcpSession(tcp_actor));
    log::info!("TCP Session opened for {}", addr);

    Ok(())
}
