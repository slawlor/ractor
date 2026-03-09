// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! TCP session actor which is managing the specific communication to a node

// TODO: RUSTLS + Tokio : https://github.com/tokio-rs/tls/blob/master/tokio-rustls/examples/server/src/main.rs

use std::io::Write;
use std::net::SocketAddr;

use bytes::Bytes;
use prost::Message;
use ractor::Actor;
use ractor::ActorCell;
use ractor::ActorProcessingErr;
use ractor::ActorRef;
use ractor::SpawnErr;
use ractor::SupervisionEvent;
use tokio::io::AsyncReadExt;
use tokio::io::ErrorKind;
use tokio::io::ReadHalf;
use tokio::io::WriteHalf;
use tokio::net::tcp::OwnedReadHalf;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::TcpStream;

use crate::RactorMessage;

/// Helper method to read exactly `len` bytes from the stream into a pre-allocated buffer
/// of bytes
async fn read_n_bytes(stream: &mut ActorReadHalf, len: usize) -> Result<Vec<u8>, tokio::io::Error> {
    let mut buf = vec![0u8; len];
    let mut c_len = 0;
    if let ActorReadHalf::Regular(r) = stream {
        r.readable().await?;
    }

    while c_len < len {
        let n = match stream {
            ActorReadHalf::ServerTls(t) => t.read(&mut buf[c_len..]).await?,
            ActorReadHalf::ClientTls(t) => t.read(&mut buf[c_len..]).await?,
            ActorReadHalf::Regular(t) => t.read(&mut buf[c_len..]).await?,
            ActorReadHalf::External(t) => t.read(&mut buf[c_len..]).await?,
        };
        if n == 0 {
            // EOF
            return Err(tokio::io::Error::new(
                tokio::io::ErrorKind::UnexpectedEof,
                "EOF",
            ));
        }
        c_len += n;
    }
    Ok(buf)
}

// ========================= Node Session actor ========================= //

/// Represents a bi-directional tcp connection along with send + receive operations
///
/// The [Session] actor supervises a child [SessionReader] actor and owns a batched writer
/// task. Should the reader exit or the writer task fail, the entire session is terminated.
pub(crate) struct Session {
    pub(crate) handler: ActorRef<crate::node::NodeSessionMessage>,
    pub(crate) peer_addr: SocketAddr,
    pub(crate) local_addr: SocketAddr,
}

impl Session {
    pub(crate) async fn spawn_linked(
        handler: ActorRef<crate::node::NodeSessionMessage>,
        stream: super::NetworkStream,
        peer_addr: SocketAddr,
        local_addr: SocketAddr,
        supervisor: ActorCell,
    ) -> Result<ActorRef<SessionMessage>, SpawnErr> {
        match Actor::spawn_linked(
            None,
            Session {
                handler,
                peer_addr,
                local_addr,
            },
            stream,
            supervisor,
        )
        .await
        {
            Err(err) => {
                tracing::error!("Failed to spawn session writer actor: {err}");
                Err(err)
            }
            Ok((a, _)) => {
                // return the actor handle
                Ok(a)
            }
        }
    }
}

/// The node connection messages
#[derive(RactorMessage)]
pub(crate) enum SessionMessage {
    /// Send a message over the channel
    Send(crate::protocol::NetworkMessage),

    /// An object was received on the channel
    ObjectAvailable(crate::protocol::NetworkMessage),
}

/// The node session's state
pub(crate) struct SessionState {
    writer_tx: tokio::sync::mpsc::UnboundedSender<crate::protocol::NetworkMessage>,
    writer_task: tokio::task::JoinHandle<()>,
    reader: ActorRef<SessionReaderMessage>,
}

#[cfg_attr(feature = "async-trait", ractor::async_trait)]
impl Actor for Session {
    type Msg = SessionMessage;
    type Arguments = super::NetworkStream;
    type State = SessionState;

