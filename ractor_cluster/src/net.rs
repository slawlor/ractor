// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! TCP server and session actors which transmit [prost::Message] encoded messages

use std::net::SocketAddr;

use tokio::net::TcpStream;

mod listener;
mod session;

pub(crate) use listener::Listener;
pub(crate) use listener::ListenerMessage;
pub(crate) use session::Session;
pub(crate) use session::SessionMessage;

/// A network port
pub(crate) type NetworkPort = u16;

/// A network data stream which can either be
/// 1. unencrypted
/// 2. encrypted and the server-side of the session
/// 3. encrypted and the client-side of the session
#[derive(Debug)]
pub enum NetworkStream {
    /// Unencrypted session
    Raw {
        /// The peer's address
        peer_addr: SocketAddr,
        /// The local address
        local_addr: SocketAddr,
        /// The stream
        stream: TcpStream,
    },
    /// Encrypted as the server-side of the session
    TlsServer {
        /// The peer's address
        peer_addr: SocketAddr,
        /// The local address
        local_addr: SocketAddr,
        /// The stream
        stream: tokio_rustls::server::TlsStream<TcpStream>,
    },
    /// Encrypted as the client-side of the session
    TlsClient {
        /// The peer's address
        peer_addr: SocketAddr,
        /// The local address
        local_addr: SocketAddr,
        /// The stream
        stream: tokio_rustls::client::TlsStream<TcpStream>,
    },
}

impl NetworkStream {
    pub(crate) fn peer_addr(&self) -> SocketAddr {
        match self {
            Self::Raw { peer_addr, .. } => *peer_addr,
            Self::TlsServer { peer_addr, .. } => *peer_addr,
            Self::TlsClient { peer_addr, .. } => *peer_addr,
        }
    }

    pub(crate) fn local_addr(&self) -> SocketAddr {
        match self {
            Self::Raw { local_addr, .. } => *local_addr,
            Self::TlsServer { local_addr, .. } => *local_addr,
            Self::TlsClient { local_addr, .. } => *local_addr,
        }
    }
}

/// Incoming encryption mode
#[derive(Clone)]
pub enum IncomingEncryptionMode {
    /// Accept sockets raw, with no encryption
    Raw,
    /// Accept sockets and establish a secure connection
    Tls(tokio_rustls::TlsAcceptor),
}

impl std::fmt::Debug for IncomingEncryptionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut wip = f.debug_struct("IncomingEncryptionMode");

        match &self {
            Self::Raw => {
                _ = wip.field("mode", &"Raw");
            }
            Self::Tls(config) => {
                _ = wip.field("mode", &"Tls").field("config", &config.config());
            }
        }

        wip.finish()
    }
}
