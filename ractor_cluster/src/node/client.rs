// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! This module contains the logic for initiating client requests to other [super::NodeServer]s

use std::fmt::Display;

use ractor::ActorRef;
use ractor::MessagingErr;
use tokio::net::TcpStream;
use tokio::net::ToSocketAddrs;
use tokio_rustls::rustls::pki_types::ServerName;

/// A client connection error. Possible issues are Socket connection
/// problems or failure to talk to the [super::NodeServer]
#[derive(Debug)]
pub enum ClientConnectErr {
    /// Socket failed to bind, returning the underlying tokio error
    Socket(tokio::io::Error),
    /// Error communicating to the [super::NodeServer] actor. Actor receiving port is
    /// closed
    Messaging(MessagingErr<super::NodeServerMessage>),
    /// Some error with encryption has occurred
    Encryption(tokio::io::Error),
}

impl std::error::Error for ClientConnectErr {
    fn cause(&self) -> Option<&dyn std::error::Error> {
        match self {
            Self::Socket(cause) => Some(cause),
            Self::Messaging(cause) => Some(cause),
            Self::Encryption(cause) => Some(cause),
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

impl From<MessagingErr<super::NodeServerMessage>> for ClientConnectErr {
    fn from(value: MessagingErr<super::NodeServerMessage>) -> Self {
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
    node_server: &ActorRef<super::NodeServerMessage>,
    address: T,
) -> Result<(), ClientConnectErr>
where
    T: ToSocketAddrs,
{
    // connect to the socket
    let stream = TcpStream::connect(address).await?;
    stream.set_nodelay(true)?;

    // Notify the NodeServer that a new connection is opened
    let addr = stream.peer_addr()?;
    let local = stream.local_addr()?;

    node_server.cast(super::NodeServerMessage::ConnectionOpened {
        stream: Box::new(crate::net::NetworkStream::Raw {
            stream,
            peer_addr: addr,
            local_addr: local,
        }),
        is_server: false,
    })?;

    tracing::info!("TCP Session opened for {addr}");
    Ok(())
}

/// Connect to another [super::NodeServer] instance with network encryption
///
/// * `node_server` - The [super::NodeServer] which will own this new connection session
/// * `address` - The network address to send the connection to. Must implement [ToSocketAddrs]
/// * `encryption_settings` - The [tokio_rustls::TlsConnector] which is configured to encrypt the socket
/// * `domain` - The server name we're connecting to ([ServerName])
///
/// Returns: [Ok(())] if the connection was successful and the [super::NodeSession] was started. Handshake will continue
/// automatically. Results in a [Err(ClientConnectError)] if any part of the process failed to initiate
pub async fn connect_enc<T>(
    node_server: &ActorRef<super::NodeServerMessage>,
    address: T,
    encryption_settings: tokio_rustls::TlsConnector,
    domain: ServerName<'static>,
) -> Result<(), ClientConnectErr>
where
    T: ToSocketAddrs,
{
    // connect to the socket
    let stream = TcpStream::connect(address).await?;
    stream.set_nodelay(true)?;

    let addr = stream.peer_addr()?;
    let local = stream.local_addr()?;

    // encrypt the socket
    let enc_stream = encryption_settings
        .connect(domain, stream)
        .await
        .map_err(ClientConnectErr::Encryption)?;

    node_server.cast(super::NodeServerMessage::ConnectionOpened {
        stream: Box::new(crate::net::NetworkStream::TlsClient {
            stream: enc_stream,
            peer_addr: addr,
            local_addr: local,
        }),
        is_server: false,
    })?;

    tracing::info!("TCP Session opened for {addr}");
    Ok(())
}

/// Connect to a node using an external transport stream.
///
/// This helper wraps a user-provided stream implementing [`crate::ClusterBidiStream`]
/// and notifies the `NodeServer` of the opened connection. No socket or TLS is
/// managed here; the caller is responsible for preparing the transport.
pub async fn connect_external(
    node_server: &ActorRef<super::NodeServerMessage>,
    stream: Box<dyn crate::net::ClusterBidiStream>,
) -> Result<(), ClientConnectErr> {
    node_server.cast(super::NodeServerMessage::ConnectionOpenedExternal {
        stream,
        is_server: false,
    })?;
    tracing::info!("External session opened (client)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_connect_err_from_tokio_io_error() {
        // Test that tokio::io::Error converts to ClientConnectErr::Socket
        let io_err = tokio::io::Error::new(
            tokio::io::ErrorKind::ConnectionRefused,
            "Connection refused",
        );
        let client_err: ClientConnectErr = io_err.into();

        match client_err {
            ClientConnectErr::Socket(_) => {
                // Expected
            }
            _ => {
                panic!("Expected ClientConnectErr::Socket");
            }
        }
    }

    #[test]
    fn test_client_connect_err_display() {
        // Test that ClientConnectErr can be displayed
        let io_err = tokio::io::Error::new(tokio::io::ErrorKind::Other, "Connection error");
        let client_err: ClientConnectErr = io_err.into();

        let _display_str = format!("{}", client_err);
        // Just verify it doesn't panic
    }

    #[test]
    fn test_client_connect_err_is_error() {
        // Test that ClientConnectErr implements Error trait
        use std::error::Error;

        let io_err = tokio::io::Error::new(tokio::io::ErrorKind::Other, "Some error");
        let client_err: ClientConnectErr = io_err.into();

        let _err: &dyn Error = &client_err;
        // Just verify it implements the Error trait
    }
}