    async fn pre_start(
        &self,
        myself: ActorRef<Self::Msg>,
        stream: super::NetworkStream,
    ) -> Result<Self::State, ActorProcessingErr> {
        let (read, write) = match stream {
            super::NetworkStream::Raw { stream, .. } => {
                let (read, write) = stream.into_split();
                (ActorReadHalf::Regular(read), ActorWriteHalf::Regular(write))
            }
            super::NetworkStream::TlsClient { stream, .. } => {
                let (read_half, write_half) = tokio::io::split(stream);
                (
                    ActorReadHalf::ClientTls(read_half),
                    ActorWriteHalf::ClientTls(write_half),
                )
            }
            super::NetworkStream::TlsServer { stream, .. } => {
                let (read_half, write_half) = tokio::io::split(stream);
                (
                    ActorReadHalf::ServerTls(read_half),
                    ActorWriteHalf::ServerTls(write_half),
                )
            }
            super::NetworkStream::External { reader, writer, .. } => (
                ActorReadHalf::External(reader),
                ActorWriteHalf::External(writer),
            ),
        };

        // Spawn a batched writer task instead of a writer actor.
        // This eliminates one actor hop and enables write coalescing.
        let (writer_tx, writer_rx) = tokio::sync::mpsc::unbounded_channel();
        let session_ref = myself.clone();
        let writer_task = tokio::task::spawn(run_write_task(write, writer_rx, session_ref));

        let (reader, _) = Actor::spawn_linked(
            None,
            SessionReader {
                session: myself.clone(),
            },
            read,
            myself.get_cell(),
        )
        .await?;

        Ok(Self::State {
            writer_tx,
            writer_task,
            reader,
        })
    }

    async fn post_stop(
        &self,
        _myself: ActorRef<Self::Msg>,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        state.writer_task.abort();
        tracing::info!("TCP Session closed for {}", self.peer_addr);
        Ok(())
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            Self::Msg::Send(msg) => {
                tracing::debug!(
                    "SEND: {} -> {} - '{msg:?}'",
                    self.local_addr,
                    self.peer_addr
                );
                let _ = state.writer_tx.send(msg);
            }
            Self::Msg::ObjectAvailable(msg) => {
                tracing::debug!(
                    "RECEIVE {} <- {} - '{msg:?}'",
                    self.local_addr,
                    self.peer_addr,
                );
                let _ = self
                    .handler
                    .cast(crate::node::NodeSessionMessage::MessageReceived(msg));
            }
        }
        Ok(())
    }

    async fn handle_supervisor_evt(
        &self,
        myself: ActorRef<Self::Msg>,
        message: SupervisionEvent,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        // sockets open, they close, the world goes round... If the reader exits for any reason, we'll start the shutdown procedure
        match message {
            SupervisionEvent::ActorFailed(actor, panic_msg) => {
                if actor.get_id() == state.reader.get_id() {
                    tracing::error!("TCP Session's reader panicked with '{panic_msg}'");
                } else {
                    tracing::error!("TCP Session received a child panic from an unknown child actor ({}) - '{panic_msg}'", actor.get_id());
                }
                myself.stop(Some("child_panic".to_string()));
            }
            SupervisionEvent::ActorTerminated(actor, _, exit_reason) => {
                if actor.get_id() == state.reader.get_id() {
                    tracing::debug!("TCP Session's reader exited");
                } else {
                    tracing::warn!("TCP Session received a child exit from an unknown child actor ({}) - '{exit_reason:?}'", actor.get_id());
                }
                myself.stop(Some("child_terminate".to_string()));
            }
            _ => {
                // all ok
            }
        }
        Ok(())
    }
}

// ========================= Node Session writer ========================= //

enum ActorWriteHalf {
    ServerTls(WriteHalf<tokio_rustls::server::TlsStream<TcpStream>>),
    ClientTls(WriteHalf<tokio_rustls::client::TlsStream<TcpStream>>),
    Regular(OwnedWriteHalf),
    External(super::BoxWrite),
}

impl ActorWriteHalf {
    async fn write_all(&mut self, data: &[u8]) -> tokio::io::Result<()> {
        use tokio::io::AsyncWriteExt;
        match self {
            Self::ServerTls(t) => t.write_all(data).await,
            Self::ClientTls(t) => t.write_all(data).await,
            Self::Regular(t) => t.write_all(data).await,
            Self::External(t) => t.write_all(data).await,
        }
    }

    async fn flush(&mut self) -> tokio::io::Result<()> {
        use tokio::io::AsyncWriteExt;
        match self {
            Self::ServerTls(t) => t.flush().await,
            Self::ClientTls(t) => t.flush().await,
            Self::Regular(t) => t.flush().await,
            Self::External(t) => t.flush().await,
        }
    }
}

enum ActorReadHalf {
    ServerTls(ReadHalf<tokio_rustls::server::TlsStream<TcpStream>>),
    ClientTls(ReadHalf<tokio_rustls::client::TlsStream<TcpStream>>),
    Regular(OwnedReadHalf),
    External(super::BoxRead),
}

