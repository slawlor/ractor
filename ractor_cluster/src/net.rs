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
    /// External transport injected by the user
    ///
    /// This variant enables custom transports to be used while preserving
    /// the existing TCP/TLS paths. The provided reader/writer must be a
    /// connected, bidirectional byte stream using the same framing
    /// (u64/usize big-endian length prefix + prost payload).
    External {
        /// Optional label for the peer (used for diagnostics)
        peer_label: Option<String>,
        /// Optional label for the local endpoint (used for diagnostics)
        local_label: Option<String>,
        /// Read half
        reader: BoxRead,
        /// Write half
        writer: BoxWrite,
    },
}

impl NetworkStream {
    pub(crate) fn peer_addr(&self) -> SocketAddr {
        match self {
            Self::Raw { peer_addr, .. } => *peer_addr,
            Self::TlsServer { peer_addr, .. } => *peer_addr,
            Self::TlsClient { peer_addr, .. } => *peer_addr,
            // External transports may not have a real socket address; return unspecified
            Self::External { .. } => SocketAddr::from(([0, 0, 0, 0], 0)),
        }
    }

    pub(crate) fn local_addr(&self) -> SocketAddr {
        match self {
            Self::Raw { local_addr, .. } => *local_addr,
            Self::TlsServer { local_addr, .. } => *local_addr,
            Self::TlsClient { local_addr, .. } => *local_addr,
            // External transports may not have a real socket address; return unspecified
            Self::External { .. } => SocketAddr::from(([0, 0, 0, 0], 0)),
        }
    }
}

impl std::fmt::Debug for NetworkStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut wip = f.debug_struct("NetworkStream");
        match self {
            Self::Raw {
                peer_addr,
                local_addr,
                ..
            } => {
                _ = wip
                    .field("mode", &"Raw")
                    .field("peer_addr", peer_addr)
                    .field("local_addr", local_addr);
            }
            Self::TlsServer {
                peer_addr,
                local_addr,
                ..
            } => {
                _ = wip
                    .field("mode", &"TlsServer")
                    .field("peer_addr", peer_addr)
                    .field("local_addr", local_addr);
            }
            Self::TlsClient {
                peer_addr,
                local_addr,
                ..
            } => {
                _ = wip
                    .field("mode", &"TlsClient")
                    .field("peer_addr", peer_addr)
                    .field("local_addr", local_addr);
            }
            Self::External {
                peer_label,
                local_label,
                ..
            } => {
                _ = wip
                    .field("mode", &"External")
                    .field("peer_label", peer_label)
                    .field("local_label", local_label);
            }
        }
        wip.finish()
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

// ========== Transport Trait and Type Aliases ==========

/// Boxed async read half for external transports
pub type BoxRead = Box<dyn tokio::io::AsyncRead + Unpin + Send + 'static>;
/// Boxed async write half for external transports
pub type BoxWrite = Box<dyn tokio::io::AsyncWrite + Unpin + Send + 'static>;

/// A generic connected, bidirectional byte stream for cluster traffic.
///
/// Implementors must provide a split into read/write halves. Optional
/// labels allow nicer diagnostics when no SocketAddr is available.
pub trait ClusterBidiStream: Send + 'static {
    /// Split the connected stream into read/write halves.
    fn split(self: Box<Self>) -> (BoxRead, BoxWrite);

    /// Optional peer label for diagnostics/logging
    fn peer_label(&self) -> Option<String> {
        None
    }

    /// Optional local label for diagnostics/logging
    fn local_label(&self) -> Option<String> {
        None
    }
}
