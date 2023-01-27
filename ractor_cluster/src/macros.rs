// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Macro helpers for remote actors

/// `serialized_rpc_forward!` converts a traditional RPC port to a port which recieves a serialized
/// [Vec<_>] message and can rebuild the reply. This is necessary for RPCs which can occur over the network.
///
/// However when defining the serialized logic, the cost will ONLY be incurred for actors which live
/// on another `node()`, never locally. Local actors will always use the local [ractor::message::BoxedMessage]
/// notation.
///
/// An example usage is
/// ```rust
/// use ractor::concurrency::Duration;
/// use ractor::{RpcReplyPort, Message};
/// use ractor::message::SerializedMessage;
/// use ractor::message::BoxedDowncastErr;
/// use ractor_cluster::serialized_rpc_forward;
///
/// enum MessageType {
///     Cast(String),
///     Call(String, RpcReplyPort<String>),
/// }
///
/// impl Message for MessageType {
///     fn serializable() -> bool {
///         true
///     }
///
///     fn serialize(self) -> SerializedMessage {
///         match self {
///             Self::Cast(args) => SerializedMessage::Cast(args.into_bytes()),
///             Self::Call(args, reply) => {
///                 let tx = serialized_rpc_forward!(reply, |bytes| String::from_utf8(bytes).unwrap());
///                 SerializedMessage::Call(args.into_bytes(), tx.into())
///             }
///         }
///     }
///
///     fn deserialize(bytes: SerializedMessage) -> Result<Self, BoxedDowncastErr> {
///         match bytes {
///             SerializedMessage::Call(args, reply) => {
///                 let tx = serialized_rpc_forward!(reply, |str: String| str.into_bytes());
///                 Ok(Self::Call(String::from_utf8(args).unwrap(), tx))
///             }
///             SerializedMessage::Cast(args) => Ok(Self::Cast(String::from_utf8(args).unwrap())),
///             _ => Err(BoxedDowncastErr),
///         }
///     }
/// }
/// ```
#[macro_export]
macro_rules! serialized_rpc_forward {
    ($typed_port:expr, $converter:expr) => {{
        let (tx, rx) = ractor::concurrency::oneshot();
        let timeout = $typed_port.get_timeout();
        ractor::concurrency::spawn(async move {
            match $typed_port.get_timeout() {
                Some(timeout) => {
                    if let Ok(Ok(result)) = ractor::concurrency::timeout(timeout, rx).await {
                        let _ = $typed_port.send($converter(result));
                    }
                }
                None => {
                    if let Ok(result) = rx.await {
                        let _ = $typed_port.send($converter(result));
                    }
                }
            }
        });
        if let Some(to) = timeout {
            RpcReplyPort::<_>::from((tx, to))
        } else {
            RpcReplyPort::<_>::from(tx)
        }
    }};
}

#[cfg(test)]
mod tests {
    use ractor::concurrency::Duration;
    use ractor::message::BoxedDowncastErr;
    use ractor::message::SerializedMessage;
    use ractor::{Message, RpcReplyPort};

    enum MessageType {
        Cast(String),
        Call(String, RpcReplyPort<String>),
    }

    impl Message for MessageType {
        fn serializable() -> bool {
            true
        }

        fn serialize(self) -> SerializedMessage {
            match self {
                Self::Cast(args) => SerializedMessage::Cast(args.into_bytes()),
                Self::Call(args, reply) => {
                    let tx =
                        serialized_rpc_forward!(reply, |bytes| String::from_utf8(bytes).unwrap());
                    SerializedMessage::Call(args.into_bytes(), tx)
                }
            }
        }

        fn deserialize(bytes: SerializedMessage) -> Result<Self, BoxedDowncastErr> {
            match bytes {
                SerializedMessage::Call(args, reply) => {
                    let tx = serialized_rpc_forward!(reply, |str: String| str.into_bytes());
                    Ok(Self::Call(String::from_utf8(args).unwrap(), tx))
                }
                SerializedMessage::Cast(args) => Ok(Self::Cast(String::from_utf8(args).unwrap())),
                _ => Err(BoxedDowncastErr),
            }
        }
    }

