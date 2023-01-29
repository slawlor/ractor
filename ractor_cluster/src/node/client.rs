// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! This module contains the logic for initiating client requests to other [super::NodeServer]s

use std::fmt::Display;

use ractor::{ActorRef, MessagingErr};
use tokio::net::{TcpStream, ToSocketAddrs};

/// A client connection error. Possible issues are Socket connection
/// problems or failure to talk to the [super::NodeServer]
#[derive(Debug)]
pub enum ClientConnectErr {
    /// Socket failed to bind, returning the underlying tokio error
    Socket(tokio::io::Error),
    /// Error communicating to the [super::NodeServer] actor. Actor receiving port is
    /// closed
    Messaging(MessagingErr),
}

impl std::error::Error for ClientConnectErr {
    fn cause(&self) -> Option<&dyn std::error::Error> {
        match self {
            Self::Socket(cause) => Some(cause),
            Self::Messaging(cause) => Some(cause),
        }
    }
}

impl Display for ClientConnectErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl From<tokio::io::Error> for ClientConnectErr {
    fn from(value: tokio::io::Error) -> Self {
        Self::Socket(value)
    }
}

impl From<MessagingErr> for ClientConnectErr {
    fn from(value: MessagingErr) -> Self {
        Self::Messaging(value)
    }
}

/// Connect to another [super::NodeServer] instance
///
/// * `node_server` - The [super::NodeServer] which will own this new connection session
/// * `address` - The network address to send the connection to. Must implement [ToSocketAddrs]
///
/// Returns: [Ok(())] if the connection was successful and the [super::NodeSession] was started. Handshake will continue
/// automatically. Results in a [Err(ClientConnectError)] if any part of the process failed to initiate
pub async fn connect<T>(
    node_server: ActorRef<super::NodeServer>,
    address: T,
) -> Result<(), ClientConnectErr>
where
    T: ToSocketAddrs,
{
    // connect to the socket
    let stream = TcpStream::connect(address).await?;

    // Startup the TCP handler, linked to the newly created `NodeSession`
    let addr = stream.peer_addr()?;

    node_server.cast(super::NodeServerMessage::ConnectionOpened {
        stream,
        is_server: false,
    })?;

    log::info!("TCP Session opened for {}", addr);
    Ok(())
}