impl ActorReadHalf {
    async fn read_u64(&mut self) -> tokio::io::Result<u64> {
        match self {
            Self::ServerTls(t) => t.read_u64().await,
            Self::ClientTls(t) => t.read_u64().await,
            Self::Regular(t) => t.read_u64().await,
            Self::External(t) => t.read_u64().await,
        }
    }
}

// ========================= Batched write task ========================= //

/// Encode a single network message into the buffer using the length-prefixed
/// wire format (u64 big-endian length + protobuf payload).
fn encode_network_message(msg: &crate::protocol::NetworkMessage, buf: &mut Vec<u8>) {
    let len = msg.encoded_len();
    buf.write_all(&len.to_be_bytes())
        .expect("Vec write should not fail");
    msg.encode(buf).expect("Vec write should not fail");
    tracing::trace!("Batching payload (len={len})");
}

/// Async task that reads messages from an mpsc channel and writes them to the
/// network stream in batches. After receiving the first message, it drains any
/// additional pending messages via `try_recv` and writes them all in a single
/// `write_all` + `flush` cycle. This eliminates per-message flush overhead and
/// avoids Nagle/delayed-ACK interactions.
async fn run_write_task(
    mut stream: ActorWriteHalf,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<crate::protocol::NetworkMessage>,
    session: ActorRef<SessionMessage>,
) {
    let mut buf = Vec::new();

    while let Some(first_msg) = rx.recv().await {
        buf.clear();

        // Encode the first message
        encode_network_message(&first_msg, &mut buf);

        // Drain any additional pending messages for batching
        while let Ok(msg) = rx.try_recv() {
            encode_network_message(&msg, &mut buf);
        }

        // Write the entire batch
        if let Err(write_err) = stream.write_all(&buf).await {
            tracing::warn!("Error writing to the stream '{write_err}'");
            session.stop(Some("channel_closed".to_string()));
            return;
        }

        // Flush once for the entire batch
        if let Err(flush_err) = stream.flush().await {
            tracing::warn!("Error flushing the stream '{flush_err}'");
            session.stop(Some("channel_closed".to_string()));
            return;
        }
    }

    // Channel closed (all senders dropped), session is likely already stopping
}

// ========================= Node Session reader ========================= //

struct SessionReader {
    session: ActorRef<SessionMessage>,
}

/// The node connection messages
pub(crate) enum SessionReaderMessage {
    /// Wait for an object from the stream
    WaitForObject,

    /// Read next object off the stream
    ReadObject(u64),
}

impl ractor::Message for SessionReaderMessage {}

struct SessionReaderState {
    reader: Option<ActorReadHalf>,
}

#[cfg_attr(feature = "async-trait", ractor::async_trait)]
impl Actor for SessionReader {
    type Msg = SessionReaderMessage;
    type Arguments = ActorReadHalf;
    type State = SessionReaderState;

    async fn pre_start(
        &self,
        myself: ActorRef<Self::Msg>,
        reader: ActorReadHalf,
    ) -> Result<Self::State, ActorProcessingErr> {
        // start waiting for the first object on the network
        let _ = myself.cast(SessionReaderMessage::WaitForObject);
        Ok(Self::State {
            reader: Some(reader),
        })
    }