    #[ractor::concurrency::test]
    async fn no_timeout_rpc() {
        let (tx, rx) = ractor::concurrency::oneshot();
        let no_timeout = MessageType::Call("test".to_string(), tx.into());
        let no_timeout_serialized = no_timeout.serialize();
        match no_timeout_serialized {
            SerializedMessage::Call(args, reply) => {
                let _ = reply.send(args);
            }
            _ => panic!("Invalid"),
        }

        let no_timeout_reply = rx.await.expect("Receive error");
        assert_eq!(no_timeout_reply, "test".to_string());
    }

    #[ractor::concurrency::test]
    async fn with_timeout_rpc() {
        let (tx, rx) = ractor::concurrency::oneshot();
        let duration = Duration::from_millis(10);
        let with_timeout = MessageType::Call("test".to_string(), (tx, duration).into());

        let with_timeout_serialized = with_timeout.serialize();
        match with_timeout_serialized {
            SerializedMessage::Call(args, reply) => {
                let _ = reply.send(args);
            }
            _ => panic!("Invalid"),
        }

        let with_timeout_reply = rx.await.expect("Receive error");
        assert_eq!(with_timeout_reply, "test".to_string());
    }

    #[ractor::concurrency::test]
    async fn timeout_rpc() {
        let (tx, rx) = ractor::concurrency::oneshot();
        let duration = Duration::from_millis(10);
        let with_timeout = MessageType::Call("test".to_string(), (tx, duration).into());

        let with_timeout_serialized = with_timeout.serialize();
        match with_timeout_serialized {
            SerializedMessage::Call(args, reply) => {
                ractor::concurrency::sleep(Duration::from_millis(50)).await;
                let _ = reply.send(args);
            }
            _ => panic!("Invalid"),
        }

        let result = rx.await;
        assert!(matches!(result, Err(_)));
    }

    #[ractor::concurrency::test]
    async fn no_timeout_rpc_decoded_reply() {
        let (tx, rx) = ractor::concurrency::oneshot();

        let no_timeout = MessageType::Call("test".to_string(), tx.into());
        let no_timeout_serialized = no_timeout.serialize();
        let no_timeout_deserialized =
            MessageType::deserialize(no_timeout_serialized).expect("Failed to deserialize port");
        if let MessageType::Call(args, reply) = no_timeout_deserialized {
            let _ = reply.send(args);
        } else {
            panic!("Failed to decode with `MessageType`");
        }

        let no_timeout_reply = rx.await.expect("Receive error");
        assert_eq!(no_timeout_reply, "test".to_string());
    }

    #[ractor::concurrency::test]
    async fn with_timeout_rpc_decoded_reply() {
        let (tx, rx) = ractor::concurrency::oneshot();

        let initial_call =
            MessageType::Call("test".to_string(), (tx, Duration::from_millis(50)).into());
        let serialized_call = initial_call.serialize();
        let deserialized_call =
            MessageType::deserialize(serialized_call).expect("Failed to deserialize port");
        if let MessageType::Call(args, reply) = deserialized_call {
            let _ = reply.send(args);
        } else {
            panic!("Failed to decode with `MessageType`");
        }

        let message_reply = rx.await.expect("Receive error");
        assert_eq!(message_reply, "test".to_string());
    }

    #[ractor::concurrency::test]
    async fn with_timeout_rpc_decoded_timeout() {
        let (tx, rx) = ractor::concurrency::oneshot();

        let initial_call =
            MessageType::Call("test".to_string(), (tx, Duration::from_millis(50)).into());
        let serialized_call = initial_call.serialize();
        let deserialized_call =
            MessageType::deserialize(serialized_call).expect("Failed to deserialize port");
        if let MessageType::Call(args, reply) = deserialized_call {
            ractor::concurrency::sleep(Duration::from_millis(100)).await;
            let _ = reply.send(args);
        } else {
            panic!("Failed to decode with `MessageType`");
        }

        assert!(rx.await.is_err());
    }
}
