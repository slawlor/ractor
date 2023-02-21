// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! TCP Server to accept incoming sessions

use ractor::{cast, ActorProcessingErr};
use ractor::{Actor, ActorRef};
use tokio::net::TcpListener;

use super::IncomingEncryptionMode;
use crate::node::NodeServerMessage;

/// A Tcp Socket [Listener] responsible for accepting new connections and spawning [super::session::Session]s
/// which handle the message sending and receiving over the socket.
///
/// The [Listener] supervises all of the TCP [super::session::Session] actors and is responsible for logging
/// connects and disconnects as well as tracking the current open [super::session::Session] actors.
pub struct Listener {
    port: super::NetworkPort,
    session_manager: ActorRef<crate::node::NodeServer>,
    encryption: IncomingEncryptionMode,
}

impl Listener {
    /// Create a new `Listener`
    pub fn new(
        port: super::NetworkPort,
        session_manager: ActorRef<crate::node::NodeServer>,
        encryption: IncomingEncryptionMode,
    ) -> Self {
        Self {
            port,
            session_manager,
            encryption,
        }
    }
}

/// The Node listener's state
pub struct ListenerState {
    listener: Option<TcpListener>,
}

#[derive(crate::RactorMessage)]
pub struct ListenerMessage;

#[async_trait::async_trait]
impl Actor for Listener {
    type Msg = ListenerMessage;
    type Arguments = ();
    type State = ListenerState;

    async fn pre_start(
        &self,
        myself: ActorRef<Self>,
        _: (),
    ) -> Result<Self::State, ActorProcessingErr> {
        let addr = format!("0.0.0.0:{}", self.port);
        let listener = match TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(err) => {
                return Err(From::from(err));
            }
        };

        // startup the event processing loop by sending an initial msg
        let _ = myself.cast(ListenerMessage);

        // create the initial state
        Ok(Self::State {
            listener: Some(listener),
        })
    }

    async fn post_stop(
        &self,
        _myself: ActorRef<Self>,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        // close the listener properly, in case anyone else has handles to the actor stopping
        // total droppage
        drop(state.listener.take());
        Ok(())
    }

    async fn handle(
        &self,
        myself: ActorRef<Self>,
        _message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        if let Some(listener) = &mut state.listener {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    let local = stream.local_addr()?;

                    let session = match &self.encryption {
                        IncomingEncryptionMode::Raw => Some(super::NetworkStream::Raw {
                            peer_addr: addr,
                            local_addr: local,
                            stream,
                        }),
                        IncomingEncryptionMode::Tls(acceptor) => {
                            match acceptor.accept(stream).await {
                                Ok(enc_stream) => Some(super::NetworkStream::TlsServer {
                                    peer_addr: addr,
                                    local_addr: local,
                                    stream: enc_stream,
                                }),
                                Err(some_err) => {
                                    log::warn!("Error establishing secure socket: {}", some_err);
                                    None
                                }
                            }
                        }
                    };

                    if let Some(stream) = session {
                        let _ = cast!(
                            self.session_manager,
                            NodeServerMessage::ConnectionOpened {
                                stream,
                                is_server: true
                            }
                        );
                        log::info!("TCP Session opened for {}", addr);
                    }
                }
                Err(socket_accept_error) => {
                    log::warn!(
                        "Error accepting socket {} on Node server",
                        socket_accept_error
                    );
                }
            }
        }

        // continue accepting new sockets
        let _ = myself.cast(ListenerMessage);
        Ok(())
    }
}