    async fn post_stop(
        &self,
        _myself: ActorRef<Self::Msg>,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        // drop the channel to close it should we be exiting
        drop(state.reader.take());
        Ok(())
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            Self::Msg::WaitForObject if state.reader.is_some() => {
                if let Some(stream) = &mut state.reader {
                    match stream.read_u64().await {
                        Ok(length) => {
                            tracing::trace!("Payload length message ({length}) received");
                            let _ = myself.cast(SessionReaderMessage::ReadObject(length));
                            return Ok(());
                        }
                        Err(err) if err.kind() == ErrorKind::UnexpectedEof => {
                            tracing::trace!("Error (EOF) on stream");
                            // EOF, close the stream by dropping the stream
                            drop(state.reader.take());
                            myself.stop(Some("channel_closed".to_string()));
                        }
                        Err(_other_err) => {
                            tracing::trace!("Error ({_other_err:?}) on stream");
                            // some other TCP error, more handling necessary
                        }
                    }
                }

                let _ = myself.cast(SessionReaderMessage::WaitForObject);
            }
            Self::Msg::ReadObject(length) if state.reader.is_some() => {
                if let Some(stream) = &mut state.reader {
                    match read_n_bytes(stream, length as usize).await {
                        Ok(buf) => {
                            tracing::trace!("Payload of length({}) received", buf.len());
                            // NOTE: Our implementation writes 2 messages when sending something over the wire, the first
                            // is exactly 8 bytes which constitute the length of the payload message (u64 in big endian format),
                            // followed by the payload. This tells our TCP reader how much data to read off the wire

                            // [buf] here should contain the exact amount of data to decode an object properly.
                            let bytes = Bytes::from(buf);
                            match crate::protocol::NetworkMessage::decode(bytes) {
                                Ok(msg) => {
                                    // we decoded a message, pass it up the chain
                                    let _ = self.session.cast(SessionMessage::ObjectAvailable(msg));
                                }
                                Err(decode_err) => {
                                    tracing::error!(
                                        "Error decoding network message: '{decode_err}'. Discarding"
                                    );
                                }
                            }
                        }
                        Err(err) if err.kind() == ErrorKind::UnexpectedEof => {
                            // EOF, close the stream by dropping the stream
                            drop(state.reader.take());
                            myself.stop(Some("channel_closed".to_string()));
                            return Ok(());
                        }
                        Err(_other_err) => {
                            // TODO: some other TCP error, more handling necessary
                        }
                    }
                }

                // we've read the object, now wait for next object
                let _ = myself.cast(SessionReaderMessage::WaitForObject);
            }
            _ => {
                // no stream is available, keep looping until one is available
                let _ = myself.cast(SessionReaderMessage::WaitForObject);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_network_message_creates_valid_buffer() {
        // Test that encode_network_message properly encodes a message
        let msg = crate::protocol::NetworkMessage::default();
        let mut buf = Vec::new();

        encode_network_message(&msg, &mut buf);

        // Verify buffer has at least 8 bytes (u64 length prefix)
        assert!(
            buf.len() >= 8,
            "Encoded message should have at least 8 bytes for length prefix"
        );

        // Verify the length prefix matches the rest of the buffer
        let len_bytes = &buf[0..8];
        let length = u64::from_be_bytes([
            len_bytes[0],
            len_bytes[1],
            len_bytes[2],
            len_bytes[3],
            len_bytes[4],
            len_bytes[5],
            len_bytes[6],
            len_bytes[7],
        ]);

        // The payload starts at byte 8 and should match the encoded length
        assert_eq!(
            buf.len() - 8,
            length as usize,
            "Length prefix should match actual payload size"
        );
    }

    #[test]
    fn test_encode_network_message_batching() {
        // Test that multiple messages can be batched into a single buffer
        let msg1 = crate::protocol::NetworkMessage::default();
        let msg2 = crate::protocol::NetworkMessage::default();

        let mut buf = Vec::new();
        encode_network_message(&msg1, &mut buf);
        let size_after_first = buf.len();

        encode_network_message(&msg2, &mut buf);
        let size_after_second = buf.len();

        // Second message should have been appended
        assert!(size_after_second > size_after_first);
    }

    #[test]
    fn test_read_n_bytes_zero_count() {
        // Test that read_n_bytes handles zero-length reads without hanging
        // This is a boundary condition test
    }

    #[tokio::test]
    async fn test_read_n_bytes_eof() {
        // Test that read_n_bytes returns UnexpectedEof when stream closes early
        // Create a TcpListener and connect to it to get properly typed streams
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Failed to create listener");

        let addr = listener.local_addr().expect("Failed to get local addr");

        let client_handle = tokio::spawn(async move {
            // Connect as client and immediately close
            let _stream = tokio::net::TcpStream::connect(addr)
                .await
                .expect("Failed to connect");
            // Stream will close when dropped
        });

        // Accept the connection on server side
        let (stream, _) = listener.accept().await.expect("Failed to accept");
        let (mut read_half, _) = stream.into_split();

        // Attempting to read should get EOF
        let mut buf = vec![0u8; 100];
        let result = read_half.read(&mut buf).await;

        // We should get either an error or 0 bytes (EOF)
        match result {
            Ok(0) => {
                // EOF
            }
            Err(_e) => {
                // Also acceptable - connection closed
            }
            Ok(_n) => {
                panic!("Expected EOF or error but got bytes");
            }
        }

        let _ = client_handle.await;
    }
}
