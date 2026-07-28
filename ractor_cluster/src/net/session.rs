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
    let mut buf = Vec::new();
    buf.try_reserve_exact(len).map_err(|reserve_err| {
        tokio::io::Error::new(
            ErrorKind::InvalidData,
            format!("cluster frame length {len} could not be allocated: {reserve_err}"),
        )
    })?;
    buf.resize(len, 0);
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
    max_inbound_frame_size: u64,
}

impl Session {
    pub(crate) async fn spawn_linked(
        handler: ActorRef<crate::node::NodeSessionMessage>,
        stream: super::NetworkStream,
        peer_addr: SocketAddr,
        local_addr: SocketAddr,
        max_inbound_frame_size: u64,
        supervisor: ActorCell,
    ) -> Result<ActorRef<SessionMessage>, SpawnErr> {
        match Actor::spawn_linked(
            None,
            Session {
                handler,
                peer_addr,
                local_addr,
                max_inbound_frame_size,
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
                max_inbound_frame_size: self.max_inbound_frame_size,
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
    let len = u64::try_from(msg.encoded_len()).expect("encoded message length exceeds u64");
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
    max_inbound_frame_size: u64,
}

/// The node connection messages
pub(crate) enum SessionReaderMessage {
    /// Wait for an object from the stream
    WaitForObject,
}

impl ractor::Message for SessionReaderMessage {}

fn checked_frame_length(length: u64, max_frame_size: u64) -> tokio::io::Result<usize> {
    if length > max_frame_size {
        return Err(tokio::io::Error::new(
            ErrorKind::InvalidData,
            format!("cluster frame length {length} exceeds configured limit {max_frame_size}"),
        ));
    }

    usize::try_from(length).map_err(|_| {
        tokio::io::Error::new(
            ErrorKind::InvalidData,
            format!("cluster frame length {length} cannot fit in memory on this platform"),
        )
    })
}

async fn read_network_message(
    stream: &mut ActorReadHalf,
    max_frame_size: u64,
) -> tokio::io::Result<crate::protocol::NetworkMessage> {
    let wire_length = stream.read_u64().await?;
    tracing::trace!("Payload length message ({wire_length}) received");

    let frame_length = checked_frame_length(wire_length, max_frame_size)?;
    let bytes = Bytes::from(read_n_bytes(stream, frame_length).await?);
    tracing::trace!("Payload of length({}) received", bytes.len());

    crate::protocol::NetworkMessage::decode(bytes).map_err(|decode_err| {
        tokio::io::Error::new(
            ErrorKind::InvalidData,
            format!("invalid cluster protobuf frame: {decode_err}"),
        )
    })
}

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
            Self::Msg::WaitForObject => {
                let read_result = match state.reader.as_mut() {
                    Some(stream) => read_network_message(stream, self.max_inbound_frame_size).await,
                    None => {
                        myself.stop(Some("channel_closed".to_string()));
                        return Ok(());
                    }
                };

                match read_result {
                    Ok(msg) => {
                        let _ = self.session.cast(SessionMessage::ObjectAvailable(msg));
                        let _ = myself.cast(SessionReaderMessage::WaitForObject);
                    }
                    Err(read_err) => {
                        let stop_reason = if read_err.kind() == ErrorKind::UnexpectedEof {
                            tracing::trace!("Cluster stream closed while reading a frame");
                            "channel_closed"
                        } else {
                            tracing::warn!(
                                "Closing cluster stream after framing error: {read_err}"
                            );
                            "frame_read_error"
                        };
                        drop(state.reader.take());
                        myself.stop(Some(stop_reason.to_string()));
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::mem::size_of;
    use std::pin::Pin;
    use std::task::Context;
    use std::task::Poll;

    use super::*;
    use tokio::io::AsyncRead;
    use tokio::io::ReadBuf;

    fn test_network_message() -> crate::protocol::NetworkMessage {
        crate::protocol::NetworkMessage {
            message: Some(crate::protocol::meta::network_message::Message::Node(
                crate::protocol::node::NodeMessage {
                    msg: Some(crate::protocol::node::node_message::Msg::Cast(
                        crate::protocol::node::Cast {
                            to: 42,
                            what: vec![1, 2, 3, 4],
                            variant: "test".to_string(),
                            metadata: None,
                        },
                    )),
                },
            )),
        }
    }

    fn encode_frame(msg: &crate::protocol::NetworkMessage) -> Vec<u8> {
        let payload = msg.encode_to_vec();
        let payload_len = u64::try_from(payload.len()).expect("test payload should fit in u64");
        let mut frame = Vec::with_capacity(size_of::<u64>() + payload.len());
        frame.extend_from_slice(&payload_len.to_be_bytes());
        frame.extend_from_slice(&payload);
        frame
    }

    #[test]
    fn encode_network_message_uses_portable_u64_prefix() {
        let msg = test_network_message();
        let mut buf = Vec::new();

        encode_network_message(&msg, &mut buf);

        let encoded_len = msg.encoded_len();
        let wire_len = u64::try_from(encoded_len).expect("test payload should fit in u64");
        assert_eq!(&buf[..size_of::<u64>()], &wire_len.to_be_bytes());
        assert_eq!(buf.len(), size_of::<u64>() + encoded_len);
    }

    #[test]
    fn encode_network_message_appends_complete_frames() {
        let msg1 = test_network_message();
        let msg2 = test_network_message();

        let mut buf = Vec::new();
        encode_network_message(&msg1, &mut buf);
        let size_after_first = buf.len();

        encode_network_message(&msg2, &mut buf);
        assert_eq!(buf.len(), size_after_first * 2);
    }

    #[tokio::test]
    async fn frame_at_configured_limit_is_accepted() {
        let expected = test_network_message();
        let payload_len =
            u64::try_from(expected.encoded_len()).expect("test payload should fit in u64");
        let frame = encode_frame(&expected);
        let mut reader = ActorReadHalf::External(Box::new(Cursor::new(frame)));

        let actual = read_network_message(&mut reader, payload_len)
            .await
            .expect("frame at configured limit should be accepted");

        assert_eq!(actual, expected);
    }

    struct HeaderOnlyReader {
        header: [u8; size_of::<u64>()],
        offset: usize,
    }

    impl AsyncRead for HeaderOnlyReader {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<tokio::io::Result<()>> {
            if self.offset == self.header.len() {
                panic!("oversized frame payload must not be read");
            }

            let count = buf
                .remaining()
                .min(self.header.len().saturating_sub(self.offset));
            let end = self.offset + count;
            buf.put_slice(&self.header[self.offset..end]);
            self.offset = end;
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn oversized_frame_is_rejected_before_payload_read() {
        let max_frame_size = 4_u64;
        let mut reader = ActorReadHalf::External(Box::new(HeaderOnlyReader {
            header: (max_frame_size + 1).to_be_bytes(),
            offset: 0,
        }));

        let error = read_network_message(&mut reader, max_frame_size)
            .await
            .expect_err("oversized frame should be rejected");

        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(error.to_string().contains("exceeds configured limit"));
    }

    #[tokio::test]
    async fn unallocatable_frame_is_rejected_before_payload_read() {
        let unallocatable_length = u64::try_from(isize::MAX)
            .expect("isize should fit in the wire length")
            .checked_add(1)
            .expect("supported platforms have an isize narrower than u64");
        let mut reader = ActorReadHalf::External(Box::new(HeaderOnlyReader {
            header: unallocatable_length.to_be_bytes(),
            offset: 0,
        }));

        let error = read_network_message(&mut reader, u64::MAX)
            .await
            .expect_err("an unallocatable frame should be rejected");

        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(error.to_string().contains("could not be allocated"));
    }

    #[tokio::test]
    async fn truncated_frame_is_rejected() {
        let message = test_network_message();
        let payload_len = message.encoded_len();
        let mut frame = encode_frame(&message);
        frame.pop();
        let mut reader = ActorReadHalf::External(Box::new(Cursor::new(frame)));

        let error = read_network_message(
            &mut reader,
            u64::try_from(payload_len).expect("test payload should fit in u64"),
        )
        .await
        .expect_err("truncated frame should be rejected");

        assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
    }

    struct FrameSink;

    #[cfg_attr(feature = "async-trait", ractor::async_trait)]
    impl Actor for FrameSink {
        type Msg = SessionMessage;
        type State = ();
        type Arguments = ();

        async fn pre_start(
            &self,
            _myself: ActorRef<Self::Msg>,
            _args: Self::Arguments,
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
    }

    async fn assert_reader_stops(reader: ActorReadHalf, max_inbound_frame_size: u64) {
        let (sink, sink_handle) = Actor::spawn(None, FrameSink, ())
            .await
            .expect("frame sink should start");
        let (_reader, reader_handle) = Actor::spawn(
            None,
            SessionReader {
                session: sink.clone(),
                max_inbound_frame_size,
            },
            reader,
        )
        .await
        .expect("session reader should start");

        tokio::time::timeout(std::time::Duration::from_secs(1), reader_handle)
            .await
            .expect("framing error should stop the reader")
            .expect("session reader task should exit cleanly");

        sink.stop(None);
        sink_handle.await.expect("frame sink should stop cleanly");
    }

    #[ractor::concurrency::test]
    async fn reader_stops_after_invalid_protobuf_frame() {
        let invalid_frame = vec![0, 0, 0, 0, 0, 0, 0, 1, 0xff];
        let reader = ActorReadHalf::External(Box::new(Cursor::new(invalid_frame)));
        assert_reader_stops(reader, 1).await;
    }

    struct ErrorAfterHeaderReader {
        header: [u8; size_of::<u64>()],
        offset: usize,
    }

    impl AsyncRead for ErrorAfterHeaderReader {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<tokio::io::Result<()>> {
            if self.offset == self.header.len() {
                return Poll::Ready(Err(tokio::io::Error::new(
                    ErrorKind::ConnectionReset,
                    "injected read failure",
                )));
            }

            let count = buf
                .remaining()
                .min(self.header.len().saturating_sub(self.offset));
            let end = self.offset + count;
            buf.put_slice(&self.header[self.offset..end]);
            self.offset = end;
            Poll::Ready(Ok(()))
        }
    }

    #[ractor::concurrency::test]
    async fn reader_stops_after_nonrecoverable_io_error() {
        let reader = ActorReadHalf::External(Box::new(ErrorAfterHeaderReader {
            header: 1_u64.to_be_bytes(),
            offset: 0,
        }));
        assert_reader_stops(reader, 1).await;
    }
}
